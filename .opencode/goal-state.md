# `tinycode` T3/OpenCode 2 compatibility

## Goal statement

> Make TinyCode usable as a normal OpenCode 2 provider in T3 Code, for provider checks, pervasive chat (create thread, send prompt, receive streamed chat-history and tool events, continue turns), without exceeding a 5 MB stripped release binary.

## Milestones

- [verified] CLI: no manual fallback; `run` and `serve` both clap-driven; remaining tests unaffected.
- [verified] Server: listening line, server info, `/api/model`, `/api/provider`, `/api/agent`, skills/commands, and session routes are catalog-shaped.
- [verified] Sessions: create/get/list/update/delete/active semantics.
- [verified] Events and history: prompt, message filing, SSE stream exposes text/tool events.
- [verified] Model selection: valid model switch, unknown model error, provider mismatch handling.
- [verified] Integration: client-driven end-to-end T3/Shroom surface completes chat flow.
- [verified] Release size: stripped release binary < 5,242,880 bytes.
- [verified] T3 surface: local HTTP server, sessions, prompts, model list, and Spit-code path all compile, run, and record tracked evidence.
## Key decisions

- Target OpenCode 2 protocol. At version 2.0.18, T3 uses OpenCode 2 exclusively.
- No async runtime or HTTP dependency. `std::thread::scope` and `std::sync::mpsc` are the primary direction.
- Ordinary chat only: MCP, native approval prompts, subagents, attachments, rollback/fork, and compaction endpoints are explicitly out of scope.
- The release-size check is in CI as a stage. CI runs cargo-clippy, cargo-fmt and then cargo-build, and finally releases if size fits.

## Blockers

- none.

## Evidence

- `cargo test --quiet` → 46/46 passing.
- `cargo clippy --all-targets --all-features -- -D warnings` → no output.
- `cargo fmt -- --check` → no output.
- `cargo build --release` → release binary 4,318,368 bytes, below 5,242,880.
- Local server smoke test: `/api/info`, `/api/model`, `POST /api/session`, `POST /api/session/ses_1/prompt` all returned 200 with OpenCode-shaped JSON.
