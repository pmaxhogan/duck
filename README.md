# 🦆 Duck

**Automatic ducking for [Voicemeeter](https://vb-audio.com/Voicemeeter/).** Duck
lowers — and optionally low‑passes — the strips you choose whenever other strips
carry sound. Point it at a microphone or a voice‑chat strip and your music and
videos dip out of the way the moment you start talking, then fade back when you
stop.

It runs quietly in the system tray, starts with Windows if you want it to, and
updates itself.

![status: pre-release](https://img.shields.io/badge/status-pre--release-f5a524)
[![CI](https://github.com/pmaxhogan/duck/actions/workflows/ci.yml/badge.svg)](https://github.com/pmaxhogan/duck/actions/workflows/ci.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

---

## Features

- **Level‑triggered ducking** — any "source" strip that rises above its threshold
  ducks the "target" strips you pick.
- **Per‑source thresholds** — every trigger strip gets its own sensitivity.
- **Smooth fades** — configurable attack, release, and hold times.
- **Optional low‑pass** — muffle the ducked strips using Voicemeeter's Color pad
  (hardware input strips only).
- **Pre‑fader metering** — triggering ignores the fader and mute, so a muted
  microphone can still duck the music.
- **Respects you** — captures each target's real fader/Color position, restores
  it exactly on release or exit, and if you nudge a fader mid‑duck your change
  carries through.
- **Tray app** — enable/disable, start‑on‑login, and settings from the tray.
- **Auto‑update** — signed updates delivered through GitHub Releases.

Works with Voicemeeter, Voicemeeter Banana, and Voicemeeter Potato.

## Install

Download the latest installer from the
[**Releases**](https://github.com/pmaxhogan/duck/releases) page and run it. Duck
is a small Windows tray app; it needs Voicemeeter installed and running.

> Requires the [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/),
> which ships with Windows 11 and current Windows 10.

## How it works

Every poll (default 50 ms) Duck reads each source strip's **pre‑fader** peak
level. When a source is above its threshold, each target strip fades down by the
duck amount and (if enabled) its Color pad moves to a low‑pass position. Once all
sources have been quiet for the hold time, the targets fade back to wherever the
faders were, and the Color pad is restored once the fade completes.

Strip numbers are 0‑based, left to right in the Voicemeeter window.

Settings live at `%APPDATA%\com.pmaxhogan.duck\config.toml` and can be edited in
the window or by hand:

```toml
poll_ms = 50

[[rule]]
sources = [
  { strip = 0, threshold_db = -40 },
]
targets    = [1]
duck_db    = -8
attack_ms  = 60
release_ms = 350
hold_ms    = 1000
# color   = { x = -0.2, y = 0 }   # optional low-pass while ducking
```

## Build from source

Prerequisites: [Rust](https://rustup.rs/), [Node.js](https://nodejs.org/), and
the Tauri prerequisites for your platform (on Windows: the MSVC build tools and
WebView2).

```bash
npm install
npm run tauri dev      # run in development
npm run tauri build    # produce an installer in src-tauri/target/release/bundle
```

The ducking engine is plain Rust with a full unit‑test suite:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

## License

[MIT](LICENSE)
