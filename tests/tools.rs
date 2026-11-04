use std::fs;
use std::path::PathBuf;
use tinycode::tools::{definitions, dispatch, ToolCtx};

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
    let path = std::env::temp_dir().join(format!("tinycode-tools-{tag}-{nanos}"));
    fs::create_dir_all(&path).expect("mkdir tempdir");
    let root = fs::canonicalize(&path).expect("canonicalize");
    TempDir(root)
}

fn ctx(root: &PathBuf) -> ToolCtx {
    ToolCtx { root: root.clone(), auto: true }
}

fn run(ctx: &ToolCtx, name: &str, json: &str) -> tinycode::tools::ToolOut {
    dispatch(ctx, name, json)
}

#[test]
fn all_tools_have_definitions() {
    let defs = definitions();
    assert_eq!(defs.len(), 6);
    for def in &defs {
        assert!(!def.description.is_empty(), "{} has description", def.name);
        assert_eq!(def.parameters["type"], "object", "{} has object schema", def.name);
    }
}

#[test]
fn write_read_edit_roundtrip() {
    let dir = tempdir("roundtrip");
    let ctx = ctx(&dir.0);

    let out = run(&ctx, "write", r#"{"path":"src/main.rs","content":"line one\nline two\nline two\nline three\n"}"#);
    assert!(out.ok, "write: {}", out.output);

    let out = run(&ctx, "read", r#"{"path":"src/main.rs"}"#);
    assert!(out.ok);
    assert!(out.output.contains(" 1\tline one"), "cat -n style: {}", out.output);

    let out = run(&ctx, "edit", r#"{"path":"src/main.rs","old_string":"line two","new_string":"line twice"}"#);
    assert!(!out.ok, "ambiguous edit must fail: {}", out.output);
    assert!(out.output.contains("2 times"), "reports match count: {}", out.output);

    let out = run(&ctx, "edit", r#"{"path":"src/main.rs","old_string":"line two","new_string":"replaced","replace_all":true}"#);
    assert!(out.ok, "replace_all edit: {}", out.output);
    let content = fs::read_to_string(dir.0.join("src/main.rs")).expect("file exists");
    assert_eq!(content, "line one\nreplaced\nreplaced\nline three\n");
}

#[test]
fn path_escapes_are_rejected() {
    let dir = tempdir("escape");
    let ctx = ctx(&dir.0);

    let out = run(&ctx, "read", r#"{"path":"../outside.txt"}"#);
    assert!(!out.ok, "relative escape: {}", out.output);
    assert!(out.output.contains("escapes"));

    let outside = std::env::temp_dir();
    let out = run(&ctx, "read", &format!(r#"{{"path":"{}"}}"#, outside.display()));
    assert!(!out.ok, "absolute outside-root read: {}", out.output);

    // A write under the root, to a path that does not exist yet, is fine.
    let out = run(&ctx, "write", r#"{"path":"deep/nested/new.txt","content":"x"}"#);
    assert!(out.ok, "nested write: {}", out.output);
    assert!(dir.0.join("deep/nested/new.txt").exists());
}

#[test]
fn grep_finds_by_regex_and_literal() {
    let dir = tempdir("grep");
    let ctx = ctx(&dir.0);
    assert!(run(&ctx, "write", r#"{"path":"a.txt","content":"alpha bravo\nalpha bravo again\n"}"#).ok);
    assert!(run(&ctx, "write", r#"{"path":"b.txt","content":"nothing here\n"}"#).ok);

    let out = run(&ctx, "grep", r#"{"pattern":"^alpha b.*$"}"#);
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("a.txt:1:"), "regex hit: {}", out.output);

    let out = run(&ctx, "grep", r#"{"pattern":"BRAVO","regex":false}"#);
    assert!(out.output.contains("no matches"), "literal is case-sensitive: {}", out.output);

    let out = run(&ctx, "grep", r#"{"pattern":"bravo","regex":false}"#);
    assert!(out.output.contains("a.txt:1"), "literal hit: {}", out.output);

    let out = run(&ctx, "grep", r#"{"pattern":"bad-pattern(["™"#);
    assert!(!out.ok, "invalid regex errors: {}", out.output);
}

#[test]
fn read_offset_limit_and_binary_skip() {
    let dir = tempdir("read");
    let ctx = ctx(&dir.0);
    let content: String = (1..=10).map(|n| format!("line {n}\n")).collect();
    let content_json = serde_json::to_string(&content).expect("valid string");
    assert!(run(&ctx, "write", &format!(r#"{{"path":"f.txt","content":{content_json}}}"#)).ok);

    let out = run(&ctx, "read", r#"{"path":"f.txt","offset":3,"limit":2}"#);
    assert!(out.output.contains(" 3\tline 3"));
    assert!(out.output.contains(" 4\tline 4"));
    assert!(!out.output.contains(" 5\t"), "respects limit: {}", out.output);

    fs::write(dir.0.join("bin.dat"), b"\x00\x01\x02binary").expect("binary file");
    let out = run(&ctx, "read", r#"{"path":"bin.dat"}"#);
    assert!(!out.ok, "binary refused: {}", out.output);
    assert!(out.output.contains("binary"));
}

#[test]
fn run_captures_output_exit_codes_and_timeout() {
    let dir = tempdir("run");
    let ctx = ctx(&dir.0);

    let out = run(&ctx, "run", r#"{"command":"printf 'stdout thing'; echo 1>&2 stderr thing"}"#);
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("stdout thing"));
    assert!(out.output.contains("stderr thing"));

    let out = run(&ctx, "run", r#"{"command":"exit 3"}"#);
    assert!(!out.ok, "nonzero exit is not ok");
    assert!(out.output.contains("exit 3"), "{}", out.output);

    let out = run(&ctx, "run", r#"{"command":"sleep 5","timeout_sec":1}"#);
    assert!(!out.ok, "timeout fails");
    assert!(out.output.contains("timeout after 1s"), "{}", out.output);

    let out = run(&ctx, "run", r#"{"command":"pwd"}"#);
    assert!(out.ok);
    let file_name = dir.0.file_name().expect("dir name").to_str().expect("utf8");
    assert!(out.output.contains(file_name), "cwd is the root: {}", out.output);
}

#[test]
fn mutating_tools_are_blocked_without_auto() {
    let dir = tempdir("blocked");
    let mut ctx = ctx(&dir.0);
    ctx.auto = false;

    let out = run(&ctx, "write", r#"{"path":"x.txt","content":"y"}"#);
    assert!(!out.ok, "write refused: {}", out.output);
    assert!(out.output.contains("--auto"), "explains the fix: {}", out.output);
    assert!(!dir.0.join("x.txt").exists(), "nothing written");

    // Read-only tools keep working.
    fs::write(dir.0.join("ok.txt"), "x").expect("manual file");
    let out = run(&ctx, "read", r#"{"path":"ok.txt"}"#);
    assert!(out.ok, "read still allowed: {}", out.output);
}
