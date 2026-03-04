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
