# Installation

## Prerequisites

- **Rust stable 1.80+** — install via [rustup](https://rustup.rs)

```bash
rustup update stable
```

## Build from Source

```bash
git clone https://github.com/your-org/pulse
cd pulse
cargo build --release
```

The binary is at `target/release/pulse-cli`.

## Put Pulse on Your PATH

**Option A — copy the binary:**

```bash
# Linux / macOS
cp target/release/pulse-cli ~/.local/bin/pulse

# Windows (PowerShell)
Copy-Item target\release\pulse-cli.exe "$env:USERPROFILE\.local\bin\pulse.exe"
```

**Option B — install via Cargo:**

```bash
cargo install --path pulse-cli
```

This places the binary in `~/.cargo/bin/`, which is already on PATH after a standard rustup install.

## First Run

```bash
pulse config init
```

Creates `~/.pulse/config.toml` with defaults. Edit it to add your credentials and personal targets before syncing.
