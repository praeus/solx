//! `solx-fake-background`, the background command `tests/background.rs`
//! drives: speaks the protocol in
//! `solx_surface::background` over stdin/stdout, std only, one thread per
//! request so responses can come back out of order.
//!
//! Ops: `echo` (returns its params), `sleep {ms, value}`, `pid`, `crash`
//! (exits 3), `host {op, params}` (calls the host and returns its answer,
//! passing on the request's `invocation_id`), `grandchild` (starts a
//! long-lived child process and returns its pid), and `shutdown`, which it
//! answers and exits on unless started with `--ignore-shutdown`.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

type Pending = Arc<Mutex<HashMap<u64, Sender<Value>>>>;

fn send(out: &Mutex<std::io::Stdout>, message: &Value) {
    let mut out = out.lock().unwrap();
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

fn main() {
    let ignore_shutdown = std::env::args().any(|a| a == "--ignore-shutdown");
    eprintln!("fake background up as {:?}", std::env::var("SOLX_BACKGROUND"));
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let pending: Pending = Arc::default();
    let next_id = Arc::new(AtomicU64::new(1));

    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let message: Value = serde_json::from_str(&line).expect("protocol line");
        if message["kind"] == "response" {
            let id = message["id"].as_u64().unwrap();
            if let Some(tx) = pending.lock().unwrap().remove(&id) {
                let _ = tx.send(message);
            }
            continue;
        }
        let id = message["id"].as_u64().unwrap();
        let op = message["op"].as_str().unwrap_or_default().to_string();
        if op == "shutdown" {
            if ignore_shutdown {
                continue;
            }
            send(&out, &json!({"kind": "response", "id": id, "ok": true, "result": null}));
            std::process::exit(0);
        }
        let (out, pending, next_id) = (out.clone(), pending.clone(), next_id.clone());
        std::thread::spawn(move || {
            let params = message["params"].clone();
            let result: Result<Value, String> = match op.as_str() {
                "echo" => Ok(params),
                "sleep" => {
                    std::thread::sleep(Duration::from_millis(params["ms"].as_u64().unwrap_or(0)));
                    Ok(params["value"].clone())
                }
                "pid" => Ok(json!(std::process::id())),
                "crash" => std::process::exit(3),
                "grandchild" => {
                    let child = if cfg!(windows) {
                        std::process::Command::new("ping")
                            .args(["-n", "600", "127.0.0.1"])
                            .stdout(std::process::Stdio::null())
                            .spawn()
                    } else {
                        std::process::Command::new("sleep")
                            .arg("600")
                            .stdout(std::process::Stdio::null())
                            .spawn()
                    };
                    child.map(|c| json!(c.id())).map_err(|e| e.to_string())
                }
                "host" => {
                    let host_id = next_id.fetch_add(1, Ordering::SeqCst);
                    let (tx, rx) = channel();
                    pending.lock().unwrap().insert(host_id, tx);
                    send(
                        &out,
                        &json!({
                            "kind": "request",
                            "id": host_id,
                            "op": params["op"],
                            "params": params["params"],
                            "invocation_id": message["invocation_id"],
                        }),
                    );
                    let answer = rx.recv().unwrap();
                    if answer["ok"] == true {
                        Ok(answer["result"].clone())
                    } else {
                        Err(answer["error"].as_str().unwrap_or_default().to_string())
                    }
                }
                other => Err(format!("unknown op '{other}'")),
            };
            let response = match result {
                Ok(result) => json!({"kind": "response", "id": id, "ok": true, "result": result}),
                Err(error) => json!({"kind": "response", "id": id, "ok": false, "error": error}),
            };
            send(&out, &response);
        });
    }
}
