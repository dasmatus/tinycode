use crate::config::ProviderKind;
use crate::usage::Usage;
use serde_json::{json, Value};
use std::error::Error;
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

const MAX_TOKENS: u32 = 16384;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Debug, Default, Clone)]
pub struct Reply {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub stop: String,
    /// Tokens the provider reported for this turn, when it reports them.
    pub usage: Usage,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw arguments JSON, streamed chunk-deltas already reassembled.
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct ResultMsg {
    pub id: String,
    pub ok: bool,
    pub output: String,
}

pub fn chat_url(kind: ProviderKind, base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    match kind {
        ProviderKind::Anthropic => format!("{base}/v1/messages"),
        ProviderKind::OpenAi => format!("{base}/chat/completions"),
    }
}

fn build_body(
    kind: ProviderKind,
    model: &str,
    system: &str,
    tools: &[ToolSpec],
    messages: &[Value],
    temperature: Option<f32>,
) -> Value {
    let mut body = match kind {
        ProviderKind::Anthropic => {
            let mut body = json!({
                "model": model,
                "max_tokens": MAX_TOKENS,
                "stream": true,
                "system": system,
                "messages": messages,
            });
            if !tools.is_empty() {
                body["tools"] = Value::Array(
                    tools
                        .iter()
                        .map(|t| {
                            json!({
                                "name": t.name,
                                "description": t.description,
                                "input_schema": t.parameters,
                            })
                        })
                        .collect(),
                );
            }
            body
        }
        ProviderKind::OpenAi => {
            let mut msgs = vec![json!({"role": "system", "content": system})];
            msgs.extend(messages.iter().cloned());
            let mut body = json!({
                "model": model,
                "max_tokens": MAX_TOKENS,
                "stream": true,
                "stream_options": {"include_usage": true},
                "messages": msgs,
            });
            if !tools.is_empty() {
                body["tools"] = Value::Array(
                    tools
                        .iter()
                        .map(|t| {
                            json!({
                                "type": "function",
                                "function": {
                                    "name": t.name,
                                    "description": t.description,
                                    "parameters": t.parameters,
                                },
                            })
                        })
                        .collect(),
                );
            }
            body
        }
    };
    // Both wire formats take the same top-level knob. Left out when the
    // caller has no preference, so the provider's default applies.
    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    body
}

/// Stream one assistant turn. Tool-call argument chunks are reassembled
/// incrementally; system memory stays proportional to the response, not to
/// any fixed buffer. `session_tag` tags the run on the wire — OpenCode's Go
/// gateway routes requests per session via `x-opencode-session`.
/// `temperature` goes on the wire only when set; `None` keeps the
/// provider's default.
pub fn stream(
    kind: ProviderKind,
    api_key: &str,
    model: &str,
    base_url: &str,
    system: &str,
    tools: &[ToolSpec],
    messages: &[Value],
    session_tag: &str,
    temperature: Option<f32>,
    on_text: impl FnMut(&str),
) -> Result<Reply, Box<dyn Error>> {
    let url = chat_url(kind, base_url);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_IDLE_TIMEOUT)
        .build();
    let mut req = match kind {
        ProviderKind::Anthropic => agent
            .post(&url)
            .set("x-api-key", api_key)
            .set("anthropic-version", "2023-06-01"),
        ProviderKind::OpenAi => agent.post(&url),
    };
    if kind == ProviderKind::OpenAi && !api_key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {api_key}"));
    }
    if is_opencode_gateway(url.as_str()) {
        let tag = if session_tag.is_empty() { "tinycode" } else { session_tag };
        req = req.set("x-opencode-session", tag);
    }
    let body = build_body(kind, model, system, tools, messages, temperature);
    let resp = match req.send_json(&body) {
        Ok(r) => r,
        Err(ureq::Error::Status(status, resp)) => {
            let mut detail = String::new();
            let _ = resp
                .into_reader()
                .take(8192)
                .read_to_string(&mut detail);
            return Err(format!("HTTP {status} from {url}: {detail}").into());
        }
        Err(e) => return Err(e.into()),
    };

    let reader = BufReader::new(resp.into_reader());
    match kind {
        ProviderKind::Anthropic => stream_anthropic(reader, on_text),
        ProviderKind::OpenAi => stream_openai(reader, on_text),
    }
}

/// OpenCode-hosted inference gateways route requests per session. Tagging
/// with `x-opencode-session` preserves session affinity/routing parity with
/// OpenCode's API behavior. Matches the canonical `/inference/...` endpoints
/// and the `/zen/...` gateway paths (both enforce the session header).
pub fn is_opencode_gateway(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if !lower.contains("opencode.ai/") {
        return false;
    }
    lower.contains("/inference/") || lower.contains("/zen/")
}

fn stream_anthropic(
    mut reader: impl BufRead,
    mut on_text: impl FnMut(&str),
) -> Result<Reply, Box<dyn Error>> {
    struct Block {
        kind: String,
        id: String,
        name: String,
        data: String,
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut stop = String::from("end_turn");
    let mut anthropic = crate::usage::AnthropicUsage::default();
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let data = line.trim_end_matches(|c| c == '\n' || c == '\r');
        let Some(data) = data.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim_start();
        if data.is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(data)?;
        Usage::from_anthropic_event(&event, &mut anthropic);
        // Error payloads ({"type":"error","error":{…}}, or bare {"error":…}).
        if let Some(err) = event.get("error").filter(|e| e.is_object()) {
            let msg = err.get("message").and_then(Value::as_str).unwrap_or("unknown error");
            return Err(format!("provider error: {msg}").into());
        }
        match event.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                let cb = event.get("content_block").cloned().unwrap_or(Value::Null);
                blocks.push(Block {
                    kind: cb.get("type").and_then(Value::as_str).unwrap_or("").into(),
                    id: cb.get("id").and_then(Value::as_str).unwrap_or("").into(),
                    name: cb.get("name").and_then(Value::as_str).unwrap_or("").into(),
                    data: String::new(),
                });
            }
            Some("content_block_delta") => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = event.get("delta").cloned().unwrap_or(Value::Null);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let text = delta.get("text").and_then(Value::as_str).unwrap_or("");
                        on_text(text);
                        if let Some(block) = blocks.get_mut(index) {
                            block.data.push_str(text);
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(chunk) = delta.get("partial_json").and_then(Value::as_str) {
                            if let Some(block) = blocks.get_mut(index) {
                                block.data.push_str(chunk);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some("message_delta") => {
                if let Some(reason) = event
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    stop = reason.into();
                }
            }
            Some("message_stop") => break,
            Some("error") => {
                let msg = event
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(format!("provider error: {msg}").into());
            }
            _ => {}
        }
    }

    let mut text_parts: Vec<&str> = Vec::new();
    let mut calls = Vec::new();
    for block in &blocks {
        if block.kind == "text" && !block.data.is_empty() {
            text_parts.push(&block.data);
        } else if block.kind == "tool_use" {
            calls.push(ToolCall {
                id: block.id.clone(),
                name: block.name.clone(),
                arguments: block.data.clone(),
            });
        }
    }
    Ok(Reply {
        text: text_parts.join("\n\n"),
        calls,
        stop,
        usage: Usage {
            input_tokens: anthropic.input,
            output_tokens: anthropic.output,
        },
    })
}

fn stream_openai(
    mut reader: impl BufRead,
    mut on_text: impl FnMut(&str),
) -> Result<Reply, Box<dyn Error>> {
    struct Call {
        id: String,
        name: String,
        args: String,
    }
    let mut text = String::new();
    let mut stop = String::from("stop");
    let mut usage = Usage::default();
    let mut calls: Vec<(usize, Call)> = Vec::new();
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let data = line.trim_end_matches(|c| c == '\n' || c == '\r');
        let Some(data) = data.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim_start();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            break;
        }
        let event: Value = serde_json::from_str(data)?;
        // Bare error payloads like {"error":{"message":…}}.
        if let Some(err) = event.get("error").filter(|e| e.is_object()) {
            let msg = err.get("message").and_then(Value::as_str).unwrap_or("unknown error");
            return Err(format!("provider error: {msg}").into());
        }
        let new_usage = Usage::from_openai_chunk(&event);
        if !new_usage.is_zero() {
            usage = new_usage;
        }
        let choices = event
            .get("choices")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for choice in choices {
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                stop = reason.into();
            }
            let Some(delta) = choice.get("delta") else {
                continue;
            };
            if let Some(t) = delta.get("content").and_then(Value::as_str) {
                if !t.is_empty() {
                    on_text(t);
                    text.push_str(t);
                }
            }
            for tc in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                if !calls.iter().any(|(i, _)| *i == index) {
                    calls.push((index, Call { id: String::new(), name: String::new(), args: String::new() }));
                }
                let position = calls
                    .iter()
                    .position(|(i, _)| *i == index)
                    .ok_or_else(|| -> Box<dyn Error> { "tool call index vanished".into() })?;
                let call = &mut calls[position].1;
                if let Some(id) = tc.get("id").and_then(Value::as_str) {
                    if !id.is_empty() {
                        call.id = id.into();
                    }
                }
                if let Some(name) = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                {
                    if !name.is_empty() {
                        call.name = name.into();
                    }
                }
                if let Some(chunk) = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                {
                    call.args.push_str(chunk);
                }
            }
        }
    }

    calls.sort_by_key(|(i, _)| *i);
    Ok(Reply {
        text,
        calls: calls
            .into_iter()
            .map(|(_, c)| ToolCall { id: c.id, name: c.name, arguments: c.args })
            .collect(),
        stop,
        usage,
    })
}

/// Convert a completed reply into the next assistant message, in the
/// provider's own wire format (messages are stored provider-native).
pub fn assistant_message(kind: ProviderKind, reply: &Reply) -> Value {
    match kind {
        ProviderKind::Anthropic => {
            let mut content: Vec<Value> = Vec::new();
            if !reply.text.is_empty() {
                content.push(json!({"type": "text", "text": reply.text}));
            }
            for call in &reply.calls {
                let args: Value = serde_json::from_str(call.arguments.trim()).unwrap_or(json!({}));
                content.push(json!({
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.name,
                    "input": args,
                }));
            }
            json!({"role": "assistant", "content": content})
        }
        ProviderKind::OpenAi => {
            let mut msg = json!({"role": "assistant"});
            msg["content"] = if reply.text.is_empty() {
                Value::Null
            } else {
                json!(reply.text)
            };
            if !reply.calls.is_empty() {
                msg["tool_calls"] = Value::Array(
                    reply
                        .calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {"name": c.name, "arguments": c.arguments},
                            })
                        })
                        .collect(),
                );
            }
            msg
        }
    }
}

pub fn tool_results_messages(kind: ProviderKind, results: &[ResultMsg]) -> Vec<Value> {
    match kind {
        ProviderKind::Anthropic => vec![json!({
            "role": "user",
            "content": results
                .iter()
                .map(|r| {
                    let mut item = json!({"type": "tool_result", "tool_use_id": r.id, "content": r.output});
                    if !r.ok {
                        item["is_error"] = json!(true);
                    }
                    item
                })
                .collect::<Vec<_>>(),
        })],
        ProviderKind::OpenAi => results
            .iter()
            .map(|r| json!({"role": "tool", "tool_call_id": r.id, "content": r.output}))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::is_opencode_gateway;

    #[test]
    fn opencode_gateway_detection_has_parity_for_inference_endpoints() {
        assert!(is_opencode_gateway(
            "https://opencode.ai/inference/go/openai/v1/chat/completions"
        ));
        assert!(is_opencode_gateway(
            "https://OPENCODE.AI/inference/custom/openai/v1/chat/completions"
        ));
        assert!(is_opencode_gateway(
            "https://opencode.ai/inference/go/anthropic/v1/messages"
        ));
        // Zen gateway paths (e.g. TINYCODE_BASE_URL=https://opencode.ai/zen/go)
        // also require x-opencode-session.
        assert!(is_opencode_gateway("https://opencode.ai/zen/go/v1/messages"));
        assert!(is_opencode_gateway(
            "https://opencode.ai/zen/go/chat/completions"
        ));
        assert!(is_opencode_gateway("https://OPENCODE.AI/zen/v1/messages"));
        assert!(!is_opencode_gateway("https://api.openai.com/v1/chat/completions"));
        assert!(!is_opencode_gateway("https://opencode.ai/docs/api"));
        assert!(!is_opencode_gateway("https://opencode.ai/console"));
    }
}
