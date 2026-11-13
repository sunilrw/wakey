# `wakey` (Beta)

> **Built by Sunil Warrier** · Open Source (MIT) · Beta Release `v0.1.0-beta`

```text
                  _
 __      ____ _ _| | _____ _   _
 \ \ /\ / / _` | |/ / _ \ | | |
  \ V  V / (_| |   <  __/ |_| |
   \_/\_/ \__,_|_|\_\___|\__, |
                         |___/  (Beta)
  Built by Sunil Warrier — Smart Cross-Platform Keep-Awake
```

A fast, cross-platform (**macOS, Windows, Linux**) `caffeinate` replacement built in **Rust** + **Tauri v2**, with a menu bar / tray popover, full CLI, AI agent detection, 90-day habit learning, morning hardware wake scheduling, and zero-login peer pairing.

> **Status: Beta** — Actively tested on macOS (Apple Silicon) and Windows; Linux CLI/daemon supported. Please report issues on GitHub!

---

## Features

- **Full `caffeinate` CLI Parity:** `-d` (keep display on), `-s` (system awake), `-a` (Windows Away Mode), `-t <DURATION>`, `--until <HH:MM>`, `-w <PID>`, `-- <COMMAND>` (preserves exit code).
- **Tray / Menu Bar Popover (`wakey-app`):**
  - Clear live status (`KEEPING AWAKE`, `PAUSED`, `NORMAL SLEEP ALLOWED`), trigger source, and exact return-to-sleep time.
  - Duration slider (`15m` → `24h` → `∞ Forever`), `+30m`, `Pause / Resume`, `Stop (Sleep Now)`, and `Keep Screen On` toggle.
  - Compact popover ↔ wide dashboard toggle (`⤢` / `⤡`) and collapsible header.
- **AI Agent Watch:** Automatically keeps your computer awake (while letting the screen turn off) when AI coding agents or local LLMs are actively working (`Claude Code`, `Codex CLI`, `Gemini CLI`, `OpenCode`, `Aider`, `Goose`, `Antigravity`, `Cursor`, `Windsurf`, `Ollama`, `LM Studio`), plus configurable cooldown.
- **90-Day Habit Learning:** Records manual sessions locally in `history.jsonl` (auto-pruned at **90 days** and **2 MiB** cap) and learns **Daily**, **Weekdays (Mon–Fri)**, **Weekends (Sat–Sun)**, and **Specific Weekday** patterns.
- **Daily & Weekly Schedules + Morning Hardware Wake (RTC):**
  - Software schedules keep the machine awake during configured windows.
  - **Morning Wake from Sleep (RTC)** schedules the OS hardware wake timer (`pmset repeat wakeorpoweron` on macOS, Task Scheduler `WakeToRun` on Windows, `rtcwake` on Linux) to wake a sleeping computer in the morning.
- **Remote PC Pairing & VPS Relay (`wakey-relay`):**
  - Zero-login Device ID + 6-digit passcode pairing (with one-click copy/paste pairing code).
  - Full status view & full remote controls for every paired PC (Start, `+30m`, Pause/Resume, Stop, Duration slider, Keep Screen On, and Wake-on-LAN).
  - Works on the same LAN or across networks via self-hosted `wakey-relay` (terminate TLS behind an HTTPS reverse proxy for public internet deployments).
- **Grok / Bot HTTP & Webhook Bridge:** Local REST API (`GET /status`, `POST /command` with Bearer token auth) plus optional outbound webhook events signed with `X-Wakey-Secret`.

---

## Prebuilt Executables (Beta)

Download prebuilt binaries from the [**GitHub Releases**](https://github.com/sunilrw/wakey/releases) page:

- **macOS (Apple Silicon):** `wakey_0.1.0_aarch64.dmg`, `wakey-macos-arm64.app.zip`, `wakey` (CLI), `wakey-relay`
- **Windows (x86_64):** `wakey-app.exe` (Desktop + Tray), `wakey.exe` (CLI), `wakey-relay.exe`

---

## Full CLI Help & Examples

Run `wakey --help` for the built-in banner and full command reference:

```bash
# Keep system awake (screen allowed to turn off) until Ctrl+C
wakey

# Keep both system and screen on for 2 hours
wakey -d -t 2h

# Stay awake until 18:30 local time
wakey --until 18:30

# Stay awake while a command runs (preserves exit code)
wakey -- cargo build --release

# Stay awake while an existing PID runs
wakey -w 12345

# Run background daemon (schedules, agent watch, habit prompts, IPC & bot HTTP bridge)
wakey daemon

# Query or control the running daemon
wakey status
wakey extend 30m
wakey pause
wakey resume
wakey stop

# Manage schedules
wakey schedule list
wakey schedule add --name Work --days mon-fri --start 09:00 --end 18:00
wakey schedule remove Work

# Diagnostics & inspection
wakey agents
wakey habits
wakey doctor
```

---

## Building from Source

### Prerequisites
- [Rust](https://rustup.rs/) (stable toolchain)
- Node.js (for `@tauri-apps/cli` when bundling `wakey-app`)

### Test & Build

```bash
# Run all workspace tests
cargo test --workspace

# Build CLI & relay in release mode
cargo build --workspace --release

# Bundle macOS .app and .dmg
cd app/src-tauri && npx --yes @tauri-apps/cli build --bundles app,dmg
```

Binaries are output to `target/release/`:
- `wakey` / `wakey.exe` (CLI)
- `wakey-app` / `wakey-app.exe` (Desktop & Tray UI)
- `wakey-relay` / `wakey-relay.exe` (Optional cross-network relay server)

---

## License

MIT License — © 2026 **Sunil Warrier**
