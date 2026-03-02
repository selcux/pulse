# Garmin Strength Training Pipeline

**Location:** `~/.openclaw/workspace/personal-ai-context/agents/oklava/strength-pipeline.md`  
**Created:** 2026-02-28  
**Model:** Kimi K2.5  
**Status:** Active (replaces webhook-based approach)

---

## Purpose

Automated retrieval of detailed strength training data (exercise, sets, reps, weight) from Garmin → Intervals.icu → SQLite → Daily Journal.

Replaces overkill webhook solution with simple on-demand polling integrated into daily rhythms.

---

## Architecture

```
Garmin Watch (structured workout)
        ↓
Garmin Connect (sync)
        ↓
Intervals.icu (stores FIT file)
        ↓
strength-check (cron: Evening Sync / Nightly Debrief)
        ↓
SQLite DB + Journal update
```

---

## Components

### 1. Main Script

**Path:** `~/.openclaw/bin/strength-check`  
**Language:** Python 3  
**Dependencies:** `garmin-fit-sdk`, `requests`, `sqlite3`

**What it does:**
- Checks Intervals.icu API for new activities since last sync
- Downloads FIT files for strength workouts
- Parses `SetMsg` records (exercise category, name, reps, weight)
- Stores in SQLite
- Updates daily journal with portrait-friendly summary
- Tracks last sync timestamp (idempotent)

### 2. Database

**Path:** `~/health-data/strength.db`

**Schema:**
```sql
-- Workouts table
CREATE TABLE workouts (
    id TEXT PRIMARY KEY,           -- Intervals.icu activity ID
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
    exercise_category TEXT,        -- e.g., 'CURL', 'BENCH_PRESS'
    exercise_name TEXT,            -- e.g., 'CABLE_CURL'
    repetitions INTEGER,
    weight_kg REAL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Sync state tracking
CREATE TABLE sync_state (
    key TEXT PRIMARY KEY,
    value TEXT,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

### 3. FIT Archive

**Path:** `~/health-data/fit-archive/`  
**Purpose:** Stores original FIT files for reprocessing if needed

### 4. Cron Integration

**Evening Sync** (17:00 Helsinki):
```
1. Run `strength-check` to fetch today's strength workouts
2. Read today's journal
3. Check Briefing
4. Prepare Sync template
5. Write to journal
```

**Nightly Debrief** (22:00 Helsinki):
```
1. Run `strength-check` (catches late workouts)
2. Run vitality
3. Fetch Garmin data
4. Pre-fill Debrief with strength details + other data
```

---

## Installation (Clean Environment)

### Prerequisites

- Python 3.11+
- `pip3`
- Intervals.icu account with API key
- Garmin Connect linked to Intervals.icu

### Step 1: Install Python Dependencies

```bash
pip3 install --break-system-packages garmin-fit-sdk requests
```

### Step 2: Create Directory Structure

```bash
mkdir -p ~/health-data/fit-archive
mkdir -p ~/.openclaw/bin
```

### Step 3: Deploy Script

Copy `strength-check` to `~/.openclaw/bin/` and make executable:

```bash
cp /path/to/strength-check ~/.openclaw/bin/
chmod +x ~/.openclaw/bin/strength-check
```

### Step 4: Environment Variables

Add to `~/.env.sh`:

```bash
# Intervals.icu API
export INTERVALS_API_KEY='your_api_key_here'
```

Source it:
```bash
source ~/.env.sh
```

### Step 5: Initialize Database

```bash
python3 -c "
import sys
sys.path.insert(0, '/home/selcuk/.openclaw/bin')
from strength_check import init_db
init_db()
print('Database initialized')
"
```

### Step 6: Update Cron Jobs

Edit Evening Sync and Nightly Debrief to include `strength-check` as Step 1.

See current configs in:
```bash
~/.openclaw/cron/jobs.json
```

---

## Configuration

### Intervals.icu Setup

1. Create account at https://intervals.icu
2. Connect Garmin Connect (Settings → Sync)
3. Get API key (Settings → Developer Settings)
4. No webhook registration needed (polling approach)

### Garmin Watch Setup

**CRITICAL:** Use structured workouts for accurate exercise names.

**Why:** FIT files contain only device-recorded data. Post-hoc edits in Garmin Connect app don't sync back to FIT.

**How:**
1. Create workout in Garmin Connect app/website
2. Define exercises with correct names
3. Sync to watch
4. Follow workout on watch during training

---

## Usage

### Manual Check

```bash
# Fetch recent workouts
strength-check

# View stored workouts
sqlite3 ~/health-data/strength.db "SELECT * FROM workouts ORDER BY start_time DESC LIMIT 5;"

# View sets for specific workout
sqlite3 ~/health-data/strength.db "SELECT * FROM exercise_sets WHERE workout_id='i12345';"
```

### Automatic (Preferred)

Happens automatically during:
- **Evening Sync** (17:00) — for review
- **Nightly Debrief** (22:00) — final summary

### Journal Output Format

```markdown
### 🏋️ Strength Workout

**Cable External Rotation**
• Set 1: 15 reps @ 5kg
• Set 2: 15 reps @ 5kg
• Set 3: 12 reps @ 5kg

**Total Volume:** 210kg
**Duration:** 12m 34s
```

---

## Data Flow

| Time | Action | Result |
|------|--------|--------|
| User works out | Garmin watch records structured workout | FIT file created |
| Post-workout | Watch syncs to Garmin Connect | Cloud updated |
| Auto | Garmin Connect → Intervals.icu | FIT file available via API |
| 17:00 | Evening Sync runs `strength-check` | Journal updated with workout |
| 22:00 | Nightly Debrief runs `strength-check` | Debrief includes strength summary |

---

## Limitations

| Limitation | Workaround |
|------------|------------|
| FIT file doesn't include post-hoc edits | Use structured workouts |
| Exercise auto-detection can be wrong | Pre-define exercises in structured workout |
| Only strength workouts with SetMsg records | Other activities use Garmin summary |
| 7-day default lookback | Manual run with custom date if needed |

---

## Migration from Webhook Version

If migrating from webhook-based solution:

```bash
# 1. Stop webhook service
strength-webhook stop  # or kill PID

# 2. Remove webhook files
rm -f ~/health-data/webhook_service.py
rm -f ~/health-data/setup.sh
rm -f ~/.openclaw/bin/strength-webhook
rm -f ~/health-data/logs/webhook.log

# 3. Remove from Caddy
# Edit ~/caddy/Caddyfile — remove fit.selcukozturk.dev block
docker exec caddy caddy reload

# 4. Remove DNS record (optional)
# Namecheap → delete fit.selcukozturk.dev A record

# 5. Deploy new solution (see Installation)
```

---

## Troubleshooting

| Issue | Check |
|-------|-------|
| No workouts found | INTERVALS_API_KEY set? Garmin synced to Intervals.icu? |
| No strength data | Using structured workouts? Activity type = strength/training? |
| Journal not updating | silverbullet CLI working? Journal file exists? |
| Database locked | Previous process crashed? Delete `strength.db-journal` |
| Parse errors | garmin-fit-sdk installed? FIT file not corrupted? |

---

## Related Files

| Path | Purpose |
|------|---------|
| `~/.openclaw/bin/strength-check` | Main script |
| `~/health-data/strength.db` | SQLite database |
| `~/health-data/fit-archive/` | FIT file storage |
| `~/.openclaw/cron/jobs.json` | Cron job definitions |
| `~/.env.sh` | Environment variables |
| `~/caddy/Caddyfile` | Reverse proxy config (no longer needs fit. subdomain) |

---

## Evolution

| Version | Approach | Status |
|---------|----------|--------|
| v1 | Webhook service + fit subdomain | Abandoned — overkill for personal use |
| v2 | On-demand polling + cron integration | **Current** — simple, reliable |

---

## References

- Original design doc: `~/health-data/README-v2.md`
- Intervals.icu API: https://intervals.icu/api-docs.html
- Garmin FIT SDK: https://developer.garmin.com/fit/overview/
- Forum thread: https://forum.intervals.icu/t/pulling-strength-workout-exercise-sets-reps-weight-from-garmin/123171

---

**Last Updated:** 2026-02-28  
**Maintained by:** Oklava (Kimi K2.5)
