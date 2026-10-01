// Agent-neutral events. Each coding agent gets an adapter that turns its own
// payloads into `AgentEvent`; the island only ever sees these. Claude Code is
// the only adapter for now.

pub mod claude_code;

use serde::Serialize;

/// What happened, independent of which agent reported it.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    SessionStart,
    SessionEnd,
    /// The user sent a prompt.
    Prompt,
    ToolStart,
    ToolEnd,
    ToolFailed,
    /// The agent asks the user before running a tool.
    Permission,
    /// Informational message from the agent.
    Notice,
    /// The agent finished its turn.
    TurnDone,
    /// The turn ended on an error.
    TurnFailed,
}

/// A permission question the island can answer.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRequest {
    /// Filled in by the pipe server; passed back to `decide(id, choice)`.
    pub id: String,
    /// How long the island has before the agent asks in its own UI.
    pub timeout_ms: u64,
    /// True when the agent can remember an "always allow" answer.
    pub can_remember: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    /// Which adapter produced this, e.g. "claude-code".
    pub agent: &'static str,
    /// The agent's session identifier.
    pub session: String,
    pub kind: Kind,
    /// Neutral tool label: Run, Read, Edit, Write, Search, Find, Fetch, …
    pub tool: Option<String>,
    /// Short human-readable target: a file name, a command, a URL.
    pub target: Option<String>,
    pub cwd: Option<String>,
    pub needs_decision: bool,

    /// Correlates a tool's start with its end, when the agent provides one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Longer text: the full command for a permission card, an error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<DecisionRequest>,
}

impl AgentEvent {
    pub fn new(agent: &'static str, session: String, kind: Kind) -> Self {
        Self {
            agent,
            session,
            kind,
            tool: None,
            target: None,
            cwd: None,
            needs_decision: false,
            key: None,
            detail: None,
            request: None,
        }
    }
}

/// Shortens to `n` characters, ending with an ellipsis when cut.
pub fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Last component of a Windows or POSIX path.
pub fn base_name(p: &str) -> String {
    p.split(['\\', '/']).filter(|s| !s.is_empty()).last().unwrap_or(p).to_string()
}
