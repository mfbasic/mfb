# plan-129: `logger` source package

Last updated: 2026-09-10
Effort: medium (1h–2h)

This plan creates `packages/logger`, a distributable MFBASIC source package. An importer will create a UTC log entry once, format it as `[TIMESTAMP] [LEVEL] MESSAGE`, then send it first to common backends and next to backends selected for that exact level.

References:

- `packages/cli/project.json`, `packages/cli/src/model.mfb`, and `packages/cli/README.md` — current small source-package manifest, export, test, and documentation precedent.
- `src/docs/spec/language/13_modules-and-packages.md` — source-package exports.
- `src/docs/spec/language/15_resource-management.md` §15.4/§15.6 — resource fields, collection slots, and aliases.
- `src/codegen/builtins/fs/func_write_all.rs` and `src/codegen/builtins/tcp/func_write.rs` — the current open-file and connected-TCP text-write APIs.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| No existing active plan claims number 129 | `find planning -maxdepth 1 -type f -name 'plan-129*' -print | wc -l` → 0 before this file is created | MET — this plan is the single current match (`find planning -maxdepth 1 -type f -name 'plan-129*' -print | wc -l` → 1) |
| The release compiler reflects HEAD | `cargo build --release && test target/release/mfb -nt Cargo.toml` | MET — `cargo build --release && test target/release/mfb -nt Cargo.toml` exited 0 |
| No acceptance gate holds the repository lock | `pgrep -f 'scripts/(test-accept|artifact-gate)\\.sh'` produces no process | MET — `pgrep -f 'scripts/(test-accept|artifact-gate)\\.sh'` exited 1 (`lock_scan_status=1`) |

Everything below assumes the current transport surface: stream sockets are `tcp::Socket` and text is sent with `tcp::write`; `net` owns no socket resource.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every command and update every status before work begins and before it stops.

## 1. Goal

- `mfb build packages/logger` produces `packages/logger/logger.mfp`; a clean importer can construct the exported logger model and `logger::log` delivers one UTC ISO-formatted entry to every common backend, followed by every backend selected for the entry’s exact `LogLevel`.

### Non-goals

- No compiler feature, built-in package, native helper, thread-safety mechanism, queue, batching, retry, filtering, rotation, reconnect, or structured-output format is added.
- The package neither opens, closes, flushes, nor takes responsibility for caller-provided file/socket handles.
- A backend present in both lists deliberately receives two dispatches, common first.
- `ConsoleBackend.use_colors` remains exported configuration but does not alter the requested `io::print` behavior; terminal color sequences require a separately specified contract.
- Existing APIs, specs, and goldens are not changed merely for this package.

## 2. Current State

- There is no `packages/logger`. Existing package directories: 7 (`find packages -mindepth 1 -maxdepth 1 -type d -print | sort | wc -l` → 7); package source files: 54 (`find packages -path '*/src/*.mfb' -type f | wc -l` → 54); package tests: 25 (`rg -l '^TESTING$' packages -g '*.mfb' | wc -l` → 25).
- `packages/cli` is the current normal `kind: package` source-package precedent; `mfb test packages/cli` runs its in-source `TESTING` blocks.
- `fs::writeText` takes a path and truncates it. `fs::writeAll(file, value)` writes to an already-open `fs::File` at its current cursor (`src/codegen/builtins/fs/func_write_all.rs`).
- The supplied `net::Socket` / `net::sendText` names are obsolete: `net` owns no resources (`src/codegen/builtins/net/mod.rs`); `tcp::Socket` is the connected stream resource and `tcp::write` accepts `String` (`src/codegen/builtins/tcp/mod.rs`, `func_write.rs`).
- A record field marked `RES` carries an alias to one resource. `LogBackend` remains a data union of backend records (not a resource union), so `Logger.common` and `Logger.backends` are ordinary collections of `LogBackend` values.

### Verified properties

- A resource union must use direct resource variants; mixing a direct resource and a data variant is rejected (`src/ir/verify/types.rs:check_type_definitions`, test `rejects_mixed_resource_union`). The proposed variants are records, so this is a data union with resource-bearing fields.
- `fs::writeAll` advances the open file cursor. The logger must append `"\n"` itself (`src/codegen/builtins/fs/func_write_all.rs`).
- `FUNC(LogEntry) AS Nothing` is valid function-value syntax; the same Nothing-returning function-value form is exercised in `tests/syntax/functions/lambda-mut-capture-invalid/src/main.mfb`.

## 3. Design Overview

`packages/logger/project.json` declares `logger` version `0.1.0` as a normal source package. `src/model.mfb` imports `collections`, `datetime`, `fs`, `io`, and `tcp`, then exports:

- `LogLevel`: `DEBUG`, `INFO`, `WARN`, `ERROR`.
- `LogEntry { level AS LogLevel, timestamp AS String, message AS String }`.
- `ConsoleBackend { use_colors AS Boolean }`.
- `FileBackend { file AS RES fs::File }`.
- `NetworkBackend { socket AS RES tcp::Socket }`.
- `CustomBackend { callback AS FUNC(LogEntry) AS Nothing }`.
- `LogBackend`, the union of those four records; and `Logger { common AS List OF LogBackend, backends AS Map OF LogLevel TO List OF LogBackend }`.

`src/logger.mfb` exports `formatEntry(entry)` and `log(logger, level, msg)`, with private `doLog(backend, entry, formattedText)`. `log` calls `datetime::now` once, converts it with `datetime::toUtc`, renders it with `datetime::toIso`, constructs one entry, then computes the formatted text once. It dispatches `common` in list order, then dispatches the exact-level list only when `collections::hasKey` is true.

`doLog` prints formatted text to console, calls `fs::writeAll(file, formattedText & "\n")`, calls `tcp::write(socket, formattedText)`, or invokes the custom callback with the original entry. It retains the supplied defensive `CASE ELSE` that fails with `77050002`. The supplied sketch declared three parameters but called `doLog` with two; this plan resolves that by passing all three arguments.

Correctness risk is resource aliases through backend records/collections and two-pass ordering. This changes runtime behavior, so package tests and a clean consumer runtime proof—not byte identity—are the gates. TCP verification uses a local loopback fixture, never a public host.

Rejected alternatives: retaining `net::Socket` cannot compile against the current registry; using `fs::writeText` with a handle cannot type-check and would imply truncating path semantics.

## 4. Detailed Design

### 4.1 Public use

```mfb
IMPORT logger

LET common AS List OF logger::LogBackend = [
  logger::ConsoleBackend[use_colors := FALSE]
]
LET byLevel AS Map OF logger::LogLevel TO List OF logger::LogBackend = _
  Map OF logger::LogLevel TO List OF logger::LogBackend {}
LET app = logger::Logger[common := common, backends := byLevel]
logger::log(app, logger::LogLevel.INFO, "started")
```

The caller owns creation and lifecycle of all resource handles. A custom backend receives the structured entry rather than its formatted text, preserving its level, timestamp, and message.

### 4.2 Dispatch contract

For one `log` call:

1. Read one instant, normalize it to UTC, and create exactly one entry.
2. Format that entry exactly once.
3. Dispatch every common backend in source-list order.
4. If the level key exists, dispatch every matching backend in source-list order.

Console and TCP receive no newline beyond the message itself; a file receives exactly one appended newline per dispatch. I/O/callback failures propagate—there is no swallowing or retry policy.

## Phases

### Phase 1 — Package model and resource-shaped tests

Establish the distributable type graph before dispatch behavior.

- [x] Add `packages/logger/project.json` with `name: logger`, version `0.1.0`, `mfb: 1.0`, `kind: package`, a `src` package source root, and an accurate description.
- [x] Add `packages/logger/src/model.mfb` with every exported enum, record, union, and logger record above; use `RES fs::File` and `RES tcp::Socket` fields.
- [x] Add package `TESTING` coverage for model construction, empty maps/lists, a callback function value, and a file backend placed in a list to prove the resource-alias shape.
- [x] Add a clean consumer fixture/smoke script that builds against `packages/logger/logger.mfp` and names every exported type.

Acceptance: `mfb build packages/logger` writes `logger.mfp`; `mfb test packages/logger` and the clean consumer build prove the entire type graph, including resource-bearing backend records, resolves through the package.
Commit: 2820a62a7

### Phase 2 — Formatting and ordered backend dispatch

Implement behavior and pin all observable backend contracts.

- [x] Add `packages/logger/src/logger.mfb` with exported `formatEntry` and `log`, private `doLog(backend, entry, formattedText)`, required imports, and the `CASE ELSE` `77050002` guard.
- [x] Implement the console/file/TCP/custom branches with `io::print`, `fs::writeAll`, `tcp::write`, and callback invocation; create one entry/time and formatted string before both dispatch passes.
- [x] Add package tests for `formatEntry` at all levels, custom-entry delivery, absent-level routing, common-before-level routing, and intentional duplicate delivery.
- [x] Add a runnable loopback consumer proof using an append-mode temporary file, a local TCP listener/peer, and a callback collector; assert console/file/peer/callback output and ordering.

Acceptance: an INFO call produces the exact `[UTC-ISO] [INFO] message`; the file contains that text plus one newline, TCP receives the unmodified text, the callback receives matching structured fields, and common delivery precedes INFO-specific delivery.
Commit: 8bf6be8de

### Phase 3 — Documentation and final package proof

Document the actual transport/resource contract and validate the shipped artifact.

- [x] Add `packages/logger/README.md` with build/test commands, `file:packages/logger/logger.mfp` consumer setup, every backend constructor, formatting/ordering/newline rules, and caller resource-lifecycle responsibility.
- [x] Make the README and manifest description agree on `tcp`, not the retired `net` stream API, and state the version-0.1.0 `use_colors` behavior.
- [x] Run `mfb pkg doc packages/logger/logger.mfp` and rebuild the README's clean consumer without in-tree source resolution.
- [x] Run the repository gates; investigate every unexpected artifact/golden diff before treating it as intentional. — user-directed completion: `cargo test` was attempted and is blocked by the host LibreSSL 3.3.6 CLI lacking `openssl s_server -naccept`; logger-specific build, tests, clean consumer, and loopback proof pass.

Acceptance: documented commands succeed and the packaged-only consumer reproduces the dispatch proof. The full repository suite was attempted but is host-blocked by LibreSSL; completion is explicitly user-directed.
Commit: b4e6802fb

## Validation Plan

- Tests: `mfb test packages/logger` covers the model, formatting, callbacks, absent-level routing, common-before-level routing, duplicate routing, and file output. The loopback consumer verifies TCP delivery without external networking.
- Coverage check: map each declaration from `rg -n '^EXPORT (TYPE|ENUM|UNION|FUNC)' packages/logger/src` to a package test and consumer assertion.
- Runtime proof: build `packages/logger`, then build/run a clean consumer against `packages/logger/logger.mfp`; compare stdout, file contents, callback trace, and loopback payload to exact expected values.
- Doc sync: `packages/logger/project.json`, `packages/logger/README.md`, and generated `mfb pkg doc` output. No embedded compiler spec changes are required.
- Acceptance: `cargo test`, `cargo build --release`, `mfb build packages/logger`, `mfb test packages/logger`, the clean consumer proof, and `scripts/test-accept.sh target/release/mfb target/accept-actual`.

## Open Decisions

- Console colors — recommend retaining `use_colors` as a no-op exactly as the requested dispatch body specifies. ANSI sequences and platform behavior should be a separately scoped contract.
- Network name — recommend retain requested public `NetworkBackend` while using `RES tcp::Socket` and `tcp::write`; renaming it `TcpBackend` is clearer but changes the requested API.
- File newline — recommend preserve the supplied behavior: add exactly one `"\n"` for each file dispatch, while console/TCP receive the formatted text unchanged.

## Corrections

- The supplied `net::Socket`/`net::sendText` API is retired; the current equivalent is `tcp::Socket`/`tcp::write` (`rg -n 'net has NO resources|write.*String' src/codegen/builtins/net/mod.rs src/codegen/builtins/tcp/func_write.rs`).
- `fs::writeText(fb.file, ...)` is invalid because `writeText` is path-based and truncating; use `fs::writeAll` on the open file.
- The two supplied `doLog` calls omitted the helper’s third parameter; callers must pass the precomputed formatted message.
- The clean consumer must import `net` when it reads `tcp::localAddress(...).port`; package imports are not transitive (`packages/logger/smoke.sh target/release/mfb` initially raised `TYPE_UNKNOWN_VALUE`, then passed after the explicit import).
- A function-valued record field cannot be invoked directly; bind `customBackend.callback` to a `FUNC(LogEntry) AS Nothing` local before calling it (`target/release/mfb test packages/logger` initially raised `MFB_PARSE_UNEXPECTED_TOKEN`).
- `mapfile` is unavailable in the bundled macOS Bash, so the loopback proof reads its three output lines from a temporary file (`packages/logger/runtime-smoke.sh target/release/mfb`).
- Full `cargo test` reached the TLS integration test `tests/net/rt_tls_connect_allow_self_signed.rs`, where macOS LibreSSL 3.3.6 rejects the required `openssl s_server -naccept` option. The test explicitly refuses a LibreSSL fallback because it would make the TLS proof unsound; user directed completion after the logger-specific gates passed.

## Summary

This is a pure source-package addition. It preserves the requested data model, UTC timestamp, format, and two-pass routing while adapting the stale network/file calls and helper arity to the current MFBASIC surface.
