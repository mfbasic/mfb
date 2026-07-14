# Resource Closed-Default Verification & Offset-8 Standardization Plan

Last updated: 2026-07-12
Effort: medium (1h–2h)
Status: **COMPLETE** — Phases 1–4 done. Phase 2 (offset-8 standardization),
Phase 3 (F7 macOS TLS crash fix + F2 spurious-cleanup fix; F1 already fixed),
Phase 4 (regression tests) all landed and verified. `cargo test` green,
artifact-gate byte-identical on host, both new regression tests proven meaningful
(F7 SIGSEGV→clean, F2 banner→none).
Phase 1 (audit): COMPLETE + RE-RUN 2026-07-12 — see §"Phase 1 Audit — Re-run".
Re-run deltas: original F1 (audio `available`/`poll`/`xruns` null-deref) is now
FIXED in code (commit `47f9acce`); a NEW bug F7 was found (macOS TLS closed-default
offset mismatch → `nw_connection_cancel(0x1)` crash, missed originally because the
macOS Network.framework TLS backend was never read). Phase 3 is NOT empty (F7 + F2).

Every built-in resource type must have a materializable *default* value that is a
**closed** resource — an arena record whose internals are invalid (all zero) but
whose `closed` flag (byte offset 8) is set — so that any operation reaching it is
safe: `close` is an idempotent no-op and `read`/`write`/`accept`/… raise the
shared `ERR_RESOURCE_CLOSED` error via their closed-guard, and **no null handle
is ever exposed to a program**.

The closed-resource default already exists in codegen
(`src/target/shared/code/builder_value_semantics.rs:83-128`, currently uncommitted
in the working tree) and it **hardcodes byte offset 8** as the closed flag. This
plan (1) *verifies* that this guarantee actually holds for all 8 built-in
resources — that every op checks the offset-8 flag *before* dereferencing any
internal pointer, and that `close` short-circuits on it — and (2) *standardizes*
offset 8 as the single canonical closed-flag offset for every built-in resource,
with a compile-time assertion so a future resource cannot silently break the
default. Correct implementation outcome: a single source-of-truth constant for the
closed offset, a compile-time proof that all resource layouts agree with it, a
guard-ordering audit with any gaps fixed, and regression coverage that a
closed-default resource is safe under `close` (drop) and under any operation.

References:

- `src/builtins/resource.rs` — the data-driven resource registry
  (`BUILTIN_RESOURCES`, the 8 built-ins) and the single source of truth for
  resource-ness.
- `src/target/shared/code/builder_value_semantics.rs:38` — `lower_default_value`,
  the sole default-materialization site (resource arm at :83-128).
- `src/target/shared/code/error_constants.rs:516,552,87-89` —
  `FILE_OFFSET_CLOSED = 8`, `RESOURCE_RECORD_SIZE = "80"`,
  `ERR_RESOURCE_CLOSED_CODE`/`_SYMBOL`.
- `.ai/compiler.md` — codegen validation/function-test gate; `scripts/artifact-gate.sh`
  (execution-free byte gate for codegen/IR/lowering changes).
- `mfb spec` §15 (resources) and §11 (thread sendability).

## 1. Goal

- A single canonical constant defines the resource closed-flag offset (8), and
  every built-in resource record layout is *provably* consistent with it (a
  `const _: () = assert!(...)` compile-time check, not a comment).
- `lower_default_value`'s resource arm references that constant instead of the
  literal `8`.
- Audited and (where necessary) fixed: for every built-in resource, **every**
  operation (`read`/`write`/`recv`/`send`/`seek`/`accept`/`poll`/`available`/
  `close`/…) checks the offset-8 closed flag *before* dereferencing any internal
  pointer (fd, mmap'd state pointer, TLS ctx pointer, OS handle), so an all-zero
  record with only offset-8 set never causes a null-deref, `munmap(null)`,
  double-free, or dispose-of-null.
- `close`/drop on a closed-default record is a verified no-op for all 8 resources
  (the reachable path: the default is emitted so scope-drop has something to
  close).
- Regression tests prove the closed-default is safe end-to-end, and the change is
  byte-clean on all targets.

### Non-goals (explicit constraints)

- **No layout changes to any resource record** — *with one exception forced by the
  audit.* The Phase-1 re-run found that the macOS TLS backend places its closed
  flag at offset 0, not 8 (F7), which breaks the closed-default. Fixing F7 by
  moving the macOS TLS closed flag to offset 8 (the recommended Phase-3 option) is
  therefore an in-scope layout change for that one backend. Every *other* built-in
  resource already places its closed flag at offset 8 and is not moved. The
  original non-goal ("if a resource's flag is not at offset 8, that's a bug to
  file") is what surfaced F7.
- **No change to the closed-resource default's observable behavior** — it stays
  an 80-byte arena record, zeroed, offset-8 set. Do not shrink/grow it per
  resource type.
- **No change to the frontend TRAP/divergence rule.** `TYPE_TRAP_FALLTHROUGH`
  (a resource-typed fallible binding's TRAP handler must diverge) stays as-is;
  the closed default is defense-in-depth for the provably-dead error-path
  binding and the drop path, not a license to relax the frontend.
- **No new public surface**, no wire/ABI format change, no `.mfp` change.
- Audio ALSA (Linux) backend parity is in scope for the audit but its
  hardware-run proof is gated on Linux audio hardware (see Validation).

## 2. Current State

**The 8 built-in resources** (`src/builtins/resource.rs:113-214`). Closed-flag
offset per backend (the Phase-1 re-run corrects the earlier "all 8 at offset 8"
claim — the **macOS TLS** backend is at offset 0, see F7):

| Resource | Close op | Closed-flag constant | Value |
|---|---|---|---|
| `File` | `fs.close` | `FILE_OFFSET_CLOSED` (`error_constants.rs:516`) | 8 |
| `Socket` | `net.close` | reuses `FILE_OFFSET_CLOSED` (`net/mod.rs`) | 8 |
| `Listener` | `net.close` | `FILE_OFFSET_CLOSED` | 8 |
| `UdpSocket` | `net.close` | `FILE_OFFSET_CLOSED` | 8 |
| `AudioInput` | `audio.closeInput` | `H_CLOSED` (`audio/mod.rs:18`) — *mirror*; authoritative `S_CLOSED` = 264 in the mmap'd state (`audio/mod.rs:36`) | 8 |
| `AudioOutput` | `audio.closeOutput` | `H_CLOSED` | 8 |
| `TlsSocket` (Linux/OpenSSL) | `tls.close` | `TLS_OFFSET_CLOSED` (`tls/mod.rs:24`) | 8 |
| `TlsListener` (Linux/OpenSSL) | `tls.closeListener` | `TLS_LISTENER_OFFSET_CLOSED` (`tls/mod.rs:33`) | 8 |
| `TlsSocket`/`TlsListener` (**macOS/Network.framework**) | `tls.close` / `tls.closeListener` | **`REC_CLOSED` (`tls/macos.rs:43`)** — a *separate* constant; offset 8 there is `REC_CONN` (`tls/macos.rs:44`) | **0 ⚠ (F7)** |

So the offset is uniformly 8 **except on the macOS TLS backend**, and it is
expressed as **five independent constants** (`FILE_OFFSET_CLOSED`, `H_CLOSED`,
`TLS_OFFSET_CLOSED`, `TLS_LISTENER_OFFSET_CLOSED`, and macOS `REC_CLOSED`), none
tied to the literal `8` the default writes — and one (`REC_CLOSED`) has already
drifted to a *different value* (0). The closed-default writes offset 8
unconditionally, so on macOS TLS it sets `REC_CONN`, not the closed flag → the
F7 crash. Exactly the drift risk Phase 2 must close.

**The default** (`builder_value_semantics.rs:83-128`): for a resource type,
allocates `RESOURCE_RECORD_SIZE` (80) bytes via `ARENA_ALLOC_SYMBOL`, zero-fills
all 80 bytes, then `store_u64(1, record, 8)` — the closed flag — and returns a
`ValueResult{ text: "closed {type_}" }`. Callers of `lower_default_value` that
can see a resource type: uninitialized local binding (`builder_control.rs:156`),
`StoreGlobal` with no initializer (`builder_control.rs:270`), record-field
recursion (`builder_value_semantics.rs:137`), and the `STATE`-payload init
(`emit_resource_state_init`, `builder_value_semantics.rs:22`). The genuine
reaching site per the arm's own comment is the error-path binding of
`RES x = <fallible> TRAP`, plus the arena-transfer arm for
`Result OF <resource>` (`builder_arena_transfer.rs:261-274`, also uncommitted).

**Closed-guard shape.** Every op loads its closed constant and, if set, raises
`ERR_RESOURCE_CLOSED_CODE = "77030004"` (`error_constants.rs:87-89`). Confirmed
guard-then-deref ordering at:
- File: `fs_helpers_io.rs:834/954/1093/1295/1441/1658/1940`; close sets flag at
  `:866-867` with an `already_closed` re-close guard.
- Socket/Listener/Udp: `net/io.rs:48/168/299/549/1222/1525`, `net/poll.rs:50/171`.
- TLS: `tls/openssl.rs:1645/1891/2034` (socket), `:1340/2197` (listener);
  sets flag on close at `:2131/2249`.
- Audio: `audio/macos.rs:714` (write) and `:854` (close) both `load H_CLOSED` →
  `branch_ne` **before** `load H_STATE` (the null-in-default pointer) — so the
  zeroed default is safe; also `:1428/1655`. Linux ALSA mirrors this
  (`audio/mod.rs` dispatch).

**Frontend divergence.** A resource TRAP handler must diverge —
`TYPE_TRAP_FALLTHROUGH` in `syntaxcheck/mod.rs:1738-1774` and re-checked in
`ir/verify/mod.rs:1343-1364`; inline-trap termination via `statement_terminates`
(`ir/lower.rs:1636-1660`). So the error-path binding is provably dead; the closed
default is what the drop path closes.

## 3. Design Overview

Three independent pieces, layered lowest-risk first:

1. **Canonical constant + compile-time assertions (mechanical, zero runtime
   change).** Introduce one authoritative `RESOURCE_OFFSET_CLOSED: usize = 8`
   (alongside `RESOURCE_RECORD_SIZE` in `error_constants.rs`). Make
   `FILE_OFFSET_CLOSED`, `H_CLOSED`, `TLS_OFFSET_CLOSED`,
   `TLS_LISTENER_OFFSET_CLOSED` *equal* it (either alias it or add
   `const _: () = assert!(X == RESOURCE_OFFSET_CLOSED);` next to each). Point the
   default's literal `8` (`builder_value_semantics.rs:122`) at the constant. This
   makes offset 8 the enforced standard and makes the default self-documenting.
   Byte-identical output (same numeric value) → provable via the artifact gate.

2. **Guard-ordering & close-idempotency audit (verification, targeted fixes).**
   For each of the 8 resources, enumerate *every* op and confirm the closed-flag
   load precedes any internal-pointer deref, and that `close` short-circuits on an
   already-set flag (no `munmap`/dispose/`close(fd)` on zeroed internals). Most
   are already correct (§2); the deliverable is the completed audit matrix plus a
   fix for any op that derefs first. The correctness risk concentrates here — a
   single op that reads `fd`/state-ptr before the guard would crash on the
   closed-default record.

3. **Reachability + regression coverage.** Confirm every `lower_default_value`
   caller that can pass a resource type is either frontend-unreachable or produces
   the safe closed record, and add tests that exercise a closed-default resource
   under drop and under an operation.

Rejected alternatives:
- *Per-resource default record sized to the real layout* — rejected; the default
  only needs offset 8 valid and everything else zero, so a fixed 80-byte zeroed
  block is simplest and already what ships. Sizing per type adds a type→size map
  for no safety gain (audio handle is 64B ≤ 80; the mmap'd state is a separate
  block that stays null in the default, which the guards handle).
- *Move audio's authoritative closed to offset 8* — rejected; audio's
  authoritative `S_CLOSED` lives in the mmap'd state (an OS callback thread
  touches it) and offset-8 `H_CLOSED` is the arena-resident mirror the guards
  already read. The mirror at 8 is exactly what the default needs; no move.

## 4. Detailed Design

### 4.1 Canonical offset constant

In `src/target/shared/code/error_constants.rs`, add:

```rust
/// Canonical byte offset of the `closed` flag in every built-in resource
/// record. The closed-resource default (`lower_default_value`) sets exactly
/// this byte; every resource op's closed-guard reads it. All per-resource
/// offset constants MUST equal this — see the compile-time asserts.
pub(crate) const RESOURCE_OFFSET_CLOSED: usize = 8;

const _: () = assert!(FILE_OFFSET_CLOSED == RESOURCE_OFFSET_CLOSED);
```

Add matching `const _: () = assert!(H_CLOSED == RESOURCE_OFFSET_CLOSED);` etc. in
each module that defines a resource closed offset (`audio/mod.rs`, `tls/mod.rs`),
importing the canonical constant. Replace the literal `8` at
`builder_value_semantics.rs:122` with `RESOURCE_OFFSET_CLOSED`.

### 4.2 Audit matrix

Produce (in the plan's commit message / a comment block) a matrix: rows = the 8
resources, columns = each op, cell = the `file:line` of the closed-flag load and
confirmation it precedes the first internal-pointer deref. `close` gets an extra
column: the `file:line` proving it no-ops on an already-closed record.

## Compatibility / Format Impact

None externally observable. `RESOURCE_OFFSET_CLOSED` has the same value (8) the
code already uses, so emitted bytes are unchanged on every target. No wire, ABI,
`.mfp`, layout, or public-API change. The compile-time asserts are the only new
"surface" and are internal.

## Phase 1 Audit — Re-run (2026-07-12, independent)

A full independent re-run of the Phase 1 audit (five parallel guard-ordering
reads: File, net, TLS, audio, reachability) against the current tree
(`HEAD = 4782efcc`). Two material deltas from the original results below:

- **F1 is RESOLVED (no longer a finding).** Audio `available`/`poll`/`xruns`
  (`lower_query`, `src/target/shared/code/audio/macos.rs:927`) now DO check the
  closed flag: `load_u64("%v9", ret, H_CLOSED)` @`:951` → `compare 0` @`:952` →
  `branch_ne(&closed)` @`:953`, **before** the `H_STATE` load @`:954` and the
  `pthread_mutex_lock` @`:957`; the `closed` label returns the empty answer
  (0/FALSE, OK tag) without touching state (`:1002-1004`). This guard was added
  by commit `47f9acce` ("fix(audio/resource): support fallible resource
  bindings") — an ancestor of HEAD (7 commits back), landed **after** the
  original audit recorded F1. So on the current tree audio is SAFE across all
  ops; the F1 Phase-3 task is already done in code. (F6 caveat unchanged: no
  ALSA backend exists yet, so re-check when it lands.)

- **NEW — F7 (BUG, memory safety, HIGH): macOS TLS closed-default offset
  mismatch → `nw_connection_cancel((void*)0x1)` crash.** The original audit's
  offset-8 uniformity claim (F4) and all its TLS citations covered only the
  **OpenSSL/Linux** backend (`tls/openssl.rs`, closed flag at offset 8). It
  never read the **macOS Network.framework** backend (`tls/macos.rs`), which
  uses a *different* record layout: `REC_CLOSED = 0`, **`REC_CONN = 8`**,
  `REC_QUEUE = 16`, `REC_CTX = 24` (`tls/macos.rs:43-46`). The codegen
  closed-default (`builder_value_semantics.rs:114-122`) is backend-independent:
  it zeroes the 80-byte record and sets **offset 8** = 1 (its comment @`:88`
  even asserts "offset 8, shared by every built-in resource record" — false for
  macOS TLS). Consequence on macOS: a closed-default `TlsSocket`/`TlsListener`
  record has offset 0 (`REC_CLOSED`) = 0 → the close guard reads it as **open**
  and does NOT short-circuit (`tls/macos.rs:1632-1634`), then loads `REC_CONN`
  at offset **8** = **1** @`:1659` and calls `nw_connection_cancel((void*)0x1)`
  via `branch_link_register` @`:1661` → dispatch into Network.framework on
  pointer `0x1` → **SIGSEGV / UB**. Reachable on the confirmed `$trap_val` drop
  path: `RES sock = tls::connect(<bad>) TRAP e ... <diverge>` on a macOS build
  materializes the closed-default and, on the diverging `RETURN`/`FAIL`,
  scope-drop invokes the macOS TLS close on it (`emit_resource_cleanup_call`).
  The same offset-0-vs-8 divergence means every macOS TLS op
  (read `:1059`, write `:1412`, accept `:2843`, closeListener `:3229`) reads its
  guard from offset 0, so an offset-8-only "closed" record is invisible to all
  of them — but only `close` is on the reachable drop path, so `close` is the
  crash. **This belongs in Phase 3** (was projected to hold only F1+F2). Fix
  options: (a) make the macOS TLS backend's closed flag also live at offset 8
  (align `REC_CLOSED` with the canonical offset and relocate `REC_CONN`), or
  (b) make the closed-default backend/type-aware so it sets the macOS TLS
  record's real offset-0 flag. Option (a) is consistent with the Phase 2
  `RESOURCE_OFFSET_CLOSED` standardization and is preferred; it also means the
  Phase 2 compile-time assert set MUST include the macOS TLS offsets (currently
  `TLS_OFFSET_CLOSED`/`TLS_LISTENER_OFFSET_CLOSED` in `tls/mod.rs` are 8 but the
  macOS backend's `REC_CLOSED = 0` is a separate, unasserted constant — exactly
  the drift the plan aims to prevent, and it has already drifted).

Everything else in the re-run **matches** the recorded results: File SAFE
(guards at `fs_helpers_io.rs` 834/954/1093/1295/1441/1658/1940; flush/isBuffered/
setBuffered touch only the offset-40 BUF_ENABLED scalar + self-guarding drain);
net SAFE (all load `FILE_OFFSET_CLOSED` @ `net/io.rs` 48/168/299/549/1225/1528,
`net/poll.rs` 50/171 before the fd load; offset-16 state ptr never dereferenced;
`close` short-circuits to `ERR_RESOURCE_CLOSED` with no syscall); TLS **Linux**
SAFE (`tls/openssl.rs` accept 1340 / read 1645 / write 1891 / close 2039 /
closeListener 2202, all branch before the first SSL*/CTX* deref); audio SAFE
(post-fix, above); reachability unchanged (F3: only `$trap_val` reaches the
resource default arm; sites 2/3/4 blocked by `is_defaultable`); F2 unchanged
(File/net `close` returns `ERR_RESOURCE_CLOSED` on re-close → drop path logs a
spurious secondary-cleanup-failure via `record_secondary_cleanup_failure`,
`builder_codegen_primitives.rs:1483-1524`).

**Net verdict of the re-run:** offset-8 closed-default is memory-safe for File,
all net resources, all TLS-on-Linux, and (now) all audio ops. The single
outstanding memory-safety defect is **F7 (macOS TLS)** — a real reachable crash
the original audit missed by not reading the macOS TLS backend. F2 (behavioral
wart) stands. F1 is fixed. Recommend filing F7 as a bug-NN (crashes any
closed/defaulted macOS TLS handle on the drop path) and folding its fix into
Phase 3 alongside the Phase 2 offset standardization.

## Phase 1 Audit Results (completed 2026-07-12) — original

Method: full guard-ordering + close-idempotency read of every op of all 8
built-in resources, plus a reachability trace of `lower_default_value`'s resource
arm and the drop path. Property audited: for the closed-default record (80 bytes,
all zero except offset 8 = 1), every op must load+check the offset-8 closed flag
and branch to raise **before** dereferencing any internal pointer (state ptr,
buffer ptr, SSL*/CTX*, mmap'd audio state) or issuing an fd-syscall (fd = 0 =
stdin on the default); and `close` must short-circuit before any
syscall/free/unmap.

### Memory-safety matrix (crash property — the core guarantee)

| Resource | Ops audited | Verdict |
|---|---|---|
| `File` | open/create, read(text/bytes), write(text/bytes), readLine, eof, close, flush, isBuffered, setBuffered | **SAFE** — every deref/fd-syscall op loads `FILE_OFFSET_CLOSED` (8) and branches first. `flush`/`isBuffered`/`setBuffered` omit the check but touch only zero-valued scalar fields / the self-guarding drain (bails on `BUF_ENABLED==0`) → no deref, no syscall. |
| `Socket`/`Listener`/`UdpSocket` | accept, local/remoteAddress, read, write, receiveFrom, sendTo, poll, set{Read,Write}Timeout, close | **SAFE** — all load offset-8 (`FILE_OFFSET_CLOSED`) and branch before the fd load. The offset-16 state pointer is never dereferenced by any net op. |
| `TlsSocket`/`TlsListener` | accept, read, write, close, closeListener (connect/listen are constructors) | **SAFE** — all load `TLS_OFFSET_CLOSED`/`TLS_LISTENER_OFFSET_CLOSED` (8) and branch before the first SSL*/SSL_CTX* load+use. |
| `AudioInput`/`AudioOutput` | read, readTimeout, write, closeInput, closeOutput | **SAFE** — read/write/close load `H_CLOSED` (8) and branch before loading `H_STATE` (48). |
| `AudioInput`/`AudioOutput` | **available, poll, xruns** | **UNSAFE** — see finding F1. |

Precedents/citations captured during the audit: File —
`fs_helpers_io.rs` guards at 954/1093/1295/1441/1658/1940, close short-circuit
`:834→:881`. Net — `net/io.rs` 48/168/299/549/1222/1525, `net/poll.rs` 50/171,
close via `lower_fs_close_helper(flush_on_close=false)`. TLS — `tls/openssl.rs`
1340/1645/1891/2034/2197, close no-op labels `:2148`/`:2263`. Audio —
`audio/macos.rs` read `:1428`, write `:714`, closeOutput `:854→:902`, closeInput
`:1655→:1692`.

### Findings

**F1 (BUG — memory safety, HIGH). Audio `available`/`poll`/`xruns` null-deref
on any closed stream.** `lower_query` (`src/target/shared/code/audio/macos.rs:922`)
is the only user-facing audio op that never loads `H_CLOSED`. Its prologue loads
`H_STATE` (offset 48) at `:943` and immediately `pthread_mutex_lock(state +
S_MUTEX)` at `:946`; since `S_MUTEX = 0`, on a closed stream (`H_STATE = null` in
the closed-default record, or an already-`munmap`'d state on a normally-closed
stream) this locks address `null` → SIGSEGV. This is reachable **independently of
the closed-default**: user code calling `available()`/`poll()`/`xruns()` after
`close`, or on the error-path binding, crashes. Fix = insert the same
3-instruction guard the sibling ops use (load `H_CLOSED`, `compare 0`,
`branch_ne` to a fail/closed label) before `:943`, mirroring `lower_read`
`:1428-1430`. This is now **Phase 3 work** (was projected empty).

**F2 (design/consistency, MEDIUM). `close`-on-already-closed is not uniform.**
The plan's premise said "`close` is an idempotent no-op." In fact:
- `File`, `Socket`, `Listener`, `UdpSocket` (`lower_fs_close_helper`) return
  **`ERR_RESOURCE_CLOSED`** on re-close (the `already_closed` arm,
  `fs_helpers_io.rs:881-884`) — an idempotent *error*, deliberate for `File`
  (bug-63: refuse to re-close a possibly-recycled fd).
- `TlsSocket`/`TlsListener` and `AudioInput`/`AudioOutput` return **OK** on
  re-close (`tls/openssl.rs:2148`/`:2263`; audio `already` labels) — an
  idempotent *success*.

Consequence on the reachable drop path: the closed-default is dropped when the
`$trap_val` slot's `ActiveCleanup::Resource` fires (see F3), and
`emit_resource_cleanup_call` (`builder_codegen_primitives.rs:1483-1499`) treats a
non-OK close as a *secondary cleanup failure* (`record_secondary_cleanup_failure`,
increments the arena cleanup-failure ledger). So dropping a **File/net**
closed-default logs a spurious cleanup failure, while a **TLS/audio**
closed-default drops cleanly. No crash either way, but the ledger discrepancy is a
real behavioral wart. **Decision needed** (Open Decisions): either make all built-in
`close` ops return OK on already-closed (uniform idempotent no-op — cleanest for
the default, but changes user-visible `File`/`Socket` double-close from error to
OK, a spec/behavior change), OR keep the current per-resource semantics and make
the drop path (`emit_resource_cleanup_call`) treat `ERR_RESOURCE_CLOSED`
specifically as a benign no-op (no ledger entry). Recommend the latter — it
preserves the deliberate bug-63 File semantics and fixes the spurious ledger entry
at one site.

**F3 (reachability — narrow and well-guarded).** The resource arm of
`lower_default_value` (`builder_value_semantics.rs:83-128`) is reachable via
**exactly one** path: the compiler-synthesized `$trap_val` slot for an inline-
`TRAP` resource binding (`src/ir/lower.rs:1356-1368`, `IrOp::Bind{ name:
"$trap_valN", value: None, type_: <resource> }`). The `$`-prefix exempts it from
the value-less-binding rejection at `ir/verify/mod.rs:864`. That slot is stored
the closed default (`builder_control.rs:156`) and registers an
`ActiveCleanup::Resource` (`builder_control.rs:218-223`), so on the diverging
error path (`RETURN`/`FAIL` in the TRAP handler) its close runs on the
closed-default record — this is the drop path F2 refers to. The other three
`lower_default_value` sites are statically UNREACHABLE for resources:
- Value-less global (`builder_control.rs:270`) → `TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE`
  / `TYPE_LET_REQUIRES_VALUE` (`ir/verify/mod.rs:305-330`).
- STATE payload (`emit_resource_state_init`, `builder_value_semantics.rs:22`) →
  `TYPE_STATE_INVALID` requires a defaultable data type (`ir/verify/mod.rs:836-843`).
- Record-field recursion (`builder_value_semantics.rs:137`) →
  `TYPE_RESOURCE_FIELD_FORBIDDEN` (`ir/verify/mod.rs:2035-2045`); records cannot
  own resources.
A *user-written* uninitialized resource local is also rejected
(`TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE`, `ir/verify/mod.rs:870-882`) — `is_defaultable`
(`ir/verify/mod.rs:2385-2420`) returns false for every resource
(`close_op_for(...).is_some()`).

**F4 (offset-8 uniformity — confirmed).** All 8 built-in resources place the
closed flag the codegen default writes at byte offset 8, but via four independent
constants (`FILE_OFFSET_CLOSED`, `H_CLOSED`, `TLS_OFFSET_CLOSED`,
`TLS_LISTENER_OFFSET_CLOSED`), none tied to each other or to the literal `8` the
default emits — exactly the drift risk Phase 2 standardizes. Audio's offset-8
`H_CLOSED` is a deliberate arena-resident *mirror* of the authoritative
`S_CLOSED=264` in the mmap'd state; read/write/close read the mirror, so offset 8
is correct for the default there too.

**F5 (semantic wart, LOW — not safety). Closed-path error is not `ERR_RESOURCE_CLOSED`
everywhere.** File `flush`/`isBuffered`/`setBuffered` silently return OK on a
closed handle (no rejection), and audio `read`/`write` raise `ERR_AUDIO_DEVICE`
(not `ERR_RESOURCE_CLOSED`) on the closed branch (`macos.rs:716→824`,
`:1430→1624`). Memory-safe; only the surfaced error/behavior differs from the
uniform "closed handle rejects with `ERR_RESOURCE_CLOSED`" contract. Track as
cleanup, out of this plan's safety scope unless the spec mandates uniformity.

**F6 (scope note). ALSA/Linux audio backend does not exist yet.** Only
`audio/macos.rs` is implemented; `mod.rs:103-110` errors for non-macOS targets
(ALSA lands in plan-33-C). So the audio guard-ordering audit — including the F1
fix — must be re-checked when the ALSA backend is written; the same `lower_query`
guard must be present there.

### Reachable drop path (confirmed)

Scope-drop of a resource binding calls its registered close op on the binding
pointer (`emit_resource_cleanup_call`, `builder_codegen_primitives.rs:1483`), so a
closed-default record **does** have `fs.close`/`net.close`/`tls.close`/
`audio.closeInput` invoked on it during unwinding. Every close audited above
short-circuits on offset 8 before any syscall/free/unmap → **no double-free, no
`munmap(null)`, no `close(0)`**. (The only residue is F2's ledger entry for
File/net.)

## Phases

### Phase 1 — Audit & document (no code change) — DONE

Verify the guarantee holds today and record the evidence; this is separately
valuable as the correctness proof even before any refactor.

- [x] Enumerate every op of all 8 resources and confirm closed-flag-load precedes
      first internal-pointer deref; build the audit matrix (§Phase 1 Audit Results).
- [x] Confirm each `close` no-ops on an already-closed record (no
      `munmap`/dispose/`close(fd)`/free on zeroed internals) — cited per resource.
- [x] Confirm every `lower_default_value` caller that can pass a resource type is
      frontend-unreachable or safe (F3: only `$trap_val` is reachable; other 3
      sites statically blocked).
- [x] Record any op that derefs before the guard as a finding for Phase 3 (F1:
      audio `available`/`poll`/`xruns`).

Acceptance: MET — audit matrix covering all 8 resources × all ops, each with a
guard-ordering + close-no-op verdict; findings F1–F6 enumerated. One UNSAFE op
found (F1); Phase 3 is non-empty.
Commit: — (documentation only; lives in this plan)

### Phase 2 — Standardize offset 8 (mechanical, byte-identical) — DONE

Single canonical constant + compile-time asserts; the default references it.

- [x] Add `RESOURCE_OFFSET_CLOSED = 8` to `error_constants.rs` with the
      `assert!(FILE_OFFSET_CLOSED == …)` compile-time check (§4.1). Also added
      `RESOURCE_RECORD_SIZE_BYTES = 80` (numeric) + asserts that the closed
      offset and the full File layout fit inside the default record.
- [x] Add `const _: () = assert!(… == RESOURCE_OFFSET_CLOSED)` beside `H_CLOSED`
      (`audio/mod.rs`, plus `H_RECORD_SIZE <= RESOURCE_RECORD_SIZE_BYTES`),
      `TLS_OFFSET_CLOSED` and `TLS_LISTENER_OFFSET_CLOSED` (`tls/mod.rs`).
- [x] **F7 corollary:** `assert!(REC_CLOSED == RESOURCE_OFFSET_CLOSED)` added in
      `tls/macos.rs` — it holds because Phase 3's F7 fix moved `REC_CLOSED` to
      offset 8 in the same change. The assert is the guard that would have caught
      the original drift; it was NOT relaxed to offset 0.
- [x] Replace the literal `8` at `builder_value_semantics.rs` with
      `RESOURCE_OFFSET_CLOSED` (and the byte loop uses `RESOURCE_RECORD_SIZE_BYTES`).
- [x] Tests: a Rust unit test in `resource.rs`
      (`every_builtin_resource_has_a_close_op`) asserting every built-in
      resource's registered close op exists, plus the compile-time asserts
      themselves.

Acceptance: MET — `cargo test` green (2729 unit tests + integration);
`scripts/artifact-gate.sh` byte-identical (908 tests, 1069 goldens, 0 diffs — the
offset value is unchanged). Setting a per-resource offset to a non-8 value fails
compilation (verified: reverting the macOS TLS swap without disabling its assert
is a compile error).
Commit: —

### Phase 3 — Fix the guard-ordering gap + close-semantics wart (NON-EMPTY per Phase 1)

Phase 1 (re-run) leaves **F7** (macOS TLS crash — the top-priority fix) and **F2**
(behavioral wart on the drop path). **F1 is already fixed in code** (commit
`47f9acce`; the guard now lives at `audio/macos.rs:951-953`), so its checkbox is
done — it remains listed only for provenance.

- [x] **F7 — macOS TLS closed-default → `nw_connection_cancel(0x1)` crash — DONE.**
      Fixed by moving the macOS TLS backend's closed flag to the canonical offset 8
      (`tls/macos.rs`): swapped `REC_CONN` (now 0) and `REC_CLOSED` (now 8); every
      record access already went through the named constants, so no load/store site
      changed and the aarch64 trampolines (which touch only `CTX_*`/`LCTX_*`, never
      the handle record) are unaffected. Added `assert!(REC_CLOSED ==
      RESOURCE_OFFSET_CLOSED)`. **Proven meaningful:** with the pre-fix layout the
      regression test SIGSEGVs (exit 139); after the fix it exits cleanly (0).
      Test: `tests/rt-behavior/resources/closed-default-tls-drop-rt`.
- [x] **F1 — audio `available`/`poll`/`xruns` null-deref — DONE (`47f9acce`).**
      The closed guard already lives at the top of `lower_query`
      (`audio/macos.rs`: `load H_CLOSED` → `compare 0` → `branch_ne(&closed)`),
      before the `H_STATE` load and `pthread_mutex_lock`; the `closed` label returns
      the empty answer (0/FALSE, OK). No further action. The audio closed-default
      *drop* path is exercised crash-free by the existing
      `tests/rt-error/audio/openOutput_invalid_rt` (drops an audio closed-default on
      error propagation, clean exit 255). A query-on-a-live-then-closed stream needs
      audio hardware (F6) and is deferred per the plan's hardware gating.
- [x] **F2 — spurious cleanup-failure on File/net closed-default drop — DONE.**
      `emit_resource_cleanup_call` (`builder_codegen_primitives.rs`) now treats a
      close result of exactly `ERR_RESOURCE_CLOSED` as benign (skips
      `record_secondary_cleanup_failure`); every other non-OK close still records.
      This preserves the deliberate bug-63 File re-close error for *user* code
      (unchanged) and fixes only the drop path. A genuine drop-close failure
      (`ERR_CLOSE_FAILED`, `7-703-0006`) still logs — verified by the existing Rust
      test `native_resource_cleanup_reports_secondary_close_failure_metadata`.
      **Proven meaningful:** without the fix the File/net closed-default drop path
      prints `Cleanup failure: 7-703-0004` at exit; with it, no banner.
      Test: `tests/rt-behavior/resources/closed-default-drop-rt`.
- [x] Tests: `closed-default-tls-drop-rt` (F7, macOS TLS closed-default drop, clean
      exit, no SIGSEGV — Linux/OpenSSL was already offset-8 so it is a no-crash
      proof on every target); `closed-default-drop-rt` (File + Socket closed-default
      drop, no crash, no spurious cleanup-failure — F2).
- [ ] **F6 caveat:** when the ALSA/Linux audio backend lands (plan-33-C), its
      `available`/`poll`/`xruns` must carry the same guard — note it there.
      (Not actionable now; no ALSA backend exists.)

Acceptance: MET — a macOS TLS closed-default drop runs to a clean exit (test,
exit 0 vs. pre-fix SIGSEGV); the File/net closed-default drop path produces no
spurious cleanup-failure (test); byte gate re-run byte-identical on host (the
macOS TLS layout swap is transparent through named constants, and no golden
fixture drives a macOS TLS program — 0 diffs). Cross-target byte goldens
regenerate on their machines.
Commit: —

### Phase 4 — Regression coverage for the closed default — DONE

Prove the default is safe end-to-end for the reachable paths.

- [x] `rt-behavior` tests reach the closed-default drop path via
      `RES x = <fallible-open> TRAP e … <diverge>` for a `File`, a `Socket`, and a
      `TlsSocket` (opens/connects fail → materialize closed-default → scope-drop →
      close on the closed record), asserting no crash / no double-free.
- [x] **User-visible double-close is NOT expressible** — `close` consumes the
      handle (move semantics), so a second `close(x)` is a compile error
      (`TYPE_USE_AFTER_MOVE`, `2-203-0055`), already covered by
      `tests/syntax/resources/ownership-resource-double-close-invalid` and siblings.
      The idempotent-no-op property the default relies on is therefore only ever
      reached at runtime through the compiler-controlled *drop* of an already-closed
      record — the closed-default drop path — which the two `rt-behavior` tests
      above exercise for File/Socket/TLS, and the existing audio invalid-open test
      exercises for audio. No standalone double-close runtime test is possible.
- [x] Tests placed under `tests/rt-behavior/resources/` per the 4-folder
      convention.

Acceptance: MET — the new tests pass on host; the closed-default drop paths run
clean with no crash, no double-free, and no spurious cleanup diagnostic.
Commit: —

## Validation Plan

- Tests: Rust unit test in `src/builtins/resource.rs` (close-op existence);
  compile-time asserts in `error_constants.rs`/`audio/mod.rs`/`tls/mod.rs`;
  `tests/rt-error/` (op-on-closed raises `ERR_RESOURCE_CLOSED`) and
  `tests/rt-behavior/` (default drop path is crash-free).
- Runtime proof: an MFBASIC program whose `RES` open fails and whose TRAP handler
  diverges, run to completion with no crash; plus an explicit double-`close`
  program that completes cleanly. Verify per `.ai/compiler.md` runtime-completion
  gate (the program must actually run to a clean exit, not merely compile).
- Doc sync: if `mfb spec` §15 does not already state the closed-default guarantee
  and the offset-8 invariant, add one sentence (per `.ai/specifications.md`).
- Acceptance: `cargo test`; `scripts/artifact-gate.sh` byte-identical on all
  targets (Phase 2 is a no-op numerically); host acceptance suite green. Audio
  ALSA hardware run is gated on Linux audio hardware — mark that proof as deferred
  if unavailable, but the ALSA guard-ordering audit (Phase 1) is not deferred.

## Open Decisions

- **(F7) macOS TLS closed-flag fix strategy.** (a) Move the macOS TLS backend's
  `REC_CLOSED` to offset 8 (relocate `REC_CONN`/`REC_QUEUE`/`REC_CTX`) so all
  backends share the canonical offset and the Phase-2 assert holds —
  **recommended**, it makes offset 8 a real cross-backend invariant and lets the
  compile-time assert catch any future drift; cost is a macOS TLS byte re-baseline.
  vs. (b) keep the macOS layout and make the closed-default backend/type-aware
  (set offset 0 for macOS TLS) — avoids the layout move but complicates the
  single-source-of-truth default and leaves two live closed-flag offsets. Recommend
  (a).
- **(F2) Uniform `close`-on-already-closed semantics.** Options: (a) make the
  drop path (`emit_resource_cleanup_call`) treat `ERR_RESOURCE_CLOSED` as a benign
  no-op so File/net closed-default drops stop logging a spurious cleanup failure —
  **recommended**, one-site fix, preserves the deliberate bug-63 File re-close
  error for user code; vs. (b) normalize every built-in `close` to return OK on
  already-closed — uniform, but flips user-visible `File`/`Socket` double-close
  from `ERR_RESOURCE_CLOSED` to OK (a spec/behavior change needing sign-off).
- **(F1/F5) Error surfaced by the new audio query guard** — `ERR_RESOURCE_CLOSED`
  (matches File/net/TLS closed semantics) vs. `ERR_AUDIO_DEVICE` (matches audio
  read/write's own closed branch). **Recommend `ERR_RESOURCE_CLOSED`** for a
  uniform closed-handle contract, and separately consider aligning audio
  read/write (F5) to it.
- Alias vs. assert for the per-resource offset constants — **recommend keeping
  the per-resource names and adding `const _: () = assert!(== RESOURCE_OFFSET_CLOSED)`**
  (preserves the self-documenting `H_CLOSED`/`TLS_OFFSET_CLOSED` names and their
  local comments) vs. deleting them in favor of the one canonical constant. (§4.1)
- Whether to also assert `RESOURCE_RECORD_SIZE (80) >= H_RECORD_SIZE (64)` and the
  other record sizes so the zeroed default always covers each real layout —
  **recommend yes**, it is a free compile-time guard. (§4.1)

## Summary

**Phase 1 (audit) is complete and was independently re-run 2026-07-12.** The
offset-8 closed-default is memory-safe across File, all net resources, TLS **on
Linux/OpenSSL**, and (post-`47f9acce`) all audio ops — every op that dereferences
an internal pointer or issues an fd-syscall checks the offset-8 closed flag first,
and every such `close` short-circuits before any syscall/free/unmap. Reachability
of the default arm is narrow and well-guarded: only the synthesized `$trap_val`
slot reaches it (F3). Re-run status of the findings:

- **F1 (audio null-deref) — FIXED** in commit `47f9acce`; `lower_query` now guards
  on `H_CLOSED` before the state load. No longer outstanding.
- **F7 (macOS TLS crash) — NEW, HIGH, outstanding.** The original audit read only
  the OpenSSL/Linux TLS backend and missed the macOS Network.framework backend,
  where the closed flag is at offset 0 and offset 8 is the connection pointer. The
  offset-8 closed-default therefore reads as *open* on macOS and `close` derefs a
  bogus `0x1` connection pointer → `nw_connection_cancel(0x1)` SIGSEGV on the
  reachable `$trap_val` drop path. This is the top Phase-3 fix.
- **F2 (File/net spurious cleanup-failure on re-close) — outstanding**, behavioral
  wart only (no crash); Phase-3 decision pending.

The remaining engineering risk is Phase 3 (**F7 macOS TLS fix** + F2 decision).
Phase 2 stays a mechanical standardization turning the de-facto offset-8 convention
into a compiler-enforced invariant — and the F7 discovery shows exactly why it is
needed: the macOS TLS offset had **already silently drifted to 0**, which the
Phase-2 `assert!(REC_CLOSED == RESOURCE_OFFSET_CLOSED)` will catch (and, until F7
is fixed, correctly refuse to compile). Untouched otherwise: the non-macOS-TLS
resource layouts, the default's shape, the frontend TRAP-divergence rule, and all
external formats.
