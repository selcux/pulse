# Health Data Systems — Comprehensive Documentation
**Location:** ~/health-data-systems.md  
**Created:** 2026-03-01  
**Purpose:** Reference for unified health data implementation

---

## Current Systems Overview

### 1. Garmin Health Analysis (Skill)
**Path:** ~/.openclaw/workspace/skills/garmin-health-analysis/  
**Type:** OpenClaw skill (external, don't modify)

**Scripts:**
• garmin_data.py — sleep, hr, hrv, stress, steps, body_battery
• garmin_auth.py — OAuth token management
• garmin_chart.py — dashboard generation

**Data flow:**
• Live API calls to Garmin Connect
• No local storage (fetches fresh each time)
• Tokens cached in ~/.clawdbot/garmin/

**Wrapper:**
• ~/.openclaw/bin/garmin — routes commands to venv

---

### 2. Vitality Score
**Path:** ~/.openclaw/bin/vitality  
**Type:** Custom Python script

**Purpose:** WHOOP-inspired composite health score

**Data sources:**
• Garmin sleep (30% weight)
• Recovery: HRV + RHR + Body Battery (25%)
• Activity: steps (25%)
• Stress (20%)

**Storage:**
• Cache only: ~/.openclaw/vitality-cache.db
• 60 minute TTL
• Config: ~/.openclaw/vitality-config.json

**Schema (cache table):**
```sql
CREATE TABLE cache (
    key TEXT PRIMARY KEY,
    data TEXT NOT NULL,
    cached_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

---

### 3. Strength Training Pipeline
**Path:** ~/.openclaw/bin/strength-check  
**Type:** Custom Python script

**Purpose:** Extract detailed strength data from Intervals.icu

**Data flow:**
Garmin Watch → Garmin Connect → Intervals.icu → strength-check → SQLite

**Database:** ~/health-data/strength.db

**Tables:**

```sql
-- Workouts table
CREATE TABLE workouts (
    id TEXT PRIMARY KEY,
    start_time TIMESTAMP,
    duration_seconds INTEGER,
    activity_type TEXT,
    calories INTEGER,
    avg_hr INTEGER,
    max_hr INTEGER,
    processed BOOLEAN DEFAULT FALSE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Exercise sets table
CREATE TABLE exercise_sets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workout_id TEXT REFERENCES workouts(id),
    set_order INTEGER,
    exercise_category TEXT,
    exercise_name TEXT,
    repetitions INTEGER,
    weight_kg REAL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Sync state table
CREATE TABLE sync_state (
    key TEXT PRIMARY KEY,
    value TEXT,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

**FIT archive:** ~/health-data/fit-archive/

---

### 4. Cron Jobs (Daily Rhythms)
**Config:** ~/.openclaw/cron/jobs.json

**Morning Briefing (09:00)**
• Fetches: sleep, vitality, weather
• Writes to: Journal/YYYY-MM-DD.md

**Evening Sync (17:00)**
• Runs: strength-check
• Prepares: Sync template

**Nightly Debrief (22:00)**
• Runs: strength-check, vitality
• Updates: Debrief section

**Weekly Review (Sat 15:00)**
• Behavioral analysis
• Progress review

---

## Authentication

### Garmin Connect
**Tokens:** ~/.clawdbot/garmin/
**Auth script:** ~/.openclaw/bin/garmin auth login
**Library:** python-garminconnect (in venv)

**Venv location:** ~/.openclaw/venvs/garmin/
**Packages:** garminconnect, fitparse, gpxpy

### Intervals.icu
**API key:** In ~/.env.sh as INTERVALS_API_KEY
**Endpoint:** https://intervals.icu/api/v1
**Auth:** HTTP Basic Auth (API_KEY:key)

---

## Data Locations Summary

| Data Type | Location | Persistence |
|-----------|----------|-------------|
| Garmin tokens | ~/.clawdbot/garmin/ | Permanent |
| Vitality cache | ~/.openclaw/vitality-cache.db | 60 min TTL |
| Vitality config | ~/.openclaw/vitality-config.json | Permanent |
| Strength DB | ~/health-data/strength.db | Permanent |
| FIT files | ~/health-data/fit-archive/ | Permanent |
| Daily journals | ~/shared-brain/Journal/ | Permanent |
| Cron config | ~/.openclaw/cron/jobs.json | Permanent |

---

## Scripts Reference

### ~/.openclaw/bin/garmin
```bash
#!/bin/bash
# Routes to venv Python
# Usage: garmin data TYPE --days N
#        garmin auth login
#        garmin chart dashboard --days N
```

### ~/.openclaw/bin/vitality
```python
# Calculates composite score
# Cache-enabled (60 min)
# Usage: vitality [--days N] [--json] [--debrief] [--no-cache]
```

### ~/.openclaw/bin/strength-check
```python
# Polls Intervals.icu API
# Parses FIT files
# Updates SQLite + journal
# Usage: strength-check (no args)
```

### ~/.openclaw/bin/silverbullet
```bash
# PKM filesystem bridge
# Usage: silverbullet read|write|list|delete|search PATH
```

---

## API Endpoints

### Garmin (python-garminconnect)
• client.get_sleep_data(date)
• client.get_hrv_data(date)
• client.get_heart_rates(date)
• client.get_body_battery(date)
• client.get_stress_data(date)
• client.get_user_summary(date)

### Intervals.icu
• GET /api/v1/athlete/0/activities?oldest=YYYY-MM-DD
• GET /api/v1/activity/{id}/file (FIT download)

---

## Configuration Files

### ~/.openclaw/openclaw.json
• Agent defaults
• Model selection
• Cron job definitions

### ~/.env.sh
• INTERVALS_API_KEY
• INTERVALS_WEBHOOK_SECRET
• PATH extensions

### ~/.openclaw/vitality-config.json
```json
{
  "age": 37,
  "sleep_target_hours": 7.5,
  "steps_target": 8000,
  "rhr_baseline": null,
  "hrv_baseline": null
}
```

---

## Known Issues

### Garmin
• Sleep window assumes 23:00-07:00
• Struggles with irregular schedules
• REM detection mediocre vs Oura/WHOOP

### Vitality
• No historical storage (cache only)
• 60 min TTL may miss recent changes
• Fetches all 30 days every hour

### Strength-check
• Requires structured workouts for accuracy
• No real-time (polls on schedule)
• Depends on Intervals.icu sync

---

## Unified System Recommendations

### Schema Proposal

```sql
-- Unified health database
-- Location: ~/health-data/unified-health.db

-- Sleep tracking
CREATE TABLE sleep (
    date TEXT PRIMARY KEY,
    total_seconds INTEGER,
    deep_seconds INTEGER,
    rem_seconds INTEGER,
    light_seconds INTEGER,
    awake_seconds INTEGER,
    sleep_score INTEGER,
    hrv_ms REAL,
    source TEXT DEFAULT 'garmin'
);

-- Cardiovascular
CREATE TABLE heart (
    date TEXT PRIMARY KEY,
    resting_hr INTEGER,
    max_hr INTEGER,
    min_hr INTEGER,
    hrv_avg REAL,
    source TEXT DEFAULT 'garmin'
);

-- Recovery metrics
CREATE TABLE recovery (
    date TEXT PRIMARY KEY,
    body_battery_charged INTEGER,
    body_battery_drained INTEGER,
    body_battery_peak INTEGER,
    body_battery_low INTEGER,
    source TEXT DEFAULT 'garmin'
);

-- Activity
CREATE TABLE activity (
    date TEXT PRIMARY KEY,
    steps INTEGER,
    active_minutes INTEGER,
    floors INTEGER,
    source TEXT DEFAULT 'garmin'
);

-- Stress
CREATE TABLE stress (
    date TEXT PRIMARY KEY,
    avg_stress INTEGER,
    max_stress INTEGER,
    source TEXT DEFAULT 'garmin'
);

-- Strength workouts (from existing strength.db)
-- Can be kept separate or merged
```

### Sync Strategy

**Option A: Daily batch (cron)**
• Morning: Fetch yesterday's data
• Store in unified DB
• Cron jobs query DB (not live API)

**Option B: Real-time with caching**
• Scripts check DB first
• If stale/missing: fetch from API
• Update DB

**Option C: Event-driven**
• Webhook from Garmin/Intervals
• Instant DB update
• Scripts always query DB

### Migration Path

1. Create unified DB schema
2. Backfill from existing sources
3. Update scripts to use DB
4. Add cache layer on top
5. Deprecate old systems gradually

---

## Documentation Files

| File | Purpose |
|------|---------|
| ~/health-data-systems.md | This file — comprehensive reference |
| ~/.openclaw/workspace/personal-ai-context/agents/oklava/strength-pipeline.md | Strength pipeline docs |
| ~/.openclaw/workspace/personal-ai-context/agents/oklava/vitality-score.md | Vitality docs |
| ~/.openclaw/workspace/personal-ai-context/agents/oklava/tools-index.md | All custom tools |

---

**Last Updated:** 2026-03-01  
**Next Review:** When implementing unified system
