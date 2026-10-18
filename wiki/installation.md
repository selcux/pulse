# Installation

## Prerequisites

- **Rust stable 1.88+** — install via [rustup](https://rustup.rs)

```bash
rustup update stable
```

## Install from crates.io

```bash
cargo install pulse-cli
```

This places the `pulse` binary in `~/.cargo/bin/`, which is already on PATH after a standard rustup install.

## Build from Source

```bash
git clone https://github.com/selcux/pulse
cd pulse
cargo build --release
```

The binary is at `target/release/pulse` (`pulse.exe` on Windows).

To put it on your PATH, either install it with Cargo:

```bash
cargo install --path pulse-cli
```

or copy it:

```bash
# Linux / macOS
cp target/release/pulse ~/.local/bin/pulse

# Windows (PowerShell)
Copy-Item target\release\pulse.exe "$env:USERPROFILE\.local\bin\pulse.exe"
```

## First Run

```bash
pulse config init
```

Creates `~/.pulse/config.toml` with defaults. Edit it to add your Intervals.icu API key and personal targets before syncing, then run `pulse garmin-login` to connect Garmin (see [Syncing data](syncing.md)).
