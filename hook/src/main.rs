//! drip-hook: the relay Claude Code runs on every hook event.
//!
//! Reads the hook JSON on stdin and hands it to Drip over `\\.\pipe\drip-<sid>`.
//! Only `PermissionRequest` waits for an answer, and turns it into the
//! documented decision JSON (https://code.claude.com/docs/en/hooks).
//!
//! Hard rule: never block or break Claude Code. We always exit 0. If Drip is
//! closed, slow, crashes, or answers anything unexpected, stdout stays empty
//! and Claude Code asks in the terminal as usual.
//!
//! The connect/deadline structure is adapted from coucou
//! (https://github.com/louis-cfm/coucou), MIT License. See THIRD_PARTY_NOTICES.md.
//!
//! Usage: `drip-hook <EventName>` (the JSON's `hook_event_name` wins when present).

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

mod win;

/// Budget for getting a pipe connection.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
/// Whole budget for an event nobody waits on.
const FIRE_AND_FORGET_BUDGET: Duration = Duration::from_secs(2);
/// How long a permission request may wait for the island.
const DECISION_BUDGET: Duration = Duration::from_secs(110);
/// `ERROR_PIPE_BUSY`: the server exists but every instance is taken. The only
/// error worth retrying.
const ERROR_PIPE_BUSY: i32 = 231;
/// Fields that are useless to the island and can be huge.
const DROPPED_FIELDS: &[&str] = &["tool_response", "transcript_path"];
/// Longest string forwarded for any one field; keeps us well under the app's
/// 1 MB cap even for a large Write.
const MAX_FIELD_LEN: usize = 4_000;

fn pipe_path() -> String {
    let key = win::current_user_sid()
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
    format!(r"\\.\pipe\drip-{key}")
}

/// Opens the pipe, retrying only while it is busy, and only if the server is us.
fn connect() -> Option<std::fs::File> {
    use std::os::windows::io::AsRawHandle;
    let path = pipe_path();
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => {
                let handle = windows::Win32::Foundation::HANDLE(file.as_raw_handle());
                return win::pipe_server_is_same_user(handle).then_some(file);
            }
            Err(err) => {
                if err.raw_os_error() != Some(ERROR_PIPE_BUSY) || Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }
    }
}

/// What came in on stdin, ready to forward.
struct Event {
    name: String,
    /// One JSON line for the pipe.
    line: String,
    /// `permission_suggestions` exactly as Claude Code sent them, untruncated,
    /// for an "always" answer.
    suggestions: Value,
}

fn main() {
    let Some(event) = read_event() else { std::process::exit(0) };

    let waits = event.name == "PermissionRequest";
    let budget = if waits { DECISION_BUDGET } else { FIRE_AND_FORGET_BUDGET };

    // Every blocking call runs on a worker. If it overruns, we stop waiting and
    // exit; the pipe handle dies with the process.
    let (tx, rx) = mpsc::channel::<Option<String>>();
    let line = event.line.clone();
    std::thread::spawn(move || {
        let _ = tx.send(talk(&line, waits));
    });

    if let Ok(Some(word)) = rx.recv_timeout(budget) {
        if let Some(out) = decision_json(&word, &event.suggestions) {
            let mut stdout = std::io::stdout();
            let _ = writeln!(stdout, "{out}");
            let _ = stdout.flush();
        }
    }
    std::process::exit(0);
}

/// The documented PermissionRequest output for a one-word reply. Anything else
/// prints nothing.
fn decision_json(word: &str, suggestions: &Value) -> Option<String> {
    let decision = match word.trim() {
        "allow" => json!({ "behavior": "allow" }),
        "always" => match suggestions.as_array() {
            Some(list) if !list.is_empty() => {
                json!({ "behavior": "allow", "updatedPermissions": list })
            }
            _ => json!({ "behavior": "allow" }),
        },
        "deny" => json!({ "behavior": "deny", "message": "Denied from Drip" }),
        _ => return None,
    };
    Some(
        json!({
            "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision }
        })
        .to_string(),
    )
}

fn read_event() -> Option<Event> {
    let mut raw = Vec::new();
    if std::io::stdin().read_to_end(&mut raw).is_err() || raw.is_empty() {
        return None;
    }
    parse_event(&raw, std::env::args().nth(1).unwrap_or_default())
}

fn parse_event(raw: &[u8], arg_event: String) -> Option<Event> {
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    let mut payload = serde_json::from_slice::<Value>(raw).ok()?;
    let map = payload.as_object_mut()?;

    let name = map
        .get("hook_event_name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(arg_event);
    if name.is_empty() {
        return None;
    }
    map.insert("hook_event_name".into(), Value::String(name.clone()));
    for field in DROPPED_FIELDS {
        map.remove(*field);
    }
    let suggestions = map.get("permission_suggestions").cloned().unwrap_or(Value::Null);

    truncate_strings(&mut payload);
    let mut line = payload.to_string();
    line.push('\n');
    Some(Event { name, line, suggestions })
}

/// Caps every string in the payload, cutting on a char boundary.
fn truncate_strings(value: &mut Value) {
    match value {
        Value::String(s) if s.len() > MAX_FIELD_LEN => {
            let mut end = MAX_FIELD_LEN;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
            s.push('…');
        }
        Value::Array(items) => items.iter_mut().for_each(truncate_strings),
        Value::Object(map) => map.values_mut().for_each(truncate_strings),
        _ => {}
    }
}

/// Connect, send, and for a permission request wait for one line back.
fn talk(line: &str, waits: bool) -> Option<String> {
    let mut pipe = connect()?;
    pipe.write_all(line.as_bytes()).ok()?;
    let _ = pipe.flush();
    if !waits {
        return None;
    }

    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') || buf.len() > 64 {
                    break;
                }
            }
        }
    }
    let answer = String::from_utf8_lossy(&buf).trim().to_string();
    (!answer.is_empty()).then_some(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_match_the_documented_shape() {
        let none = Value::Null;
        let allow: Value = serde_json::from_str(&decision_json("allow", &none).unwrap()).unwrap();
        assert_eq!(
            allow,
            json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": { "behavior": "allow" } } })
        );
        let deny: Value = serde_json::from_str(&decision_json("deny\n", &none).unwrap()).unwrap();
        assert_eq!(deny["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(deny["hookSpecificOutput"]["decision"]["message"], "Denied from Drip");
    }

    #[test]
    fn always_echoes_the_suggestions() {
        let s = json!([{ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test:*" }],
                         "behavior": "allow", "destination": "localSettings" }]);
        let out: Value = serde_json::from_str(&decision_json("always", &s).unwrap()).unwrap();
        assert_eq!(out["hookSpecificOutput"]["decision"]["updatedPermissions"], s);
        // No suggestions: a plain allow.
        let out: Value = serde_json::from_str(&decision_json("always", &json!([])).unwrap()).unwrap();
        assert!(out["hookSpecificOutput"]["decision"].get("updatedPermissions").is_none());
    }

    #[test]
    fn anything_else_prints_nothing() {
        for w in ["", "ask", "maybe", "ALLOW", "allow please", r#"{"behavior":"allow"}"#] {
            assert!(decision_json(w, &Value::Null).is_none(), "{w}");
        }
    }

    #[test]
    fn parsing_strips_bom_and_heavy_fields() {
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend_from_slice(
            br#"{"hook_event_name":"PostToolUse","transcript_path":"x","tool_response":{"a":1},"cwd":"C:\\p"}"#,
        );
        let e = parse_event(&raw, String::new()).unwrap();
        assert_eq!(e.name, "PostToolUse");
        assert!(!e.line.contains("transcript_path") && !e.line.contains("tool_response"));
        assert!(e.line.ends_with('\n'));
    }

    #[test]
    fn argv_names_the_event_when_json_does_not() {
        let e = parse_event(br#"{"cwd":"x"}"#, "Stop".into()).unwrap();
        assert!(e.line.contains(r#""hook_event_name":"Stop""#));
        assert!(parse_event(b"not json", "Stop".into()).is_none());
    }

    #[test]
    fn long_strings_are_cut_on_a_char_boundary() {
        let mut v = json!({ "tool_input": { "content": "é".repeat(5000) } });
        truncate_strings(&mut v);
        let s = v["tool_input"]["content"].as_str().unwrap();
        assert!(s.len() <= MAX_FIELD_LEN + 4 && s.ends_with('…'));
    }
}
