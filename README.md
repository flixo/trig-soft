# trig-soft

Cross-platform clipboard poller for Linux (Wayland) and Windows 11.

## Current scope

The app reads the text clipboard every second and prints changes.

## Requirements

- Rust toolchain (stable)
- Linux Wayland session or Windows 11 desktop session

### Linux (Wayland)

Install Wayland clipboard tooling:

- Debian/Ubuntu: `sudo apt install wl-clipboard`
- Fedora: `sudo dnf install wl-clipboard`
- Arch: `sudo pacman -S wl-clipboard`

## Run

```bash
cargo run
```

Copy something new and the app will print the updated clipboard value.

## Next step

This is the functionality check. After this, we can convert it into a proper background service/daemon process.
