use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Token counts as reported by the provider on a single streaming turn.
#[derive(Debug, Default, Clone)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Usage {
    pub fn is_zero(&self) -> bool {
        self.input_tokens == 0 && self.output_tokens == 0
    }

    /// Pull Anthropic's message_start / message_delta usage blocks.
    pub fn from_anthropic_event(event: &Value, state: &mut AnthropicUsage) {
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                let u = &event["message"]["usage"];
                state.input = u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
            }
            Some("message_delta") => {
                let u = &event["usage"];
                state.output = u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
            }
            _ => {}
        }
    }

    /// Pull OpenAI's terminal usage chunk (requested via include_usage).
    pub fn from_openai_chunk(chunk: &Value) -> Usage {
        let u = &chunk["usage"];
        if u.is_null() {
            return Usage::default();
        }
        Usage {
            input_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
            output_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct AnthropicUsage {
    pub input: u64,
    pub output: u64,
}

/// One recorded provider turn.
#[derive(Debug, Clone)]
pub struct TurnRecord {
    pub provider: String,
    /// Wire dialect: "anthropic" | "openai"
    pub kind: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub turn: u32,
    pub duration_ms: u64,
    pub ok: bool,
    pub error: Option<String>,
    /// Short, per-run session tag (gateway session header / record linkage).
    pub session: String,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// No stored key material here — only identifiers and counters.
pub fn file_path() -> PathBuf {
    if let Ok(p) = std::env::var("TINYCODE_USAGE_FILE") {
        return PathBuf::from(p);
    }
    let data = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".local/share")
        });
    data.join("tinycode").join("usage.jsonl")
}

pub fn record(rec: &TurnRecord) -> Result<(), String> {
    record_to(&file_path(), rec)
}

/// Longest error text kept per record; provider error bodies can be large.
const MAX_ERR_CHARS: usize = 200;

/// Append one record as a single write, so concurrent tinycode processes
/// appending to the same file (O_APPEND) never interleave partial lines.
pub fn record_to(path: &Path, rec: &TurnRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("usage dir {}: {e}", parent.display()))?;
    }
    let err: String = rec.error.as_deref().unwrap_or("").chars().take(MAX_ERR_CHARS).collect();
    let mut line = json!({
        "ts": now_ms(),
        "provider": rec.provider,
        "kind": rec.kind,
        "model": rec.model,
        "turn": rec.turn,
        "in": rec.input_tokens,
        "out": rec.output_tokens,
        "ms": rec.duration_ms,
        "ok": rec.ok,
        "err": err,
        "session": rec.session,
    })
    .to_string();
    line.push('\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)
        .map_err(|e| format!("usage file {}: {e}", path.display()))?;
    file.write_all(line.as_bytes()).map_err(|e| format!("usage write: {e}"))?;
    Ok(())
}

pub fn read_records(path: &Path) -> Result<Vec<Value>, String> {
    let text = if path.as_os_str() == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .map_err(|e| format!("stdin: {e}"))?;
        buf
    } else {
        fs::read_to_string(path).map_err(|e| format!("usage file {}: {e}", path.display()))?
    };
    Ok(text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect())
}

#[derive(Debug, Default, Clone)]
pub struct Aggregate {
    pub provider: String,
    pub model: String,
    pub runs: u64,
    pub failed: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub fn aggregate(records: &[Value]) -> Vec<Aggregate> {
    let mut order: Vec<(String, String)> = Vec::new();
    let mut by_key: std::collections::BTreeMap<(String, String), Aggregate> =
        std::collections::BTreeMap::new();
    for rec in records {
        let provider = rec.get("provider").and_then(Value::as_str).unwrap_or("").to_string();
        let model = rec.get("model").and_then(Value::as_str).unwrap_or("").to_string();
        let key = (provider.clone(), model.clone());
        if !by_key.contains_key(&key) {
            order.push(key.clone());
        }
        let entry = by_key.entry(key).or_insert_with(|| Aggregate {
            provider: provider.clone(),
            model: model.clone(),
            ..Aggregate::default()
        });
        entry.runs += 1;
        if rec.get("ok").and_then(Value::as_bool) == Some(false) {
            entry.failed += 1;
        }
        entry.input_tokens += rec.get("in").and_then(Value::as_u64).unwrap_or(0);
        entry.output_tokens += rec.get("out").and_then(Value::as_u64).unwrap_or(0);
    }
    order.into_iter().map(|k| by_key.remove(&k).unwrap()).collect()
}

pub fn total(records: &[Value]) -> Aggregate {
    Aggregate {
        provider: "total".into(),
        model: "(all models)".into(),
        runs: records.len() as u64,
        failed: records
            .iter()
            .filter(|r| r.get("ok").and_then(Value::as_bool) == Some(false))
            .count() as u64,
        input_tokens: records.iter().map(|r| r.get("in").and_then(Value::as_u64).unwrap_or(0)).sum(),
        output_tokens: records.iter().map(|r| r.get("out").and_then(Value::as_u64).unwrap_or(0)).sum(),
    }
}

fn since_ms(range: &str) -> Option<u64> {
    let now = now_ms();
    match range.trim() {
        "today" => {
            let day_ms = 24 * 60 * 60 * 1000;
            Some(now - (now % day_ms) + day_ms / 3) // coarse UTC-ish day start
        }
        "24h" => Some(now.saturating_sub(24 * 60 * 60 * 1000)),
        "48h" => Some(now.saturating_sub(48 * 60 * 60 * 1000)),
        "7d" => Some(now.saturating_sub(7 * 24 * 60 * 60 * 1000)),
        "30d" => Some(now.saturating_sub(30 * 24 * 60 * 60 * 1000)),
        "all" | "" => None,
        date if date.len() == 10 => {
            // YYYY-MM-DD → midnight UTC via a simple civil→epoch conversion.
            let parts: Vec<u64> = date.split('-').filter_map(|p| p.parse().ok()).collect();
            if parts.len() != 3 {
                return None;
            }
            let (y, m, d) = (parts[0], parts[1], parts[2]);
            if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
                return None;
            }
            let days = days_from_civil(y, m, d);
            Some(days * 24 * 60 * 60 * 1000)
        }
        _ => None,
    }
}

/// Howard Hinnant's days_from_civil, for date-only UTC math without a time crate.
fn days_from_civil(y: u64, m: u64, d: u64) -> u64 {
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe + 719468) as u64
}

const USAGE_USAGE: &str = "\
tinycode usage — show recorded provider usage (tokens per model).

USAGE:
    tinycode usage [OPTIONS]

OPTIONS:
    --since <RANGE>    all | today | 24h | 48h | 7d | 30d | YYYY-MM-DD
    --tail <N>         print the last N raw records instead of the summary
    --json             NDJSON summary records on stdout
    --file <PATH>      read this usage file (or \"-\" for stdin)
    -h, --help         this help
";

/// Entry point for the `tinycode usage` subcommand. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let mut file = file_path();
    let mut since: Option<String> = None;
    let mut tail: Option<usize> = None;
    let mut json_mode = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print!("{USAGE_USAGE}");
                return 0;
            }
            "--json" => json_mode = true,
            "--file" => {
                match args.get(i + 1) {
                    Some(v) => {
                        file = PathBuf::from(v);
                        i += 1;
                    }
                    None => return eprintln_error("usage: missing value for --file"),
                }
            }
            "--since" => {
                match args.get(i + 1) {
                    Some(v) => {
                        since = Some(v.clone());
                        i += 1;
                    }
                    None => return eprintln_error("usage: missing value for --since"),
                }
            }
            "--tail" => {
                match args.get(i + 1).and_then(|v| v.parse::<usize>().ok()) {
                    Some(n) => {
                        tail = Some(n);
                        i += 1;
                    }
                    None => return eprintln_error("usage: --tail needs a number"),
                }
            }
            other => return eprintln_error(&format!("usage: unknown option: {other}")),
        }
        i += 1;
    }

    let all = match read_records(&file) {
        Ok(r) => r,
        Err(e) => return eprintln_error(&e),
    };
    if let Some(n) = tail {
        let skip = all.len().saturating_sub(n);
        for rec in &all[skip..] {
            if json_mode {
                println!("{rec}");
            } else {
                println!("{}", pretty_record(rec));
            }
        }
        return 0;
    }

    let cutoff = since.as_deref().and_then(since_ms);
    let filtered: Vec<&Value> = all
        .iter()
        .filter(|r| match cutoff {
            Some(c) => r.get("ts").and_then(Value::as_u64).unwrap_or(0) >= c,
            None => true,
        })
        .collect();
    let borrowed: Vec<Value> = filtered.into_iter().cloned().collect();
    let aggs = aggregate(&borrowed);
    let tot = total(&borrowed);

    if json_mode {
        for a in &aggs {
            println!("{}", json!({
                "provider": a.provider, "model": a.model, "runs": a.runs,
                "failed": a.failed, "in_tokens": a.input_tokens, "out_tokens": a.output_tokens,
            }));
        }
        println!("{}", json!({
            "provider": "total", "model": "(all models)", "runs": tot.runs,
            "failed": tot.failed, "in_tokens": tot.input_tokens, "out_tokens": tot.output_tokens,
        }));
        return 0;
    }

    eprintln!("tinycode usage — {} ({} of {} records)", since.as_deref().unwrap_or("all time"), borrowed.len(), all.len());
    println!(
        "{:<14} {:<26} {:>6} {:>5} {:>10} {:>10}",
        "provider", "model", "runs", "fail", "in_tok", "out_tok"
    );
    for a in &aggs {
        println!(
            "{:<14} {:<26} {:>6} {:>5} {:>10} {:>10}",
            a.provider, a.model, a.runs, a.failed, a.input_tokens, a.output_tokens
        );
    }
    println!(
        "{:<14} {:<26} {:>6} {:>5} {:>10} {:>10}",
        tot.provider, tot.model, tot.runs, tot.failed, tot.input_tokens, tot.output_tokens
    );
    0
}

fn pretty_record(rec: &Value) -> String {
    let ts = rec.get("ts").and_then(Value::as_u64).unwrap_or(0);
    let model = rec.get("model").and_then(Value::as_str).unwrap_or("");
    let ok = rec.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let flag = if ok { "ok" } else { "ERR" };
    format!(
        "{ts} {flag} {model} in={} out={} ms={} turn={}",
        rec.get("in").and_then(Value::as_u64).unwrap_or(0),
        rec.get("out").and_then(Value::as_u64).unwrap_or(0),
        rec.get("ms").and_then(Value::as_u64).unwrap_or(0),
        rec.get("turn").and_then(Value::as_u64).unwrap_or(0),
    )
}

fn eprintln_error(msg: &str) -> i32 {
    eprintln!("error: {msg}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn anthropic_usage_capture() {
        let mut state = AnthropicUsage::default();
        Usage::from_anthropic_event(&json!({"type":"message_start","message":{"usage":{"input_tokens":19}}}), &mut state);
        Usage::from_anthropic_event(&json!({"type":"message_delta","usage":{"output_tokens":3}}), &mut state);
        assert_eq!(state.input, 19);
        assert_eq!(state.output, 3);
    }

    #[test]
    fn openai_usage_capture() {
        let u = Usage::from_openai_chunk(&json!({"usage":{"prompt_tokens":7,"completion_tokens":2}}));
        assert_eq!(u.input_tokens, 7);
        assert_eq!(u.output_tokens, 2);
        assert!(Usage::from_openai_chunk(&json!({"choices":[],"unused":true})).is_zero());
    }

    #[test]
    fn aggregate_groups_and_sums() {
        let recs = vec![
            json!({"provider":"opencode-go","model":"glm-5.3-flash","ok":true,"in":5,"out":2}),
            json!({"provider":"opencode-go","model":"glm-5.3-flash","ok":false,"in":4,"out":0}),
            json!({"provider":"opencode-go","model":"beam-free","ok":true,"in":1,"out":1}),
        ];
        let aggs = aggregate(&recs);
        assert_eq!(aggs.len(), 2);
        assert_eq!(aggs[0].runs, 2);
        assert_eq!(aggs[0].failed, 1);
        assert_eq!(aggs[0].input_tokens, 9);
        assert_eq!(aggs[1].model, "beam-free");
    }

    #[test]
    fn record_roundtrip_and_truncation() {
        let path = std::env::temp_dir().join(format!("tinycode-usage-t-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut rec = TurnRecord {
            provider: "opencode-go".into(),
            kind: "openai".into(),
            model: "glm-5.3-flash".into(),
            input_tokens: 19,
            output_tokens: 3,
            turn: 1,
            duration_ms: 812,
            ok: true,
            error: None,
            session: "probe".into(),
        };
        record_to(&path, &rec).expect("record");
        rec.ok = false;
        rec.error = Some("x".repeat(5000));
        record_to(&path, &rec).expect("record error");
        let recs = read_records(&path).expect("read");
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0]["in"], 19);
        assert_eq!(recs[0]["provider"], "opencode-go");
        assert_eq!(recs[0]["ok"], true);
        assert_eq!(recs[1]["err"].as_str().unwrap().chars().count(), MAX_ERR_CHARS);
        let _ = fs::remove_file(&path);
    }
}
