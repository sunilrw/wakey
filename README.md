# `wakey`

A fast, cross-platform (**Windows, macOS, Linux**) `caffeinate` replacement built in **Rust**, featuring:
- Full `caffeinate` CLI parity (`-d`, `-s`, `-a`, `-t`, `--until`, `-w <PID>`, `-- <COMMAND>`)
- **AI Agent Watch**: automatically keeps your machine awake while popular AI agents (`Claude Code`, `Codex CLI`, `Gemini CLI`, `OpenCode`, `Aider`, `Goose`, `Antigravity`, `Cursor`, `Windsurf`, `Ollama`, `LM Studio`) are actively working — while allowing your **screen to turn off**
- **Daily & Weekly Schedules** (`%APPDATA%\wakey\config.toml` on Windows, `~/Library/Application Support/wakey/config.toml` on macOS, `~/.config/wakey/config.toml` on Linux)
- **Opt-in Habit Learning & Pre-Expiry Extend Warnings**
- **Bidirectional Grok Bot / Webhook Bridge** (outbound events + inbound `POST /command` & `GET /status`)
- **Clean Tauri v2 Desktop Dashboard + System Tray** (`wakey-app`)

## Quick Start (CLI)

```bash
# Keep system awake (display allowed to turn off) until Ctrl+C
wakey

# Keep both system and screen on for 2 hours
wakey -d -t 2h

# Stay awake until 18:00 local time
wakey --until 18:00

# Stay awake while a command runs (preserves exit code)
wakey -- cargo build --release

# Stay awake while an existing PID runs
wakey -w 12345

# Run background daemon (schedules, agent watch, habit prompts, IPC & bot HTTP bridge)
wakey daemon

# Control running daemon
wakey status
wakey extend 30m
wakey pause
wakey resume
wakey stop

# Diagnostics & inspection
wakey agents
wakey habits
wakey schedule list
wakey doctor
```

## Build & Test

```bash
# Run all unit tests across workspace
cargo test --workspace

# Build CLI & Dashboard app
cargo build --workspace --release
```

Binaries will be in `target/release/wakey` (`wakey.exe` on Windows) and `target/release/wakey-app` (`wakey-app.exe` on Windows).
