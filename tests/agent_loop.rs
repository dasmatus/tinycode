//! End-to-end agent-loop tests against a scripted mock provider, one per
//! wire protocol. A passing test here means: request bodies are valid for the
//! provider, streamed SSE tool calls are reassembled correctly, the tool ran,
//! and its result went back into the next turn.
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use tinycode::agent::{Agent, Event};
use tinycode::config::{Config, ProviderKind};


struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tempdir(tag: &str) -> TempDir {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("tinycode-agent-{tag}-{nanos}"));
    fs::create_dir_all(&path).expect("mkdir tempdir");
    let root = fs::canonicalize(&path).expect("canonicalize");
    TempDir(root)
}

/// HTTP/1.1 server that records each request body, then answers with the
/// next scripted SSE response and closes the connection.
fn mock_server(responses: Vec<String>) -> (SocketAddr, mpsc::Receiver<(String, serde_json::Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    let total = responses.len();
    std::thread::spawn(move || {
        let mut served = 0usize;
        'accept: for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let reader_stream = stream.try_clone().expect("clone socket");
            let mut reader = BufReader::new(reader_stream);
            let mut line = String::new();
            let mut content_len = 0usize;
            let mut method_path = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    break;
                }
                if method_path.is_empty() {
                    method_path = trimmed.to_string();
                }
                if let Some(v) = trimmed.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; content_len];
            let _ = std::io::Read::read_exact(&mut reader, &mut body);
            let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
            let reply = if served < total {
                served += 1;
                responses[served - 1].clone()
            } else {
                "data: {\"error\":{\"message\":\"script exhausted\"}}\n\n".to_string()
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n",
                reply.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
            let _ = tx.send((method_path, parsed));
            if served >= total {
                break 'accept;
            }
        }
    });
    (addr, rx)
}

fn openai_sse(deltas: Vec<serde_json::Value>) -> String {
    let mut out = String::new();
    for delta in deltas {
        out.push_str(&format!("data: {}\n\n", serde_json::to_string(&delta).expect("json")));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

fn anthropic_sse(events: Vec<serde_json::Value>) -> String {
    let mut out = String::new();
    for event in events {
        out.push_str(&format!("data: {}\n\n", serde_json::to_string(&event).expect("json")));
    }
    out
}

fn config(kind: ProviderKind, base_url: String, root: PathBuf) -> Config {
    Config {
        provider: kind,
        api_key: "test-key".into(),
        model: "test-model".into(),
        base_url,
        provider_id: None,
        // Every test records usage into its own temp dir, never ~/.local.
        usage_file: Some(root.join("usage.jsonl")),
        root,
        auto: true,
        max_turns: 6,
        json: false,
        quiet: true,
        session_file: None,
        system_prompt: None,
        task: "make a file".into(),
    }
}

fn capture_done() -> (
    Arc<Mutex<Option<(u32, String)>>>,
    impl FnMut(&Event),
) {
    let slot = Arc::new(Mutex::new(None::<(u32, String)>));
    let slot2 = slot.clone();
    let reporter = move |event: &Event| {
        if let Event::Done { text, turns, .. } = event {
            *slot2.lock().expect("lock") = Some((*turns, text.clone()));
        }
    };
    (slot, reporter)
}

#[test]
fn agent_loop_openai_e2e() {
    let dir = tempdir("openai");
    let turn1 = openai_sse(vec![
        serde_json::json!({"choices":[{"index":0,"delta":{"content":"I'll write it."}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[
            {"index":0,"id":"call_1","function":{"name":"write","arguments":"{\"pa"}}
        ]}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[
            {"index":0,"function":{"arguments":"th\":\"hello.txt\",\"content\":\"hello world\"}"}}
        ]}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
    ]);
    let turn2 = openai_sse(vec![
        serde_json::json!({"choices":[{"index":0,"delta":{"content":"wrote it."}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
    ]);
    let (addr, requests) = mock_server(vec![turn1, turn2]);

    let mut agent = Agent::new(config(
        ProviderKind::OpenAi,
        format!("http://{addr}"),
        dir.0.clone(),
    ))
    .expect("agent");
    let (done, mut reporter) = capture_done();
    let code = agent.run(&mut reporter).expect("run to finish");
    assert_eq!(code, 0, "completed normally");
    assert_eq!(
        fs::read_to_string(dir.0.join("hello.txt")).expect("file written"),
        "hello world",
        "split tool-call arguments were reassembled"
    );
    assert_eq!(done.lock().expect("lock").as_ref().expect("done event").1, "wrote it.");

    let (_first_req, second_req) = {
        let first = requests.recv().expect("first request");
        let second = requests.recv().expect("second request");
        (first.1, second.1)
    };
    let tool_msg = second_req["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("tool result message")
        .clone();
    assert_eq!(tool_msg["tool_call_id"], "call_1");
    assert_eq!(tool_msg["content"], "wrote 11 bytes to hello.txt");

    // Session was persisted at the end of each turn.
    let session_dir = dir.0.join(".tinycode/sessions");
    let saved: Vec<_> = fs::read_dir(&session_dir).expect("session dir").collect();
    assert_eq!(saved.len(), 1, "one session file");
}

#[test]
fn agent_loop_anthropic_e2e() {
    let dir = tempdir("anthropic");
    let turn1 = anthropic_sse(vec![
        serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text"}}),
        serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"writing"}}),
        serde_json::json!({"type":"content_block_stop","index":0}),
        serde_json::json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"write"}}),
        serde_json::json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"pa"}}),
        serde_json::json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"th\":\"hi.txt\",\"content\":\"hi\"}"}}),
        serde_json::json!({"type":"content_block_stop","index":1}),
        serde_json::json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
        serde_json::json!({"type":"message_stop"}),
    ]);
    let turn2 = anthropic_sse(vec![
        serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text"}}),
        serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done summarising"}}),
        serde_json::json!({"type":"content_block_stop","index":0}),
        serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
        serde_json::json!({"type":"message_stop"}),
    ]);
    let (addr, requests) = mock_server(vec![turn1, turn2]);

    let mut agent = Agent::new(config(
        ProviderKind::Anthropic,
        format!("http://{addr}"),
        dir.0.clone(),
    ))
    .expect("agent");
    let (done, mut reporter) = capture_done();
    let code = agent.run(&mut reporter).expect("run to finish");
    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(dir.0.join("hi.txt")).expect("file"), "hi");
    assert_eq!(done.lock().expect("lock").as_ref().expect("done").1, "done summarising");

    let (first_path, first_req) = requests.recv().expect("first request");
    let (second_path, second_req) = requests.recv().expect("second request");
    let _ = (first_path, first_req);
    // first.1 contains the tools; second.0 is the request line.
    assert!(second_path.contains("/v1/messages"), "anthropic endpoint: {second_path}");
    let result = second_req["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|m| m["role"] == "user" && m["content"].as_array().map_or(false, |c| !c.is_empty() && c[0]["type"] == "tool_result"))
        .expect("anthropic tool_result inside a user message");
    assert_eq!(result["content"][0]["tool_use_id"], "toolu_1");
}

/// A model that never stops calling tools must not run forever: the run
/// halts at --max-turns with exit code 2, and the session is resumable.
#[test]
fn max_turns_limit_stops_runaway_loops() {
    let dir = tempdir("runaway");
    let looping = |id: &str| {
        openai_sse(vec![
            serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"id":id,"function":{"name":"read","arguments":"{\"path\":\"x.txt\"}"}}
            ]}}]}),
            serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ])
    };
    // 4 turns of read/x.txt loop, then... still looping; max_turns is 4.
    let (addr, _requests) = mock_server(vec![looping("c1"), looping("c2"), looping("c3")]);

    let mut cfg = config(ProviderKind::OpenAi, format!("http://{addr}"), dir.0.clone());
    cfg.max_turns = 3;
    cfg.auto = false; // read is allowed without --auto anyway
    let mut agent = Agent::new(cfg).expect("agent");
    let code = agent
        .run(&mut |_event| {})
        .expect("loop itself never errors");
    assert_eq!(code, 2, "max-turns exit code");

    let file = agent.session_path().clone();
    let session: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(file).expect("session file")).expect("session");
    assert_eq!(session["messages"].as_array().expect("msgs").len() >= 1, true);
}

/// Usage capture: token counts reported by the provider end up as one
/// JSONL record per turn in the configured usage file, with model and turn.
#[test]
fn usage_records_openai_tokens() {
    let dir = tempdir("usage-openai");
    let turn = openai_sse(vec![
        serde_json::json!({"choices":[{"index":0,"delta":{"content":"pong."}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{}}],"usage":{"prompt_tokens":19,"completion_tokens":3}}),
        serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
    ]);
    let (addr, _requests) = mock_server(vec![turn]);

    let usage_path = dir.0.join("usage.jsonl");

    let mut agent = Agent::new(config(
        ProviderKind::OpenAi,
        format!("http://{addr}"),
        dir.0.clone(),
    ))
    .expect("agent");
    let code = agent.run(&mut |_event| {}).expect("run");
    assert_eq!(code, 0);

    let text = fs::read_to_string(&usage_path).expect("usage file written");
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("record is valid JSON"))
        .collect();
    assert_eq!(records.len(), 1, "one turn → one usage record");
    let rec = &records[0];
    assert_eq!(rec["provider"], "openai");
    assert_eq!(rec["model"], "test-model");
    assert_eq!(rec["in"], 19);
    assert_eq!(rec["out"], 3);
    assert_eq!(rec["ok"], true);
    assert_eq!(rec["turn"], 1);
    assert!(rec["session"].as_str().map_or(false, |s| s.contains(".json")));
}

/// Same expectation for the Anthropic dialect (message_start input
/// tokens + message_delta output tokens).
#[test]
fn usage_records_anthropic_tokens() {
    let dir = tempdir("usage-anthropic");
    let turn = anthropic_sse(vec![
        serde_json::json!({"type":"message_start","message":{"usage":{"input_tokens":42}}}),
        serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text"}}),
        serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}}),
        serde_json::json!({"type":"content_block_stop","index":0}),
        serde_json::json!({"type":"message_delta","usage":{"output_tokens":7},"delta":{"stop_reason":"end_turn"}}),
        serde_json::json!({"type":"message_stop"}),
    ]);
    let (addr, _requests) = mock_server(vec![turn]);

    let usage_path = dir.0.join("usage.jsonl");

    let mut agent = Agent::new(config(
        ProviderKind::Anthropic,
        format!("http://{addr}"),
        dir.0.clone(),
    ))
    .expect("agent");
    let code = agent.run(&mut |_event| {}).expect("run");
    assert_eq!(code, 0);

    let text = fs::read_to_string(&usage_path).expect("usage file written");
    let rec: serde_json::Value =
        serde_json::from_str(text.lines().next().expect("one line")).expect("record");
    assert_eq!(rec["provider"], "anthropic");
    assert_eq!(rec["in"], 42);
    assert_eq!(rec["out"], 7);
    assert_eq!(rec["ok"], true);
}

/// A failed provider turn still produces a record, marked not-ok with a
/// short error, so usage never loses track of attempts. The Anthropic
/// dialect surfaces stream errors; we use it to trigger a failing turn.
#[test]
fn usage_records_failed_turns() {
    let dir = tempdir("usage-failed");
    let (addr, _requests) = mock_server(vec![]); // serves nothing → script exhausted
    let usage_path = dir.0.join("usage.jsonl");

    let mut agent = Agent::new(config(
        ProviderKind::Anthropic,
        format!("http://{addr}"),
        dir.0.clone(),
    ))
    .expect("agent");
    let result = agent.run(&mut |_event| {});
    assert!(result.is_err(), "exhausted script errors out");

    let text = fs::read_to_string(&usage_path).expect("usage file written even on failure");
    let rec: serde_json::Value =
        serde_json::from_str(text.lines().next().expect("one line")).expect("record");
    assert_eq!(rec["ok"], false);
    assert!(rec["err"].as_str().map_or(false, |e| !e.is_empty()));
}
