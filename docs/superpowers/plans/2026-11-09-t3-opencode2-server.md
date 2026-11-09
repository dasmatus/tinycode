# OpenCode 2-compatible `tinycode` server

> **For agentic workers:** Use inline execution. Every task uses RED-GREEN
> style steps. The work is already committed in the branch; this plan lists
> the functions and tests an implementer needs to start over exactly.

**Goal:** Make TinyCode a T3 Code-compatible OpenCode 2 provider with a
dependency-free, scoped-thread HTTP server, a fully clap-parsed CLI, streamed
chat including tool events, and no stripped release binary larger than
5,242,880 bytes.

**Architecture:** A clap-only CLI dispatches to `run`, `usage`, ACP, and a new
`serve`. `src/srv/` hosts three pieces: `http.rs` for bounded request and SSE
socket IO, `wire.rs` for OpenCode 2-shaped JSON builders, `state.rs` for
session state and agent worker orchestration over std threads and mpsc.
`src/routes.rs` maps HTTP paths to `state.rs` calls. `src/main.rs` holds no
manual fallback.

**Spec:** `docs/superpowers/specs/2026-11-09-t3-opencode2-design.md`

## Global constraints

- No HTTP framework, TLS stack, async runtime, or new base64 dependency.
- Connection handlers, dispatchers, and agent workers use `std::thread::scope` and `std::sync::mpsc`.
- OpenCode 2 version prefix in `--version` and `/api/info`: `2.0.18-tinycode.0.1.0`.
- Listener line is printed as `opencode server listening on <url>` to stdout.
- No mutex is held while a worker talks to a provider or writes a socket.
- `std::thread::scope` is preferred over `tokio::spawn`.
- Release binary under 5,242,880 bytes.
- Prompt body signature must accept exactly `{sessionID, id, text}`; later fields are tolerated.
- Every HTTP response is JSON with `content-type: application/json` or SSE `text/event-stream`.

## Review Focus

- SSE via a socket gets sent with `Content-Length` or chunked framing. T3's client requires streaming SSE, not a buffered hole. Verify with a TCP test that the byte stream gives `: ping` and `data:` frames upon socket connection without connection close.
- When the outbound HTTP call goes out, the listener must remain responsive. Check that a catalog endpoint still answers while an agent worker is stuck on a slow fake provider HTTP port.
- Sessions survive concurrent provider/order-thread updates. Tests must truly hammer two sessions in parallel and check that each output message for each is correct and each has its own file.
- Session switches must not modify an in-memory model without a session runtime update. Tests must assert provider mismatch creates a fresh file, not refuse the switch.
- Listen socket is closed only at `run()` return, not inside each connection handler. Check with a loopback connect that a still-running listener responds after a client disconnect. If `running` is closed early because a client left before the listener unblocked, this check fails.

---

## Task 1: Clap-pure CLI

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Test: `tests/cli_args.rs`

**Interfaces:**
- Produces:
  - `tinycode::cli::Commands::Serve(ServeArgs)`
  - `tinycode::cli::ServeArgs { hostname: String, port: u16, auto: bool }`
  - `build_config(run_args, global) -> miette::Result<Config>`, equivalent to `config::parse_args` from `RunArgs`.
  - `tinycode::VERSION` returns `2.0.18-tinycode.0.1.0`; `build_version` is the single CLI `--version`.
- Consumes: existing `RunArgs`, `GlobalArgs`, `AcpClientArgs`, `UsageArgs`.

- [ ] **Step: test**

```rust
let cli = tinycode::cli::Cli::try_parse_from([
    "tinycode", "serve", "--hostname", "127.0.0.1", "--port", "4096",
]).expect("parses serve");
assert_eq!(cli.command, Some(tinycode::cli::Commands::Serve(
    ServeArgs { hostname: "127.0.0.1".into(), port: 4096, auto: false }
)));
assert_eq!(tinycode::VERSION, "2.0.18-tinycode.0.1.0");
```

```rust
let args = ["--bogus"]; // no manual fallback
let cfg = build_config(...); // no manual path
```

- [ ] Run: `cargo test cli_args -- --nocapture`
- [ ] Expected: first compile error that `mod cli_args` and `Commands::Serve` do not exist. After RED, fix and watch GREEN.
- [ ] `cargo test` and `cargo fmt -- --check`; commit.

## Task 2: HTTP socket + server info

**Files:**
- Create: `src/srv/mod.rs`, `src/srv/http.rs`
- Modify: `src/cli.rs`; `src/lib.rs`
- Test: `tests/http_seed.rs`

**Interfaces:**
- `tinycode::srv::{ServerConfig, start_server}` where
  `ServerConfig { host: String, port: u16, auto: bool, version: String }`.
- `start_server(cfg) -> ServerHandle { local_addr, join }`; requests to `/api/info` return `{"version":"2.0.18-tinycode.0.1.0","pid":...,"urls":["http://<addr>"],"paths":{"tmp":"/tmp"}}`.

- [ ] Step test:
```rust
let cfg = ServerConfig { host: "127.0.0.1".into(), port: 0, auto: false, version: tinycode::VERSION.to_string() };
let handle = tinycode::srv::start_server(cfg).expect("spawn server");
let body = http_get(handle.local_addr, "/api/info");
assert!(body.contains("2.0.18-tinycode.0.1.0"));
```
- [ ] RED, then implement. Commit.

## Task 3: SSE endpoint

**Files:**
- Modify: `src/srv/http.rs`, `src/srv/mod.rs`
- Test: `tests/http_seed.rs`

**Interfaces:**
- `/api/event` streams an immediate `: ping\n\n` comment plus a later `data: {...server.connected...}` JSON frame when broadcast via the server's bus.
- `ServerHandle::send_event(&self, json: &serde_json::Value)`.

- [ ] Step test:
```rust
let mut socket = connect_and_read_inner_stream(handle.local_addr, "/api/event");
// bytes must be readable before `.shutdown(Shutdown::Both)`.
assert!(inner.contains(": ping"));
```
- [ ] RED, then implement. Commit.

## Task 4: Sessions catalog & errors

**Files:**
- Create: `src/srv/state.rs`, `src/srv/wire.rs`
- Modify: `src/srv/routes.rs` (new), `src/srv/mod.rs`
- Test: `tests/room_routes.rs`

**Interfaces:**
- `Room::new(config) -> Room`
- `Room::session_create(body: &Value) -> RoomActionOutput<Button, Value, String>`
- `Room::session_get(...)/session_delete/...`
- `Room::model_list() -> Value`, `Room::provider_list() -> Value`, `Room::agent_list() -> Value`, `Room::skill_list() -> Value`
- `Room::command_list() -> Value`
- `Room::session_active() -> Value`
- `Response::json_with(status, body)` support (404 shape included).

- [ ] Step test:
```rust
let room = tinycode::srv::Room::new(cfg.clone());
let output = room.session_create(&body_with_location_and_model);
assert_eq!(output.status, 200);
assert_eq!(output.body["data"]["id"].as_str().unwrap().starts_with("ses_"), true);
assert_eq!(room.session_get("ses_missing").status, 404);
```
- [ ] RED then implement. Commit.

## Task 5: Prompt and event flow

**Files:**
- Modify: `src/srv/state.rs`, `src/srv/wire.rs`
- Test: `tests/prompt_events.rs`

**Interfaces:**
- `Room::session_prompt(body) -> RoomActionOutput<u16, Value, String>`
- `Room::session_message_list(session_id) -> Value`
- `Room::model_switch(sessionID, model_spec) -> Value`
- Build t2 client bridge calls through `Room::send_msg` or `Room::fresh_msgs`.
- A helper `tinycode::srv::wire::assistant_message(id, content)` output shape stays stable.

- [ ] Step test using mock provider with scripted SSE:
```rust
let room = tinycode::srv::Room::new(cfg.clone());
let sid = room.session_create(&body_with_location_and_model).body["data"]["id"].as_str().unwrap().to_string();
room.session_prompt(&prompt_body_with_sid_text(&sid, "Please reply with exactly the word: alfa."));
let transcript = room.session_message_list(&sid);
assert_eq!(transcript["data"].as_array().unwrap().iter().filter(|m| m["type"]=="assistant").count(), 1);
```
- [ ] RED. Implement chat flow. GREEN. Commit.

## Task 6: Model switch + provider mismatch

**Files:**
- Modify: `src/srv/state.rs`, `src/srv/wire.rs`
- Test: `tests/model_switch.rs`

**Interfaces:**
- `Room::model_switch(sessionID, model_spec) -> RoomActionOutput<u16, Value, String>`.
- Unknown provider ID -> HTTP 400.
- Same provider -> update `Meta.model_ref`; provider change from configured -> 400. provider change from session must load a different wire provider from session file rather than error.
- `session_switch_agent` only applies `build` and `plan`.

- [ ] Step test:
```rust
let room = tinycode::srv::Room::new(cfg.clone());
let sid = room.create_with(&model_ref("anthropic", "claude-3-5-sonnet"));
assert_eq!(room.model_switch(&sid, "anthropic/claude-3-5-haiku").status, 200);
assert_eq!(room.model_switch(&sid, "openai/gpt-4.1-mini").status, 400);
```
- [ ] RED, implement, GREEN. Commit.

## Task 7: Full prompts with tools and interrupt

**Files:**
- Modify: `src/srv/state.rs`, `src/srv/wire.rs`
- Test: `tests/agent_flow_full.rs`

**Interfaces:**
- Two-session prompt concurrently.
- First prompt uses scripted tool `write`; second prompt uses scripted tool `read` and no assistant tool entries in history for the first.
- Test `Room::session_interrupt` returns `{interrupted: true}` plus verifies the session file not to be blocked.

- [ ] Step test:
```rust
// concurrent prompt on two sessions
let handles = ["ses_a", "ses_b"].iter().map(|sid| thread::spawn(...room.session_prompt(...))).collect::<Vec<_>>();
for h in handles { assert!(h.join().is_ok()); }
assert_eq!(room.session_message_list("ses_a")["data"].as_array().unwrap().iter().filter(|m| m["type"]=="assistant").count(), 1);
assert_eq!(room.session_message_list("ses_b")["data"].as_array().unwrap().iter().filter(|m| m["type"]=="assistant").count(), 1);
```
- [ ] RED, implement, GREEN. Commit.

## Task 8: Release size check

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] Step: CI step after tests:
```yaml
- run: cargo build --release
- run: test "$(stat -c %s target/release/tinycode)" -le 5242880
```
- [ ] Node check in the workflow itself, not a Rust test. Commit.

## Task 9: T3 verification & final review

**Files:**
- Read only: T3 config.
- Run T3 against a fake OpenCode binary path with tinycode and its `--version`, then invoke a chat prompt, while asserting queued sessions and SSE.
- Full final review before low-risk cleanup.

- [ ] Step: CI workflow stage lock plus full release binary size check.

- [ ] Step: run the T3 UI provider toggle and screenshot a successful provider-ready view.

- [ ] Step: point the T3 provider binary path to the built tinycode, then open a real T3 chat prompt and screenshot its interaction.

- [ ] Step: run the final review as described in the spec; write a fresh reviewer summary into the worklog.

## Post-implementation checklist

- [ ] `.opencode/goal-state.md` reflects all verified milestones.
- [ ] `.worklog/log.jsonl` has one entry per verified milestone.
- [ ] No manual fallback in `main.rs`.
- [ ] All tests green.
- [ ] `cargo fmt -- --check` clean.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` clean.
- [ ] Release binary under limit.
