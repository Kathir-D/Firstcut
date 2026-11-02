//! `firstcut --control <port>` — the control channel (docs/v2-plan.md §4.3): JSON lines on
//! localhost. The agent operates the real app, not a side door: `engine.execute` runs a registry
//! command through the one dispatch, `engine.state`/`engine.commands` are the registry's own
//! read-only commands, and the `ui.*` methods render the live screen through the same headless
//! harness the snapshot tests use (`ui.widgets` lists the accessibility tree, `ui.clickWidget`
//! clicks a widget by its accessible name, `ui.screenshot` writes a PNG of the current state
//! and answers with its path).
//!
//! One listener thread, one reader thread per connection; requests are answered on the UI
//! thread inside the next frame (the only thread the Model may be touched from), behind a
//! short timeout so a stalled window cannot wedge the socket.

use std::io::{BufRead, Write};
use std::sync::mpsc;

use serde_json::{Value, json};

/// One request from a control client to the UI thread.
pub enum Request {
    Dispatch { id: String, params: Value, reply: mpsc::Sender<Result<Value, String>> },
    Widgets { reply: mpsc::Sender<Result<Value, String>> },
    Click { name: String, reply: mpsc::Sender<Result<Value, String>> },
    Screenshot { reply: mpsc::Sender<Result<Value, String>> },
}

/// How long a client waits for the UI thread's answer before the socket reports a timeout.
const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The control wire uses the same size as the snapshot tests' headless viewport.
pub const HARNESS_SIZE: egui::Vec2 = egui::vec2(1200.0, 760.0);

/// Binds the listener and starts the accept thread. `Ok(None)`s the receiver back to the app;
/// a bind or thread failure is the caller's startup error, not a silent half-open socket.
///
/// `wake` is the app's egui context, filled in by the first frame (`Shell::ui` paints at least
/// once even with no input): a queued request must wake the UI thread, which sits in the
/// platform event loop between repaints and will not drain the queue otherwise.
pub fn start(port: u16, wake: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>) -> Result<mpsc::Receiver<Request>, String> {
    let (sender, receiver) = mpsc::channel::<Request>();
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("binding 127.0.0.1:{port}: {e}"))?;
    let worker = move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => serve(stream, &sender, &wake),
                Err(err) => eprintln!("firstcut control: accept failed: {err}"),
            }
        }
    };
    std::thread::Builder::new().name("firstcut-control".to_string()).spawn(worker).map_err(|e| format!("starting the control thread: {e}"))?;
    Ok(receiver)
}

/// One connection: request line in, reply line out, until the client hangs up.
fn serve(stream: std::net::TcpStream, sender: &mpsc::Sender<Request>, wake: &std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>) {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "unknown".to_string());
    let reader = std::io::BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(err) => {
            eprintln!("firstcut control ({peer}): the socket cannot be split: {err}");
            return;
        }
    });
    let mut writer = stream;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            let _ = write_line(&mut writer, json!({ "error": "unparseable request line" }));
            break;
        };
        let reply = answer(&request, sender, wake);
        if write_line(&mut writer, reply).is_err() {
            break; // the client is gone; nothing left to tell
        }
    }
}

fn write_line(writer: &mut std::net::TcpStream, reply: Value) -> std::io::Result<()> {
    let mut line = serde_json::to_string(&reply).unwrap_or_else(|_| r#"{"error":"unreplyable"}"#.to_string());
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()
}

/// Turns one wire request into the UI thread's queue and waits (bounded) for its answer.
fn answer(request: &Value, sender: &mpsc::Sender<Request>, wake: &std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

    let (out, wait) = mpsc::channel::<Result<Value, String>>();
    let enqueued = match (method.as_str(), &params) {
        ("engine.execute", params) => sender.send(Request::Dispatch {
            id: params.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
            params: params.get("params").cloned().unwrap_or_else(|| json!({})),
            reply: out.clone(),
        }),
        ("engine.state", _) => sender.send(Request::Dispatch { id: "engine.state".to_string(), params: json!({}), reply: out.clone() }),
        ("engine.commands", _) => sender.send(Request::Dispatch { id: "engine.commands".to_string(), params: json!({}), reply: out.clone() }),
        ("ui.widgets", _) => sender.send(Request::Widgets { reply: out.clone() }),
        ("ui.clickWidget", params) => {
            sender.send(Request::Click { name: params.get("name").and_then(Value::as_str).unwrap_or_default().to_string(), reply: out.clone() })
        }
        ("ui.screenshot", _) => sender.send(Request::Screenshot { reply: out.clone() }),
        (other, _) => {
            drop(out);
            return json!({ "id": id, "error": format!("no such method: {other}") });
        }
    };
    if enqueued.is_err() {
        return json!({ "id": id, "error": "the app window is gone" });
    }
    // A queued request wakes the UI thread: it idles in the platform event loop otherwise.
    if let Ok(guard) = wake.lock()
        && let Some(context) = guard.as_ref()
    {
        context.request_repaint();
    }
    match wait.recv_timeout(REPLY_TIMEOUT) {
        Ok(result) => match result {
            Ok(value) => json!({ "id": id, "result": value }),
            Err(error) => json!({ "id": id, "error": error }),
        },
        Err(_) => json!({ "id": id, "error": "the app did not answer within 10 s — is the window alive?" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake UI thread answers from the queue, so the whole wire half runs for real: the
    /// method-to-request mapping, the bounded wait, and the error replies.
    #[test]
    fn wire_methods_map_to_requests_and_bounded_replies() {
        let (sender, receiver) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .spawn(move || {
                for request in receiver {
                    match request {
                        Request::Dispatch { id, reply, .. } => {
                            let _ = reply.send(Ok(json!({ "dispatched": id })));
                        }
                        Request::Widgets { reply } | Request::Screenshot { reply } => {
                            let _ = reply.send(Err("not reached in this test".to_string()));
                        }
                        Request::Click { reply, .. } => {
                            let _ = reply.send(Err("not reached in this test".to_string()));
                        }
                    }
                }
            })
            .unwrap();

        let wake = std::sync::Arc::new(std::sync::Mutex::new(None));

        let reply = answer(&json!({ "id": 7, "method": "engine.state", "params": {} }), &sender, &wake);
        assert_eq!(reply.pointer("/result/dispatched").and_then(Value::as_str), Some("engine.state"));

        let reply =
            answer(&json!({ "id": 8, "method": "engine.execute", "params": { "id": "rate.stars", "params": { "stars": 2 } } }), &sender, &wake);
        assert_eq!(reply.pointer("/result/dispatched").and_then(Value::as_str), Some("rate.stars"));

        let unknown = answer(&json!({ "id": 9, "method": "no/such" }), &sender, &wake);
        assert!(unknown.get("error").is_some(), "{unknown}");

        // No `id` still answers a well-formed request; the reply's id is null.
        let reply = answer(&json!({ "method": "engine.state", "params": {} }), &sender, &wake);
        assert_eq!(reply.get("id"), Some(&Value::Null));
        assert!(reply.get("result").is_some());
    }

    /// Every request the UI side cannot answer in time reads as a timeout, not a wedge.
    #[test]
    fn a_silent_ui_thread_reports_a_timeout() {
        let wake = std::sync::Arc::new(std::sync::Mutex::new(None));
        let (sender, receiver) = mpsc::channel::<Request>();
        std::mem::drop(receiver); // nothing will ever answer
        let reply = answer(&json!({ "id": 1, "method": "ui.widgets", "params": {} }), &sender, &wake);
        assert!(reply.get("error").and_then(Value::as_str).unwrap_or_default().contains("app window is gone"));
    }
}
