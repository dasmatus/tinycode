# `tinycode` T3/OpenCode 2 compatibility

## Goal statement

> Make TinyCode usable as a normal OpenCode 2 provider in T3 Code, for provider checks, pervasive chat (create thread, send prompt, receive streamed chat-history and tool events, continue turns), without exceeding a 5 MB stripped release binary.

## Milestones

- [verified] CLI: no manual fallback; `run` and `serve` both clap-driven; remaining tests unaffected.
- [verified] Server: listening line, server info, `/api/model` returns a non-empty catalog, auth.
- [verified] Sessions: create/get/list/update/delete/active semantics.
- [blocked] Events and history: prompt, message filing, SSE stream exposes text/tool events because `/api/model` is fixed first.
- [pending] Model selection: valid model switch, unknown model error, provider mismatch handling.
- [pending] Integration: client-driven end-to-end T3/Shroom surface completes chat flow.
- [pending] Release size: stripped release binary < 5,242,880 bytes.

## Key decisions

- Target OpenCode 2 protocol. At version 2.0.18, T3 uses OpenCode 2 exclusively.
- No async runtime or HTTP dependency. `std::thread::scope` and `std::sync::mpsc` are the primary direction.
- Ordinary chat only: MCP, native approval prompts, subagents, attachments, rollback/fork, and compaction endpoints are explicitly out of scope.
- The release-size check is in CI as a stage. CI runs cargo-clippy, cargo-fmt and then cargo-build, and finally releases if size fits.

## Blockers

- none currently.
