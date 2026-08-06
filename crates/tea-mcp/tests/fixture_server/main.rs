#![forbid(unsafe_code)]

use std::{
    env, fs,
    io::{self, BufRead, Write},
    path::Path,
    process::{self, Command, Stdio},
    thread,
    time::Duration,
};

use serde_json::{Value, json};

#[allow(clippy::too_many_lines)] // One hermetic entrypoint makes every failure mode explicit.
pub fn main() {
    let arguments = env::args().collect::<Vec<_>>();
    let mode = arguments.get(1).map_or("normal", String::as_str);
    if mode == "crash" {
        process::exit(17);
    }
    if mode == "child-sleeper" {
        sleep_forever();
    }

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut stdout = io::stdout().lock();
    let Some(Ok(initialize_line)) = lines.next() else {
        return;
    };
    let Ok(initialize) = serde_json::from_str::<Value>(&initialize_line) else {
        return;
    };

    let reconnect_scenario = arguments.get(2).map_or("stale-match", String::as_str);
    let reconnect_launch = if mode == "reconnect" {
        let Some(state_path) = arguments.get(3) else {
            return;
        };
        let launch = increment_counter(state_path);
        if let Some(marker) = arguments.get(4) {
            append_marker(Some(marker), &format!("launch:{launch}\n"));
        }
        if reconnect_scenario == "retry-match" && matches!(launch, 2..=4) {
            process::exit(19);
        }
        if reconnect_scenario == "blocked-match" && launch == 2 {
            let Some(gate) = arguments.get(5) else {
                return;
            };
            while !Path::new(gate).exists() {
                thread::sleep(Duration::from_millis(10));
            }
        }
        launch
    } else {
        0
    };

    let manager_scenario = arguments.get(2).map_or("ready", String::as_str);
    if mode == "manager" {
        if let Some(marker) = arguments.get(3) {
            append_marker(Some(marker), "started\n");
        }
        if matches!(manager_scenario, "gated" | "fail-after-gate") {
            let Some(gate) = arguments.get(4) else {
                return;
            };
            while !Path::new(gate).exists() {
                thread::sleep(Duration::from_millis(10));
            }
        }
        if manager_scenario == "fail-after-gate" {
            process::exit(18);
        }
    }

    match (mode, arguments.get(2).map(String::as_str)) {
        ("slow", Some("partial")) => {
            let _ = stdout.write_all(b"{\"jsonrpc\":\"2.0\"");
            let _ = stdout.flush();
            sleep_forever();
        }
        ("slow", _) => sleep_forever(),
        ("malformed", Some("oversized")) => {
            let _ = stdout.write_all(b"{\"oversized\":\"");
            let _ = stdout.write_all(&vec![b'x'; 8 * 1024]);
            let _ = stdout.write_all(b"\"}\n");
            let _ = stdout.flush();
            return;
        }
        ("malformed", Some("incomplete")) => {
            let _ = stdout.write_all(b"{\"jsonrpc\":\"2.0\"");
            let _ = stdout.flush();
            return;
        }
        ("malformed", Some("invalid-utf8")) => {
            let _ = stdout.write_all(&[0xff, b'\n']);
            let _ = stdout.flush();
            sleep_forever();
        }
        ("malformed", None) => {
            let _ = stdout.write_all(b"not-json\n");
            let _ = stdout.flush();
            return;
        }
        _ => {}
    }

    let Some(id) = initialize.get("id").cloned() else {
        return;
    };
    let protocol_version =
        if mode == "reconnect" && reconnect_scenario == "protocol-drift" && reconnect_launch > 1 {
            "2026-07-28"
        } else {
            "2025-11-25"
        };
    let implementation_version = if (mode == "manager" && manager_scenario == "identity-v2")
        || (mode == "reconnect" && reconnect_scenario == "identity-drift" && reconnect_launch > 1)
    {
        "0.2.0"
    } else {
        "0.1.0"
    };
    write_json(
        &mut stdout,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": protocol_version,
                "capabilities": {"tools": {"listChanged": mode == "reconnect"}},
                "serverInfo": {
                    "name": "tea-mcp-fixture",
                    "version": implementation_version
                }
            }
        }),
    );

    let Some(Ok(initialized_notification_line)) = lines.next() else {
        return;
    };
    let Ok(initialized) = serde_json::from_str::<Value>(&initialized_notification_line) else {
        return;
    };
    if initialized.get("method").and_then(Value::as_str) != Some("notifications/initialized") {
        return;
    }

    match mode {
        "normal" => {
            if let Some(path) = arguments.get(2) {
                write_state(path, arguments.len());
            }
            if let Some(bytes) = arguments
                .get(3)
                .and_then(|value| value.parse::<usize>().ok())
            {
                write_stderr(bytes);
            }
            request_loop(lines, &mut stdout, false, false);
        }
        "secret-stderr" => {
            if let Some(value) = arguments.get(2) {
                write_stderr_text(value);
            }
            request_loop(lines, &mut stdout, false, false);
        }
        "flood" => {
            for _ in 0..2_048 {
                write_json(
                    &mut stdout,
                    &json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}),
                );
            }
            sleep_forever();
        }
        "ignore-cancel" => request_loop(lines, &mut stdout, true, false),
        "spawn-child" => {
            let Some(pid_path) = arguments.get(2) else {
                return;
            };
            let Ok(executable) = env::current_exe() else {
                return;
            };
            let Ok(child) = Command::new(executable)
                .arg("child-sleeper")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                return;
            };
            let _ = fs::write(pid_path, child.id().to_string());
            sleep_forever();
        }
        "ignore-shutdown" => sleep_forever(),
        "manager" => manager_request_loop(lines, &mut stdout, arguments.get(3).map(String::as_str)),
        "reconnect" => reconnect_request_loop(
            lines,
            &mut stdout,
            reconnect_scenario,
            reconnect_launch,
            arguments.get(4).map(String::as_str),
            arguments.get(5).map(String::as_str),
        ),
        "catalog" => catalog_request_loop(
            lines,
            &mut stdout,
            arguments.get(2).map_or("empty", String::as_str),
        ),
        "execute" => execute_request_loop(
            lines,
            &mut stdout,
            arguments.get(2).map_or("success", String::as_str),
            arguments.get(3).map(String::as_str),
        ),
        "malformed" => match arguments.get(2).map(String::as_str) {
            Some("unknown") => {
                write_json(&mut stdout, &json!({"jsonrpc":"2.0","id":999,"result":{}}));
                sleep_forever();
            }
            Some("duplicate") => request_loop(lines, &mut stdout, false, true),
            _ => {}
        },
        _ => {}
    }
}

fn reconnect_request_loop(
    lines: impl Iterator<Item = io::Result<String>>,
    stdout: &mut impl Write,
    scenario: &str,
    launch: u64,
    marker: Option<&str>,
    gate: Option<&str>,
) {
    for line in lines {
        let Ok(message) =
            line.and_then(|line| serde_json::from_str::<Value>(&line).map_err(io::Error::other))
        else {
            break;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        match message.get("method").and_then(Value::as_str) {
            Some("tools/list") => {
                let mut tool = execution_tool();
                if scenario == "catalog-drift" && launch > 1 {
                    tool["description"] = json!("Changed reconnect fixture descriptor.");
                }
                write_json(
                    stdout,
                    &json!({"jsonrpc":"2.0","id":id,"result":{"tools":[tool]}}),
                );
                if launch == 1
                    && !matches!(
                        scenario,
                        "stale-during-call" | "crash-during-call" | "shutdown-during-call"
                    )
                {
                    write_json(
                        stdout,
                        &json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}),
                    );
                }
            }
            Some("tools/call") => {
                append_marker(marker, "called\n");
                if launch == 1 && scenario == "crash-during-call" {
                    return;
                }
                if launch == 1 && scenario == "stale-during-call" {
                    write_json(
                        stdout,
                        &json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}),
                    );
                    let Some(gate) = gate else {
                        return;
                    };
                    while !Path::new(gate).exists() {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
                if scenario == "shutdown-during-call" {
                    sleep_forever();
                }
                write_json(stdout, &success_response(&id, "reconnected"));
            }
            _ => write_json(stdout, &json!({"jsonrpc":"2.0","id":id,"result":{}})),
        }
    }
    append_marker(marker, &format!("stopped:{launch}\n"));
}

fn increment_counter(path: impl AsRef<Path>) -> u64 {
    let path = path.as_ref();
    let current = fs::read_to_string(path)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let next = current.saturating_add(1);
    let _ = fs::write(path, next.to_string());
    next
}

fn manager_request_loop(
    lines: impl Iterator<Item = io::Result<String>>,
    stdout: &mut impl Write,
    marker: Option<&str>,
) {
    for line in lines {
        let Ok(message) =
            line.and_then(|line| serde_json::from_str::<Value>(&line).map_err(io::Error::other))
        else {
            break;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let response = if message.get("method").and_then(Value::as_str) == Some("tools/list") {
            json!({"jsonrpc":"2.0","id":id,"result":{"tools":[execution_tool()]}})
        } else {
            json!({"jsonrpc":"2.0","id":id,"result":{}})
        };
        write_json(stdout, &response);
    }
    append_marker(marker, "stopped\n");
}

fn execute_request_loop(
    lines: impl Iterator<Item = io::Result<String>>,
    stdout: &mut impl Write,
    scenario: &str,
    marker: Option<&str>,
) {
    let mut pending: Option<(Value, Value)> = None;
    for line in lines {
        let Ok(message) =
            line.and_then(|line| serde_json::from_str::<Value>(&line).map_err(io::Error::other))
        else {
            return;
        };
        let method = message.get("method").and_then(Value::as_str);
        if method == Some("notifications/cancelled") {
            append_marker(marker, "cancelled\n");
            if let Some((id, token)) = pending.take() {
                write_progress(stdout, &token, 2.0, Some(2.0), "late progress");
                write_json(stdout, &success_response(&id, "late"));
            }
            continue;
        }
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        match method {
            Some("tools/list") => write_json(
                stdout,
                &json!({"jsonrpc":"2.0","id":id,"result":{"tools":[execution_tool()]}}),
            ),
            Some("tools/call") => {
                append_marker(marker, "called\n");
                let token = message
                    .pointer("/params/_meta/progressToken")
                    .cloned()
                    .unwrap_or_else(|| json!("missing"));
                match scenario {
                    "progress" => {
                        write_progress(stdout, &token, 1.0, Some(2.0), "starting");
                        write_progress(stdout, &token, 2.0, Some(2.0), "finished");
                        write_json(stdout, &success_response(&id, "ok"));
                    }
                    "content" => write_json(
                        stdout,
                        &json!({
                            "jsonrpc":"2.0",
                            "id":id,
                            "result":{
                                "content":[
                                    {"type":"text","text":"plain"},
                                    {"type":"image","data":"aGVsbG8=","mimeType":"image/png"},
                                    {"type":"resource","resource":{"uri":"memory://text","mimeType":"text/plain","text":"embedded"}},
                                    {"type":"resource","resource":{"uri":"memory://image","mimeType":"image/png","blob":"aGVsbG8="}}
                                ],
                                "structuredContent":{"echo":"mapped"},
                                "isError":false
                            }
                        }),
                    ),
                    "structured-only" => write_json(
                        stdout,
                        &json!({"jsonrpc":"2.0","id":id,"result":{"structuredContent":{"echo":"structured"},"isError":false}}),
                    ),
                    "unsupported" => write_json(
                        stdout,
                        &json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"audio","data":"aGVsbG8=","mimeType":"audio/wav"}],"structuredContent":{"echo":"bad"}}}),
                    ),
                    "schema" => write_json(
                        stdout,
                        &json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"bad schema"}],"structuredContent":{"wrong":true}}}),
                    ),
                    "is-error" => write_json(
                        stdout,
                        &json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"untrusted server detail"}],"isError":true}}),
                    ),
                    "flood" => {
                        for index in 0..16 {
                            write_progress(stdout, &token, f64::from(index), Some(16.0), "flood");
                        }
                        write_json(stdout, &success_response(&id, "flood"));
                    }
                    "duplicate" => {
                        let response = success_response(&id, "duplicate");
                        write_json(stdout, &response);
                        write_json(stdout, &response);
                    }
                    "mismatch" => write_json(
                        stdout,
                        &json!({"jsonrpc":"2.0","id":999_999,"result":{"content":[{"type":"text","text":"wrong id"}],"structuredContent":{"echo":"wrong"}}}),
                    ),
                    "cancel" | "timeout" => pending = Some((id, token)),
                    _ => write_json(stdout, &success_response(&id, "ok")),
                }
            }
            _ => write_json(stdout, &json!({"jsonrpc":"2.0","id":id,"result":{}})),
        }
    }
}

fn execution_tool() -> Value {
    json!({
        "name":"echo",
        "description":"Echoes one bounded value.",
        "inputSchema":{
            "type":"object",
            "properties":{"value":{"type":"string"}},
            "required":["value"],
            "additionalProperties":false
        },
        "outputSchema":{
            "type":"object",
            "properties":{"echo":{"type":"string"}},
            "required":["echo"],
            "additionalProperties":false
        }
    })
}

fn success_response(id: &Value, value: &str) -> Value {
    json!({
        "jsonrpc":"2.0",
        "id":id,
        "result":{
            "content":[{"type":"text","text":value}],
            "structuredContent":{"echo":value},
            "isError":false
        }
    })
}

fn write_progress(
    stdout: &mut impl Write,
    progress_token: &Value,
    progress: f64,
    total: Option<f64>,
    message: &str,
) {
    write_json(
        stdout,
        &json!({
            "jsonrpc":"2.0",
            "method":"notifications/progress",
            "params":{
                "progressToken":progress_token,
                "progress":progress,
                "total":total,
                "message":message
            }
        }),
    );
}

fn append_marker(marker: Option<&str>, value: &str) {
    let Some(marker) = marker else {
        return;
    };
    let result = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(marker)
        .and_then(|mut file| file.write_all(value.as_bytes()));
    let _ = result;
}

fn catalog_request_loop(
    lines: impl Iterator<Item = io::Result<String>>,
    stdout: &mut impl Write,
    scenario: &str,
) {
    for line in lines {
        let Ok(message) =
            line.and_then(|line| serde_json::from_str::<Value>(&line).map_err(io::Error::other))
        else {
            return;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        if message.get("method").and_then(Value::as_str) != Some("tools/list") {
            write_json(stdout, &json!({"jsonrpc":"2.0","id":id,"result":{}}));
            continue;
        }
        let cursor = message.pointer("/params/cursor").and_then(Value::as_str);
        let result = match (scenario, cursor) {
            ("paginated", None) => json!({
                "tools":[tool("zeta"), tool("disabled")],
                "nextCursor":"page-2"
            }),
            ("paginated", Some("page-2")) => {
                json!({"tools":[tool("alpha"), tool("undeclared")]})
            }
            ("duplicate", None) => {
                json!({"tools":[tool("duplicate")],"nextCursor":"page-2"})
            }
            ("duplicate", Some("page-2")) => json!({"tools":[tool("duplicate")]}),
            ("loop", _) => json!({"tools":[],"nextCursor":"loop"}),
            _ => json!({"tools":[]}),
        };
        write_json(stdout, &json!({"jsonrpc":"2.0","id":id,"result":result}));
    }
}

fn tool(name: &str) -> Value {
    json!({
        "name":name,
        "description":format!("Fixture tool {name}."),
        "inputSchema":{"type":"object"},
        "outputSchema":{"type":"object"}
    })
}

fn request_loop(
    lines: impl Iterator<Item = io::Result<String>>,
    stdout: &mut impl Write,
    ignore_requests: bool,
    duplicate_responses: bool,
) {
    for line in lines {
        let Ok(line) = line else {
            return;
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        if ignore_requests {
            continue;
        }
        let response = json!({"jsonrpc":"2.0","id":id,"result":{}});
        write_json(stdout, &response);
        if duplicate_responses {
            write_json(stdout, &response);
        }
    }
}

fn write_json(output: &mut impl Write, value: &Value) {
    let _ = serde_json::to_writer(&mut *output, value);
    let _ = output.write_all(b"\n");
    let _ = output.flush();
}

fn write_state(path: impl AsRef<Path>, argument_count: usize) {
    let state = format!(
        "initialized=true\nenv_count={}\nargument_count={argument_count}\n",
        env::vars_os().count()
    );
    let _ = fs::write(path, state);
}

fn write_stderr(bytes: usize) {
    let mut stderr = io::stderr().lock();
    let chunk = [b'e'; 8 * 1024];
    let mut remaining = bytes;
    while remaining > 0 {
        let count = remaining.min(chunk.len());
        if stderr.write_all(&chunk[..count]).is_err() {
            return;
        }
        remaining -= count;
    }
    let _ = stderr.flush();
}

fn write_stderr_text(value: &str) {
    let mut stderr = io::stderr().lock();
    let _ = stderr.write_all(value.as_bytes());
    let _ = stderr.flush();
}

fn sleep_forever() -> ! {
    loop {
        thread::park();
    }
}
