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

---

## Syncing Both Providers

```bash
pulse sync           # syncs all enabled providers
pulse sync --days 7  # backfill the past 7 days
```

---

## Stored Responses

Pulse keeps every response body it receives from Garmin and Intervals.icu (see `[storage]` in [Configuration](configuration.md)). Later versions use them to fill new columns for days you have already synced, and `pulse rebuild` refills the data tables from them without network access.

Only days synced from this version on have a stored body. To cover older days, sync them once:

```bash
pulse sync --days 365
```

Garmin takes about 5 requests per day, so a year takes roughly 12 to 19 minutes. If Garmin throttles you and some days report errors, run the same command again, or sync in chunks (for example `--days 90`).

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
