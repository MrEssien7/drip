// Named-pipe server for drip-hook.
//
// `\\.\pipe\drip-<sid>`, one task per connection. Every payload is normalized
// into an agent-neutral `AgentEvent` (see agents/) and sent to the island as a
// `hook` event. An event that needs a decision keeps its connection open
// until the island calls `decide(id, choice)`; we then write the bare word
// (`allow`, `always` or `deny`) and drip-hook turns it into Claude Code's JSON.
// No decision within 108 s, or an "ask", closes the connection with nothing
// written, so Claude Code asks in the terminal.
//
// Structure adapted from coucou (https://github.com/louis-cfm/coucou), MIT
// License. See THIRD_PARTY_NOTICES.md.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::oneshot;

use crate::agents::claude_code;
use crate::island::WINDOW_LABEL;

/// Just under drip-hook's own 110 s, so we always let go first.
pub const DECISION_TIMEOUT: Duration = Duration::from_secs(108);
const MAX_PAYLOAD: usize = 1 << 20;

/// Permission requests waiting for the island, by request id.
#[derive(Default)]
pub struct Pending(Mutex<HashMap<String, oneshot::Sender<String>>>);

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Must match drip-hook's `pipe_path()` exactly.
pub fn pipe_name() -> String {
    let key = crate::win::current_user_sid()
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
    format!(r"\\.\pipe\drip-{key}")
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let name = pipe_name();
        // Refuse to join a pipe someone else already owns under our name.
        let mut server = match ServerOptions::new().first_pipe_instance(true).create(&name) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("[drip] cannot open {name}: {err}");
                return;
            }
        };
        loop {
            if server.connect().await.is_err() {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            let next = match ServerOptions::new().create(&name) {
                Ok(s) => s,
                Err(err) => {
                    eprintln!("[drip] cannot reopen {name}: {err}");
                    return;
                }
            };
            let connected = std::mem::replace(&mut server, next);
            let app = app.clone();
            tauri::async_runtime::spawn(async move { handle(app, connected).await });
        }
    });
}

/// Reads one line, capped at 1 MB. Anything bigger is dropped.
async fn read_line(pipe: &mut NamedPipeServer) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.iter().position(|b| *b == b'\n') {
                    buf.truncate(i);
                    break;
                }
                if buf.len() > MAX_PAYLOAD {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
    (buf.len() <= MAX_PAYLOAD).then_some(buf)
}

async fn handle(app: AppHandle, mut pipe: NamedPipeServer) {
    let Some(line) = read_line(&mut pipe).await else { return };
    let Ok(payload) = serde_json::from_slice::<Value>(&line) else { return };
    // The pipe only carries Claude Code payloads today; another agent would get
    // its own adapter (and its own pipe or a field saying which agent it is).
    let timeout_ms = DECISION_TIMEOUT.as_millis() as u64;
    let Some(mut event) = claude_code::normalize(&payload, timeout_ms) else {
        let _ = pipe.disconnect();
        return;
    };

    if !event.needs_decision {
        let _ = app.emit_to(WINDOW_LABEL, "hook", event);
        let _ = pipe.disconnect();
        return;
    }

    let id = format!("{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let (tx, rx) = oneshot::channel::<String>();
    app.state::<Pending>().0.lock().unwrap().insert(id.clone(), tx);
    if let Some(req) = event.request.as_mut() {
        req.id = id.clone();
    }
    let _ = app.emit_to(WINDOW_LABEL, "hook", event);

    // Wait for the island, the deadline, or the relay going away (Claude Code
    // killed it, or it gave up). The relay never writes again, so any read
    // that completes means it is gone.
    let mut probe = [0u8; 1];
    let answer = tokio::select! {
        r = tokio::time::timeout(DECISION_TIMEOUT, rx) => r.ok().and_then(Result::ok),
        _ = pipe.read(&mut probe) => None,
    };
    let was_pending = app.state::<Pending>().0.lock().unwrap().remove(&id).is_some();
    if was_pending {
        // Nobody answered on the island: take the card down.
        let _ = app.emit_to(WINDOW_LABEL, "permission-gone", &id);
    }

    if let Some(word) = answer {
        if matches!(word.as_str(), "allow" | "always" | "deny") {
            let _ = pipe.write_all(format!("{word}\n").as_bytes()).await;
            let _ = pipe.flush().await;
        }
    }
    // Anything else (ask, timeout, island gone): close with nothing written.
    let _ = pipe.disconnect();
}

/// The island's answer for request `id`: `allow`, `always`, `deny`, or `ask`
/// to hand the question back to the terminal.
pub fn decide(app: &AppHandle, id: &str, choice: &str) {
    let sender = app.state::<Pending>().0.lock().unwrap().remove(id);
    if let Some(tx) = sender {
        let _ = tx.send(choice.to_string());
    }
}
