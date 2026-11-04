# tinycode

A minimal, resource-efficient coding agent. One static Rust binary, a few MB
of RSS, streaming SSE, capped tool outputs, atomic session files. No node, no
tokio, no expanding queue that eats your VPS. Meant to be operated from
**T3 Code** (or any harness that can run a CLI and read NDJSON).

## What it does

`tinycode run "<task>"` starts an agent loop in a workspace root. The model
gets six tools:

| Tool    | Mutating | What                                                           |
|---------|----------|----------------------------------------------------------------|
| `read`  | no       | cat -n style read, capped, with `offset`/`limit`                |
| `write` | yes      | create/overwrite a file (parent dirs created)                   |
| `edit`  | yes      | exact-string replace; must match once unless `replace_all`      |
| `ls`    | no       | list files, depth 3                                             |
| `grep`  | no       | regex or literal line search, binary files skipped              |
| `run`   | yes      | `sh -c` in the workspace root, timeout, output capped at 64 KB  |

Every path it touches is checked against the workspace root (symlink-safe
through canonicalization). Mutating tools are refused unless you pass
`--auto`.

## Resource story

- single `-C target-feature=+crt-static` musl binary, ~3.6 MB, no glibc NSS issues
- **peak RSS measured at ~2.5 MB** for a full two-turn run (vs hundreds of MB for a Node agent runtime)
- SSE streamed and parsed line-by-line; no unbounded buffers anywhere
- tool outputs hard-capped (read 64 KB, run 64 KB, grep 200 matches), oversized text keep head+tail with a note
- long sessions self-compact: tool results older than the last 8 messages shrink to 400-byte stubs once history passes ~280 KB
- `run` kills timed-out commands; grandchild-safe (readers abandoned at process exit)
- sessions saved by atomic rename at end of each turn — a crash mid-turn just replays that turn on resume

## Build

Any Linux box with Rust:

```sh
cargo build --release
```

Static musl (what you deploy to the VPS; see `deploy/install.sh`):

```sh
# needs a musl cross-cc named x86_64-linux-musl-gcc on PATH (ring's build
# probes that exact name) plus rustup + the musl target
RUSTFLAGS="-C target-feature=+crt-static -C linker=x86_64-linux-musl-gcc" \
  cargo build --release --target x86_64-unknown-linux-musl
```

Result: `target/x86_64-unknown-linux-musl/release/tinycode`, ~3.6 MB, `file`
says `static-pie linked, stripped`. scp it; no runtime deps.

## Configuration (env)

| Variable             | Meaning                                                        |
|----------------------|----------------------------------------------------------------|
| `TINYCODE_API_KEY`   | API key (falls back to `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`)  |
| `TINYCODE_PROVIDER`  | `anthropic` or `openai` (default: which key exists)             |
| `TINYCODE_MODEL`     | model name (default: provider default)                          |
| `TINYCODE_BASE_URL`  | base URL without the versioned path — points it at any OpenAI-compatible endpoint (`http://127.0.0.1:11434/v1`, LM Studio, a router…) |

Provider wire formats: Anthropic Messages and OpenAI Chat Completions, both
streaming, with tool-call chunk reassembly covered by tests.

## CLI

```sh
tinycode run --auto --json --max-turns 24 \
  --root /srv/project \
  --task-file task.txt          # or a positional prompt
```

- `--json` — one NDJSON event per line on stdout for T3 Code:
  `{"type":"start","session":…}`, `{"type":"turn","n":1}`, `{"type":"text",…}`,
  `{"type":"tool","name":…,"input":…}`, `{"type":"tool_result","ok":true,"bytes":…}`,
  `{"type":"done","turns":…,"text":…}`
- without `--json`: final answer text goes to stdout, narration to stderr
- exit codes: `0` done, `1` hard error, `2` hit `--max-turns` (resumable)
- `-s/--session <FILE>` — continue; the file path is in every event
- without `--auto`, write/edit/run are refused with a message telling the
  model to ask for permission — so you can run it read-only first

## Running under systemd (VPS)

See `deploy/`. `tinycode@.service` is a user unit: `systemctl --user start
tinycode@task-name` runs the task file by that name, logs in journald, RAM
capped by the unit.

## Tests

```sh
cargo test
```

17 integration tests: tool semantics (roundtrip, path escape rejection,
grep, timeout/exit codes, `--auto` gating), session roundtrip and provider
guard, arg parsing, and two full agent loops against scripted mock providers
(one per wire protocol) including split tool-call argument chunks and a
runaway-tool-loop capped by `--max-turns`.

## Known limits (chosen, on purpose)

- single provider call at a time; no MCP, no subagents, no menus — depth of stack is the point
- model context bounding relies on output caps + history compaction, not a tokeniser
- the `run` tool shell is `/bin/sh`

## License

tinycode is free software: you can redistribute it and/or modify it under
the terms of the **GNU Affero General Public License** as published by the
Free Software Foundation, either version 3 of the License, or (at your
option) any later version — `AGPL-3.0-or-later` in SPDX. [LICENSE](LICENSE)
carries the full text; there is NO WARRANTY, to the extent permitted by law.
