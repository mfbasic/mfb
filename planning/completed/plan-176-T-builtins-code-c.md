# plan-176-T: Built-ins code, part C (net, process, regex, tls, vector)

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-S

T splits the nine over-budget code files of `src/codegen/builtins/` in the T list: the ICMP
ping lowering (`net/gen_ping.rs`), the two process backends (`process/gen_unix.rs`,
`process/gen_windows.rs`), the regex package (`regex/mod.rs`), the four TLS backends
(`tls/gen_macos/client.rs`, `tls/gen_macos/server.rs`, `tls/gen_openssl.rs`,
`tls/gen_schannel_server.rs`) and the SIMD float kernels (`vector/builder_simd_float_math.rs`).
Eight files are pure code motion: each becomes a directory module whose `mod.rs` keeps the
surface, except `regex/mod.rs`, which gains a sibling, and `gen_schannel_server.rs`, which is
`include!`d and so splits into a second included file. One function is over the 1000-LOC code
limit on its own: `lower_tls_listen_macos` (1007). It is decomposed by extract-method refactoring
into three phase functions and two private structs, in its own phase, after every pure-motion
phase. Every phase is gated by byte-identical codegen.

Single outcome: after T lands, `cargo test --test file_size_budget` prints no row for any of the
nine files or the files they become, and `tests/guards/file_size_ceilings.txt` has no `ceiling`
row for any of them. The rule (owner, 2026-10-18): code ≤ 1000, inline test ≤ 250, test-only file
≤ 1000 non-comment non-blank LOC; there are no exemptions.

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, the guard, and the
  "Splitting an over-budget file (plan-176)" conventions in `.ai/build-tooling.md`.
- `.ai/compiler.md`, `.ai/codegen-invariants.md` — vreg allocation order and register lifetimes:
  the extract phase must not reorder a `vregs.next()`, `HelperScratch::declare` or instruction push.
- `.ai/net-tls.md` — TLS readiness/timeout behavior the macOS listen lowering implements.
- `.ai/arch-abi.md` — names `gen_windows.rs` (§4.9 repoints it); Win64 spawn ABI.
- `.ai/testing-gates.md` — the artifact gate, per-target goldens (`windows-x86_64` carries `tls`
  and `process` goldens), never two gates at once.
- `.ai/build-tooling.md` — the clippy precision trap that names `builder_simd_float_math.rs`
  (§4.8 repoints it); rustfmt policy (`rustfmt --edition 2021 <files>`).
- Spec topics whose citations move: `stdlib/16_icmp.md`, `stdlib/17_transports.md`,
  `architecture/18_math-kernels.md`, `language/18_builtin-functions.md`.
- Split maps used to write this plan: `/tmp/loc/maps/T.md`, `/tmp/loc/maps/T-extract.md`
  (`lower_tls_listen_macos`; it supersedes T.md's exemption note for `server/listen.rs`).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-S complete and archived | `ls planning/plan-176-S-* planning/completed/plan-176-S-*` → only `planning/completed/…` | MET (2026-10-20, `ls planning/completed/` → `plan-176-S-builtins-code-b.md`; none in `planning/`) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-20, `cargo test --test file_size_budget` → passes) |
| T's files have no uncommitted changes | `git status --porcelain -- src/codegen/builtins/net src/codegen/builtins/process src/codegen/builtins/regex src/codegen/builtins/tls src/codegen/builtins/vector src/codegen/builtins/math/gen_pow.rs src/codegen/builtins/tests src/target/macos_aarch64/tls.rs src/docs/spec/stdlib/16_icmp.md src/docs/spec/stdlib/17_transports.md src/docs/spec/architecture/18_math-kernels.md src/docs/spec/language/18_builtin-functions.md tools/net-probes tools/math-kernels .ai/build-tooling.md .ai/arch-abi.md tests/codegen/codegen_tls_c_return_x86_64.rs tests/net/rt_tls_connect_allow_self_signed.rs tests/codegen/codegen_helper_scratch_release.rs tests/guards/file_size_ceilings.txt` → empty | MET (2026-10-20, empty output) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify — a
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you. The reader needs to know how far off the gate is,
> not just which row you hit first.

## 1. Goal

- Each of the nine files is replaced by a directory module (or, for `regex/mod.rs` and
  `gen_schannel_server.rs`, gains a sibling) whose every file is ≤ 1000 code LOC, and ≤ 900 except
  `tls/gen_openssl/connect.rs` (one function of 964 LOC, §3), by
  `python3 -I tools/loc-census/census.py --file <path>`.
- No function in T's files is over 1000 code LOC. `lower_tls_listen_macos` is decomposed exactly
  as §4.10 specifies; the largest function afterwards is `lower_tls_connect_openssl` (964, unchanged).
- `.ncode` (and every other artifact-gate dump kind) is byte-identical on every target before and
  after each phase.
- Every `[[…]]` spec citation, `.ai/` reference and code comment that names one of the nine old
  paths names the file that now holds the cited symbol.

### Non-goals (explicit constraints)

- No behavior change and no change to emitted code on any target.
- No logic edit except the decided extraction (§4.10). Pure-motion phases move text verbatim; the
  only other edits are `pub(super)` visibility, `use` lines, `mod` declarations, re-exports, one
  `#[path]` attribute (§4.6), one `include!` line (§4.7) and comment repoints.
- No crate path that callers use changes: `net::gen_ping::lower_net_ping_helper`,
  `process::gen_unix::*`/`gen_windows::*` (glob importers), `tls::gen_openssl::lower_tls_*_openssl`,
  `tls::gen_macos::{client,server}` re-exports, `vector::builder_simd_float_math::{FloatKernel,
  FloatBinaryKernel, MATH_CONST_POOL_SYMBOL, math_const_pool_words, math_const_pool_data_value}`
  all resolve unchanged.
- No test fn renamed, no test module renamed (`error_path_release_tests` and `pool_layout_tests`
  keep their names), no `[[test]]` target renamed. The unit-test count of the `mfb` binary is
  unchanged by every phase.
- No guard weakened; no guard needs an edit (§2).
- `bugs/*.md` that name the old paths are historical records and are not edited.

## 2. Current State

### Files (as of 17a9b5799)

LOC from `/tmp/loc/groups.json` (planning census `python3 -I /tmp/loc/census.py "$PWD"`), re-measured
on 2026-10-17 with `python3 -I /tmp/loc/span.py <file>` → same values. After A the command is
`python3 -I tools/loc-census/census.py --file <path>`. Since 17a9b5799 only `regex/mod.rs` changed
(`git diff 17a9b5799 HEAD -- <the nine files>` → one line inside `DESC`, line 213, same line count),
so every line range below is valid at HEAD.

| File | Kind | Code LOC | Inline test LOC | `wc -l` |
|---|---|---|---|---|
| `src/codegen/builtins/net/gen_ping.rs` | code | 1567 | 102 (stays inline) | 2054 |
| `src/codegen/builtins/process/gen_unix.rs` | code | 1134 | 0 | 1432 |
| `src/codegen/builtins/process/gen_windows.rs` | code | 1191 | 101 (stays inline) | 1642 |
| `src/codegen/builtins/regex/mod.rs` | code | 1003 | 169 (stays inline) | 1294 |
| `src/codegen/builtins/tls/gen_macos/client.rs` | code | 2025 | 0 | 2392 |
| `src/codegen/builtins/tls/gen_macos/server.rs` | code | 2041 | 0 | 2448 |
| `src/codegen/builtins/tls/gen_openssl.rs` | code | 2976 | 301 at planning; moved to `gen_openssl/tests/mod.rs` by plan-176-H | 3812 |
| `src/codegen/builtins/tls/gen_schannel_server.rs` | code | 1005 | 0 | 1243 |
| `src/codegen/builtins/vector/builder_simd_float_math.rs` | code | 2147 | 47 (stays inline) | 2850 |

State left by earlier letters:
- plan-176-H moves `gen_openssl.rs`'s `mod error_path_release_tests { … }` (lines 3435-3812, 9
  tests) to one test-only file `tls/gen_openssl/tests/mod.rs` (≤ 1000, so not split) and declares it in
  `gen_openssl.rs` as `#[cfg(test)] #[path = "gen_openssl/tests/mod.rs"] mod error_path_release_tests;`
  (H map). Lines 1-3434 are unchanged by H. T map's own test split of that module is superseded.
- `tls/gen_macos/tests.rs` (23 tests, 884 LOC) is a test-only file ≤ 1000, so no letter splits it.
  plan-176-H splits `src/codegen/builtins/tests/platform_failure.rs` (1088 LOC, over 1000) into parts
  ≤ 900 whose names H chooses. Both import through `gen_macos::`/`super::*` only (`grep -rn "client::\|server::" src/codegen/builtins/tls`
  → only `client.rs:2`), so T's moves need no test edit.
- plan-176-S repoints `vector/builder_simd_float_math.rs:2535`'s comment to `gen_math/`; that
  line moves with `lower_simd_float_binary` into `dispatch.rs` here.

All line ranges are "as of 17a9b5799"; item and test NAMES are authoritative if lines drift.

### Measured populations

| What | Count | Command |
|---|---|---|
| Spec citations of the nine files | 27 (4 gen_ping, 0 gen_unix, 0 gen_windows, 7 regex, 2 client, 2 server, 4 gen_openssl, 2 gen_schannel_server, 6 builder_simd_float_math) | `grep -rnoF "[[src/codegen/builtins/<path>" src/docs/spec \| wc -l` per path |
| — of which repointed | 20 (the 7 regex citations stay: every symbol is outside the moved lines 90-239) | §4.4 |
| Code/test/tool/`.ai` comment mentions of the old paths | 22 lines: 20 repointed (§4.9), `gen_schannel.rs:288` (the `include!` itself, kept) and `no_operator_strings.rs:630` (kept); plus `tls/mod.rs:227-228`, found by reading `tls/mod.rs:226` (bare `client.rs:`/`server.rs:` without a backtick) | `grep -rnE "gen_ping\.rs\|gen_unix\.rs\|gen_windows\.rs\|gen_macos/client\.rs\|gen_macos/server\.rs\|\`client\.rs\|\`server\.rs\|gen_openssl\.rs\|gen_schannel_server\.rs\|builder_simd_float_math\.rs" src tests tools .ai --include='*.rs' --include='*.md' --include='*.py' \| grep -v '^src/docs/spec' \| grep -v 'testing-gates.md:850'` → 22 (`.ai/testing-gates.md:850` names the repository crate's `server.rs`) |
| — of which pre-existing stale line citations | 2 (`tls/mod.rs:227` `client.rs:628-630`, `:228` `server.rs:1650-1652`) | `sed -n 626,631p src/codegen/builtins/tls/gen_macos/client.rs` → an `EmitCtx` literal; `sed -n 1648,1653p …/server.rs` → `let` bindings, neither a record store |
| `emit_cstring(` sites under `src/codegen/builtins/tls` | 12 (asserted by `codegen_helper_scratch_release::every_tls_marshalling_site_is_handed_a_scratch`) | test source, `tests/codegen/codegen_helper_scratch_release.rs:718` walks the directory recursively |
| Non-test `#[cfg(...)]` gates in T files | 0 | `grep -n '#\[cfg(' <nine files>` → only the `#[cfg(test)]` at `gen_ping.rs:1922`, `gen_windows.rs:1520`, `regex/mod.rs:1099`, `gen_openssl.rs:3435`, `builder_simd_float_math.rs:2771` |
| Guard-allowlist / coverage-exception entries for T files | 0 | `grep -rnE "gen_ping\|gen_unix\|gen_windows\|regex/mod\|gen_macos\|gen_openssl\|gen_schannel_server\|builder_simd_float_math" tests/guards scripts/coverage-exceptions.txt` → only a comment at `tests/guards/no_operator_strings.rs:630` |
| Overflow labels in T files | 0 | `grep -c 'abi::label(&.*overflow' <nine files>` → 0 each |

### Verified properties

- **The citation checker is file-level** (`src/docs/spec/mod.rs::spec_citations_resolve`,
  plan-176-A §2). Converting `foo.rs` → `foo/mod.rs` makes every `[[foo.rs:…]]` fail, which forces
  the sweep for seven files. `regex/mod.rs` and `gen_schannel_server.rs` keep their paths: their
  citations are handled by the explicit lists in §4.4 and §4.7.
- **Glob-cycle imports compile without warnings** (`use super::*;` in a part, `use part::*;` in
  `mod.rs`): probe `/tmp/loc/probe2` printed `6` with no warning. A child's `use super::*;` also
  brings the parent's private `use` imports into scope (`rustc --edition 2021 --crate-type lib
  /tmp/loc/work_T/globtest/lib.rs`: an import in the parent that only a child uses raised no
  `unused_imports`). Parent→child references need no visibility edit; child→parent and
  sibling→sibling references need `pub(super)`, each listed below.
- **Moving Rust code cannot change compiler output through source locations** (plan-176-A §2).
- **No host-`cfg` code in T.** The Windows (process, Schannel), Linux (OpenSSL, POSIX ping) and
  macOS emitters are compiled on every host (`#[cfg(` count above), so `cargo check --all-targets`
  on the macOS host compiles all of T's code; no Windows or Linux `cargo check` is needed. Their
  output is gated per target by the `windows-x86_64`, `linux-*` and `macos-aarch64` goldens.
- **The vector BODY checker is unaffected.** `tools/vector-gen/check_vector_bodies.py` (run by
  `scripts/check-generated.sh`) globs `src/codegen/builtins/vector/{func_*,helper_*}.rs`
  non-recursively; `builder_simd_float_math.rs` matches neither, has no raw strings
  (`grep -c 'r#\+"'` → 0), and its new directory is not scanned. No new part is named `func_*`/`helper_*`.
- **The TLS source-walk test survives** whole-function moves: it walks `src/codegen/builtins/tls`
  recursively (`fn walk`) and checks `scratch` within 10 lines after each `emit_cstring(`. The
  extract phase keeps the listen site's scratch argument on its line (§4.10).
- **The scoped artifact gate reaches T's code:** `grep -rl "<pkg>::<member>" tests/byte-identity/*/src`
  finds `tls::listen/accept/connect/poll` (tls), `net::ping` (net, tls), `process::spawn/shell/send`
  (process), `regex::` (regex), `vector::` (vector). `tls::closeListener` has no golden-owning
  caller (same grep over `tests/` → none); it is reached by `tests/cli/cli_tls_listen_accept_build.rs`.
- **Lint levels are inherited by child modules**, so `builder_simd_float_math/mod.rs`'s two inner
  `#![allow(clippy::approx_constant)]`/`#![allow(clippy::excessive_precision)]` cover every part.

## 3. Design Overview

**Conventions** (plan-176-A, `.ai/build-tooling.md` "Splitting an over-budget file"):
`<stem>.rs` → `<stem>/mod.rs` + parts (`git mv <stem>.rs <stem>/mod.rs` first so history follows).
The parent declaration is unchanged. Each part starts with `use super::*;` plus only the `use`
lines the moved text needs that the parent does not import; unused imports are trimmed per file.
`CodeBuilder` methods move inside an `impl CodeBuilder<'_> { … }` block in the part; a private
method called from another part becomes `pub(super)`. Free items outside callers use are
re-exported from `mod.rs` with `pub(crate) use`. ≤ 900 code LOC per part. Pure motion: text is
moved verbatim (rustfmt may re-wrap; no token change).

**Gate class: byte-identity.** Every phase runs the common check sequence (Phases section):
`cargo build --release`, the scoped `sh scripts/artifact-gate.sh target/release/mfb <builtin>` for
the built-ins whose lowering moved (named per phase, ~1 min each), the moved modules' unit tests,
`cargo check --all-targets` with no new warning, `spec_citations_resolve`, the three name-based
guards and the file-size tighten. Letter end, once: `sh scripts/artifact-gate.sh target/release/mfb all`
(15–20 min) — code motion in codegen can change emission for any fixture on any target, and only
the full gate covers all targets. A diff is never a stop: localize it on ONE fixture
(`mfb build -q -ncode -target <t>` with HEAD's compiler vs. the phase's, diff the two `.ncode`),
fix the motion or extraction, re-run.

**Risk concentrates** in Phase 5, the `lower_tls_listen_macos` extraction: it re-plumbs 16
labels, 5 scratches, 2 vregs and 39 frame consts across three new functions. It is safe because
the phases are consecutive slices of the original body called in source order, no new vreg,
label or instruction is introduced, and the gate is the `macos-aarch64` `tls` golden (which calls
`tls::listen`) plus the macOS listener runtime tests. Phases 1–4 are pure motion.

**Interim NEW row.** Phase 4 puts `lower_tls_listen_macos` (1007), whole, into
`server/listen.rs`, which is then ≈1008 code LOC: between Phase 4 and Phase 5 the banner shows
`NEW code 1008 > 1000 src/codegen/builtins/tls/gen_macos/server/listen.rs` (the guard warns and
never fails, plan-176-A §Decisions). No ceiling row is added for it; Phase 5 turns `listen.rs` into
`listen/` and the row disappears.

**`gen_openssl/connect.rs` at 965 code LOC.** It is one function, `lower_tls_connect_openssl`
(964 LOC, `python3 -I /tmp/loc/span.py src/codegen/builtins/tls/gen_openssl.rs 23 1132` → 964),
with no inner items. It is under the 1000 limit, so the rule needs no row and the owner's
extract-method decision (functions over the limit) does not reach it. The 900 part target is
headroom guidance that a single function under the limit cannot meet by motion; the file lands at
965 with no ceiling and no exemption.

**No guard edit.** T moves no overflow label (count 0, §2), no file a name-based guard classifies,
and nothing `no_cross_member_os_seam_dispatcher` keys on (no `_os_seam`, no `func_*` + `body:` +
`_native(` in the moved text). `tests/guards/no_operator_strings.rs:630` mentions
`process/gen_windows.rs` in a historical comment about a removed allowlist entry; it stays.

**Rejected alternatives.**
- *Exempt `server/listen.rs`* (T map) — superseded by the owner's extract-method decision and the
  T-extract design (§4.10).
- *`lower_tls_listen_macos` phases as free fns taking every local* — 30+ parameters
  (`clippy::too_many_arguments`). Two structs (`ListenEnv`, read-only; `ListenScratch`, owned)
  plus `ins`/`rel`/`vregs` give a uniform 5-parameter signature.
- *`ListenEnv::new(…)`* — one more fn for a struct literal used once; the literal stays inline.
- *Identity phase in `server/identity.rs`* — it reads 19 of the listen consts, which would have to
  become `pub(super)` across siblings; it stays in `listen/mod.rs` beside the consts.
- *Listen consts in `server/mod.rs`* — they would leak `HOST`/`PORT`/`DATA`/`QUEUE`/`FRAME_SIZE`
  into `accept.rs`/`close_listener.rs`, whose own fn-local consts would shadow them.
- *`gen_schannel_server.rs` → a module* — it is `include!`d into `gen_schannel.rs:288` and shares
  that module's flat namespace (`stl::`, `win_call`, `st::`); the in-tree precedent is a chain of
  included siblings (`gen_schannel.rs` → `gen_schannel_impl.rs` → `gen_schannel_io.rs` →
  `gen_schannel_read_close.rs`).
- *A plain `mod error_path_release_tests;` in `gen_openssl/mod.rs`* — would look for
  `gen_openssl/error_path_release_tests.rs`, outside the `tests/` directory H created (the coverage
  gate ignores `tests/` directories, plan-176-A §2). The `#[path]` attribute keeps H's file.
- *Re-pointing `bugs/*.md`* — bug documents record the tree of their time.

## 4. Detailed Design

LOC figures: `python3 -I /tmp/loc/span.py <file> <a> <b>` over the stated ranges. "+N" = the
part's own `use`/`impl`/`mod` lines (guess). Re-measure every new file with
`python3 -I tools/loc-census/census.py --file <path>` before committing.

### 4.1 `process/gen_unix.rs` → `process/gen_unix/` (Phase 1)

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `gen_unix/mod.rs` | 1-168, 1075-1432 | ≈416 | header, uses, `SIGKILL`, `WNOHANG`, `LIST_*`/`ENTRY_*`, `emit_decode_status`, `lower_process_drop_helper`; `lower_process_send_helper`, `POLLOUT`, `emit_poll_wait`, `lower_process_reaper_helper`; `mod spawn; pub(crate) use spawn::*;` |
| `gen_unix/spawn.rs` | 169-1074 | ≈721 | spawn frame consts `STDIN_P`…`SPAWN_LOCAL`, `emit_copy_to_cstring`, `emit_child_apply_env`, `emit_spawn_tail`, private `emit_spawn_attr_destroy`, `emit_cstring_literal` |

Surface/visibility: 0 edits. The glob re-export keeps every `pub(crate)` spawn item reachable as
`gen_unix::X` for the glob importers `func_shell.rs`, `func_spawn.rs`, `func_is_running.rs`,
`func_wait_for.rs` (`use super::gen_unix::*`). `process/mod.rs:44` and `gen_shared.rs:85` are
unchanged. Citations 0; tests 0.

### 4.2 `process/gen_windows.rs` → `process/gen_windows/` (Phase 1)

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `gen_windows/mod.rs` | 1-103, 944-1642 | ≈510 code + 101 test | header, uses, `WIN_SPAWN_*` frame consts through `WIN_SPAWN_SCRATCH`; `SI_*`, `HANDLE_FLAG_INHERIT`, `STARTF_USESTDHANDLES`, `STARTUPINFOEXA_SIZE`, `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, `EXTENDED_STARTUPINFO_PRESENT`, `emit_win_spawn_tail`, `emit_write_all`, `lower_process_send_helper`, `lower_process_drop_helper`; the inline `#[cfg(test)] mod tests` (1520-1642, reads only consts that stay here); `mod command_line; mod environment; pub(crate) use command_line::*; pub(crate) use environment::*;` |
| `gen_windows/command_line.rs` | 104-421 | ≈249 | `WIN_CMD_LIST`, private `WIN_CMD_*`, `WIN_CMDLINE_SCRATCH_END`, `emit_win_build_cmdline` |
| `gen_windows/environment.rs` | 422-943 | ≈438 | `WIN_ENV_*` consts, `WIN_ENV_SCRATCH_END`, `emit_win_build_cwd`, `emit_win_build_env_block` (reads `WIN_CMDLINE_SCRATCH_END` through the parent's glob re-export) |

Surface/visibility: 0 edits. The private `WIN_CMD_*`/`WIN_ENV_*` consts are used only in their own
file. `func_spawn.rs`/`func_shell.rs` (`use super::gen_windows::*`) and `func_send*.rs`/`gen_shared.rs:83`
are unchanged. Citations 0.

### 4.3 `regex/mod.rs` gains `regex/package_doc.rs` (Phase 1)

| File | Source lines | LOC | Contents |
|---|---|---|---|
| `regex/mod.rs` | all except 90-239 | ≈856 code + 169 test | unchanged otherwise, plus `mod package_doc; use package_doc::{DESC, INTRO};`. `register` (303-1097, 744 LOC) stays whole; the inline tests (1099-1294) stay and do not reference `INTRO`/`DESC` |
| `regex/package_doc.rs` | 90-239 | ≈149 | `INTRO`, `DESC` (raw-string package prose), each `pub(super)` |

Surface: 2 `pub(super)` edits. Citations: all 7 stay (`stdlib/01_regex.md`: `__regex_Node` ×2,
`register`, `is_tier_a_query`, `MatchInfo`, `__regex_ClassItem`; `architecture/09_modules.md:38`
whole-file) — each symbol is outside 90-239 (`grep -n`: `__regex_Node` 511…, `__regex_ClassItem`
401…, `MatchInfo` 946/971, `is_tier_a_query` 290, `register` 303). The `include_str!("../../string/unicode/unicode_script_names.mfb")`
at 1084 is inside `register`, so its relative path is unchanged. No checker reads `INTRO`/`DESC`
(`scripts/man-census.sh`/`man-manual.sh` only list package directory names).

### 4.4 `net/gen_ping.rs` → `net/gen_ping/` (Phase 2)

The POSIX backend becomes a sub-directory module because its socket helpers read the POSIX
frame consts.

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `gen_ping/mod.rs` | 1-162, 167-192, 1921-2054 | ≈112 code + 102 test | `//!` header, uses, `IPPROTO_ICMP`, `ICMP_*`, `STATUS_*`, `PING_MAX_PAYLOAD`, `RECV_CAPACITY`, `ADDRESS_OFFSET_PORT`, `PingResultSlots`, `emit_ping_result`; `lower_net_ping_helper`; the inline `#[cfg(test)] mod tests` (uses only `STATUS_*`, `ADDRESS_OFFSET_PORT`, `PING_MAX_PAYLOAD`); `mod posix; mod windows; use posix::lower_ping_posix; use windows::lower_ping_windows;` |
| `gen_ping/posix/mod.rs` | 163-164, 193-1137 | ≈799 | `NANOS_PER_MILLI` + doc; POSIX frame consts `FRAME_SIZE`…`CMSG_CAPACITY`, `MSGHDR_*`, `CMSGHDR_*`, `lower_ping_posix`; `mod socket; use socket::{SockOptSource, PING_RECV_BUFFER, emit_int_sockopt, emit_monotonic_nanos, emit_icmp_checksum};` |
| `gen_ping/posix/socket.rs` | 165, 1138-1321 | ≈144 | `NANOS_PER_SECOND`; `SockOptSource`, `PING_RECV_BUFFER`, `emit_int_sockopt`, `emit_monotonic_nanos`, `emit_icmp_checksum` (read `FD_OFFSET`/`OPTVAL_OFFSET`/`SIZE_OFFSET`/`TS_OFFSET`/`TTL_OFFSET` from `posix/mod.rs`) |
| `gen_ping/windows.rs` | 1322-1920 | ≈522 | `IP_*` status consts, `REPLY_*`, `OPTINFO_TTL`, `WIN_UNBOUNDED_ATTEMPT_MS`, `WIN_FRAME_SIZE`, `W_*`, `lower_ping_windows` |

Surface/visibility: `func_ping.rs` calls `super::gen_ping::lower_net_ping_helper`, which stays.
7 `pub(super)` edits: `lower_ping_posix`, `lower_ping_windows`, `SockOptSource`, `PING_RECV_BUFFER`,
`emit_int_sockopt`, `emit_monotonic_nanos`, `emit_icmp_checksum`. `mod.rs`'s `//!` doc link
[`lower_ping_windows`] still resolves through the `use`. Citations 4 (`stdlib/16_icmp.md`):
`lower_ping_posix` → `gen_ping/posix/mod.rs`; `PING_RECV_BUFFER` → `gen_ping/posix/socket.rs`;
`lower_net_ping_helper` → `gen_ping/mod.rs`; `lower_ping_windows` → `gen_ping/windows.rs`.

### 4.5 `vector/builder_simd_float_math.rs` → `vector/builder_simd_float_math/` (Phase 2)

The single `impl CodeBuilder<'_>` (439-2750) splits into one `impl CodeBuilder<'_>` block per part.
Method ranges include their doc comments.

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `…/mod.rs` | 1-438, 2751-2850 | ≈281 code + 47 test | the two `#![allow(clippy::…)]` inner attributes (first lines, unchanged), uses, all f64 consts, `TWO_OVER_PI_WORDS`, `ATAN_*`, the math const pool (`MATH_CONST_POOL_SYMBOL`, `math_const_pool_words`, `two_over_pi_table_offset`, `math_const_pool_offset`, `math_const_pool_data_value`), `FloatKernel`, `FloatError`, `KernelRegs`, `TrigRegs` and their impls; `FloatBinaryKernel` + impl (2751-2770); inline `#[cfg(test)] mod pool_layout_tests` (2771-2850, unchanged); `mod dispatch; mod exp_log; mod inverse_trig; mod primitives; mod trig; mod trig_reduction;` |
| `…/dispatch.rs` | 441-743, 2533-2749 | ≈423 | `float_kernel_regs`, `lower_simd_float_unary`, `emit_float_error_reduce`, `emit_result_nan_into_mask`, `lower_simd_float_scalar`, `emit_float_kernel_setup`, `emit_float_kernel_body`; `lower_simd_float_binary`, `lower_simd_float_binary_scalar`, `emit_float_binary_setup`, `emit_float_binary_body` |
| `…/inverse_trig.rs` | 745-814, 824-1203 | ≈394 | `emit_asin_acos_body`; `emit_atan_core`, `emit_atan_poly_recombine`, `emit_atan_core_scalar` |
| `…/trig_reduction.rs` | 1205-1562, 1564-1570 | ≈280 | `emit_sincos_reduce`, `emit_rem_pio2_large_lanes`, `emit_rem_pio2_large_lane`, `emit_shl_pair`, `emit_join_piece`, `emit_scaled_piece`; `emit_load_pool_double` (used only here) |
| `…/trig.rs` | 1572-2019 | ≈365 | `emit_zero_argument_passthrough` (with the `/// sin/cos kernel…` doc at 1572 above it), `emit_save_trig_argument`, `emit_sin_cos_body`, `emit_sin_cos_body_scalar`, `emit_cos_r_into`, `emit_sin_r_into`, `emit_tan_sincos_dd`, `emit_tan_body`, `emit_tan_divide`, `emit_tan_body_scalar` |
| `…/exp_log.rs` | 2021-2349 | ≈285 | `emit_exp_body`, `emit_log_body` |
| `…/primitives.rs` | 815-822, 2351-2531 | ≈143 | `emit_vsel`; `emit_twoprod`, `emit_twosum`, `emit_compensated_horner`, `emit_horner`, `math_pool_base_reg`, `emit_load_math_pool_base`, `broadcast_f64`, `broadcast_i64` |

Surface/visibility: the external paths listed in §1 stay in `mod.rs`. The `pub(crate)` methods
`lower_simd_float_{unary,scalar,binary,binary_scalar}` keep their visibility. **20 methods →
`pub(super)`** (method-call cross-reference by part, `python3 /tmp/loc/work_T/xref2.py`):
`broadcast_f64`, `broadcast_i64`, `emit_vsel`, `emit_twoprod`, `emit_twosum`,
`emit_compensated_horner`, `emit_horner`, `math_pool_base_reg`, `emit_load_math_pool_base`,
`emit_asin_acos_body`, `emit_atan_core`, `emit_atan_core_scalar`, `emit_exp_body`, `emit_log_body`,
`emit_result_nan_into_mask`, `emit_sin_cos_body`, `emit_sin_cos_body_scalar`, `emit_sincos_reduce`,
`emit_tan_body`, `emit_tan_body_scalar`. No method is called from `mod.rs`'s own code
(`grep -n "Self::[a-z_]*("` → 0).

Citations 6: `[[…/builder_simd_float_math.rs]]` whole-file (`architecture/18_math-kernels.md:14`)
→ `…/builder_simd_float_math/mod.rs`; `emit_sincos_reduce` (18_math-kernels.md:171) →
`…/trig_reduction.rs`; `emit_rem_pio2_large_lane` (18_math-kernels.md:184 and
`language/18_builtin-functions.md:143`) → `…/trig_reduction.rs`; `TWO_OVER_PI_WORDS` (:189) and
`math_const_pool_words` (:206) → `…/mod.rs`.

### 4.6 `tls/gen_openssl.rs` → `tls/gen_openssl/` (Phase 3)

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `gen_openssl/mod.rs` | 1-22 + H's test declaration | ≈30 | `//!` header and all uses; `mod accept; mod close; mod connect; mod io; mod listen;`; `#[cfg(test)] #[path = "tests/mod.rs"] mod error_path_release_tests;` (H's `#[path = "gen_openssl/tests/mod.rs"]`, re-based: a `#[path]` in a `mod.rs` is relative to that file's directory); `pub(crate) use` of all 8 lowerers |
| `gen_openssl/connect.rs` | 23-1132 | ≈965 | `lower_tls_connect_openssl` (964, §3) |
| `gen_openssl/listen.rs` | 1133-1705 | ≈503 | `lower_tls_listen_openssl` |
| `gen_openssl/accept.rs` | 1706-2310 | ≈542 | private `emit_monotonic_ms` (used only at 1946/2041), `lower_tls_accept_openssl` |
| `gen_openssl/io.rs` | 2311-3121 | ≈687 | `lower_tls_read_openssl`, `lower_tls_poll_openssl`, `lower_tls_write_openssl` |
| `gen_openssl/close.rs` | 3122-3434 | ≈271 | `lower_tls_close_openssl`, `lower_tls_close_listener_openssl` |

Surface/visibility: 0 `pub(super)` edits. All 8 lowerers have outside users (`gen_shared.rs`
`openssl::lower_tls_{connect,listen,accept,read,write,poll,close,close_listener}_openssl`,
`resource/tests/mod.rs:170,175`), served by the re-exports. The 9 tests in
`gen_openssl/tests/mod.rs` (`use super::*;`, `super` = `gen_openssl`) reach the re-exported lowerers and
`tls::gen_shared` helpers through the `mod.rs` glob chain; they call no private item of a part
(`emit_monotonic_ms` stays private in `accept.rs`). `tls/mod.rs:33 pub(crate) mod gen_openssl;` and
`gen_shared.rs:556 use super::gen_openssl as openssl;` are unchanged. Citations 4
(`stdlib/17_transports.md`): `lower_tls_read_openssl`, `lower_tls_write_openssl` →
`gen_openssl/io.rs`; `lower_tls_accept_openssl`, `emit_monotonic_ms` → `gen_openssl/accept.rs`.

### 4.7 `tls/gen_schannel_server.rs` splits into a second include (Phase 3)

`gen_schannel_server.rs` is not a module: `tls/gen_schannel.rs:288 include!("gen_schannel_server.rs");`.

| File | Source lines | LOC | Contents |
|---|---|---|---|
| `tls/gen_schannel_server.rs` | 1-7, 13-16, 19-686 | ≈541 | header, `SECPKG_CRED_INBOUND`, `CRYPT_STRING_BASE64HEADER`, `X509_PKCS7_ENCODING`, `mod stl`, `LISTENER_BLOCK_SIZE`, `win_call`, `emit_read_file`, `emit_pem_to_der`, `emit_container_name`, `lower_tls_listen`. Its header comment, which says it covers listen/accept/closeListener, now says it covers listen and that accept/closeListener are in `gen_schannel_accept.rs` |
| `tls/gen_schannel_accept.rs` (new) | 8-11, 17-18, 687-1243 | ≈464 | `ASC_REQ_FLAGS` + comment (used only at 949), `POLLRDNORM` + comment (used only at 803), `emit_hs_refresh`, `lower_tls_accept`, `lower_tls_close_listener`. First line: a `// Included into gen_schannel.rs …` line comment. **No `//!` and no inner attributes** (illegal in an `include!`d file) |
| `tls/gen_schannel.rs` | — | +1 | `include!("gen_schannel_accept.rs");` directly after line 288 (the path is relative to `gen_schannel.rs`'s directory) |

Surface/visibility: 0 edits; everything shares the `gen_schannel` module scope. Citations 2
(`stdlib/17_transports.md:241-242`): `lower_tls_accept`, `emit_hs_refresh` → `tls/gen_schannel_accept.rs`.

### 4.8 `tls/gen_macos/client.rs` and `server.rs` → directories (Phase 4)

`client.rs` → `gen_macos/client/`:

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `client/mod.rs` | 1-6 | ≈15 | the original header incl. `use super::server::emit_cancel_drain;` (`super` is still `gen_macos`); `mod close; mod connect; mod poll; mod read; mod write;`; `pub(crate) use` of the 5 lowerers |
| `client/connect.rs` | 7-967 | ≈837 | `lower_tls_connect_macos` (836) |
| `client/read.rs` | 968-1419 | ≈385 | `lower_tls_read_macos` |
| `client/write.rs` | 1420-1779 | ≈294 | `lower_tls_write_macos` |
| `client/poll.rs` | 1780-2143 | ≈327 | `lower_tls_poll_macos` |
| `client/close.rs` | 2144-2392 | ≈182 | `lower_tls_close_macos` (the only `emit_cancel_drain` user, through the glob) |

`server.rs` → `gen_macos/server/`:

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `server/mod.rs` | 1-52 | ≈36 | header uses, section banner, `emit_cancel_drain` (already `pub(super)`, still visible to `gen_macos` for client); `mod accept; mod close_listener; mod identity; mod listen;`; `pub(crate) use` of the 3 lowerers |
| `server/identity.rs` | 53-421 | ≈299 | `emit_read_whole_file`, `emit_import_pem_item`, `emit_cf_release_slot` — each `pub(super)` (used by listen) |
| `server/listen.rs` | 422-1622 | ≈1008 | `lower_tls_listen_macos` + `use super::identity::{emit_read_whole_file, emit_import_pem_item, emit_cf_release_slot};` — interim (§3); Phase 5 turns it into `listen/` |
| `server/accept.rs` | 1623-2173 | ≈482 | private `emit_accept_deadline`, `lower_tls_accept_macos` |
| `server/close_listener.rs` | 2174-2448 | ≈228 | `lower_tls_close_listener_macos` |

Surface/visibility: client 0 edits; server 3 `pub(super)` (identity). `gen_macos/mod.rs:947`
`pub(crate) use client::{…}` and `:951` `pub(crate) use server::{…}` are unchanged.
Citations 4 (`stdlib/17_transports.md`): `lower_tls_read_macos` → `gen_macos/client/read.rs`;
`lower_tls_write_macos` → `gen_macos/client/write.rs`; `emit_accept_deadline`,
`lower_tls_accept_macos` → `gen_macos/server/accept.rs` (:226-227).

### 4.9 `.ai`, code-comment, tool and stale-reference sweep

Each item is done in the phase that moves its target (phase in brackets).

| Site | Old | New | Phase |
|---|---|---|---|
| `src/codegen/builtins/process/gen_windows.rs:26` (→ `gen_windows/mod.rs`) | "twin of `gen_unix.rs`'s `emit_spawn_tail`" | "… `gen_unix/spawn.rs`'s …" | 1 |
| `src/codegen/builtins/process/gen_unix.rs:619` (→ `gen_unix/spawn.rs`) | "(see `gen_windows.rs`)" | "(see `gen_windows/mod.rs`, `emit_win_spawn_tail`)" | 1 |
| `src/codegen/builtins/process/func_spawn.rs:619` | "`WIN_SPAWN_SI` in `gen_windows.rs`" | "… in `gen_windows/mod.rs`" | 1 |
| `.ai/arch-abi.md:834` | `` `gen_windows.rs` passes `bInheritHandles = TRUE` `` | `` `process/gen_windows/mod.rs` (`emit_win_spawn_tail`) passes … `` | 1 |
| `src/codegen/builtins/net/mod.rs:410` | "`ping` in `gen_ping.rs`" | "… in `gen_ping/mod.rs`" | 2 |
| `tools/net-probes/README.md:11` | `src/codegen/builtins/net/gen_ping.rs` | `src/codegen/builtins/net/gen_ping/` | 2 |
| `.ai/build-tooling.md:46`, `:50` | `src/codegen/builtins/vector/builder_simd_float_math.rs`; "to both files" | `…/builder_simd_float_math/mod.rs`; add: "The allows on `builder_simd_float_math/mod.rs` cover its part files, because child modules inherit lint levels." | 2 |
| `src/codegen/builtins/math/gen_pow.rs:8`, `:41` | `builder_simd_float_math.rs` | `builder_simd_float_math/mod.rs` (:8, the allow block) and `builder_simd_float_math/` (:41, the kernels) | 2 |
| `tools/math-kernels/gen_coeffs.py:444` | `builder_simd_float_math.rs::emit_atan_core` | `builder_simd_float_math/inverse_trig.rs::emit_atan_core` (comment only; the tool reads no source) | 2 |
| the `platform_failure` test module's header (plan-176-H splits `tests/platform_failure.rs` into parts; locate with `grep -rn "tls/gen_openssl.rs" src`) | "`tls/gen_openssl.rs` has 80 by itself" | "`tls/gen_openssl/` has 80 by itself" | 3 |
| `tests/codegen/codegen_tls_c_return_x86_64.rs:1` | `src/codegen/builtins/tls/gen_openssl.rs` | `src/codegen/builtins/tls/gen_openssl/` | 3 |
| `tests/net/rt_tls_connect_allow_self_signed.rs:65` | "(`gen_openssl.rs`'s note on `X509_VERIFY_PARAM_set1_ip`)" | "(`gen_openssl/connect.rs`'s note …)" | 3 |
| `tests/codegen/codegen_helper_scratch_release.rs:452` | "OpenSSL (`gen_openssl.rs`)" | "OpenSSL (`gen_openssl/`)" | 3 |
| `src/codegen/builtins/tls/gen_openssl.rs:2552` (→ `gen_openssl/io.rs`) | "Mirrors the guard in `gen_macos/client.rs`." | "… in `gen_macos/client/read.rs`." (the guard is at `client.rs:1331`, inside `lower_tls_read_macos`) | 4 |
| `src/target/macos_aarch64/tls.rs:143` | `gen_macos/client.rs:lower_tls_write_macos` | `gen_macos/client/write.rs:lower_tls_write_macos` | 4 |
| `src/codegen/builtins/tls/gen_macos/server.rs:28` (→ `server/mod.rs`) | "the connect-path drain (`client.rs`, bug-380)" | "… (`client/connect.rs`, bug-380)" (the drain is at `client.rs:874`, inside `lower_tls_connect_macos`) | 4 |
| `src/codegen/builtins/tls/gen_macos/tests.rs:565` (not moved by any letter) | "(bug-380, `client.rs`)" | "(bug-380, `client/connect.rs`)" | 4 |
| `src/codegen/builtins/tls/mod.rs:226-228` — **pre-existing stale** line citations (§2) | "`tls::listen` (gen_macos/server.rs) … `tls::connect` (client.rs:628-630) nor `tls::accept` (server.rs:1650-1652)" | "`tls::listen` (`gen_macos/server/listen/start.rs`, `emit_listen_start`) … `tls::connect` (`gen_macos/client/connect.rs`, `lower_tls_connect_macos`) nor `tls::accept` (`gen_macos/server/accept.rs`, `lower_tls_accept_macos`)" — written in Phase 4 with `server/listen.rs` for listen, and Phase 5 sets the `listen/start.rs` path | 4, 5 |

`tests/guards/no_operator_strings.rs:630` stays (a historical note about a removed entry). The
`repository/src/{client,server,store,abi}.rs` and `.ai/testing-gates.md:850` hits name other files.
Each phase ends with a sweep command proving no live mention remains:
`grep -rnE "<old basenames of the phase>" src tests tools .ai --include='*.rs' --include='*.md' --include='*.py' | grep -vE '^src/docs/spec|no_operator_strings.rs:630|^repository/'`
→ no hit naming an old path of the phase.

### 4.10 Extract-method: `lower_tls_listen_macos` (Phase 5)

Function: `server.rs:422-1622` (Phase 4 moves it verbatim to `server/listen.rs`),
`pub(crate) fn lower_tls_listen_macos(symbol, platform_imports, platform) -> Result<TlsBodyParts, String>`,
1007 code LOC (`python3 -I /tmp/loc/span.py src/codegen/builtins/tls/gen_macos/server.rs 422 1622` → 1007).
`server/listen.rs` becomes the directory module `server/listen/` (`mod.rs`, `endpoint.rs`,
`start.rs`). `server/mod.rs` keeps `mod listen;` and `pub(crate) use listen::lower_tls_listen_macos`,
so callers and the module path are unchanged. Line numbers below are the original `server.rs` lines.

**Structure:**

| id | lines | LOC | content |
|---|---|---|---|
| R0 | 422-426 | 5 | signature |
| R1 | 427-429 | 3 | `vregs`, `v9`, `v10` (the first two `vregs.next()` mints) |
| R2 | 430-499 | 45 | 39 frame-offset `const`s + 3 `const _: () = assert!` |
| R3 | 501-516 | 16 | 16 exit labels, each `format!("{symbol}_…")` (pure strings; no label counter) |
| R4 | 518-557 | 7 | `ins`, `rel`; 5 × `HelperScratch::declare(&mut vregs, &mut ins)` (each mints 2 vregs and emits 2 `move_immediate`) |
| R5 | 558-582 | 10 | parks x0..x3; NULLs DATA/ITEMS/CERTREF/KEYREF/RESOLVEDCSTR |
| R6 | 583-773 | 180 | identity: read the cert/key PEMs, dlopen ×3, import PEM ×2, `SecIdentityCreate`, release the refs, `sec_identity_create` |
| R7 | 774-936 | 151 | the configure-TLS block literal, `nw_parameters_create_secure_tcp`, `set_reuse_local_address` |
| R8 | 937-1083 | 114 | host: null-host branch, `emit_cstring`, `inet_pton`/`getaddrinfo`/`inet_ntop` resolve, the arena copy of ANYHOST, `emit_port_itoa` |
| R9 | 1084-1227 | 126 | endpoint, free the resolved buffer, `set_local_endpoint`, `nw_listener_create`, release ×2, `dispatch_queue_create` |
| R10 | 1228-1473 | 235 | the listener ctx calloc and init, the semaphore/retain/edomain fns, `set_queue`, the 2 handler blocks, `start`, the wait loop |
| R11 | 1474-1510 | 28 | builds the Listener record; `branch(&done)` |
| R12 | 1511-1601 | 68 | failure exits: `listen_fail`, `net_fail`, `read_fail_fd`, `cert_fail` (CF releases), `inet_ntop_fail`, `addr_invalid_fail`, `load_fail`, `alloc_fail` |
| R13 | 1602-1622 | 19 | `done`: `emit_helper_scratch_release`, `ret`, `Ok((ins, rel, FRAME_SIZE))` |

Sum 1007.

**Control-flow audit** (`sed -n 422,1622p … | grep -n "return\|break\|continue\||[a-z_]*|"`,
`grep -c ")?;"` → 42). No `return` keyword (every hit is `abi::return_register()`/`abi::mfb_return`/
`abi::return_()`), no `break`, no `continue`, no labelled block, no immediately-invoked closure. The
only closure, 581 `.map(|slot| abi::store_u64(abi::ZERO, abi::stack_pointer(), slot))`, captures
nothing and stays in R5. The 42 `?` sites are all `Result<(), String>` → `?` (R6: 13, one inside
`for slot in [CERTREF, KEYREF]` at 736-751; R7–R9: 15; R10: 11; R12: 3 + 1 inside the
`for slot in [CERTREF, KEYREF, ITEMS, DATA]` loop). A `?` inside a `for` exits the fn; after
extraction it exits the phase fn with the same `Err`, and the caller's `?` propagates it unchanged.
Control transfers between regions are emitted branches to labels, not Rust control flow, so
R6, R7–R9 and R10–R11 become plain `phase(..)?;` calls.

**Shared state and its carriers.**

a) **Consts (R2) → module level of `listen/mod.rs`.** Lines 430-499 move verbatim, with their
comments and the 3 `const _` asserts, as private module consts; the children see them through
`use super::*`. Inside the fn the consts shadowed same-named glob imports; at module level an
explicit item shadows a glob import the same way, and `listen` holds only this lowering, so name
resolution in the moved bodies is unchanged. `const _: () = assert!(ANYHOST.len() + 1 == 8);`
(1052, inside R8) stays in place as a fn-body item.

b) **`struct ListenEnv<'a>`** (read-only, passed `&ListenEnv`):
```rust
/// `lower_tls_listen_macos`'s fixed inputs shared by its phases: the helper's symbol and
/// platform, the two body-wide scratch vregs, and every exit label.
struct ListenEnv<'a> {
    symbol: &'a str,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn CodegenPlatform,
    v9: VirtualRegister,
    v10: VirtualRegister,
    cert_fail: String, read_fail_fd: String, net_fail: String, listen_fail: String,
    load_fail: String, alloc_fail: String, null_host: String, host_ready: String,
    addr_invalid_fail: String, inet_ntop_fail: String, inet_ntop_fail_buf_freed: String,
    resolved_buf_freed: String, host_is_numeric: String, wait_loop: String, ready: String,
    done: String,
}
```
Built with a struct literal in the remainder right after R1: `symbol, platform_imports, platform,
v9, v10`, then R3's `format!`s verbatim as field initialisers (`format!` touches no counter, so
order is free). `v9`/`v10` stay minted at 428-429, before R4's declares, so the vreg mint order is
unchanged. `listen/mod.rs` adds `use crate::codegen::engine::operand::VirtualRegister;` (the path
`collection/map/map_bucket_remove.rs:34` uses; no `tls`/`gen_macos` glob re-exports it:
`grep -rn "pub(crate) use .*VirtualRegister" src` → none).

c) **`struct ListenScratch`** (owned by the remainder, passed `&ListenScratch`, destructured by
value in R13):
```rust
/// bug-575/bug-984: the listen body's five scratch blocks (see the declaration comment).
struct ListenScratch {
    cert_scratch: HelperScratch, key_scratch: HelperScratch,
    host_scratch_until_the_listener_owns_it: HelperScratch,
    cert_buf_scratch: HelperScratch, key_buf_scratch: HelperScratch,
}
```
The 5 `let x = HelperScratch::declare(&mut vregs, &mut ins);` lines (553-557) stay verbatim and in
order (they mint vregs and emit), followed by `let scratch = ListenScratch { cert_scratch,
key_scratch, host_scratch_until_the_listener_owns_it, cert_buf_scratch, key_buf_scratch };`. In
R13, `let ListenScratch { cert_scratch, key_scratch, host_scratch_until_the_listener_owns_it,
cert_buf_scratch, key_buf_scratch } = scratch;` goes right before `emit_helper_scratch_release`,
whose array stays verbatim (cert, key, cert_buf, key_buf, host). `#[must_use]` on `HelperScratch`
is satisfied: every value still reaches the release.

d) **`&mut` params:** `ins: &mut Vec<CodeInstruction>`, `rel: &mut Vec<CodeRelocation>`,
`vregs: &mut Vregs`; they stay owned locals of the remainder, which moves `ins`/`rel` into the
return tuple.

**Phase signature** (5 params; no `too_many_arguments`):
```rust
fn emit_listen_X(env: &ListenEnv, scratch: &ListenScratch, ins: &mut Vec<CodeInstruction>,
                 rel: &mut Vec<CodeRelocation>, vregs: &mut Vregs) -> Result<(), String>
```
Prologue: `let ListenEnv { symbol, platform_imports, platform, v9, .. } = *env;` (identity), with
`v10` also bound in endpoint and start (R6 uses no `v10`). This copies only `Copy` fields out of a
borrowed struct, which is legal with `..`, and keeps every `symbol`/`platform`/`platform_imports`/
`v9`/`v10` token in the moved bodies verbatim, including the 36 `&mut EmitCtx { symbol,
platform_imports, platform, … }` shorthand literals. Label `String`s are never bound there (that
would be a move out of a borrow); they are read as `env.x`.

**Mechanical body edits** inside the moved ranges (code tokens only, not comments):
1. `&mut ins` → `&mut *ins`, `&mut rel` → `&mut *rel`, `&mut vregs` → `&mut *vregs` (the
   `instructions:`/`relocations:` fields and the trailing args of `emit_cstring`/`emit_alloc`/
   `emit_data_address`/`emit_arena_free`/`emit_port_itoa`/`emit_external_call`). `ins.extend(..)`/
   `ins.push(..)` are unchanged (auto-deref). Never write `instructions: ins` relying on an
   implicit reborrow; `&mut ins` on a `&mut Vec` param would be `&mut &mut Vec`, a type error.
2. Each label ident → `env.<label>` (`&cert_fail` → `&env.cert_fail`, …), by word-boundary
   match: `ready` must not touch `host_ready`, and `inet_ntop_fail` must not touch
   `inet_ntop_fail_buf_freed`.
3. Scratch idents → `scratch.<name>` (`&cert_scratch` → `&scratch.cert_scratch`;
   `host_scratch_until_the_listener_owns_it.size` → `scratch.host_scratch_until_the_listener_owns_it.size`).

R12 stays in the remainder, where `ins`/`rel`/`vregs`/`symbol`/`platform*`/`v9` are plain locals;
only edit 2 applies there, and it references no scratch.

Per-region usage (`sed -n a,bp | grep -oE "\b(<labels>|<scratches>|v9|v10|vregs)\b" | sort | uniq -c`):
R6 — `alloc_fail cert_fail load_fail read_fail_fd`; `cert_scratch key_scratch cert_buf_scratch
key_buf_scratch`; `v9 vregs`. R7–R9 — `addr_invalid_fail alloc_fail host_is_numeric host_ready
inet_ntop_fail load_fail net_fail null_host resolved_buf_freed`; `host_scratch…` ×7; `v9 v10 vregs`.
R10–R11 — `alloc_fail done listen_fail load_fail ready wait_loop`; `host_scratch…` ×1 (`.pointer`);
`v9 v10 vregs`.

**Remainder and emission order:**
```rust
pub(crate) fn lower_tls_listen_macos(symbol: &str, platform_imports: &HashMap<String, String>,
        platform: &dyn CodegenPlatform) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();                 // R1 verbatim
    let v9 = vregs.next();
    let v10 = vregs.next();
    let env = ListenEnv { symbol, platform_imports, platform, v9, v10,
                          cert_fail: format!("{symbol}_cert_fail"), /* … R3, verbatim strings */ };
    let mut ins: Vec<CodeInstruction> = Vec::new();   // R4 verbatim (keep the bug-575/984 comment)
    let mut rel = Vec::new();
    let cert_scratch = HelperScratch::declare(&mut vregs, &mut ins);  // ×5, same order
    …
    let scratch = ListenScratch { … };
    /* R5 verbatim */
    emit_listen_identity(&env, &scratch, &mut ins, &mut rel, &mut vregs)?;  // R6
    emit_listen_endpoint(&env, &scratch, &mut ins, &mut rel, &mut vregs)?;  // R7+R8+R9
    emit_listen_start(&env, &scratch, &mut ins, &mut rel, &mut vregs)?;     // R10+R11
    /* R12 verbatim, labels → env.X */
    ins.push(abi::label(&env.done));
    let ListenScratch { … } = scratch;
    emit_helper_scratch_release(symbol, &[cert_scratch, key_scratch, cert_buf_scratch,
        key_buf_scratch, host_scratch_until_the_listener_owns_it], &mut vregs, &mut ins, &mut rel);
    ins.push(abi::return_());
    { Ok((ins, rel, FRAME_SIZE)) }
}
```
Every `vregs.next()` (direct, through `HelperScratch::declare`, and inside helpers such as
`emit_cf_release_slot`) and every instruction push happens in the original sequence, because the
phases are consecutive slices of the original body. `Vregs::next` mints from the single
per-compilation id source (`engine/util/vreg_frame.rs` → `regalloc::mint_vreg`), so that order is
the byte-identity invariant, and it is preserved. `emit_cf_release_slot` names its skip label from
`ctx.instructions.len()` (`server.rs:394`), which is identical at each call because the stream is.
`rel`'s element type is inferred from the `&mut Vec<CodeRelocation>` params; no clone is introduced.

**New files:**

| file | items | LOC |
|---|---|---|
| `server/listen/mod.rs` | `use super::*; use super::identity::{emit_read_whole_file, emit_import_pem_item, emit_cf_release_slot}; use crate::codegen::engine::operand::VirtualRegister; mod endpoint; mod start; use endpoint::emit_listen_endpoint; use start::emit_listen_start;`; the R2 consts; `ListenEnv`; `ListenScratch`; `pub(crate) fn lower_tls_listen_macos`; `fn emit_listen_identity` (R6) | ≈440 (guess: header 8, consts 45, structs ≈32, remainder ≈165, identity 187) |
| `server/listen/endpoint.rs` | `use super::*;` + `pub(super) fn emit_listen_endpoint` (R7+R8+R9) | ≈400 (guess: 391 + 8) |
| `server/listen/start.rs` | `use super::*;` + `pub(super) fn emit_listen_start` (R10+R11) | ≈272 (guess: 263 + 8) |

Remainder ≈165 LOC (guess: 3 + env literal ≈22 + 7 + 3 + 10 + 3 calls + R12 68 + R13 ≈22). Largest
new fn ≈400 (`emit_listen_endpoint`); 3 new fns, 2 structs; largest file ≈440.

Visibility: `ListenEnv`/`ListenScratch` and their fields are private to `listen` (children see
them). `emit_listen_endpoint`/`emit_listen_start` are `pub(super)`, exactly as visible as the
private struct types in their signature, so `private_interfaces` does not fire. The three
identity helpers stay `pub(super)` (Phase 4); `emit_cf_release_slot` is called by
`emit_listen_identity` (orig. 736) and by R12 (orig. 1552), both in `listen/mod.rs`, which imports
it. `emit_arena_free` (endpoint orig. 1122, R12 orig. 1588) arrives via `use super::*` from
`server/mod.rs`'s header import. Spec citations of `lower_tls_listen_macos`: none
(`grep -rn "lower_tls_listen_macos" src/docs/spec` → none).

**Hazards.** The TLS source-walk test: the listen `emit_cstring(` site (orig. 944) moves to
`endpoint.rs`; its scratch argument becomes `&scratch.host_scratch_until_the_listener_owns_it`,
still containing `scratch` on the same relative line — do not collapse the call onto one line
with a variable lacking "scratch". Borrowck: the `&mut EmitCtx { …, instructions: &mut *ins,
relocations: &mut *rel }` passed alongside `&mut *vregs` and `&env.load_fail` borrows distinct
params; in the remainder `&env` (shared) and `&mut ins`/`&mut rel`/`&mut vregs` are distinct
locals; `scratch` is borrowed `&` by the phases and moved only after the last call (NLL).
`lower_tls_listen_macos` is emitted for macOS targets only, so the gate is the `tls` fixture's
`macos-aarch64` golden plus the macOS listener runtime tests.

## Phases

> **NOTE — keep the checkboxes current as you go. This is not bookkeeping; it is
> the only way anyone can see where the work actually is.**
>
> - Tick `- [x]` **in the same commit as the work it describes** — never in a
>   batch at the end of a phase, never "I'll update the plan after". If it is a
>   separate step it will be skipped, and the list becomes a lie.
> - Use `- [~]` for partially done and say in one line what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>` rather than
>   deleting it. A vanished task is indistinguishable from a forgotten one.
> - Fill the phase's `Commit:` line with the hash(es) the moment it lands.
> - Add any task you discover you needed; an unlisted task that got done is a
>   task the next reader will do twice.
>
> **An unticked box means NOT DONE.** Someone reading this file — including the
> person who has to decide whether to stop — is entitled to trust that.

**Common per-phase check sequence** (each phase's Acceptance names which built-ins and filters):

1. Before editing: `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → record N;
   `cargo check --all-targets 2>&1 | grep -c '^warning'` → record W.
2. After editing: `rustfmt --edition 2021 <each new/changed .rs file>`; `git diff --stat` shows
   only this phase's files.
3. `cargo check --all-targets 2>&1 | grep -c '^warning'` → W (no new warning) (est. 2 min, guess).
4. `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → N (est. 2 min, guess).
5. `cargo test -p mfb --bin mfb spec_citations_resolve` → passes (est. 2 min, guess).
6. `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings` → pass (est. 2 min, guess).
7. `cargo build --release` (est. 5 min, guess), then each named
   `sh scripts/artifact-gate.sh target/release/mfb <builtin>` → 0 diffs (~1 min each). Do not run
   two gates at once (`pgrep -f "artifact-gate|test-accept"` empty first; `.ai/testing-gates.md`).
8. `cargo test --test file_size_budget 2>&1 | grep -E 'src/codegen/builtins/(net|process|regex|tls|vector)/'`
   → only the rows the phase names (a `STALE` row for each converted `foo.rs`, whose path is gone;
   `TIGHTEN` for a file that keeps its path); then `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`
   and commit `tests/guards/file_size_ceilings.txt` with the phase (est. 1 min, guess).
9. Every new file: `python3 -I tools/loc-census/census.py --file <path>` → code ≤ 900 (≤ 1000 for
   `gen_openssl/connect.rs`, §3), inline test ≤ 250.
10. The phase's sweep command (§4.9) → no hit naming an old path of the phase.

### Phase 1 — process backends and regex prose (pure motion)

- [x] `process/gen_unix.rs` → `process/gen_unix/{mod,spawn}.rs` per §4.1 (`git mv` first); glob re-export.
- [x] `process/gen_windows.rs` → `process/gen_windows/{mod,command_line,environment}.rs` per §4.2;
      the inline tests stay in `mod.rs`; two glob re-exports.
- [x] `regex/mod.rs`: lines 90-239 → `regex/package_doc.rs` per §4.3; `INTRO`/`DESC` `pub(super)`;
      `mod package_doc; use package_doc::{DESC, INTRO};`.
- [x] The 4 sweep items of §4.9 marked Phase 1 (incl. `.ai/arch-abi.md:834`).

Acceptance: spawn, shell, send, wait and the Windows command-line/environment builders emit
byte-identical code on every target (incl. `windows-x86_64`); the regex package's man text is unchanged.
  Check: the common sequence with built-ins `process`, `regex`; plus
  `cargo test -p mfb --bin mfb -- codegen::builtins::process codegen::builtins::regex` → the inline
  tests pass; `target/release/mfb man regex` before vs after → `diff` empty (est. 30 s); step 8
  rows: `STALE` for `process/gen_unix.rs` and `process/gen_windows.rs`, `TIGHTEN` for
  `regex/mod.rs`, all gone after tighten (est. 20 min total, guess).
  Done: `codesplit.py` moved the §4.1/§4.2 items by name (`t1-unix.json`, `t1-win.json`); `t1wire.py` moved `INTRO`/`DESC`.
  0 visibility edits for process (as §4.1/§4.2 predicted: `cargo check` clean before any edit); 2 `pub(super)` in regex.
  Verified 2026-10-20: `motion.py` → only the wiring lines and the four §4.9 comment repoints differ; N = 5018 before
  and after, W = 0; `spec_citations_resolve` + `codegen::builtins::process` + `codegen::builtins::regex` → 17 passed;
  guards → 9, 8, 7 + 1 ignored; `bash scripts/artifact-gate.sh … process` and `… regex` → 0 diffs each; `mfb man regex`
  before/after → `diff` empty (199 lines); budget → the two STALE rows and `TIGHTEN code 856 (ceiling 1003)` for
  regex, none after the tighten; census → gen_unix mod 415 / spawn 722, gen_windows mod 510 + 101 test / command_line
  249 / environment 438, regex mod 856 + 169 test / package_doc 149; sweep → only the audio `gen_windows.rs` (another file).
Commit: 887471f80

### Phase 2 — ICMP ping and SIMD float kernels (pure motion)

- [x] `net/gen_ping.rs` → `net/gen_ping/{mod.rs,posix/mod.rs,posix/socket.rs,windows.rs}` per §4.4;
      the 7 `pub(super)` edits; the inline tests stay in `mod.rs`.
- [x] `vector/builder_simd_float_math.rs` → `vector/builder_simd_float_math/{mod,dispatch,inverse_trig,trig_reduction,trig,exp_log,primitives}.rs`
      per §4.5; the two `#![allow]`s stay first in `mod.rs`; the 20 `pub(super)` edits;
      `pool_layout_tests` stays in `mod.rs`.
- [x] Repoint the 4 `gen_ping.rs` and 6 `builder_simd_float_math.rs` citations (§4.4, §4.5).
- [x] The 5 sweep items of §4.9 marked Phase 2 (incl. both `.ai/build-tooling.md` lines).
- [x] `cargo clippy --all-targets 2>&1 | grep -c 'excessive_precision\|approx_constant'` → 0
      (the inherited allows cover the parts) (est. 3 min, guess).

Acceptance: ping and every SIMD float kernel (and the scalar math that uses them) emit
byte-identical code on every target; the vector BODY checker still passes.
  Check: the common sequence with built-ins `net`, `vector`, `math`, `tls` (calls `net::ping`);
  plus `cargo test -p mfb --bin mfb -- gen_ping pool_layout_tests` → pass;
  `sh scripts/check-generated.sh` → passes (est. 1 min, guess); step 8 rows: `STALE` for
  `net/gen_ping.rs` and `vector/builder_simd_float_math.rs` only, gone after tighten
  (est. 25 min total, guess).
  Done: `codesplit.py` split gen_ping (`t2-ping.json`), then `posix/mod.rs` (`t2-posix.json`); `items.py` split the
  `impl CodeBuilder<'_>` by method (`t2-vec.json`) and the emptied shell was removed; `autovis.py` added exactly the
  20 §4.5 `pub(super)`s. gen_ping took 6 `pub(super)`, not 7 (Corrections). Cites: 4 gen_ping + 7 vector (Corrections).
  Verified 2026-10-20: `motion.py` → only visibility, wiring and rustfmt re-wraps differ; N = 5018, W = 0; clippy
  37065 (unchanged); `spec_citations_resolve` + `gen_ping` + `pool_layout_tests` → 6 passed; guards → 9, 8, 7 + 1
  ignored; artifact gates `net`, `vector`, `math`, `tls` → 0 diffs each; `check-generated.sh` → exit 0 (173 vector
  FUNC bodies match); clippy precision grep → 5, all in `string/format/float_format_sci_ref/tests/mod.rs`, which T
  does not touch (Corrections), 0 in T's files; budget → the two STALE rows, none after the tighten; census →
  gen_ping mod 111 + 102 test / posix 799 / socket 144 / windows 522; builder_simd_float_math mod 279 + 47 test /
  dispatch 423 / inverse_trig 394 / trig_reduction 280 / trig 365 / exp_log 285 / primitives 148; sweep → only the
  `.ai/build-tooling.md:50` history note, reworded to name the directory.
Commit: 6b729705f

### Phase 3 — OpenSSL and Schannel TLS backends (pure motion)

- [x] `tls/gen_openssl.rs` → `tls/gen_openssl/{mod,connect,listen,accept,io,close}.rs` per §4.6;
      the test declaration becomes `#[cfg(test)] #[path = "tests/mod.rs"] mod error_path_release_tests;`
      (H's `gen_openssl/tests/mod.rs` stays where it is); the 8 `pub(crate) use` re-exports.
- [x] `tls/gen_schannel_server.rs` → itself + new `tls/gen_schannel_accept.rs` per §4.7 (no `//!`,
      no inner attributes); header comment updated; `include!("gen_schannel_accept.rs");` after
      `gen_schannel.rs:288`.
- [x] Repoint the 4 `gen_openssl.rs` and 2 `gen_schannel_server.rs` citations (§4.6, §4.7).
- [x] The 4 sweep items of §4.9 marked Phase 3.

Acceptance: every OpenSSL (Linux targets) and Schannel (`windows-x86_64`) TLS lowering emits
byte-identical code; the 9 error-path tests still run under their old names.
  Check: the common sequence with built-ins `tls`, `resource-xfer-slots` (calls `tls::connect`);
  plus `cargo test -p mfb --bin mfb error_path_release_tests` → 9 passed;
  `cargo test -p mfb --bin mfb codegen::builtins::tls` → passes;
  `cargo test --test codegen_helper_scratch_release --test codegen_tls_c_return_x86_64` → pass
  (the 12-site TLS walk) (est. 3 min, guess); step 8 rows: `STALE` for `tls/gen_openssl.rs`,
  `TIGHTEN` for `tls/gen_schannel_server.rs`, gone after tighten (est. 25 min total, guess).
- [x] Fix the stale include names found here: the `schannel*.rs` headers of `gen_schannel_{server,accept,impl,io,
      read_close}.rs`, `.ai/net-tls.md:54` and `.ai/build-tooling.md:14` name `schannel.rs`/`schannel_impl.rs`/
      `schannel_io.rs`, files renamed to `gen_schannel*.rs` (`ls src/codegen/builtins/tls/` → only `gen_schannel*`);
      `gen_schannel_impl.rs:1-3` also said listen/accept "stay unadvertised on Windows", but
      `gen_shared.rs::lower_tls_listen_helper` dispatches `PlatformFamily::Windows => schannel::lower_tls_listen`.
  Done: `codesplit.py` split gen_openssl by name (`t3-openssl.json`); `t3wire.py` wrote the 8 re-exports, re-based
  the `#[path]` to `tests/mod.rs` and split the schannel include by line range (the include files are not
  rustfmt'd: rustfmt does not follow `include!`, `.ai/build-tooling.md`). `connect.rs` kept
  `super::gen_shared::connect_arg_prologue`, written when `super` was `tls`; `gen_openssl/mod.rs` gained
  `use super::gen_shared;` with a comment (no other edit).
  Verified 2026-10-20: `motion.py` → only the wiring, the alias, the `#[path]` and the header comments differ; N = 5018,
  W = 0; `spec_citations_resolve` + `error_path_release_tests` + `codegen::builtins::tls` → 50 passed, of which
  `error_path_release_tests` → 9; guards → 9, 8, 7 + 1 ignored; `codegen_helper_scratch_release` → 12 passed,
  `codegen_tls_c_return_x86_64` → 1; artifact gates `tls`, `resource-xfer-slots` → 0 diffs each; budget → the STALE
  row and `TIGHTEN code 541 (ceiling 1006)`, none after the tighten; census → gen_openssl mod 24 + 3 test / connect
  980 / listen 509 / accept 542 / io 687 / close 271, gen_schannel_server 541, gen_schannel_accept 465; sweep → only
  lines naming the kept `gen_schannel_server.rs`.
Commit: 8457ae1c6

### Phase 4 — macOS TLS client and server (pure motion)

- [x] `tls/gen_macos/client.rs` → `tls/gen_macos/client/{mod,connect,read,write,poll,close}.rs` per §4.8.
- [x] `tls/gen_macos/server.rs` → `tls/gen_macos/server/{mod,identity,listen,accept,close_listener}.rs`
      per §4.8; the 3 identity `pub(super)` edits; `lower_tls_listen_macos` moves whole into `listen.rs`.
- [x] Repoint the 4 citations (§4.8).
- [x] The sweep items of §4.9 marked Phase 4, incl. the **pre-existing stale** line citations in
      `tls/mod.rs:226-228` (evidence §2), written with `gen_macos/server/listen.rs` for listen.

Acceptance: every macOS TLS lowering emits byte-identical code on `macos-aarch64`.
  Check: the common sequence with built-in `tls`; plus `cargo test -p mfb --bin mfb gen_macos` →
  the 23 `gen_macos` tests pass; `cargo test --test cli_tls_listen_accept_build --test codegen_helper_scratch_release`
  → pass (`closeListener` has no golden caller) (est. 3 min, guess); step 8 rows: `STALE` for
  `tls/gen_macos/client.rs` and `tls/gen_macos/server.rs`, gone after tighten, and the one
  interim `NEW code 1008 > 1000 src/codegen/builtins/tls/gen_macos/server/listen.rs` (§3), which
  stays until Phase 5 (est. 25 min total, guess).
- [x] Repoint `gen_macos/tests.rs:734` "(server.rs DATA/ITEMS/CERTREF/KEYREF)" → `server/listen.rs` (the listen
      frame consts; not in §4.9's list, found by the Phase 4 sweep).
- [x] Rewrap every plan-176-T part header to 96 columns (`hdrwrap.py`): 23 were over 100 columns as generated.
  Done: `codesplit.py` split both files by name (`t4-client.json`, `t4-server.json`); `t4wire.py` wrote the 5 + 3
  re-exports and listen's identity import; `autovis.py` added exactly the 3 identity `pub(super)`s.
  Verified 2026-10-20: `motion.py` → only wiring and the 3 `pub(super)` differ; N = 5018, W = 0;
  `spec_citations_resolve` + `gen_macos` → 24 passed, `gen_macos` → 23; guards → 9, 8, 7 + 1 ignored;
  `cli_tls_listen_accept_build` → 3, `codegen_helper_scratch_release` → 12 passed; artifact gate `tls` → 0 diffs;
  budget → the two STALE rows and `NEW code 1018 > 1000 …/server/listen.rs` (the §3 interim row, 1018 not ≈1008),
  STALE rows gone after the tighten; census → client mod 15 / connect 846 / read 385 / write 294 / poll 327 / close
  182; server mod 35 / identity 299 / listen 1018 / accept 482 / close_listener 228; sweep → only the repository
  crate's `server.rs` (`.ai/testing-gates.md:850`, `:868`).
Commit: 76eb35793

### Phase 5 — Extract `lower_tls_listen_macos` phases (largest blast radius last)

- [x] `git mv tls/gen_macos/server/listen.rs tls/gen_macos/server/listen/mod.rs`; move the R2
      consts to module level; add `ListenEnv`, `ListenScratch` and the remainder per §4.10.
- [x] `fn emit_listen_identity` (R6) in `listen/mod.rs`; `listen/endpoint.rs` with
      `pub(super) fn emit_listen_endpoint` (R7–R9); `listen/start.rs` with
      `pub(super) fn emit_listen_start` (R10–R11); the three mechanical body edits of §4.10 only.
- [x] `tls/mod.rs:226` listen reference → `gen_macos/server/listen/start.rs` (`emit_listen_start`).
- [x] `cargo clippy --all-targets 2>&1 | grep -c '^warning'` → the count recorded before the phase.
- [x] `python3 -I tools/loc-census/census.py --file <each of the 3 listen files>` → code ≤ 900;
      record the largest fn's LOC (`emit_listen_endpoint`) here.
      Done: census → `listen/mod.rs` 440, `endpoint.rs` 429, `start.rs` 281; per fn (non-blank, non-comment lines
      of the fn span) → `emit_listen_endpoint` 428, `emit_listen_start` 280, `emit_listen_identity` 196,
      `lower_tls_listen_macos` 161.
- [x] `gen_macos/tests.rs:734`'s Phase 4 repoint → `server/listen/mod.rs` (the frame consts now live there).

Acceptance: `tls::listen` on macOS emits the same instruction stream, vreg numbering and
relocations as before, so every macOS TLS program is byte-identical.
  Check: steps 1–6 and 8–10 of the common sequence; `cargo build --release` then
  `sh scripts/artifact-gate.sh target/release/mfb tls` → 0 diffs (its `macos-aarch64` golden calls
  `tls::listen`) (~1 min); `cargo test --test codegen_helper_scratch_release` → passes (12 sites,
  each with `scratch` in range); `cargo test -p mfb --bin mfb gen_macos` → the 23 tests pass;
  `cargo test --test rt_tls_listen_host_validation --test rt_tls_listener_close_frees --test rt_tls_listener_local_address --test rt_tls_accept_handshake_budget`
  → pass on the macOS host (real listeners through the extracted identity, endpoint and start
  phases) (est. 5 min, guess); step 8 rows: none for `tls/gen_macos/server/` (the interim NEW row
  is gone) (est. 20 min total, guess).
  Done: `t5extract.py` asserted every region boundary of the Phase 4 file, then built `listen/mod.rs` (consts dedented
  to module level, `ListenEnv`, `ListenScratch`, the remainder, `emit_listen_identity`), `endpoint.rs` and `start.rs`,
  applying the three §4.10 edits to code tokens only (never inside a comment or string). Each phase's prologue binds
  only the `ListenEnv` fields its body reads (identity: no `v10`). The function had drifted since planning: a 17th
  label, `port_invalid` (bug-876), read by R5's `emit_port_range_check` and by R12 (Corrections).
  Verified 2026-10-20: `t5verify.py` (undo the three edits, compare) → `emit_listen_identity VERBATIM 191 191`,
  `emit_listen_endpoint VERBATIM 454 454`, `emit_listen_start VERBATIM 283 283`, the remainder's original lines an
  in-order subsequence with only the carrier literals, the three calls and the destructure added, the R2 consts all
  at module level; `cargo check` → 0 warnings before and after rustfmt; clippy → 37065 (unchanged; the listen hits
  are existing warnings that moved with their lines); N = 5018; `spec_citations_resolve` ok; `gen_macos` → 23;
  guards → 9, 8, 7 + 1 ignored; `codegen_helper_scratch_release` → 12 passed; `bash scripts/artifact-gate.sh …
  tls` → 0 diffs; `rt_tls_accept_handshake_budget` 2, `rt_tls_listen_host_validation` 3,
  `rt_tls_listener_close_frees` 4, `rt_tls_listener_local_address` 2 → all passed on the macOS host; budget → no
  row for `tls/gen_macos/server/` (the interim NEW row is gone); sweep → no `server/listen.rs` mention left.
Commit: 31932d45f

## Validation Plan

- Tests: no test is added or removed. The inline tests of `gen_ping`, `gen_windows`, `regex` and
  `builder_simd_float_math`, the 9 `error_path_release_tests` and the 23 `gen_macos` tests run in
  the phases that move their code; the `mfb` binary's test count is equal before and after every phase.
- Coverage check: every T built-in is reached by its `tests/byte-identity/<builtin>` fixture on
  every target it emits for (§2). Not reached by a golden: `tls::closeListener` (covered by
  `cli_tls_listen_accept_build`).
- Runtime proof: the four `rt_tls_*listen*`/`accept` runtime targets in Phase 5 open real macOS
  listeners through the extracted code.
- Doc sync: no `mfb spec` or `mfb man` contract changes; only citations move (4 spec files,
  `spec_citations_resolve`), two pre-existing stale line citations in `tls/mod.rs` are fixed, and
  `.ai/build-tooling.md`, `.ai/arch-abi.md` are repointed.
- Final gate (run ONCE, after Phase 5): `cargo build --release` then
  `sh scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (15–20 min; code motion in
  codegen can change emission for any fixture on any target, and only the full gate builds all
  targets). Then
  `cargo test --test file_size_budget 2>&1 | grep -cE 'src/codegen/builtins/(net/gen_ping|process/gen_(unix|windows)|regex/|tls/gen_macos/(client|server)|tls/gen_openssl|tls/gen_schannel_(server|accept)|vector/builder_simd_float_math)'`
  → 0, and `grep -cE '^ceiling .*(net/gen_ping|process/gen_(unix|windows)|regex/mod\.rs|tls/gen_macos/(client|server)|tls/gen_openssl|tls/gen_schannel_server|vector/builder_simd_float_math)' tests/guards/file_size_ceilings.txt`
  → 0. The full workspace suite runs once, in plan-176-V's Validation Plan.
  Done 2026-10-20 at 31932d45f: `bash scripts/artifact-gate.sh target/release/mfb all` → `1703 tests, 1882 build(s),
  2362 golden(s) checked, 0 diff(s)`; the budget grep → 0; the ceiling grep → 0.

## Decisions

- Rule revised — owner, 2026-10-18 (replaces the 2026-10-14 code ≤ 1000 / test ≤ 250 rule): three
  kinds, non-comment non-blank LOC, scope `.rs`/`.mfb` under `src/`, `packages/`, `examples/`,
  `repository/`, `tests/`: `code` (a non-test-only file's code) ≤ 1000 → split into modules;
  `inline` (`#[cfg(test)]` / `TESTING … END TESTING` LOC inside a non-test-only file) ≤ 250 → moved
  out to a test-only file (one `tests/mod.rs` unless > 1000); `test` (a test-only file's total) ≤ 1000
  → split into modules. Parts target ≤ 900. Ceiling rows are `ceiling <code|inline|test> <n> <path>`.
  No exemptions; only `GENERATED`-banner files are skipped; the guard warns with a banner and never
  fails; a new module is always an acceptable split. In T: `gen_openssl`'s 301 inline test LOC moved
  to `gen_openssl/tests/mod.rs` in plan-176-H; the others (102, 101, 169, 47) stay inline.
- A single function over the code limit is decomposed by extract-method refactoring, gated by
  byte-identical `.ncode` — owner, 2026-10-14. In T: `lower_tls_listen_macos` (§4.10). There are no
  exemptions (rule revised 2026-10-18); the T map's exemption note for `server/listen.rs` is superseded.
- The extract-method refactor is its own phase, after every pure-motion phase — author (Phase 5).
- `lower_tls_listen_macos`'s state travels as `ListenEnv` (read-only) and `ListenScratch` (owned)
  plus `ins`/`rel`/`vregs`; its consts move to `listen/mod.rs`'s module level; the identity phase
  stays in `listen/mod.rs` — author (§3, §4.10).
- `gen_openssl/connect.rs` lands at 965 code LOC with no row: one function under the limit —
  author (§3).
- `gen_schannel_server.rs` splits into a second `include!`d file, following the in-tree include
  chain — author (§4.7).
- Code comments, tools and `.ai/` lines that name a moved file are repointed to the file holding
  the named symbol, or to the directory module when they name the module as a whole — author (§4.9).
- Pre-existing stale line citations in `tls/mod.rs` are replaced by symbol references in T —
  owner rule (AGENTS.md "never leave a bug"), applied by the author.

## Corrections

- 2026-10-20 — §4.10: `lower_tls_listen_macos` has 17 exit labels, not 16 — bug-876 added `port_invalid`
  (`format!("{symbol}_port_invalid")`), branched to from R5's `emit_port_range_check` and emitted in R12. It is a
  `ListenEnv` field like the others, so R5 (in the remainder) also takes edit 2 (`&port_invalid` →
  `&env.port_invalid`). The function was 1018 code LOC at Phase 4, not 1007; the regions keep their order and
  content (identity = old lines 178-368, endpoint 369-822, start 823-1105 of the Phase 4 file).
- 2026-10-20 — §3/§4.6: `gen_openssl/connect.rs` is 980 code LOC, not ≈965 (`census.py --file` → 980;
  `lower_tls_connect_openssl` now spans lines 23-1144 of the old file, not 23-1132). Still one function under 1000,
  so §3's reasoning holds unchanged.
- 2026-10-20 — §4.4: `PING_RECV_BUFFER` is read only inside `posix/socket.rs` (`grep -rn PING_RECV_BUFFER src` → the
  const, its use in `emit_int_sockopt` and comments), so it stays private and `posix/mod.rs` does not import it:
  6 `pub(super)` edits, not 7 (the 7th raised `unused import`).
- 2026-10-20 — §4.5: 7 `builder_simd_float_math.rs` citations, not 6 — `18_math-kernels.md:67`
  `emit_float_binary_body` was added after planning; it now names `…/dispatch.rs`.
- 2026-10-20 — Phase 2's clippy check `grep -c 'excessive_precision\|approx_constant'` → 5, not 0: the hits are 4
  pre-existing warnings in `src/codegen/string/format/float_format_sci_ref/tests/mod.rs` (not a T file; total clippy
  count unchanged at 37065). The check is 0 for T's files.
- 2026-10-20 — Every `sh scripts/artifact-gate.sh` runs as `bash` (the script needs bash).

- 2026-10-18: Rule revised by the owner (code ≤ 1000, inline ≤ 250, test-only file ≤ 1000, no exemptions); file list and split design unchanged; rule restated in intro, common step 9 and Decisions; H's OpenSSL tests are one `gen_openssl/tests/mod.rs`, `gen_macos/tests.rs` (884) is no longer split, `platform_failure.rs` part names are H's.

## Summary

The risk is in Phase 5. It moves 1007 lines of the macOS listen lowering into three consecutive
phase functions, re-plumbing labels and scratches through two private structs with three
mechanical token edits. It is safe because no allocation, label or instruction is added or
reordered, and the `tls` golden on `macos-aarch64` plus four runtime listener tests prove it.
Phases 1–4 are pure motion whose hazards are bookkeeping: 20 spec citations, 22 comment and doc
mentions plus two already-stale line citations, one `#[path]` re-base for plan-176-H's OpenSSL tests, and one
`include!` chain. Untouched: every emitted byte, every crate path callers use, every test name,
`lower_tls_connect_openssl` (964, under the limit), the test module plan-176-H moved, and the bug
documents.
