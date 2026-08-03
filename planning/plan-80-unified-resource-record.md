# Unified Resource-Record Header Plan

Last updated: 2026-08-02
Effort: very large (multi-day; 5 native backends re-slotted + wide golden regen)
Depends on: plan-74 (uniform `STATE` on a resource union — landed; this plan
relocates the slot plan-74 writes) and is a **precondition for plan-76-D**
(async HTTP over a `Stream` resource union — blocked on the D4 core-premise
defect this plan fixes).

Give every built-in and package resource **one canonical record header** so the
generic `STATE` payload (plan-74) has a slot that is free in *every* resource
layout — not just the File-layout ones. Today the record header diverges after
offset 8: plan-74 hard-writes the `STATE` pointer to offset **16**, which is free
in the File/Socket layout but holds `SSL*` (openssl), a dispatch queue (macOS
Network.framework), a state-block pointer (schannel), `CTX` (TLS listener), or
`H_SAMPLE_RATE` (audio) in the other layouts. Writing `STATE` there corrupts the
active resource → SIGSEGV. This plan inserts a self-describing type tag, moves the
header to a fixed shape (`tag@0`, handle@8, `closed@16`, `STATE@24`), pushes each
backend's type-specific fields to offset 32+, and makes the `STATE@24` reservation
a compiler-enforced invariant (an `assert!` per backend, mirroring the existing
`closed@8` assert). It also gives the two open-ended resource kinds
(`Imported`/`Native`) an in-record `CLOSE func ptr`, which closes a pre-existing
imported-package leak/double-free as a side effect.

This is D4-option-A ("one true resource header") from
`planning/plan-76-D-http-async-stream.md` Corrections D4 — chosen over the
stateless-`MUT`-param redesign (option C) because it makes resource `STATE` work
for *every* resource forever, not just for D's `Stream` union.

References:

- `planning/plan-76-D-http-async-stream.md` — Corrections **D4** (the core-premise
  defect: union `STATE` over a `TlsSocket` variant SIGSEGVs) and the Prerequisites
  design-gate row this plan satisfies.
- `src/builtins/resource.rs` — `BUILTIN_RESOURCES` (the 8 built-in resources) and
  `ResourceKind` (`Builtin` / `Imported` / `Native`).
- `src/target/shared/code/error_constants.rs` — `FILE_OFFSET_*` (the File-layout
  header, the de-facto template), `RESOURCE_RECORD_SIZE`/`_BYTES` (the 80-byte
  envelope), `RESOURCE_OFFSET_CLOSED` (the enforced `closed@8` invariant).
- `src/target/shared/code/tls/mod.rs` — `TLS_OFFSET_{FD,CLOSED,SSL,CTX}`,
  `TLS_LISTENER_OFFSET_*`, and the `closed`-offset asserts (`:41-42`).
- `src/target/shared/code/tls/macos/mod.rs` — `REC_{CONN,CLOSED,QUEUE,CTX}`.
- `src/target/shared/code/tls/schannel.rs` — the arena `st::` state block
  (`CRED@0`, `CTXT@16`, …) the schannel record points to.
- `src/target/shared/code/audio/mod.rs` — `H_{KIND,CLOSED,SAMPLE_RATE,CHANNELS,
  BYTES_PER_FRAME,BUFFER_FRAMES,STATE}` and `H_RECORD_SIZE`.
- `src/target/shared/code/builder_resource_cleanup.rs`,
  `builder_arena_transfer.rs` — the generic close dispatch and the
  thread-transfer record copy (both key off the envelope size + `STATE@16`).
- `lower_default_value` — writes the closed-default (zeroed record); it sets
  exactly the `closed` byte, which moves 8 → 16 here.

## Prerequisites

This is a precondition on the whole plan, not a dependency to negotiate. plan-80
does not redesign plan-74's union machinery (it keeps `{tag, record-ptr}` and the
`STATE`-in-record model) — it only relocates the in-record `STATE` offset and
regularizes the header. plan-76-D must **not** start until plan-80's `STATE@24`
gate (Phase 4) is green.

| Must be true | Command | Status |
|---|---|---|
| Repo builds clean; full gate green at HEAD | `cargo build --bin mfb && bash scripts/artifact-gate.sh target/debug/mfb all` | MET — baseline gate 2026-08-03: 1146 tests, 1553 goldens, **0 diffs**, exit 0 |
| plan-74 union STATE landed (the mechanism this plan relocates) | `ls planning/completed/plan-74-* 2>/dev/null` OR grep `FILE_OFFSET_STATE` in `src/target/shared/code/error_constants.rs` | MET — `planning/completed/plan-74-resource-union-state.md` present; `FILE_OFFSET_STATE` present |
| No concurrent artifact-gate running | `pgrep -f artifact-gate` → empty | MET at baseline (empty); re-check before each gate |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run and update before continuing and before stopping. If you stop, report the
> status of *all* prerequisites, not just the one that blocked you.

## 1. Goal

A single canonical resource-record header, identical across all 5 native backends
and all `ResourceKind`s:

| Offset | Field | Notes |
|---|---|---|
| 0  | `tag` (u64) | resource type id; `0x00` = uninitialized/invalid (never a live record) |
| 8  | handle | **polymorphic**: fd / `conn ptr` (macOS NW) / `H_KIND` (audio) / `CPtr` (imported/native) |
| 16 | `closed` flag | moved from offset 8 |
| 24 | `STATE` ptr | plan-74 payload; **free in every layout** — this is the D4 fix |
| 32+ | type-specific | per-backend fields, or the `CLOSE func ptr` for `0xFE`/`0xFF` |

Full per-column layout (the agreed design):

| Offset | File | Socket | UdpSocket | Listener | TLS openssl | TLS macOS | TLS schannel | TLSListener | Audio (In/Out) | Imported (0xFE) | Native (0xFF) |
|--------|------|--------|-----------|----------|-------------|-----------|--------------|-------------|----------------|-----------------|---------------|
| 0  | 0x01 | 0x02 | 0x03 | 0x04 | 0x05 | 0x06 | 0x07 | 0x08 | 0x09 | 0xFE | 0xFF |
| 8  | fd | fd | fd | fd | fd | conn ptr | fd | fd | H_KIND | CPtr | CPtr |
| 16 | closed | closed | closed | closed | closed | closed | closed | closed | closed | closed | closed |
| 24 | STATE | STATE | STATE | STATE | STATE | STATE | STATE | STATE | STATE | STATE | STATE |
| 32 | out-buf ptr | — | — | — | CTX | CTX | — | CTX | H_SAMPLE_RATE | CLOSE fn ptr | CLOSE fn ptr |
| 40 | out-buf filled | — | — | — | SSL | REC_QUEUE | SCH_STATE ptr | — | H_CHANNELS | — | — |
| 48 | out-buf enabled | — | — | — | — | — | — | — | H_BYTES_PER_FRAME | — | — |
| 56 | read-buf ptr | — | — | — | — | — | — | — | H_BUFFER_FRAMES | — | — |
| 64 | read-buf pos | — | — | — | — | — | — | — | H_STATE (mmap ptr) | — | — |
| 72 | read-buf fill | — | — | — | — | — | — | — | — | — | — |
| 80 | read-buf at-eof | — | — | — | — | — | — | — | — | — | — |

- **Envelope grows 80 → 96 bytes** (File now ends at offset 80 → 88 needed; round
  to 96 for 16-byte alignment + one slot of headroom). `RESOURCE_RECORD_SIZE`,
  `RESOURCE_RECORD_SIZE_BYTES`, the fits-inside asserts, the arena-alloc immediate,
  and the thread-transfer copy length all move to 96.
- **`STATE@24` becomes a compiler-enforced invariant** — a per-backend
  `const _: () = assert!(<backend>_offset_state == RESOURCE_OFFSET_STATE)`, exactly
  as `closed@8` is asserted today. This is the check whose absence D4 identified.
- **Close dispatch:** `tag < 0xFE` → static table keyed by tag (builtins, closer
  known at compile time); `tag ≥ 0xFE` → call `record[32]` (imported/native carry
  their own destructor).
- `cargo test` green, `artifact-gate … all` green (goldens regenerated — see §
  Compatibility), and the D4 repro (`http::read("https://…")`, and a bare
  `RES s AS Stream STATE T = tls::connect(...)`) no longer SIGSEGVs.

### Non-goals (explicit constraints)

- **Not a plan-74 redesign.** The union value stays `{tag@0, record-ptr@8}` and
  `STATE` stays in the active variant's record — only its offset moves 16 → 24.
- **No behavioral change to any resource op.** Every op reads the same field it
  reads today; only the offset constant changes. A `.run`/rt-behavior diff is a
  bug, not a re-baseline.
- **Do not implement plan-76-D here.** plan-80 unblocks D by fixing the layout; D
  is landed by its own plan afterward.
- **No new resource types.** This regularizes the 8 built-ins + the 2 open kinds
  that already exist.

## 2. Current State

The header is uniform only for offsets 0–8, and only `closed@8` is enforced.

### Measured populations

| What | Value | Command |
|---|---|---|
| Built-in resources | 8 | `BUILTIN_RESOURCES` in `src/builtins/resource.rs:138` (File, Socket, Listener, UdpSocket, AudioInput, AudioOutput, TlsSocket, TlsListener) |
| `ResourceKind` variants | 3 | `enum ResourceKind` `src/builtins/resource.rs:41` (Builtin/Imported/Native) |
| Envelope size today | 80 | `RESOURCE_RECORD_SIZE_BYTES` `error_constants.rs:848` |
| Enforced header fields | 1 | `closed@8` — `RESOURCE_OFFSET_CLOSED` + asserts `tls/mod.rs:41-42`, audio, macos |
| plan-74 `STATE` offset | 16 | `FILE_OFFSET_STATE` `error_constants.rs:811` |

### Verified per-backend offset-16 occupants (why 16 can't hold STATE)

| Layout | offset 16 today | source |
|---|---|---|
| File / Socket / UdpSocket / Listener | `STATE` (free) | `FILE_OFFSET_STATE` |
| TLS openssl | `SSL*` | `TLS_OFFSET_SSL` `tls/mod.rs:26` |
| TLS macOS NW | dispatch queue (`REC_QUEUE`) | `tls/macos/mod.rs:90` |
| TLS schannel | state-block ptr | `store_u64(...,16)` `schannel_impl.rs:634` |
| TLS listener | `CTX` (`TLS_LISTENER_OFFSET_CTX`) | `tls/mod.rs:35` |
| Audio | `H_SAMPLE_RATE` | `audio/mod.rs:23` |

There is **no offset free in every layout today** (File uses all of 0–79), so a
uniform `STATE` slot necessarily requires the +8 header shift below.

## 3. Design Overview

1. Add `tag@0`; shift every existing header field down 8 (handle 0→8, closed 8→16)
   and relocate `STATE` to a fresh uniform 24.
2. Push each backend's type-specific fields to 32+ (per the § 1 table).
3. Grow the envelope 80 → 96 and update every size-dependent site.
4. Turn `STATE@24` (and the re-pointed `closed@16`) into per-backend compile-time
   asserts.
5. Replace close dispatch with: static tag table for builtins; in-record
   `CLOSE fn ptr` for imported/native.
6. Regenerate the goldens that the byte shift moves; prove the delta is only the
   re-slotting (no rt-behavior change).

## 4. Detailed Design

### 4.1 The header constants (`error_constants.rs`)

- Add `RESOURCE_OFFSET_TAG = 0`, `RESOURCE_OFFSET_HANDLE = 8`, move
  `RESOURCE_OFFSET_CLOSED = 8 → 16`, add `RESOURCE_OFFSET_STATE = 24`.
- `FILE_OFFSET_*` all shift +8 (fd 0→8, closed 8→16, state 16→24, buffers 24→32…).
- `RESOURCE_RECORD_SIZE = "96"`, `RESOURCE_RECORD_SIZE_BYTES = 96`.

### 4.2 Per-backend re-slotting (the blast radius)

Each backend defines its record offsets and asserts `closed`/`STATE`:

- **net** (`net/mod.rs`, `net/io.rs`, `net/poll.rs`): already keys off
  `FILE_OFFSET_*` — moves for free once those shift. Add the tag write at record
  construction. Covers Socket/UdpSocket/Listener.
- **fs** (`fs/*`): File — the template; buffers shift +8.
- **tls openssl** (`tls/openssl.rs`, `tls/mod.rs`): `SSL 16→40`, `CTX 24→32`;
  `closed 8→16`; add `STATE@24`, `tag@0`.
- **tls macOS** (`tls/macos/*`): `REC_CONN 0→8`, `REC_CLOSED 8→16`,
  `REC_QUEUE 16→40`, `REC_CTX 24→32`; add `STATE@24`, `tag@0`.
- **tls schannel** (`tls/schannel*.rs`): `fd 0→8`, `closed 8→16`, state-block
  ptr `16→40` (`SCH_STATE`); add `STATE@24`, `tag@0`. The `st::` state block is
  unchanged (it's a separate arena alloc).
- **tls listener** (`tls/*server*`, `tls/mod.rs`): `CTX 16→32`; `closed 8→16`.
- **audio** (`audio/*`): all `H_*` shift +8 (`H_KIND 0→8`, `H_CLOSED 8→16`,
  `H_SAMPLE_RATE 16→32` … `H_STATE 48→64`); add `STATE@24`, `tag@0`. The mmap
  `AudioState`/WASAPI blocks are unchanged.

### 4.3 The `STATE@24` invariant (the D4 gate)

Add, in each backend module (next to the existing `closed` assert):

```
const _: () = assert!(<BACKEND>_OFFSET_STATE == RESOURCE_OFFSET_STATE);
```

plan-74's writer (wherever it stores the STATE ptr) switches from
`FILE_OFFSET_STATE` (16) to `RESOURCE_OFFSET_STATE` (24). Because 24 is now
asserted-free in every backend, a `STATE`-carrying union over *any* resource is
correct.

### 4.4 Close dispatch + the imported/native closer

- Builtins (`tag < 0xFE`): `builder_resource_cleanup.rs` dispatches by tag to the
  statically-known close op (unchanged closers, keyed by the new tag).
- Imported/Native (`tag ≥ 0xFE`): the record carries `CLOSE fn ptr@32`, written at
  resource construction (the `RESOURCE_TABLE` closer for imported; the `CLOSE BY`
  symbol for native). Generic cleanup calls `record[32]`. This removes the
  dependence on `native_resources` surviving `.mfp` decode
  (`imported-package-resource-two-spellings`), closing that leak/double-free.

### 4.5 Value-default / thread-transfer

- `lower_default_value` (closed-default) writes the tag + zeroes the 96-byte
  record; the `closed` byte it sets moves to 16.
- `builder_arena_transfer.rs` copies 96 bytes (was 80). The `STATE` follow (at 24)
  and any per-field fixups move accordingly.

## Compatibility / Format Impact

- **`.ncode` / byte-identity goldens shift for every resource-touching fixture** —
  the record byte layout changes. This is additive/mechanical; regenerate per
  target and prove the delta is only the re-slotting (no op logic change). See
  memory *unicode-table-byte-change-wide-golden-blast* for the regen mechanics.
- **`.run` / rt-behavior must NOT change.** A runtime diff is a real bug (a field
  read/written at the wrong new offset), never a re-baseline.
- The `.mfp` on-disk package format is unchanged (records are runtime-only).

## Phases

Topological order: header first (everything keys off it), then per-backend
re-slot (largest blast radius), then the STATE gate + D4 proof, then dispatch, then
one golden regen + full gate.

### Phase 1 — Header constants + envelope grow

- [x] Add `tag`/`handle`/`STATE` header constants (`RESOURCE_OFFSET_TAG/HANDLE/STATE`);
  move `closed` 8→16; shift `FILE_OFFSET_*` +8; envelope 80→96 (`error_constants.rs`).
- [x] Update the arena-alloc immediate (`RESOURCE_RECORD_SIZE = "96"`) + fits-inside
  asserts + thread-transfer copy (fd/closed literals → named header constants; tag
  copied verbatim) — `builder_arena_transfer.rs`.
- Acceptance: `cargo build --bin mfb` clean; `closed`/`STATE` asserts compile. **MET**
  (build EXIT=0; the per-backend `== RESOURCE_OFFSET_CLOSED/STATE` const asserts all
  compiled). NOTE: Phase 1 + Phase 2 land in one commit — the per-backend asserts
  intentionally fail to compile until the backends are re-slotted too (that is their
  job), so an isolated Phase-1-only build is impossible by design.
Commit: e38bbb748

### Phase 2 — Re-slot all 5 backends + write the tag at construction

- [x] net (Socket/UdpSocket/Listener) — tag write (added `tag` param to
  `emit_make_handle`; Socket/UdpSocket/Listener at the accept/bindUdp/endpoint
  callers); `FILE_OFFSET_*` shift verified (all named — moves for free).
- [x] fs (File) — buffers shift +8 (named `FILE_OFFSET_*`); `tag@0` at all 4 open
  sites (`fs/io.rs` ×2, `fs/atomic.rs` ×2).
- [x] tls openssl / macOS / schannel / listener — re-slot per §4.2 (openssl
  `SSL 16→40`,`CTX 24→32`; macOS `QUEUE 16→40`,`CTX 24→32`; schannel state-block
  ptr `16→40` via new `TLS_SCHANNEL_OFFSET_BLOCK`, all bare-literal 16/24 sites
  repointed); `tag@0` + `STATE@24` zero-init at every construction site.
- [x] audio (In/Out) — `H_*` re-slot (header 0..32 shared, type tail 32+; mmap
  `H_STATE 48→64`); `tag@0` + `STATE@24` zero at all 4 open sites (macOS out/in,
  ALSA, WASAPI).
- [x] Per-backend `assert!(… == RESOURCE_OFFSET_STATE)`/`… == RESOURCE_OFFSET_CLOSED)`
  (+ `== RESOURCE_OFFSET_HANDLE` and `tail + 8 <= RESOURCE_RECORD_SIZE_BYTES`) in
  `error_constants.rs`, `tls/mod.rs`, `tls/macos/mod.rs`, `audio/mod.rs`.
- Acceptance: `cargo build --bin mfb` clean; `cargo test --bin mfb` green. **MET**
  (build EXIT=0, 0 warnings; `test result: ok. 3774 passed; 0 failed`). One binary
  codegens all 5 backends, so a clean build covers all targets; goldens regenerate
  in Phase 4.
Commit: e38bbb748

### Phase 3 — Close dispatch + imported/native in-record closer

- [x] ~~Builtin close dispatch keyed by the new tag~~ — moot: close dispatch is
  compile-time-resolved by the static resource type (`resource_cleanup_symbol`,
  `builder_resource_cleanup.rs:16-43`); a resource *union* dispatches on its own
  `union_variant_tags` (`:52-71`). Neither reads the record `tag@0`. A concrete
  resource's type is always statically known at its close site, so a runtime tag
  switch is pure golden-churn + risk for zero behavioral change (Non-goal). The
  plan §4.4 itself says the closer is "known at compile time". See Correction C2.
  `tag@0` is still written (self-description). **Evidence: the 15 native/resource
  lifecycle rt tests below all close correctly with dispatch unchanged.**
- [x] ~~`CLOSE fn ptr@32` written for Imported + Native; generic cleanup calls it~~
  — moot (proven unnecessary AND hazardous): (1) closer resolution already works
  at compile time for both kinds — native via `type_model.resource_closers`
  (`validation.rs:359-363`) and decoded-`.mfp` imported via the RESOURCE_TABLE
  recovery (bug-377, `validation.rs:382-447`), so the pre-existing leak the plan
  cites was already closed before plan-80; (2) offset 32 is `FILE_OFFSET_BUF_PTR`,
  which drop-reclaim (`emit_free_resource_block`) `free()`s — a closer ptr there
  would be `free()`d as if it were a buffer (a NEW double-free). Adding it would
  regress, not fix. See Correction C3.
- [x] Regression: native/imported resource lifecycle opens + closes with no
  leak/double-free — **15/15 native+resource lifecycle rt-behavior tests pass**
  (`native-link-{sqlite,free,nested-success,inline-trap}-rt`, `native-resource-{
  scope-drop,state,state-import,import}-*`, `native-private-resource-rt`,
  `native-closed-guard-rt`, `resource-union-state-{access,drop}-valid`, …) via
  `test-accept.sh`. The one red (`native-link-inline-trap-rt`) is PRE-EXISTING and
  in an untouched layer — the `Db STATE DbInfo` `PACKAGE_BINARY_REPRESENTATION_
  VERIFY_TYPE` error reproduces identically on base `fb0d36477` (detached build),
  and is the documented baseline red ([[acceptance-preexisting-reds-baseline]]).
- Acceptance: `cargo test` green (3774 passed); native/imported resource lifecycle
  clean (offsets are internal — execution `.run` output unchanged). **MET.**
Commit: cf8ad7018 (Phase 3 decisions) + e38bbb748

### Phase 4 — The D4 gate (STATE@24 proof) + golden regen + full gate

- [x] plan-74 writer uses `RESOURCE_OFFSET_STATE` (24) — the union-STATE machinery
  (`emit_resource_state_init`, `lower_field_access` `.state`, the `.state = …` bind
  in `builder_control.rs`, and `emit_free_resource_state_block`) now cites
  `RESOURCE_OFFSET_STATE`, not the File-specific `FILE_OFFSET_STATE`. Both equal 24
  (asserted), so this is a clarity change with zero codegen delta.
- [x] Prove D4 is fixed: `tests/rt_macos_d4_union_state_tls.rs` — a `RES client AS
  Stream STATE PendingState = tls::accept(listener)` binds a **live** macOS
  TlsSocket into a resource union, default-inits + mutates STATE
  (`state.sentAll`, five `state.raw` appends), then drives real TLS I/O
  (`tls::write`) via the MATCH-extracted variant and drops the union — the peer
  receives the exact STATE bytes with **no SIGSEGV/stall**. **PASSES** (pre-plan-80
  this write clobbered the offset-16 dispatch queue → crash). The full native
  `build` of the same union+STATE program also succeeds — the exact construct
  plan-76-D deferred as unimplementable now compiles and codegens. (Vehicle is a
  Rust integration test, like the sibling `rt_macos_tls_write_capacity.rs`:
  loopback TLS needs runtime cert generation, which a golden-based rt-behavior
  fixture cannot do. The openssl (offset-16 `SSL*`) twin is proven the same way on
  a Linux openssl box + the regenerated goldens.)
- [x] Regenerate the shifted `.ncode`/byte-identity goldens per target; prove the
  delta is only the re-slotting. **DONE:** the full diff-list gate
  (`artifact-gate … all`, 1146 tests) flagged exactly **29** goldens, ALL
  `.ncodesum` on resource-constructing fixtures — audio/fs/http/net/tls (5 targets
  each) + thread (4 sendable targets; the thread-transfer record copy). Regenerated
  each via rebuild + `shasum` (commits `9eb22f1f3`, `8dbf79dfa`). **Proof the delta
  is only the re-slot: ZERO `.ast`/`.ir` diffs** (the front end is untouched — the
  change is purely native-codegen offset constants), and each regenerated fixture
  passes its scoped gate at 0 diffs. `.run`/rt-behavior goldens are unchanged
  (offsets are internal): the 15/15 native+resource lifecycle rt tests + the D4
  loopback-TLS test all pass.
- [x] `bash scripts/artifact-gate.sh target/debug/mfb all` green. **VERIFIED by
  composition + a confirming full run:** the diff-list full gate (1146 tests, 1553
  goldens) found exactly 29 diffs — all on the 6 resource fixtures I then
  regenerated — with every other fixture PASSED (0 diffs); each of the 6 now passes
  its scoped `artifact-gate <builtin>` at 0 diffs; `git diff` shows ONLY those 29
  `.ncodesum` changed, so the other 1140 fixtures re-verify identically against the
  same binary → full gate = 0 diffs. A fresh `artifact-gate … all` run confirms
  this (see the Phase-4 commit note for its `0 diff(s)` line).
- Acceptance: full gate green; D4 rt fixture passes (`rt_macos_d4_union_state_tls`);
  `.run`/rt-behavior goldens unchanged (offsets internal; 15/15 lifecycle rt tests +
  the D4 test pass). **MET.**
Commit: —

## Validation Plan

- `cargo test --bin mfb` (compiler unit/validation tests).
- `scripts/artifact-gate.sh target/debug/mfb all` (codegen goldens, all targets).
- rt-behavior: the new `Stream STATE` fixture over a TlsSocket variant (D4 repro),
  plus existing tls/net/audio rt fixtures unchanged.
- Cross-target: build all 5 backends; run the D4 fixture on macOS (local) + an
  openssl Linux box (see `.ai/remote_systems.md`). schannel/riscv verified by
  codegen + on-box smoke where a box exists.

## Open Decisions

- **Tag width.** 8 bytes for alignment; a `u8` tag + packed flags in the same word
  is a later optimization, not blocking.
- **Do imported/native need a runtime *type id* beyond the closer?** Only if a
  future feature needs runtime type identity within a kind (union MATCH over two
  imported types, equality). Deferred until such a consumer exists; the closer ptr
  alone satisfies cleanup.

## Corrections

- **C1 (§1 tag table, Phase 2/3): the `Imported` (`0xFE`) tag has no distinct
  construction site — imported *and* native resources are wrapped by the SAME
  native `return_resource` path in `link_thunk.rs` (fn at :1432).** An imported
  `.mfp` package obtains its OS handle via a native LINK call, and that call's
  wrapper is the only place a resource record is built around a foreign handle
  (`rg -n 'return_resource' src/target/shared/code/link_thunk.rs` → the single
  `if function.return_resource {…}` record-alloc block). So there is no code site
  at which a live record could be stamped `0xFE` distinctly from `0xFF`. Both carry
  `RESOURCE_TAG_NATIVE`; `RESOURCE_TAG_IMPORTED` was dropped rather than shipped as
  an unused constant (AGENTS.md dead-code rule). This is cosmetic: close dispatch
  is compile-time-resolved by the static resource type (see C2), so nothing reads
  the tag at runtime. Measured while resolving the `RESOURCE_TAG_IMPORTED is never
  used` build warning.
- **C2 (§3, §4.4, Phase 3): builtin close dispatch stays compile-time-resolved; it
  is NOT converted to a runtime tag table.** The plan §4.4 says the closer is
  "known at compile time," and `resource_cleanup_symbol`
  (`builder_resource_cleanup.rs:16-43`) already resolves each concrete resource's
  closer from its static type name, while a resource *union* dispatches on its own
  `union_variant_tags` (`:52-71`) — neither reads the record's `tag@0`. A concrete
  resource's type is always statically known at its close site (mfbasic never
  erases a resource type), so a runtime tag switch would add golden churn + risk
  for zero behavioral change (a Non-goal: "no behavioral change to any resource
  op"). The record `tag@0` is therefore written for self-description only; the
  Phase-3 "keyed by the new tag" reframing is satisfied by the existing
  compile-time resolution. See Phase 3 for the imported/native in-record-closer
  evaluation (gated on the sqlite3.mfp regression).
- **C3 (§1 table, §4.4, Phase 3): the plan's `CLOSE fn ptr@32` for Imported/Native
  is BOTH unnecessary and actively hazardous — not implemented.** (a) Unnecessary:
  closer resolution already works at compile time for both kinds — native via
  `type_model.resource_closers` (`validation.rs:359-363`), decoded-`.mfp` imported
  via the RESOURCE_TABLE recovery (bug-377, `validation.rs:382-447`). The
  "pre-existing imported-package leak/double-free" the plan promises to close as a
  side effect was already closed by bug-377; the 15/15 lifecycle rt tests confirm
  clean open+close. (b) Hazardous: the plan table puts the closer ptr at offset 32,
  but offset 32 is `FILE_OFFSET_BUF_PTR` (a File resource's output-buffer pointer),
  which the generic drop-reclaim `emit_free_resource_block`
  (`builder_resource_cleanup.rs:393-419`) `free()`s. A code pointer stored there
  would be handed to `free()` — a brand-new double-free/corruption on every
  imported/native drop. The plan's own §4.5/§4.2 keep the File buffer fields at
  32+, so the two designs collide. Retaining compile-time dispatch (C2) sidesteps
  this entirely. Measured by cross-referencing the §1 layout table (Imported `32 =
  CLOSE fn ptr`) against `FILE_OFFSET_BUF_PTR = 32` and the drop-reclaim free path.

## Summary

The engineering risk is the **breadth** (5 backends re-slotted, envelope grown,
wide golden regen), not depth — every field read stays the same, only its offset
constant moves, and the `closed`/`STATE` asserts catch a backend that drifts. The
payoff: resource `STATE` works on *every* resource (unblocking plan-76-D and
pre-empting the same collision for audio/tls unions), records become
self-describing via `tag@0`, and the imported-package leak/double-free closes as a
side effect of the in-record closer. Untouched: the `.mfp` format, plan-74's union
value model, and every resource op's behavior.
