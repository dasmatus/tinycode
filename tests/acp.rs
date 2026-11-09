use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn tinycode() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tinycode"))
}

fn tmpdir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("tinycode-acp-test-{name}-{nanos}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn show(out: &Output) -> String {
    format!(
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn acp_server_handshake_via_client() {
    let cwd = tmpdir("handshake");
    let out = tinycode()
        .args([
            "acp-client",
            "--handshake-only",
            "--cwd",
            cwd.to_str().unwrap(),
        ])
        .output()
        .expect("run acp-client");

    assert!(
        out.status.success(),
        "acp-client handshake failed:\n{}",
        show(&out)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("initialized")
            || stderr.contains("session created")
            || stderr.contains("ACP handshake OK"),
        "expected handshake markers in stderr:\n{}",
        show(&out)
    );

    let _ = std::fs::remove_dir_all(&cwd);
}

#[test]
fn acp_subcommand_is_exposed() {
    // `tinycode acp` starts a stdio server; just verify the binary recognizes
    // the subcommand path far enough to not print the "no task given" legacy
    // error. We can't easily do a full server smoke test without a framed
    // client here, so we rely on acp_server_handshake_via_client for that.
    let help = tinycode().args(["--help"]).output().expect("run --help");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    assert!(text.contains("acp"), "help should mention acp:\n{text}");
    assert!(
        text.contains("acp-client")
            || text.contains("AcpClient")
            || text.to_lowercase().contains("acp-client"),
        "help should mention acp-client:\n{text}"
    );
}
