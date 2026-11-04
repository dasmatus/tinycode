use crate::provider::ToolSpec;
use regex::Regex;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct ToolCtx {
    /// Canonical workspace root from `Config::root`.
    pub root: PathBuf,
    /// Mutating tools are refused when false.
    pub auto: bool,
}

pub struct ToolOut {
    pub ok: bool,
    pub output: String,
}

const READ_CAP_BYTES: usize = 64_000;
const GREP_MATCH_CAP: usize = 200;
const GREP_FILE_CAP: usize = 4_000;
const LS_LINE_CAP: usize = 400;
const RUN_OUTPUT_CAP: usize = 64_000;
const RUN_TIMEOUT_CAP: u64 = 300;
const MUTATING: &[&str] = &["write", "edit", "run"];

pub fn definitions() -> Vec<ToolSpec> {
    let obj = |props: Value, required: &[&str]| {
        let mut v = json!({"type": "object", "properties": props});
        v["required"] = json!(required);
        v
    };
    vec![
        ToolSpec {
            name: "read",
            description: "Read a text file, lines numbered like cat -n. Large reads are capped; use offset/limit for more.",
            parameters: obj(
                json!({
                    "path": {"type": "string", "description": "path relative to the workspace root (or absolute inside it)"},
                    "offset": {"type": "integer", "minimum": 1, "description": "1-based first line"},
                    "limit": {"type": "integer", "minimum": 1, "description": "max lines to return"},
                }),
                &["path"],
            ),
        },
        ToolSpec {
            name: "write",
            description: "Create or overwrite a file. Creates parent directories. Content must be the complete file.",
            parameters: obj(
                json!({
                    "path": {"type": "string"},
                    "content": {"type": "string"},
                }),
                &["path", "content"],
            ),
        },
        ToolSpec {
            name: "edit",
            description: "Replace text inside a file. The old_string must match exactly once unless replace_all is true.",
            parameters: obj(
                json!({
                    "path": {"type": "string"},
                    "old_string": {"type": "string"},
                    "new_string": {"type": "string"},
                    "replace_all": {"type": "boolean"},
                }),
                &["path", "old_string", "new_string"],
            ),
        },
        ToolSpec {
            name: "ls",
            description: "List files and subdirectories under a path, up to 3 levels deep.",
            parameters: obj(
                json!({"path": {"type": "string", "description": "default ."}}),
                &[],
            ),
        },
        ToolSpec {
            name: "grep",
            description: "Search files under a path, line by line (regex by default, literal substring when regex=false). Binary files are skipped.",
            parameters: obj(
                json!({
                    "pattern": {"type": "string"},
                    "path": {"type": "string", "description": "file or directory, default ."},
                    "regex": {"type": "boolean", "description": "default true"},
                }),
                &["pattern"],
            ),
        },
        ToolSpec {
            name: "run",
            description: "Run a shell command (sh -c) in the workspace root and capture its output. Hard timeout cap 300s.",
            parameters: obj(
                json!({
                    "command": {"type": "string"},
                    "timeout_sec": {"type": "integer", "minimum": 1, "maximum": 300},
                }),
                &["command"],
            ),
        },
    ]
}

pub fn dispatch(ctx: &ToolCtx, name: &str, arguments: &str) -> ToolOut {
    if MUTATING.contains(&name) && !ctx.auto {
        return ToolOut {
            ok: false,
            output: format!(
                "tool '{name}' mutates the workspace and is currently not allowed; \
                 tell the operator to restart tinycode with --auto to permit it"
            ),
        };
    }
    let args: Value = if arguments.trim().is_empty() {
        Value::Null
    } else {
        match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolOut { ok: false, output: format!("bad tool arguments: {e}") },
        }
    };
    match name {
        "read" => read_file(ctx, &args),
        "write" => write_file(ctx, &args),
        "edit" => edit_file(ctx, &args),
        "ls" => list_dir(ctx, &args),
        "grep" => grep(ctx, &args),
        "run" => run_cmd(ctx, &args),
        other => ToolOut { ok: false, output: format!("unknown tool '{other}'") },
    }
}

fn tool_error(msg: String) -> ToolOut {
    ToolOut { ok: false, output: msg }
}

/// Every tool path must stay inside the workspace root. When the path exists,
/// it is canonicalized so symlink escapes are caught too; otherwise a lexical
/// check handles not-yet-existing files.
fn resolve(ctx: &ToolCtx, raw: &str) -> Result<PathBuf, ToolOut> {
    let path = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        ctx.root.join(raw)
    };
    let normalized = normalize(&path);
    if !normalized.starts_with(&ctx.root) {
        return Err(tool_error(format!("path escapes the workspace root: {raw}")));
    }
    if normalized.exists() {
        let canon = fs::canonicalize(&normalized)
            .map_err(|e| tool_error(format!("{}: {e}", normalized.display())))?;
        if !canon.starts_with(&ctx.root) {
            return Err(tool_error(format!("path escapes the workspace root: {raw}")));
        }
        return Ok(canon);
    }
    Ok(normalized)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn opt_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolOut> {
    opt_str(args, key).map(|v| v).ok_or_else(|| tool_error(format!("missing required argument '{key}'")))
}

fn rel(ctx: &ToolCtx, path: &Path) -> String {
    path.strip_prefix(&ctx.root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.display().to_string())
}

fn read_file(ctx: &ToolCtx, args: &Value) -> ToolOut {
    let raw = match required_str(args, "path") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path = match resolve(ctx, raw) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let offset = opt_u64(args, "offset").unwrap_or(1).max(1) as usize;
    let limit = opt_u64(args, "limit").unwrap_or(4_000).clamp(1, 20_000) as usize;

    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => return tool_error(format!("{raw}: {e}")),
    };
    let mut buf = Vec::new();
    if file
        .take((READ_CAP_BYTES * 2) as u64)
        .read_to_end(&mut buf)
        .is_err()
    {
        return tool_error(format!("{raw}: read failed"));
    }
    if buf.contains(&0) {
        return tool_error(format!("{raw}: looks like a binary file; not usable as text"));
    }
    let text = String::from_utf8_lossy(&buf);
    let mut out = String::new();
    let mut shown = 0usize;
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if n < offset {
            continue;
        }
        if shown >= limit || out.len() > READ_CAP_BYTES {
            out.push_str(&format!("[{raw}: output truncated at line {n}; use offset/limit]\n"));
            break;
        }
        shown += 1;
        out.push_str(&format!("{:>6}\t{}\n", n, line));
    }
    if out.is_empty() {
        return ToolOut { ok: true, output: format!("{raw}: no lines in range") };
    }
    ToolOut { ok: true, output: out }
}

fn write_file(ctx: &ToolCtx, args: &Value) -> ToolOut {
    let (path_raw, content) = match (required_str(args, "path"), opt_str(args, "content")) {
        (Ok(p), Some(c)) => (p, c),
        (Ok(_), None) => return tool_error("missing required argument 'content'".into()),
        (Err(e), _) => return e,
    };
    let path = match resolve(ctx, path_raw) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return tool_error(format!("{path_raw}: mkdir {}: {e}", parent.display()));
        }
    }
    match fs::write(&path, content.as_bytes()) {
        Ok(()) => ToolOut { ok: true, output: format!("wrote {} bytes to {}", content.len(), rel(ctx, &path)) },
        Err(e) => tool_error(format!("{path_raw}: {e}")),
    }
}

fn edit_file(ctx: &ToolCtx, args: &Value) -> ToolOut {
    let path_raw = match required_str(args, "path") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let (old, new) = match (opt_str(args, "old_string"), opt_str(args, "new_string")) {
        (Some(o), Some(n)) => (o.to_string(), n.to_string()),
        _ => return tool_error("missing required arguments 'old_string' and 'new_string'".into()),
    };
    let replace_all = opt_u64_bool(args, "replace_all");
    let path = match resolve(ctx, path_raw) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let source = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => return tool_error(format!("{path_raw}: {e}")),
    };
    let count = source.matches(&old).count();
    if count == 0 {
        return tool_error(format!("{path_raw}: old_string not found"));
    }
    if count > 1 && !replace_all {
        return tool_error(format!(
            "{path_raw}: old_string matches {count} times; provide a larger, more unique string or set replace_all"
        ));
    }
    let updated = if replace_all { source.replace(&old, &new) } else { source.replacen(&old, &new, 1) };
    match fs::write(&path, updated.as_bytes()) {
        Ok(()) => ToolOut { ok: true, output: format!("replaced {count} occurrence(s) in {}", rel(ctx, &path)) },
        Err(e) => tool_error(format!("{path_raw}: {e}")),
    }
}

fn opt_u64_bool(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn list_dir(ctx: &ToolCtx, args: &Value) -> ToolOut {
    let raw = opt_str(args, "path").unwrap_or(".");
    let path = match resolve(ctx, raw) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if !path.is_dir() {
        return tool_error(format!("{raw}: not a directory"));
    }
    let mut out = String::new();
    let mut queue = vec![(path.clone(), 0u8)];
    while let Some((dir, depth)) = queue.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        let mut child_dirs = Vec::new();
        for entry in entries.flatten() {
            if out.lines().count() >= LS_LINE_CAP {
                out.push_str("[ls truncated]\n");
                return ToolOut { ok: true, output: out };
            }
            let name = entry.file_name();
            match entry.file_type() {
                Ok(t) if t.is_dir() => {
                    out.push_str(&format!("{}/\n", rel(ctx, &entry.path())));
                    child_dirs.push(entry.path());
                }
                Ok(_) => out.push_str(&format!("{}\n", rel(ctx, &entry.path()))),
                Err(_) => out.push_str(&format!("{} (unreadable)\n", rel(ctx, &entry.path()))),
            }
            let _ = name;
        }
        if depth < 3 {
            for cd in child_dirs {
                queue.push((cd, depth + 1));
            }
        }
    }
    if out.is_empty() {
        out.push_str("(empty)\n");
    }
    ToolOut { ok: true, output: out }
}

fn grep(ctx: &ToolCtx, args: &Value) -> ToolOut {
    let pattern = match required_str(args, "pattern") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let regex_mode = args.get("regex").and_then(Value::as_bool).unwrap_or(true);
    let re = if regex_mode {
        match Regex::new(pattern) {
            Ok(r) => Some(r),
            Err(e) => return tool_error(format!("bad pattern '{pattern}': {e}")),
        }
    } else {
        None
    };
    let raw = opt_str(args, "path").unwrap_or(".");
    let path = match resolve(ctx, raw) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let mut files = Vec::new();
    if path.is_file() {
        files.push(path);
    } else {
        collect_files(&path, 6, &mut files);
    }
    let mut out = String::new();
    let mut matches = 0usize;
    for file in &files {
        if matches >= GREP_MATCH_CAP {
            break;
        }
        for (line_no, line) in lines_of(file) {
            let hit = match &re {
                Some(r) => r.is_match(&line),
                None => line.contains(pattern),
            };
            if hit {
                matches += 1;
                out.push_str(&format!("{}:{}: {}\n", rel(ctx, file), line_no, line.trim_end()));
                if matches >= GREP_MATCH_CAP {
                    out.push_str(&format!("[grep truncated at {GREP_MATCH_CAP} matches]\n"));
                    break;
                }
            }
        }
    }
    if out.is_empty() {
        return ToolOut { ok: true, output: format!("no matches for '{pattern}'") };
    }
    ToolOut { ok: true, output: out }
}

fn collect_files(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    if out.len() >= GREP_FILE_CAP {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(t) if t.is_dir() => collect_files(&entry.path(), depth - 1, out),
            Ok(t) if t.is_file() => out.push(entry.path()),
            _ => {}
        }
        if out.len() >= GREP_FILE_CAP {
            return;
        }
    }
}

/// Read plain-text lines of a file, skipping anything with a NUL in the first
/// 4 KiB (binary) and capping the read.
fn lines_of(path: &Path) -> Vec<(usize, String)> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut buf = Vec::new();
    if file.take(1 << 20).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    if buf.iter().take(4096).any(|b| *b == 0) {
        return Vec::new();
    }
    String::from_utf8_lossy(&buf)
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.to_string()))
        .collect()
}

fn run_cmd(ctx: &ToolCtx, args: &Value) -> ToolOut {
    let command = match required_str(args, "command") {
        Ok(c) => c,
        Err(e) => return e,
    };
    let timeout = opt_u64(args, "timeout_sec").unwrap_or(120).clamp(1, RUN_TIMEOUT_CAP);

    let mut child = match Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(&ctx.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return tool_error(format!("spawn: {e}")),
    };
    let rx_out = spawn_reader(child.stdout.take(), RUN_OUTPUT_CAP);
    let rx_err = spawn_reader(child.stderr.take(), RUN_OUTPUT_CAP);

    let start = Instant::now();
    let poll = Duration::from_millis(50);
    let mut killed = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Ok(st),
            Ok(None) => {}
            Err(e) => break Err(e),
        }
        if start.elapsed().as_secs_f64() > timeout as f64 {
            let _ = child.kill();
            killed = true;
            break child.wait().map_err(|e| e);
        }
        std::thread::sleep(poll);
    };

    // The process is gone at this point, so the pipes close soon; short
    // timeouts only protect against grandchildren still holding them open.
    let stdout = rx_out.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    let stderr = rx_err.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    let text = match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n--- stderr ---\n{stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => String::new(),
    };
    let mut text = trim_output(&text, RUN_OUTPUT_CAP);
    if killed {
        text.push_str(&format!("[timeout after {timeout}s, process killed]\n"));
    }
    match status {
        Ok(st) if st.success() => ToolOut { ok: true, output: text },
        Ok(st) => {
            text.push_str(&format!("[exit {}]\n", st.code().unwrap_or(-1)));
            ToolOut { ok: false, output: text }
        }
        Err(e) => ToolOut { ok: false, output: format!("{command}: {e}") },
    }
}

fn spawn_reader<R: Read + Send + 'static>(stream: Option<R>, cap: usize) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    match stream {
        Some(s) => {
            std::thread::spawn(move || {
                let mut reader = BufReader::new(s);
                let mut kept = String::new();
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) if kept.len() < cap => {
                            kept.push_str(&String::from_utf8_lossy(&buf[..n]));
                        }
                        Ok(_) => {}
                    }
                }
                // On channel failure the receiving side has hung up already.
                let _ = tx.send(kept);
            });
        }
        None => {
            let _ = tx.send(String::new());
        }
    }
    rx
}

/// Keep the head 2/3 and the tail 1/3 of oversized tool output, with a note.
fn trim_output(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut head = cap / 3 * 2;
    while head > 0 && !s.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = s.len() - cap / 3;
    while tail < s.len() && !s.is_char_boundary(tail) {
        tail += 1;
    }
    let mid = &s[head..tail];
    format!(
        "{}\n[... {} bytes truncated ...]\n{}",
        &s[..head],
        mid.len(),
        &s[tail..]
    )
}
