// Installs and removes Drip's Claude Code hooks in %USERPROFILE%\.claude\settings.json.
//
// Rules: show the exact change first, take a timestamped backup, merge with
// what is there (never drop anybody else's hooks or settings), and refuse to
// write if the file changed since the preview. Uninstall removes only the
// handlers whose command points at drip-hook.exe.
//
// The merge/backup/fingerprint approach is adapted from coucou
// (https://github.com/louis-cfm/coucou), MIT License. See THIRD_PARTY_NOTICES.md.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// Every event Drip listens to, with the timeout written to settings.json.
pub const HOOK_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 10),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PostToolUse", 10),
    ("PostToolUseFailure", 10),
    ("PermissionRequest", 120),
    ("Notification", 10),
    ("Stop", 10),
    ("StopFailure", 10),
];

const EXE_NAME: &str = "drip-hook.exe";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub install: bool,
    pub settings_path: String,
    pub hook_path: String,
    /// Where the backup will go, or None when there is no file to back up yet.
    pub backup_path: Option<String>,
    pub before: String,
    pub after: String,
    pub diff: String,
    pub changed: bool,
    /// True when writing also re-indents lines the user wrote by hand, so a
    /// line can show as removed and re-added without anything being lost.
    pub reformatted: bool,
    /// Identifies the bytes the preview was computed from.
    pub fingerprint: String,
    /// Timestamp in the backup name; `write` reuses it so the backup is the
    /// file the preview named.
    pub stamp: String,
}

/// `%USERPROFILE%\.claude\settings.json`, or `settings.json` in
/// `CLAUDE_CONFIG_DIR` when that is set, as Claude Code itself does.
pub fn settings_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("settings.json");
    }
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| ".".into());
    home.join(".claude").join("settings.json")
}

/// `%LOCALAPPDATA%\Drip\bin\drip-hook.exe`
pub fn hook_exe_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| ".".into());
    base.join("Drip").join("bin").join(EXE_NAME)
}

// ── Pure JSON work ───────────────────────────────────────────────────────────

/// Only a missing file means "start from nothing". Unreadable or invalid JSON
/// is an error, never an empty object we would then write back.
fn parse(bytes: &[u8], path: &Path) -> Result<Value, String> {
    let text = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    match serde_json::from_slice::<Value>(text) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(format!("{} isn't a JSON object. Drip won't touch it.", path.display())),
        Err(e) => Err(format!("{} isn't valid JSON ({e}). Fix it, then try again. Drip won't overwrite it.", path.display())),
    }
}

fn read(path: &Path) -> Result<(Value, Option<Vec<u8>>), String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok((parse(&bytes, path)?, Some(bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((json!({}), None)),
        Err(e) => Err(format!("Can't read {}: {e}", path.display())),
    }
}

fn handler_is_ours(handler: &Value) -> bool {
    handler
        .get("command")
        .and_then(Value::as_str)
        .map(|c| c.to_ascii_lowercase().replace('\\', "/").contains(EXE_NAME))
        .unwrap_or(false)
}

fn command_for(exe: &Path) -> String {
    // Forward slashes: valid for Windows, and safe if a shell ever sees it.
    exe.to_string_lossy().replace('\\', "/")
}

/// `hooks` with every Drip handler removed. A matcher group is dropped only if
/// removing ours left it empty; an event is dropped only if left empty.
fn strip_ours(hooks: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (event, groups) in hooks {
        let Some(list) = groups.as_array() else {
            out.insert(event.clone(), groups.clone());
            continue;
        };
        let mut kept = Vec::new();
        for group in list {
            let Some(handlers) = group.get("hooks").and_then(Value::as_array) else {
                kept.push(group.clone());
                continue;
            };
            if !handlers.iter().any(handler_is_ours) {
                kept.push(group.clone());
                continue;
            }
            let rest: Vec<Value> = handlers.iter().filter(|h| !handler_is_ours(h)).cloned().collect();
            if !rest.is_empty() {
                let mut g = group.clone();
                g["hooks"] = Value::Array(rest);
                kept.push(g);
            }
        }
        if !kept.is_empty() {
            out.insert(event.clone(), Value::Array(kept));
        }
    }
    out
}

pub fn with_ours(existing: &Value, exe: &Path) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let current = root.get("hooks").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut hooks = strip_ours(&current);
    for (event, timeout) in HOOK_EVENTS {
        let mut list = hooks.get(*event).and_then(Value::as_array).cloned().unwrap_or_default();
        list.push(json!({
            "hooks": [{
                "type": "command",
                "command": command_for(exe),
                "args": [event],
                "timeout": timeout,
            }]
        }));
        hooks.insert((*event).to_string(), Value::Array(list));
    }
    root.insert("hooks".into(), Value::Object(hooks));
    Value::Object(root)
}

pub fn without_ours(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let Some(current) = root.get("hooks").and_then(Value::as_object).cloned() else {
        return Value::Object(root);
    };
    let hooks = strip_ours(&current);
    if hooks.is_empty() {
        root.remove("hooks");
    } else {
        root.insert("hooks".into(), Value::Object(hooks));
    }
    Value::Object(root)
}

pub fn is_installed(settings: &Value) -> bool {
    settings
        .get("hooks")
        .and_then(Value::as_object)
        .map(|h| {
            h.values()
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(|g| g.get("hooks").and_then(Value::as_array))
                .flatten()
                .any(handler_is_ours)
        })
        .unwrap_or(false)
}

fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s
}

/// FNV-1a: only answers "is this still the file I showed?".
fn fingerprint(bytes: Option<&[u8]>) -> String {
    let Some(bytes) = bytes else { return "missing".into() };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn stamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

fn backup_path_for(path: &Path, stamp: &str) -> PathBuf {
    path.with_file_name(format!("settings.json.drip-backup-{stamp}"))
}

/// `YYYYMMDD-HHMMSS`, as `stamp()` makes it. Anything else is refused, so
/// the front end can't steer the backup name.
fn valid_stamp(s: &str) -> bool {
    s.len() == 15 && s.bytes().enumerate().all(|(i, b)| if i == 8 { b == b'-' } else { b.is_ascii_digit() })
}

/// A backup path that doesn't exist yet: never overwrite an earlier backup.
fn fresh_backup_path(path: &Path, stamp: &str) -> PathBuf {
    let first = backup_path_for(path, stamp);
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| path.with_file_name(format!("settings.json.drip-backup-{stamp}-{n}")))
        .find(|p| !p.exists())
        .expect("some backup name is free")
}

// ── Preview and write ────────────────────────────────────────────────────────

pub fn preview_at(install: bool, path: &Path, exe: &Path) -> Result<Preview, String> {
    let (current, bytes) = read(path)?;
    let next = if install { with_ours(&current, exe) } else { without_ours(&current) };
    // "Before" is the file as it is on disk when it exists, so the diff is the
    // exact change, formatting included.
    let before = match &bytes {
        Some(b) => String::from_utf8_lossy(b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b)).into_owned(),
        None => String::new(),
    };
    let after = pretty(&next);
    let norm = |t: &str| t.replace("
", "
").trim_end().to_string();
    let reformatted = bytes.is_some() && norm(&before) != norm(&pretty(&current));
    let stamp = stamp();
    Ok(Preview {
        install,
        settings_path: path.display().to_string(),
        hook_path: exe.display().to_string(),
        backup_path: bytes.as_ref().map(|_| backup_path_for(path, &stamp).display().to_string()),
        changed: current != next,
        diff: unified_diff(&before, &after),
        before,
        after,
        reformatted,
        fingerprint: fingerprint(bytes.as_deref()),
        stamp,
    })
}

/// Writes after a backup. Returns the backup path, if one was needed.
pub fn write_at(install: bool, path: &Path, exe: &Path, expected: &str, stamp: &str) -> Result<Option<String>, String> {
    if !valid_stamp(stamp) {
        return Err("Invalid backup timestamp. Nothing was written.".into());
    }
    let (current, bytes) = read(path)?;
    if fingerprint(bytes.as_deref()) != expected {
        return Err(format!("{} changed since the preview. Nothing was written; review the new preview.", path.display()));
    }
    let next = if install { with_ours(&current, exe) } else { without_ours(&current) };

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
    }
    let backup = match bytes {
        Some(_) => {
            let b = fresh_backup_path(path, stamp);
            std::fs::copy(path, &b).map_err(|e| format!("Backup failed, nothing was written: {e}"))?;
            Some(b.display().to_string())
        }
        None => None,
    };

    // Write beside the file and rename over it, so a failure never leaves half a file.
    let temp = path.with_extension(format!("json.drip-{}", std::process::id()));
    std::fs::write(&temp, pretty(&next)).map_err(|e| format!("Write failed: {e}"))?;
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Write failed: {e}"));
    }
    Ok(backup)
}

pub fn preview(install: bool) -> Result<Preview, String> {
    preview_at(install, &settings_path(), &hook_exe_path())
}

pub fn write(app: &AppHandle, install: bool, fingerprint: &str, stamp: &str) -> Result<Option<String>, String> {
    if install {
        copy_hook_exe(app)?;
    }
    write_at(install, &settings_path(), &hook_exe_path(), fingerprint, stamp)
}

pub fn installed() -> bool {
    read(&settings_path()).map(|(v, _)| is_installed(&v)).unwrap_or(false)
}

/// Where the freshly built relay is: next to the app when bundled (resources),
/// or in the workspace's target\release during `tauri dev`.
fn find_hook_exe(app: &AppHandle) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(p) = app.path().resolve(EXE_NAME, tauri::path::BaseDirectory::Resource) {
        candidates.push(p);
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join(EXE_NAME));
        candidates.push(dir.join("..").join("release").join(EXE_NAME));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Copies drip-hook.exe to %LOCALAPPDATA%\Drip\bin.
pub fn copy_hook_exe(app: &AppHandle) -> Result<(), String> {
    let src = find_hook_exe(app).ok_or("drip-hook.exe wasn't found next to Drip. Build it with `npm run hook`.")?;
    let dest = hook_exe_path();
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
    }
    let same = match (std::fs::read(&src), std::fs::read(&dest)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Ok(());
    }
    match std::fs::copy(&src, &dest) {
        Ok(_) => Ok(()),
        // A hook may be running and hold the old copy open; keep it.
        Err(_) if dest.exists() => Ok(()),
        Err(e) => Err(format!("Can't copy drip-hook.exe to {}: {e}", dest.display())),
    }
}

// ── A small line diff (LCS), plenty for a settings file ─────────────────────

fn unified_diff(before: &str, after: &str) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            out.push(format!("  {}", a[i]));
            i += 1;
            j += 1;
        } else if j >= m || (i < n && lcs[i + 1][j] >= lcs[i][j + 1]) {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe() -> PathBuf {
        PathBuf::from(r"C:\Users\me\AppData\Local\Drip\bin\drip-hook.exe")
    }

    fn mine() -> Value {
        json!({
            "model": "opus",
            "theme": "dark",
            "hooks": {
                "PreToolUse": [
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "my-linter.exe" }] }
                ],
                "Stop": [
                    { "hooks": [
                        { "type": "command", "command": "notify.sh" },
                        { "type": "command", "command": "C:/old/place/drip-hook.exe Stop" }
                    ] }
                ],
                "FileChanged": [{ "hooks": [{ "type": "command", "command": "keep.exe" }] }]
            }
        })
    }

    #[test]
    fn install_keeps_everything_else_and_registers_every_event() {
        let after = with_ours(&mine(), &exe());
        assert_eq!(after["model"], "opus");
        assert_eq!(after["hooks"]["FileChanged"], mine()["hooks"]["FileChanged"]);
        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre[0], mine()["hooks"]["PreToolUse"][0]);
        for (event, timeout) in HOOK_EVENTS {
            let groups = after["hooks"][*event].as_array().unwrap();
            let ours: Vec<&Value> = groups
                .iter()
                .flat_map(|g| g["hooks"].as_array().unwrap())
                .filter(|h| handler_is_ours(h))
                .collect();
            assert_eq!(ours.len(), 1, "{event} should have exactly one Drip handler");
            assert_eq!(ours[0]["args"], json!([event]));
            assert_eq!(ours[0]["timeout"], json!(timeout));
            assert_eq!(ours[0]["command"], "C:/Users/me/AppData/Local/Drip/bin/drip-hook.exe");
        }
        // The stale Drip handler sharing a group with notify.sh is replaced; notify.sh stays.
        let stop = after["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop[0]["hooks"], json!([{ "type": "command", "command": "notify.sh" }]));
        // Installing twice changes nothing.
        assert_eq!(with_ours(&after, &exe()), after);
    }

    #[test]
    fn uninstall_removes_only_drip_handlers() {
        let installed = with_ours(&mine(), &exe());
        let removed = without_ours(&installed);
        assert!(!is_installed(&removed));
        assert_eq!(removed["hooks"]["PreToolUse"], mine()["hooks"]["PreToolUse"]);
        assert_eq!(removed["hooks"]["Stop"], json!([{ "hooks": [{ "type": "command", "command": "notify.sh" }] }]));
        assert_eq!(removed["hooks"]["FileChanged"], mine()["hooks"]["FileChanged"]);
        assert!(removed["hooks"].get("SessionStart").is_none());
        // With nothing else there, the hooks key goes away entirely.
        let bare = without_ours(&with_ours(&json!({ "theme": "dark" }), &exe()));
        assert_eq!(bare, json!({ "theme": "dark" }));
    }

    #[test]
    fn bad_files_are_refused_never_treated_as_empty() {
        let p = Path::new("settings.json");
        assert!(parse(b"{ nope", p).is_err());
        assert!(parse(b"[1]", p).is_err());
        assert_eq!(parse(b"", p).unwrap(), json!({}));
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(br#"{"a":1}"#);
        assert_eq!(parse(&bom, p).unwrap(), json!({ "a": 1 }));
    }

    #[test]
    fn write_backs_up_first_and_refuses_a_file_that_changed() {
        let dir = std::env::temp_dir().join(format!("drip-hooks-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let original = br#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"notify.sh"}]}]}}"#;
        std::fs::write(&path, original).unwrap();

        let plan = preview_at(true, &path, &exe()).unwrap();
        assert!(plan.changed && plan.diff.contains("+ ") && plan.backup_path.is_some());
        assert!(plan.reformatted, "a compact file gets re-indented");
        let backup = write_at(true, &path, &exe(), &plan.fingerprint, &plan.stamp).unwrap().unwrap();
        // The backup is exactly the file the preview named.
        assert_eq!(Some(backup.clone()), plan.backup_path);
        assert_eq!(std::fs::read(&backup).unwrap(), original);
        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(is_installed(&after));
        assert_eq!(after["theme"], "dark");

        // Changed since the preview: refused, file untouched.
        let stale = preview_at(false, &path, &exe()).unwrap();
        std::fs::write(&path, br#"{"theme":"light"}"#).unwrap();
        assert!(write_at(false, &path, &exe(), &stale.fingerprint, &stale.stamp).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), br#"{"theme":"light"}"#);

        // No file yet: nothing to back up, file is created.
        let fresh = dir.join("sub").join("settings.json");
        let plan = preview_at(true, &fresh, &exe()).unwrap();
        assert!(plan.backup_path.is_none());
        assert_eq!(write_at(true, &fresh, &exe(), &plan.fingerprint, &plan.stamp).unwrap(), None);
        assert!(fresh.exists());

        // A second write with the same stamp keeps the first backup.
        let again = preview_at(false, &path, &exe()).unwrap();
        let second = write_at(false, &path, &exe(), &again.fingerprint, &backup[backup.len() - 15..]).unwrap().unwrap();
        assert_ne!(second, backup);
        assert_eq!(std::fs::read(&backup).unwrap(), original);

        // A file already in the standard layout isn't flagged as reformatted.
        let tidy = dir.join("tidy.json");
        std::fs::write(&tidy, "{
  \"theme\": \"dark\"
}
").unwrap();
        assert!(!preview_at(true, &tidy, &exe()).unwrap().reformatted);

        assert!(write_at(true, &path, &exe(), "x", "../../evil").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
