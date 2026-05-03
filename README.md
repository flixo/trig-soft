# trig-soft

Host-side bridge between a Trig USB HID device and the desktop clipboard.

## What it does

`trig-soft` continuously looks for HID devices whose product or manufacturer name contains `Trig`, selects the best RAW HID-style interface, and then:

- Sends current Unix time to the device every 1 second (`time` tag).
- Listens for inbound RAW HID reports from the device.
- Handles clipboard sync commands:
	- `past`: device requests paste value from host clipboard.
	- `copy`: device sends value to copy into host clipboard.

All reports are fixed-size 32-byte payloads.

## Clipboard behavior

- Linux: reads/writes both `Clipboard` and `Primary` selections through `arboard` Linux extensions.
- Windows: uses standard clipboard read/write through `arboard`.

When serving a `past` request, clipboard text is parsed as a number, normalized, and sent back using the `inpt` tag.

## HID transport by platform

- Windows: uses `hidapi` backend (HID-class access) to avoid interface-claim issues with driver-bound HID interfaces.
- Non-Windows: uses `nusb` backend and claims the selected HID interface.

## Requirements

- Rust 1.93 (pinned via [rust-toolchain.toml](rust-toolchain.toml)).
- A connected Trig HID device.

### Linux notes

Wayland clipboard tools may be required by your distro:

- Debian/Ubuntu: `sudo apt install wl-clipboard`
- Fedora: `sudo dnf install wl-clipboard`
- Arch: `sudo pacman -S wl-clipboard`

## Run

```bash
cargo run
```

Expected startup flow:

- Poll for matching Trig device every 2s.
- Print matched interfaces.
- Select first claimable/best-priority interface.
- Start time send loop and clipboard command handling.

## Troubleshooting

- `No matching device found for 'Trig'`:
	- Check USB connection and product/manufacturer string matching.
- `No claimable interface found` on non-Windows:
	- Interface is likely owned by another driver/process.
- Clipboard errors:
	- Ensure desktop clipboard service is available in current session.
