# TinyCode OpenCode 2 server design

## Intent

Make TinyCode work as an OpenCode 2 provider in T3 Code. A user can enable the provider, see its configured model, create a thread, send prompts, watch assistant text and tool activity, and continue the conversation. The stripped release binary must remain below 5,242,880 bytes.

## Current failure

T3 Code classifies `tinycode 0.1.0` as incompatible because OpenCode 2 requires a 2.x version. TinyCode also has no `serve` subcommand or HTTP API, so passing the version check alone would not make chat work. The CLI uses clap for newer subcommands but retains a second manual parser and dispatch path for legacy invocation.

## Design

### CLI

Clap owns every command and option. Add `tinycode serve` with `--hostname=127.0.0.1`, `--port=0`, and `--auto`; the server resolves the current environment and OpenCode settings itself, like ACP mode. Remove the manual fallback from `main.rs`; construct run configuration directly from clap's `RunArgs` instead of serializing arguments back into strings for the legacy parser. Keep the existing configuration resolver for environment and OpenCode settings.

`--version` prints the package version line containing `tinycode 2.0.18-tinycode.0.1.0`; T3's generic probe extracts the semver prefix. The server version stays in sync across both paths. The listener line goes to stdout as `opencode server listening on <url>`.

### CLI compat

`tinycode run` keeps traditional `--auto` behavior: mutating tools are refused without it. `tinycode serve` enables the same tool set when `--auto` is passed; without it it is read-only, exactly like the CLI. T3 Code is expected to launch it from its provider configuration with `--auto`; this preserves the operator's existing opt-in behavior rather than changing tool gating.

### HTTP server

Add a small OpenCode 2 server using `std::net::TcpListener` and `TcpStream`. Do not add dependencies. Parse bounded HTTP requests, JSON bodies through `serde_json`, deep-object location queries, and UTF-8 Basic authentication when `OPENCODE_PASSWORD` is set. Print `opencode server listening on http://<host>:<port>` after binding so T3 can discover the managed server.

Implement the routes T3 uses for provider discovery and ordinary chat. Responses follow OpenCode's `{ "data": ... }` convention. Request paths, body fields, event spellings, and IDs come from the OpenCode 2 schema in `/tmp/opencode/tinycode-opencode-v2-openapi.json`; the test fixtures must assert those exact shapes, including catalog model IDs, session timestamps, message IDs, token summaries, event envelope fields, and SSE framing.

- server info, model/provider/agent catalogs, empty skill and command catalogs;
- create/get/list/update/delete sessions and report active sessions;
- switch session model and agent, and store T3 runtime instructions;
- submit prompts, read message history, and request interruption;
- return empty pending permission and form lists;
- stream `server.connected`, execution, step, text, tool, and idle events over `/api/event` SSE.

Expose the currently configured TinyCode provider and model as a one-model catalog. Honor later model selections through the existing qualified-model resolver; unknown selections return the protocol invalid-request error rather than silently selecting the default. If a session file changes provider between model switches, create a new TinyCode session file instead of entering it with the old provider.

- server info, model/provider/agent catalogs, empty skill and command catalogs;
- create/get/list/update/delete sessions and report active sessions;
- switch session model and agent, and store T3 runtime instructions;
- submit prompts, read message history, and request interruption;
- return empty pending permission and form lists;
- stream `server.connected`, execution, step, text, tool, and idle events over `/api/event` SSE.

Expose the currently configured TinyCode provider and model as a one-model catalog. Honor later model selections through the existing qualified-model resolver. Return protocol-shaped errors instead of HTML or unstructured text.

### Threads and events

Use `std::thread::scope` for connection, dispatcher, and agent workers. Use `std::sync::mpsc` channels for agent results and SSE subscribers. A dispatcher owns conversion of TinyCode agent events into OpenCode events and message history. Shared session metadata and subscriber senders use short-held mutexes; no mutex is held during provider network calls or socket writes.

Each prompt runs the existing blocking `Agent` on a scoped worker. Assistant chunks become text start/delta/end events. Tool calls become input-started, called, and success/failed events. Completion emits step and execution terminal events so T3 does not leave a turn running. Session files continue using TinyCode's existing atomic store.

The session workspace comes from the session create body's location directory and is canonicalized for path-safety. If the client omits it, the server's current directory is the fallback. A later session move is outside this patch; future work should create a moved session rather than changing the worker's root. T3's per-session runtime instructions are stored per session and appended to TinyCode's built-in system prompt, so workspace path rules and completion behavior remain.

Submitting another prompt to a session that is already running returns the OpenCode busy/conflict error. Sessions may run in parallel; each owns its own session file. Prompt IDs are deterministic so messages map cleanly back to turns.

Interruption is cooperative between agent turns. Late worker results are discarded after interruption so they cannot complete a newer T3 turn.

### Size and verification

The baseline stripped release binary is 4,285,568 bytes. Add a release-size check in the CI workflow as a standalone step that builds the release binary and fails above 5,242,880 bytes. Locally, run the same script; ordinary tests do not use the release profile. No HTTP framework, TLS stack, async runtime, or new base64 dependency is added.

Tests will exercise real TCP requests and SSE frames against the server, including the same wrapped response shapes and event fields required by T3's OpenCode 2 client. Tests will also cover clap dispatch, authentication, model switching, prompt continuation, tool events, and failed provider runs. Run the full Rust suite, formatting, clippy, and the release-size check.

## Boundaries

This change targets ordinary T3 chat, not the whole OpenCode 2 API. MCP, native permission prompts, subagents, image attachments, fork/rollback, compaction commands, and synthetic handoff messages are outside this patch. TinyCode's existing tool permission behavior remains in force. In-memory T3 session metadata is not a new cross-process session database; TinyCode agent history retains its current file-based behavior.

## Success criteria

- `tinycode --version` passes T3's OpenCode 2 version probe.
- T3 can start `tinycode serve` and decode server info and a nonempty model catalog.
- A T3 client can create a session, submit a prompt, decode streamed text/tool events, and read coherent history.
- Existing CLI and ACP tests keep passing.
- The stripped release binary stays below 5 MB.
