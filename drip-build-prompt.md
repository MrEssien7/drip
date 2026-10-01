# Build Drip: a Windows companion for Claude Code

Paste everything below into Claude Code, started inside your `drip` folder.

---

I want to build **Drip**, a Windows 10/11 desktop app made with **Tauri 2** (Rust backend, plain TypeScript front end, no UI framework). It hangs from the top edge of the screen, shows what my Claude Code sessions are doing, and lets me approve or deny Claude Code permission requests from it.

## Inputs in this folder

- `drip.html` is a working browser prototype. It is the design spec: the droplet character drawn on canvas, the island's four modes (hidden, peek, compact, expanded), its views (activity, approval, finished, error, chat), colors, fonts and animations. All behavior runs through one function, `applyHook(payload)`. Port it faithfully. Drop the "Simulate Claude Code" and "What each hook does" sections; they were only for the prototype.
- `coucou-ref/` is a clone of https://github.com/louis-cfm/coucou (MIT-licensed code). Its `windows/` folder solves the same problems. Read `windows/hook/src/main.rs`, `windows/hook/src/win.rs`, `windows/src-tauri/src/pipe.rs`, `windows/src-tauri/src/hooks.rs` and `windows/src-tauri/src/island.rs` before writing the matching parts. You may reuse code patterns from it with an MIT notice, but do NOT use its name, the Mochi character, its icons, its sounds or its media. Those are not covered by the MIT license.

## Before you write code

Read the Claude Code hooks docs at https://code.claude.com/docs/en/hooks. Use the documented input fields and the documented PermissionRequest output format. Where my prototype's sample payloads differ from the docs, the docs win.

## What to build

1. **Cargo workspace** with two binaries: the Tauri app (`drip`) and the hook relay (`drip-hook`).

2. **The island window**: transparent, frameless, always on top, hidden from the taskbar, centered at the top of the primary monitor's work area. Resize the window to match the island's size in each mode so it never blocks clicks on the rest of the screen. Hovering the very top-center edge of the screen wakes it (peek). Esc collapses it.

3. **`drip-hook.exe`** (the relay Claude Code runs on every hook event):
   - Reads the hook JSON from stdin (strip a UTF-8 BOM if present). Removes `tool_response` and `transcript_path` before forwarding.
   - Connects to the named pipe `\\.\pipe\drip-<current user SID>` with a 300 ms connect budget. Verifies the pipe server runs as the same user before sending anything.
   - For every event except `PermissionRequest`: send one JSON line, exit.
   - For `PermissionRequest`: wait up to 110 s for a one-word reply (`allow`, `always`, `deny`) and print the documented decision JSON to stdout.
   - Hard rule: never block or break Claude Code. Always exit 0. If the app is closed, slow, crashes, or the reply is anything unexpected, print nothing, so Claude Code asks in the terminal as usual.

4. **Pipe server in the Tauri backend**: tokio named pipe with `first_pipe_instance(true)`, one task per connection, 1 MB payload cap. Forward every payload to the front end as a `hook` event. For `PermissionRequest`, keep the connection open and wait for the front end to call a `decide(id, choice)` command. If no decision arrives within 108 s, close the connection without writing anything.

5. **Front end**: call the ported `applyHook` from a `listen("hook", ...)` handler. The approval buttons call `decide`. Hide the Chat tab for v1.

6. **Hook installer** (from a tray menu item "Install Claude Code hooks…"):
   - Copy `drip-hook.exe` to `%LOCALAPPDATA%\Drip\bin\`.
   - Show me a preview of the exact change to `%USERPROFILE%\.claude\settings.json` before writing.
   - Take a timestamped backup first. Merge with existing hooks, never overwrite or remove mine. Register these events: SessionStart, SessionEnd, UserPromptSubmit, PreToolUse, PostToolUse, PostToolUseFailure, PermissionRequest (timeout 120), Notification, Stop, StopFailure (timeout 10 for all the others).
   - "Uninstall hooks" removes only entries whose command points at `drip-hook.exe`.

7. **Tray icon** with: Show, Install hooks…, Uninstall hooks, Quit. Draw the icon in code from the droplet shape.

## How to work

- `git init` first. Build in these milestones, test each one, and commit after each:
  1. Window and island render with sample data and all four modes work.
  2. Relay and pipe: pipe a sample JSON file into `drip-hook.exe` from PowerShell and confirm the island reacts.
  3. Permission round trip: a sample PermissionRequest piped in prints the right JSON for Allow, Deny and Always, and prints nothing on timeout or when the app is closed.
  4. Hook installer, with the preview and backup.
  5. A real Claude Code session in a separate test folder triggers the island.
- Ask me before you write to my real `settings.json`. Everything else, proceed with reasonable choices and tell me what you chose.
- Keep a short README with build steps: `npm install`, `npm run tauri dev`, `npm run tauri build`.
