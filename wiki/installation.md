# Installation

## Prebuilt Binary (no Rust needed)

Every release from 0.14.0 on attaches binaries to its [GitHub Release](https://github.com/selcux/pulse/releases/latest):

| Platform | Archive |
|---|---|
| Windows (x86_64) | `pulse-cli-x86_64-pc-windows-msvc-vX.Y.Z.zip` |
| Linux (x86_64, static, any distro) | `pulse-cli-x86_64-unknown-linux-musl-vX.Y.Z.tar.gz` |
| macOS (Apple Silicon) | `pulse-cli-aarch64-apple-darwin-vX.Y.Z.tar.gz` |
| macOS (Intel) | `pulse-cli-x86_64-apple-darwin-vX.Y.Z.tar.gz` |

Each archive has a `.sha256` file next to it. Check the download, unpack it and put `pulse` on your PATH:

```bash
# Linux / macOS
sha256sum -c pulse-cli-<target>-vX.Y.Z.tar.gz.sha256   # macOS: shasum -a 256 -c ...
tar -xzf pulse-cli-<target>-vX.Y.Z.tar.gz
mkdir -p ~/.local/bin && mv pulse ~/.local/bin/
```

```powershell
# Windows (PowerShell): compare with the hash in the .sha256 file
(Get-FileHash pulse-cli-x86_64-pc-windows-msvc-vX.Y.Z.zip).Hash
Expand-Archive pulse-cli-x86_64-pc-windows-msvc-vX.Y.Z.zip -DestinationPath "$env:USERPROFILE\.local\bin"
```

The binaries are not code-signed:

- **macOS** blocks a downloaded binary the first time it runs. Clear the quarantine flag with
  `xattr -d com.apple.quarantine ~/.local/bin/pulse`.
- **Windows** SmartScreen may warn on first run: choose *More info* → *Run anyway*.

With [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) installed, `cargo binstall pulse-cli` fetches the
matching binary for you.

## Prerequisites (crates.io and source builds)

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
