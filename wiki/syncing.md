# Syncing Data

## Garmin Connect

1. **Enable in config:**

```toml
[providers.garmin]
enabled = true
username = "you@example.com"
password = "your-password"
```

2. **Authenticate** (required once; session is cached):

```bash
pulse garmin-login
```

If your account has two-factor authentication enabled, wait for the MFA prompt and pass the code:

```bash
pulse garmin-login --mfa 123456
```

3. **Sync:**

```bash
pulse sync garmin
```

### Data Latency

Garmin sleep data is typically available the **next morning**, not immediately after waking. If today's sleep score is missing, sync again after ~9:00 AM local time.

### VO2 Max

Garmin estimates VO2 max only after qualifying activities, such as an outdoor run or walk with GPS and heart rate. Pulse stores the estimate on the day it was made; other days stay empty, and the vitality score uses the average of the last 30 days instead.

Before 0.5.1, pulse never stored VO2 max. To fill earlier days, sync them once:

```bash
pulse sync garmin --days 30
```

### Sleep times, SpO2, respiration, stress breakdown, calories and distance

Since 0.6.0, pulse stores bed and wake time, average SpO2 and respiration during sleep, the time spent at rest and at low, medium and high stress, and the day's calories (total, active, BMR) and distance. When 0.6.0 first opens your database, it fills them for every day synced since 0.5.0, from the stored Garmin responses. Days synced before 0.5.0 stay empty until you sync them once:

```bash
pulse sync garmin --days 365
```

Bed and wake times are the watch's local time. SpO2 is recorded only when pulse ox tracking is on.

---

## Intervals.icu

1. **Get your API key:** Log in to [intervals.icu](https://intervals.icu) → Settings → Developer → API Key.

2. **Add to config:**

```toml
[providers.intervals]
enabled = true
athlete_id = "i12345"   # from your profile URL
api_key = "your-api-key"
```

3. **Sync:**

```bash
pulse sync intervals
```

### Distance and pace

Since 0.9.0, pulse stores each workout's distance and average speed, and `pulse workouts` shows distance in km and pace per km. When 0.9.0 first opens your database, it fills them for every workout synced since 0.5.0, from the stored Intervals.icu responses. Older workouts stay empty until you sync them once:

```bash
pulse sync intervals --from 2026-01-01
```

Pace comes from the average speed your watch recorded, so it can differ slightly from duration divided by distance: the duration is Intervals.icu's moving time.

### Heart rate zones

Since 0.10.0, pulse stores how long each workout spent in each heart rate zone, and `pulse workouts` shows it under the workout line (`HR zones: Z1 8m · Z2 2m · …`, Z1 being the lowest). The zones are the ones configured in Intervals.icu, and each workout's times are counted against the zones Intervals.icu used for it, so after you change your zones, older workouts keep their times for the old zones. When 0.10.0 first opens your database, it fills the zone times for every workout synced since 0.5.0, from the stored Intervals.icu responses. Older workouts stay empty until you sync them once:

```bash
pulse sync intervals --from 2026-01-01
```

The zone times add up to the time with a heart rate reading, so their sum can differ from the workout's duration by a minute or two. A workout recorded without heart rate shows no zones.

### Training load

Since 0.11.0, pulse stores each workout's training load as Intervals.icu computed it, and `pulse workouts` shows it at the end of the workout line (`load 43`). Intervals.icu works it out from power, heart rate or pace, so a workout recorded without any of them has no load. When 0.11.0 first opens your database, it fills the load for every workout synced since 0.5.0, from the stored Intervals.icu responses. Older workouts stay empty until you sync them once:

```bash
pulse sync intervals --from 2026-01-01
```

If Intervals.icu recalculates a workout's load later, for example after you change your thresholds there, sync that day again to pick up the new value.

### Weight, body fat, fitness and fatigue

Since 0.12.0, pulse also syncs Intervals.icu's daily wellness data: weight, body fat, fitness (CTL) and fatigue (ATL). `pulse wellness` shows them. Fitness and fatigue are the values on Intervals.icu's fitness chart, computed from your workouts' training load. Weight and body fat appear only when something records them in Intervals.icu, such as a connected smart scale or a manual entry. When you train on a day without a weigh-in, Intervals.icu copies your last weight to that day; pulse leaves those copies out, so the weight column holds only real weigh-ins.

pulse did not fetch wellness data before 0.12.0, so there is nothing to fill in from stored responses. To load your history, sync it once; Intervals.icu returns any range in a single request:

```bash
pulse sync intervals --from 2026-01-01
```

Intervals.icu recalculates fitness and fatigue when you add, edit or delete an activity, or change its load settings. Sync the affected days again to pick up the new values.

---

## Syncing Both Providers

```bash
pulse sync           # syncs all enabled providers
pulse sync --days 7  # backfill the past 7 days
pulse sync --from 2026-09-01 --to 2026-09-30  # sync exactly September
```

---

## Stored Responses

Pulse keeps every response body it receives from Garmin and Intervals.icu (see `[storage]` in [Configuration](configuration.md)). Later versions use them to fill new columns for days you have already synced, and `pulse rebuild` refills the data tables from them without network access.

Only days synced from this version on have a stored body. To cover older days, sync them once:

```bash
pulse sync --days 365
```

Garmin takes about 5 requests per day, so a year takes roughly 12 to 19 minutes. If Garmin throttles you and some days report errors, run the same command again, or sync in chunks (for example `pulse sync garmin --from 2025-01-01 --to 2025-03-31`).

---

## Automation

### Linux / macOS — cron

```cron
# Run daily at 09:30, backfilling 2 days to catch late Garmin uploads
30 9 * * * /home/user/.cargo/bin/pulse sync --days 2
```

### Windows — Task Scheduler

```powershell
$action = New-ScheduledTaskAction -Execute "pulse" -Argument "sync --days 2"
$trigger = New-ScheduledTaskTrigger -Daily -At "09:30"
Register-ScheduledTask -TaskName "PulseSync" -Action $action -Trigger $trigger
```

The `--days 2` overlap ensures that any data that arrived late (e.g., last night's sleep) is captured without re-syncing the full history.
