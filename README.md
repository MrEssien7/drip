# Drip

A Windows 10/11 companion for Claude Code. It hangs from the top edge of your
screen, shows what your Claude Code sessions are doing, and lets you approve
or deny permission requests without going back to the terminal.

Built with Tauri 2 (Rust) and plain TypeScript.

## Build

Needs Node 20+, Rust (MSVC toolchain), and Visual Studio Build Tools with the
"Desktop development with C++" workload.

```
npm install
npm run tauri dev      # run with hot reload; starts on a sample session
npm run tauri build    # installer in target/release/bundle/nsis/
```

Both commands build `drip-hook.exe` first (`npm run hook`).
`npm run icons` redraws the app icons from the droplet code.

## Using it

1. Start Drip. Its droplet appears in the notification area.
2. Pick **Install Claude Code hooks…** from the tray menu. Drip shows the exact
   change to `%USERPROFILE%\.claude\settings.json`, backs the file up next to
   itself (`settings.json.drip-backup-YYYYMMDD-HHMMSS`), and only then writes.
   Your own hooks and settings are kept. The relay is copied to
   `%LOCALAPPDATA%\Drip\bin\drip-hook.exe`.
3. Start Claude Code. Drip wakes on `SessionStart`.

Hover the top-centre edge of the screen to wake the island; click it to cycle
peek → compact → expanded; Esc collapses it (after you've clicked it once,
since it doesn't take keyboard focus on its own).

**Uninstall hooks** removes only the hook entries that run `drip-hook.exe`.

The same previews open from the command line with `Drip.exe --install-hooks`
or `Drip.exe --uninstall-hooks` (handed to Drip if it is already running).
If you set `CLAUDE_CONFIG_DIR`, Drip edits `settings.json` there instead of
`%USERPROFILE%\.claude`, as Claude Code does.

## How it works

```
Claude Code ──stdin JSON──▶ drip-hook.exe ──\\.\pipe\drip-<SID>──▶ Drip (Rust) ──"hook" event──▶ island (applyHook)
            ◀─decision JSON─              ◀──── allow / always / deny ───────── decide(id, choice) ◀─ buttons
```

- The backend normalizes each payload into an agent-neutral event,
  `{agent, session, kind, tool, target, cwd, needsDecision}` (plus `key`,
  `detail`, `request`), in `src-tauri/src/agents/`. The island only sees those
  events. Claude Code is the one adapter (`agents/claude_code.rs`); another
  agent would get its own adapter next to it.
- `drip-hook.exe` strips `tool_response` and `transcript_path`, connects within
  300 ms, and checks the pipe server runs as you before sending anything.
- Only `PermissionRequest` waits: up to 110 s in the relay, 108 s in the app.
  **Allow** and **Deny** print the documented `hookSpecificOutput` JSON;
  **Always allow** also echoes Claude Code's `permission_suggestions` back as
  `updatedPermissions`.
- If Drip is closed, slow or crashes, or you don't answer, the relay prints
  nothing and exits 0, so Claude Code asks in the terminal as usual.
- Claude Code shows its own prompt while the island waits, so you can answer
  either place. A card leaves the island once its session moves on (the tool
  runs or fails, or the turn ends).
- Several sessions can ask at once: cards wait in line, each with its own
  countdown. Questions (`AskUserQuestion`) and plan reviews (`ExitPlanMode`)
  can't be answered with Allow/Deny, so they stay in the terminal and the
  island only notes them.
- Approval buttons ignore presses in a card's first 400 ms, and a click only
  counts if it started on that button, so a double-click can't answer the
  next card.

## Test without Claude Code

With Drip running (`npm run tauri dev`), from PowerShell:

```powershell
Get-Content samples\session-start.json | target\release\drip-hook.exe
Get-Content samples\permission-request.json | target\release\drip-hook.exe   # waits for a click
```

## Licence

MIT. Some code patterns come from coucou (MIT); see THIRD_PARTY_NOTICES.md.
