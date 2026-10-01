// Claude Code adapter: hook payloads (https://code.claude.com/docs/en/hooks)
// in, `AgentEvent` out. Its output side is drip-hook.exe, which turns the
// island's one-word answer into Claude Code's PermissionRequest JSON.

use serde_json::Value;

use super::{base_name, cut, AgentEvent, DecisionRequest, Kind};

pub const AGENT: &str = "claude-code";

fn s<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str).filter(|v| !v.is_empty())
}

/// Claude Code tool name → neutral label. Unknown tools keep their own name.
fn label(tool: &str) -> String {
    match tool {
        "Bash" | "PowerShell" => "Run",
        "Read" => "Read",
        "Edit" | "MultiEdit" => "Edit",
        "Write" => "Write",
        "Grep" => "Search",
        "Glob" => "Find",
        "WebFetch" => "Fetch",
        "WebSearch" => "Web search",
        "Task" | "Agent" => "Agent",
        "NotebookEdit" => "Notebook",
        other => other,
    }
    .to_string()
}

/// One short line describing what the tool acts on.
fn target(tool: &str, input: &Value) -> String {
    let get = |k: &str| s(input, k);
    if let Some(c) = get("command") {
        return cut(c, 48);
    }
    if let Some(f) = get("file_path").or_else(|| get("notebook_path")) {
        return base_name(f);
    }
    if let Some(p) = get("pattern") {
        if tool == "Grep" {
            let place = get("path").map(|p| format!(" in {}", base_name(p))).unwrap_or_default();
            return format!("\"{}\"{place}", cut(p, 30));
        }
        return cut(p, 40);
    }
    if let Some(u) = get("url") {
        let bare = u.trim_start_matches("https://").trim_start_matches("http://");
        return cut(bare, 44);
    }
    if let Some(q) = get("query").or_else(|| get("description")) {
        return cut(q, 44);
    }
    String::new()
}

/// The full thing the user is asked to approve.
fn approval_detail(input: &Value) -> String {
    ["command", "url", "file_path", "notebook_path"]
        .iter()
        .find_map(|k| s(input, k).map(str::to_string))
        .unwrap_or_else(|| serde_json::to_string_pretty(input).unwrap_or_default())
}

/// Normalizes one hook payload. Events Drip doesn't use return None.
pub fn normalize(p: &Value, decision_timeout_ms: u64) -> Option<AgentEvent> {
    let kind = match s(p, "hook_event_name")? {
        "SessionStart" => Kind::SessionStart,
        "SessionEnd" => Kind::SessionEnd,
        "UserPromptSubmit" => Kind::Prompt,
        "PreToolUse" => Kind::ToolStart,
        "PostToolUse" => Kind::ToolEnd,
        "PostToolUseFailure" => Kind::ToolFailed,
        "PermissionRequest" => Kind::Permission,
        "Notification" => Kind::Notice,
        "Stop" => Kind::TurnDone,
        "StopFailure" => Kind::TurnFailed,
        _ => return None,
    };
    let mut e = AgentEvent::new(AGENT, s(p, "session_id").unwrap_or_default().to_string(), kind);
    e.cwd = s(p, "cwd").map(str::to_string);

    let tool_name = s(p, "tool_name");
    let input = p.get("tool_input").cloned().unwrap_or(Value::Null);
    if let Some(t) = tool_name {
        e.tool = Some(label(t));
        e.target = Some(target(t, &input));
    }
    e.key = s(p, "tool_use_id").map(str::to_string);

    match kind {
        Kind::Prompt => e.target = s(p, "prompt").map(|t| cut(t, 60)),
        Kind::Notice => e.target = s(p, "message").map(|t| cut(t, 60)),
        Kind::Permission => {
            e.needs_decision = true;
            e.detail = Some(approval_detail(&input));
            let can_remember = p
                .get("permission_suggestions")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty());
            e.request = Some(DecisionRequest { id: String::new(), timeout_ms: decision_timeout_ms, can_remember });
        }
        Kind::TurnFailed => {
            // `error` is a type such as rate_limit; the readable text is in
            // last_assistant_message and error_details.
            let first = s(p, "last_assistant_message").or_else(|| s(p, "error")).unwrap_or("Request failed");
            let mut lines = vec![first.to_string()];
            if let Some(d) = s(p, "error_details").filter(|d| *d != first) {
                lines.push(d.to_string());
            }
            e.detail = Some(lines.join("\n"));
        }
        _ => {}
    }
    Some(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base(name: &str) -> Value {
        json!({ "session_id": "abc", "cwd": "C:\\code\\weather-cli", "transcript_path": "x", "hook_event_name": name })
    }

    fn with(name: &str, extra: Value) -> Value {
        let mut v = base(name);
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    }

    #[test]
    fn tool_start_gets_a_neutral_label_and_target() {
        let e = normalize(
            &with("PreToolUse", json!({ "tool_name": "Read", "tool_input": { "file_path": "C:\\code\\weather-cli\\src\\forecast.ts" }, "tool_use_id": "toolu_1" })),
            108_000,
        )
        .unwrap();
        assert_eq!(e.agent, "claude-code");
        assert_eq!(e.session, "abc");
        assert_eq!(e.kind, Kind::ToolStart);
        assert_eq!(e.tool.as_deref(), Some("Read"));
        assert_eq!(e.target.as_deref(), Some("forecast.ts"));
        assert_eq!(e.cwd.as_deref(), Some("C:\\code\\weather-cli"));
        assert_eq!(e.key.as_deref(), Some("toolu_1"));
        assert!(!e.needs_decision);
    }

    #[test]
    fn targets_follow_the_prototype() {
        assert_eq!(target("Grep", &json!({ "pattern": "round1", "path": "C:\\p\\src" })), "\"round1\" in src");
        assert_eq!(target("WebFetch", &json!({ "url": "https://api.example.com/v1" })), "api.example.com/v1");
        assert_eq!(target("Bash", &json!({ "command": "x".repeat(60) })).chars().count(), 48);
        assert_eq!(label("PowerShell"), "Run");
        assert_eq!(label("mcp__github__create_issue"), "mcp__github__create_issue");
    }

    #[test]
    fn permission_needs_a_decision() {
        let e = normalize(
            &with("PermissionRequest", json!({
                "tool_name": "Bash",
                "tool_input": { "command": "npm test -- --run forecast", "description": "Run tests" },
                "permission_suggestions": [{ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test:*" }], "behavior": "allow", "destination": "localSettings" }]
            })),
            108_000,
        )
        .unwrap();
        assert_eq!(e.kind, Kind::Permission);
        assert!(e.needs_decision);
        assert_eq!(e.tool.as_deref(), Some("Run"));
        assert_eq!(e.detail.as_deref(), Some("npm test -- --run forecast"));
        assert_eq!(e.request, Some(DecisionRequest { id: String::new(), timeout_ms: 108_000, can_remember: true }));

        let plain = normalize(&with("PermissionRequest", json!({ "tool_name": "WebFetch", "tool_input": { "url": "https://x.dev" } })), 1).unwrap();
        assert!(!plain.request.unwrap().can_remember);
    }

    #[test]
    fn other_kinds() {
        let p = normalize(&with("UserPromptSubmit", json!({ "prompt": "Round the temps" })), 1).unwrap();
        assert_eq!((p.kind, p.target.as_deref()), (Kind::Prompt, Some("Round the temps")));
        let f = normalize(&with("StopFailure", json!({ "error": "rate_limit", "error_details": "429 Too Many Requests", "last_assistant_message": "API Error: Rate limit reached" })), 1).unwrap();
        assert_eq!(f.detail.as_deref(), Some("API Error: Rate limit reached\n429 Too Many Requests"));
        assert_eq!(normalize(&base("Stop"), 1).unwrap().kind, Kind::TurnDone);
        assert!(normalize(&base("PreCompact"), 1).is_none());
        assert!(normalize(&json!({ "cwd": "x" }), 1).is_none());
    }

    #[test]
    fn serializes_with_the_neutral_field_names() {
        let e = normalize(&base("SessionStart"), 1).unwrap();
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "session_start");
        assert_eq!(v["needsDecision"], false);
        for k in ["agent", "session", "kind", "tool", "target", "cwd", "needsDecision"] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
    }
}
