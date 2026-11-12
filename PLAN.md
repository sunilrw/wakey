# `wakey` — Full Architecture & Implementation Plan

> A fast, clean **cross-platform (Windows, macOS, Linux)** app + CLI built in **Rust** (`wakey-core` + `wakey-cli` + `wakey-app` Tauri v2 dashboard) that brings full `caffeinate` parity everywhere, **auto-detects running AI agents**, supports **daily/weekly schedules**, **learns your habits** to proactively ask *"Do you want me to stay awake until ...?"*, and lets the **screen turn off** while the machine keeps working.

---

## 1. Core `caffeinate` Parity (CLI) & Cross-Platform Power Backends

| Flag / Mode | Behavior |
| :--- | :--- |
| `wakey` (no args) | Keep system awake until `Ctrl+C` |
| `-d`, `--display` | Keep **display/screen** on (prevents screen off & lock) |
| `-s`, `--system` | Keep **system** awake while **allowing screen to turn off** (default) |
| `-a`, `--away` | Allow Away Mode (Windows) / AC-style system assertion |
| `-t`, `--time <DURATION>` | Stay awake for duration (`30m`, `2h`, `1h30m`, or seconds) |
| `--until <HH:MM>` | Stay awake until a specific local clock time today/tomorrow |
| `-w`, `--wait <PID>` | Stay awake while an existing process ID is running |
| `wakey -- <CMD...>` | Spawn a command and stay awake until it exits (exit code preserved) |

### Platform Power Backends (`crates/wakey-core/src/power/`)
- **Windows:** `PowerCreateRequest` + `PowerSetRequest` (`PowerRequestSystemRequired`, `PowerRequestDisplayRequired`, `PowerRequestAwayModeRequired`) + `GetSystemPowerStatus` for battery guard. Shows in `powercfg /requests` and auto-releases on crash/exit.
- **macOS:** `IOKit` `IOPMAssertionCreateWithName` / `IOPMAssertionRelease` (`kIOPMAssertionTypePreventUserIdleSystemSleep`, `kIOPMAssertionTypePreventUserIdleDisplaySleep`) — the exact native API `caffeinate` uses, visible in `pmset -g assertions`.
- **Linux:** D-Bus `org.freedesktop.login1.Manager.Inhibit` / `org.freedesktop.ScreenSaver.Inhibit` (with `systemd-inhibit` fallback).
- **IPC (`ipc.rs`):** Windows Named Pipe (`\\.\pipe\wakey`) on Windows; Unix Domain Socket (`$XDG_RUNTIME_DIR/wakey.sock` or `/tmp/wakey-$UID.sock`) on macOS/Linux.
- **Notifications (`notify.rs`):** Windows interactive Toasts, macOS UserNotifications / `osascript` dialog fallback, Linux `notify-rust` / D-Bus.

---

## 2. AI Agent & Tool Auto-Detection ("No Sleep While Agents Run")

Yes — `wakey` can automatically detect popular AI coding agents and local LLM runtimes and keep the PC awake while allowing the **screen to turn off** after your chosen idle time.

### Supported Built-in Presets (Toggleable in Settings)
- **CLI & Terminal Agents:** Claude Code (`claude`), OpenAI Codex CLI (`codex`), Gemini CLI (`gemini`), OpenCode (`opencode`), Aider (`aider`), Goose (`goose`)
- **IDE / Desktop Agents:** Antigravity, Cursor, Windsurf, VS Code Copilot/Agent workers
- **Local LLM / Inference Runtimes:** Ollama (`ollama`), LM Studio (`lm-studio`), `llama-server`, `vllm`
- **Custom Watchlist:** Add any `.exe` or command-line pattern (e.g., `ffmpeg.exe`, `robocopy.exe`, `cargo.exe`, `python.exe my_script.py`)

### Smart Detection Modes (Configurable per Rule or Globally)
1. **Actively Working Mode (Recommended default):**
   - Prevents idle-open apps (like an idle Cursor window left open overnight) from blocking sleep forever.
   - Checks **activity signals**: CPU usage above threshold (e.g., > 1%), active child processes (`cargo`, `npm`, `pytest`, `git`, `pwsh`), or active network/disk activity.
2. **Process Exists Mode:**
   - Stays awake whenever the process is running at all (great for `ollama` / batch scripts).
3. **Grace Period (Cooldown):**
   - Keeps the PC awake for a configurable buffer (e.g., `5 min`) after an agent finishes before allowing sleep.
4. **Screen-Off Behavior:**
   - Default for Agent Watch: **Keep PC awake (`SystemRequired`), allow screen to turn off** after your configured idle timeout (e.g., 3–10 mins).

---

## 3. Clean, Modern Windows Dashboard (`wakey-app`)

Built with **Tauri v2** (Rust backend + lightweight native WebView2 UI, ~5–10 MB, zero Electron bloat) + **System Tray**:

### Dashboard Views
1. **Home / Status:**
   - Big clean status indicator: **Awake** (with active reason: *"Agent: Claude Code"*, *"Manual: 1h 42m left"*, or *"Schedule: Workday"*) vs **Sleeping Allowed**.
   - One-click quick toggles: **Stay Awake (`1h`, `2h`, `Until 18:00`, `Indefinite`)**, **Keep Screen On** toggle, **Pause / Resume**.
   - Live list of **Currently Detected Agents / Watched Processes** with activity badges.
2. **Schedules:**
   - Clean visual cards for **Daily** and **Weekly** schedules (e.g., `Mon–Fri 09:00–17:30`).
   - Per-schedule toggles: *Allow screen off*, *Run on battery*, *Skip today*.
3. **Agent Watch:**
   - Toggle switches for popular AI agents & local LLMs, detection mode (*Active only* vs *Always*), grace period slider, and custom process adder.
4. **Habits:**
   - Shows learned patterns (e.g., *"Weekdays ~09:00 -> usually awake until 17:30"*), confidence, and prompt history.
   - One-click *"Convert habit to permanent schedule"* or *"Forget pattern"*.
5. **Settings (Everything Configurable):**
   - **Display & Sleep:** Allow screen off while awake (yes/no), custom screen-off idle minutes.
   - **Battery & Power Safety:** Pause on battery (yes/no), auto-release below `X%` battery (default `20%`).
   - **Habits & Notifications:** Enable habit learning, minimum occurrences before asking, prompt cooldown, quiet hours / Focus Assist respect.
   - **System:** Launch at Windows startup, start minimized to tray, theme (System / Dark / Light).

---

## 4. Daily & Weekly Schedules + Habit Learning

### Config (`%APPDATA%\wakey\config.toml`)
- All UI settings and CLI commands read/write the same clean TOML config.
- **IPC (`\\.\pipe\wakey`)**: CLI commands (`wakey status`, `wakey stop`, `wakey extend 30m`, `wakey schedule ...`) communicate directly with the running background tray/dashboard.

### Proactive Habit Prompts & Pre-Expiry Extend Warnings (Toast + Bot + CLI)
- **Privacy-first history (`%APPDATA%\wakey\history.jsonl`)**: Logs only `{ timestamp, weekday, duration_mins, end_time, keep_display, source }`.
- **Interactive Habit Toast:**
  - **Title:** *"Keep PC awake until 18:00?"*
  - **Body:** *"You usually keep your PC awake around this time on weekdays."*
  - **Buttons:** `[Yes, until 18:00]` · `[+1 hour]` · `[Not today]` · `[Don't ask again]`
- **Pre-Expiry Warning (`warn_before_minutes = 5` in Settings):**
  - Fires `X` minutes before a manual timer or schedule ends.
  - Shows a Windows Toast (**"Wakey ending in 5m — Extend?"** with `[+15m]`, `[+30m]`, `[+1h]`, `[Let it sleep]`) **and** forwards the exact same prompt to your **Grok Bot** (if enabled).
  - If you extend from **Windows Toast**, **Grok Bot**, or **CLI** (`wakey extend 1h`), `wakey` updates the timer and sends a confirmation back to the Grok Bot.

### Bidirectional Grok Bot Bridge (`bot_bridge.rs`)
- **Outbound (`wakey` -> Grok Bot):** Sends signed JSON webhooks/messages on events: `session_started`, `agent_detected`, `pre_expiry_warning`, `habit_prompt`, `session_ended`, `battery_cutoff`.
- **Inbound (`Grok Bot` -> `wakey`):** Accepts commands (`status`, `awake 2h`, `until 22:00`, `extend 1h`, `stop`, `skip-today`) via:
  1. **Outbound Poll / WebSocket Client** (no open firewall ports needed — `wakey` connects out to your bot/relay).
  2. **Authenticated Local HTTP API** (`127.0.0.1:<port>` with Bearer token, usable via Tailscale/Cloudflare Tunnel or local scripts).
- **Full CLI Parity:** Every bot action also works via CLI (`wakey extend 1h`, `wakey status`, `wakey stop`).

---

## 5. Must-Have, Useful & "Fancy" Enhancements

### Must-Haves
- **AC vs. Battery Profiles:** Separate rules when plugged in vs. on battery (e.g., Agent Watch & Schedules enabled on AC, disabled or capped at `20%` on battery).
- **Global Hotkey (e.g., `Ctrl+Alt+W`):** Instant toggle or `+1h` boost from anywhere without opening the UI.
- **Session Audit Log ("Why didn't my PC sleep?"):** Clean timeline in the dashboard showing every active block, trigger reason, and duration.
- **Windows Modern Standby (S0ix) & Lid-Close (`wakey doctor`):** Checks and explains Windows power plan settings (closing a laptop lid overrides normal `PowerRequestSystemRequired` unless configured).

### Very Useful Power-User Features
- **CPU / GPU / Network Activity Guard:** Auto-stay-awake while a big download/upload (> X KB/s) or GPU/CPU render is running, then release after an idle cooldown.
- **Manual Lock (`Win+L`) Rule (Optional):** *"If I manually lock my PC, allow sleep unless an Agent/Schedule is marked critical."*
- **Portable Mode:** If `wakey.toml` sits next to `wakey.exe`, use it instead of `%APPDATA%\wakey\`.

### Fancy & Polished
- **Windows 11 Mica / Acrylic Dashboard** with dark/light theme sync and a **live Tray Icon** (animated/filled coffee cup + hover tooltip showing *"Awake: 1h 12m left · Claude active"*).
- **One-Click Tray Presets:** Right-click tray for `30m`, `1h`, `2h`, `Until 18:00`, `Pause`.
- **Wake-to-Run Note:** `wakey` prevents Windows from sleeping; it cannot wake an *already sleeping* PC (that requires Windows Task Scheduler `WakeToRun` or Wake-on-LAN).

### Companion App Note: Walk / Stretch / Water Reminders
- **Recommendation:** Keep `wakey` focused on **power & sleep**, and build **Walk / Stand-Up / Water / 20-20-20 Eye-Rest** as a **separate companion app** (sharing the same Rust + Tauri v2 + Windows Toast stack).
- **Why separate:** Break reminders should *pause* when you're away from the keyboard (`GetLastInputInfo`) or in a meeting/fullscreen app, and track daily water/break streaks — the opposite of `wakey`, which runs when you step away.

---

## 6. Workspace Structure (Rust Cargo Workspace)

```text
wakey/
├── Cargo.toml                  # Cargo workspace
├── PLAN.md
├── README.md
├── crates/
│   ├── wakey-core/             # Shared Rust library
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── power.rs        # PowerCreateRequest / PowerSetRequest + battery status
│   │       ├── process.rs      # Child command runner & PID watcher (-w)
│   │       ├── agent_watch.rs  # Popular AI agent & process activity detector
│   │       ├── config.rs       # TOML config & settings (%APPDATA%\wakey\config.toml)
│   │       ├── scheduler.rs    # Daily/weekly schedule evaluator
│   │       ├── habits.rs       # History logger (JSONL) & pattern detector
│   │       ├── notify.rs       # Interactive Windows Toast notifications
│   │       └── ipc.rs          # Windows Named Pipe client/server
│   └── wakey-cli/              # Standalone CLI binary (wakey.exe)
│       └── src/main.rs
└── app/                        # Tauri v2 Desktop Dashboard + System Tray (wakey-app)
    ├── src-tauri/
    └── ui/                     # Clean HTML/CSS/TS dashboard UI
```

---

## 7. Phased Build Plan

- **Phase 1 — Rust Workspace & Core CLI (`caffeinate` parity):** `wakey-core` (`power.rs`, `process.rs`) + `wakey-cli` (`-d`, `-s`, `-a`, `-t`, `--until`, `-w`, `-- <cmd>`).
- **Phase 2 — Config, IPC & Agent Watch:** `config.rs`, `ipc.rs`, `agent_watch.rs` (popular AI agent presets + CPU/child-process activity detection + grace period), battery safety guard, `wakey status` / `stop` / `doctor`.
- **Phase 3 — Daily & Weekly Schedules + Habit Engine:** `scheduler.rs`, `habits.rs`, `notify.rs` (interactive Windows Toasts).
- **Phase 4 — Clean Tauri v2 Dashboard & System Tray:** `app/` with Home, Schedules, Agent Watch, Habits, and Settings views + tray icon.
- **Phase 5 — Polish & Windows Packaging:** Cross-compile/test on Windows (`x86_64-pc-windows-msvc`), unit tests, `README.md`, installer / Scoop / Winget manifest.
