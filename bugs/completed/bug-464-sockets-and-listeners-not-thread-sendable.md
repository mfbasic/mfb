# bug-464: three of the five socket/listener resources are not thread-sendable, and the resource-transfer copy cannot carry them

Last updated: 2026-08-31
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Footgun (feature gap guarding a truncating transfer copy)

Status: FIXED
Regression Test: `tests/rt-behavior/threads/thread-transfer-tls-socket-rt/` (new), `tests/rt-behavior/threads/thread-transfer-tcp-listener-rt/` (new), `tests/syntax/threads/thread-plane-tls-socket-sendable/` (new)

Every socket and listener resource should be movable to another thread, so that a
server can accept on one thread and hand each connection to a worker. Today only
two of five are: `tcp::Socket` and `udp::Socket` are `sendable: true`, while
`tcp::Listener`, `tls::Socket` and `tls::Listener` are `sendable: false`. Declaring
the thread shape is refused outright:

```
error[2-203-0063 TYPE_THREAD_NOT_SENDABLE]: thread boundary type is not sendable
               Thread resource type requires a thread-sendable type, got `tls.Socket`.
```

**The single correct behavior a fix produces:** `Thread OF RES tls::Socket TO T`,
`Thread OF RES tls::Listener TO T` and `Thread OF RES tcp::Listener TO T` all
compile, and `thread::transfer` moves the handle such that the receiving thread
can perform every operation the sending thread could (`tls::read`/`tls::write` on
a transferred socket, `tls::accept`/`tcp::accept` on a transferred listener), with
the resource closed exactly once, by the receiver.

**This is not a one-line flag flip.** The `sendable` bit is not merely a policy
label — it is load-bearing for a copy routine that only knows how to move a
4-slot resource header. Flipping the bit without generalizing that copy produces
a receiver-side TLS socket whose `SSL*` slot is **NULL**, so the first
`tls::read`/`tls::write` on the receiving thread cannot work. (This paragraph
originally said the slot held *uninitialized* bytes and that the naive fix was
memory-unsafe; see the Correction under Root Cause — the copy zeroes that region
rather than skipping it, so the naive fix is broken rather than unsafe. The phase
ordering is unchanged.) An implementer who reads only the registry comments ("not
thread-sendable in v1") will conclude the bit is a leftover scope decision and
flip it; it is that *and* a guard.

References:

- `src/docs/spec/language/15_resource-management.md:24` — "A concrete resource handle may be sent to a thread only when that resource type is thread-sendable."
- `src/docs/spec/language/17_native-libraries.md:43` — sendability is an explicit per-resource opt-in (`THREAD_SENDABLE`), not a default.
- `src/docs/spec/memory/04_arenas.md:140-150` — `arena_alloc` contract; the block-grow path calls `arena_fill_random` to poison freshly mapped memory.
- `planning/completed/plan-03-net.md:280-283` — the original deferral: "**`sendable = false` for v1** — sending a TLS session across threads adds bridge/state-ownership complexity with no spec requirement … do not opt in here yet."
- `planning/completed/plan-06-tls-server.md:61-63` — the same deferral for the server handles: "`TlsListener` and the server-accepted `TlsSocket` are **not** thread-sendable in V1".
- bug-463 (`bugs/bug-463-thread-plane-res-collection-parse.md`) — adjacent: a `RES` collection on a thread plane fails to parse before this rule is reached. Independent, but both land in the thread-plane sendability path.
- Memory: `arena-state-is-per-thread` — a spawned thread sees its own zeroed `x19`; no thread may free another's block. Governs the close-obligation half of this fix.
- Found during: the 2026-08-30 review of the `websockets` section of `planning/todo.md`, which this bug's resolution would materially change (a threaded `wss://` server is currently impossible).

## Failing Reproduction

Minimal project; single file. `project.json` is the standard executable shape.

```
$ cat src/main.mfb
IMPORT tls
IMPORT tcp

FUNC useTlsSocket(t AS Thread OF RES tls::Socket TO Integer) AS Integer
  RETURN 0
END FUNC

FUNC useTlsListener(t AS Thread OF RES tls::Listener TO Integer) AS Integer
  RETURN 0
END FUNC

FUNC useTcpListener(t AS Thread OF RES tcp::Listener TO Integer) AS Integer
  RETURN 0
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC

$ mfb build
```

- Observed: all three are refused in one build (verbatim, `target/release/mfb build`, macos-aarch64):

  ```
  ./src/main.mfb:4 error[2-203-0063 TYPE_THREAD_NOT_SENDABLE]: thread boundary type is not sendable
                 Thread resource type requires a thread-sendable type, got `tls.Socket`.
  ./src/main.mfb:8 error[2-203-0063 TYPE_THREAD_NOT_SENDABLE]: thread boundary type is not sendable
                 Thread resource type requires a thread-sendable type, got `tls.Listener`.
  ./src/main.mfb:12 error[2-203-0063 TYPE_THREAD_NOT_SENDABLE]: thread boundary type is not sendable
                 Thread resource type requires a thread-sendable type, got `tcp.Listener`.
  ```

  The diagnostic fires on each **type declaration**, before any `thread::transfer` call is reached — note there is no `thread::transfer` anywhere in the reproduction.
- Expected: builds clean, as the `tcp::Socket` form already does.

Contrast case that works today (same file shape, `tcp::Socket` substituted) —
this is the regression guard that must keep passing:

```
$ cat src/main.mfb
IMPORT tcp
FUNC useTcp(t AS Thread OF RES tcp::Socket TO Integer) AS Integer
  RETURN 0
END FUNC
FUNC main AS Integer
  RETURN 0
END FUNC

$ mfb build
Building ws_send_probe (executable) for macos-aarch64
Wrote executable to ./build/ws_send_probe.out
```

Both probes were run on macos-aarch64 with `target/release/mfb`. The check is in
the target-independent IR verifier (`src/ir/verify/resources.rs`), so the
rejection is platform-independent; the *runtime* half of the fix is not (see the
matrix in Fix Design).

## Root Cause

Two layers, and only the first is obvious.

**Layer 1 — the registry bit.** `is_resource_sendable`
(`src/ir/verify/resources.rs:310`) consults the project's own
`RESOURCE … THREAD_SENDABLE` opt-in, else the builtin registry's `sendable` field
via `is_builtin_sendable_resource_type` (`src/codegen/resource/mod.rs:107`).
`require_thread_sendable("Thread resource type", …)`
(`src/ir/verify/resources.rs:544`) runs it over the `Thread OF … RES T TO …`
plane. `tls::Socket`/`tls::Listener` are `sendable: false`
(`src/codegen/builtins/tls/mod.rs:162,174`) and `tcp::Listener` is `sendable: false`
(`src/codegen/builtins/tcp/mod.rs:180`). Per plan-03-net.md §4.4 and
plan-06-tls-server.md §1 these were **v1 scope deferrals**, never revisited.

**Layer 2 — the copy the bit is really gating.** `copy_resource_to_current_arena`
(`src/codegen/memory/arena/builder_arena_transfer.rs:506`) is the routine that
materializes a transferred handle in the receiver's arena. Its own doc comment
states the assumption: *"The handle is a two-word struct (a host resource word
such as a file descriptor, followed by a closed flag)."* It allocates
`RESOURCE_RECORD_SIZE` = **96 bytes**
(`src/codegen/error/constants/error_constants.rs:743`) and then copies exactly
four slots:

| slot | offset | copied? |
| --- | --- | --- |
| tag | 0 | yes, verbatim |
| handle / fd | 8 | yes, verbatim |
| closed flag | 16 | yes, verbatim |
| STATE | 24 | yes — deep-copied into the receiver arena (bug-257) |
| **everything from 32 to 87** | 32–87 | **overwritten with ZERO** (corrected — see below) |
| headroom | 88–95 | never written |

> **Correction (Phase 1, 2026-08-31).** The two rows above are the corrected
> ones; this document originally claimed 32–95 was "never written" and that a
> naive bit-flip therefore yielded *uninitialized/poisoned* arena bytes and a
> wild `SSL*` dereference inside libssl. **That is wrong.** The routine ends with
> eleven *unconditional* stores covering every 8-byte slot from 0 to 80:
> `FILE_OFFSET_BUF_PTR`@32, `BUF_FILLED`@40, `BUF_ENABLED`@48 and
> `FILE_OFFSET_READ_PTR`@56 / `READ_POS`@64 / `READ_FILL`@72 / `READ_AT_EOF`@80
> are each stored `ZERO` (`builder_arena_transfer.rs:624-660`, and the
> `RESOURCE_RECORD_SIZE_BYTES`/`FILE_OFFSET_*` constants at
> `error_constants.rs:772-824`). Those stores exist to reset `fs::File`'s
> write buffer and read cache on a move, but they are emitted for *every*
> resource — `copy_resource_to_current_arena` is type-agnostic, and its only two
> callers (`builder_arena_transfer.rs:395` and `:1290`) reach it for all
> resource kinds.
>
> So the real defect is **truncation by zeroing, not by omission**: a naively
> opted-in `tls::Socket` reaches its receiver with `SSL_CTX*` = `SSL*` = **NULL**.
> The consequence is a null dereference or a guard rejection on the first
> `tls::read`, not an arbitrary wild pointer.
>
> **This does not weaken the phase ordering, and the Non-goal below still
> stands** — a naive flip still ships a `tls::Socket` that cannot work on its
> receiver, which is a correctness bug either way. It does downgrade the *stated*
> severity of the tempting wrong fix from "memory-unsafe" to "broken". The fix is
> unchanged: the copy must carry each resource's declared live slots.

The `sendable: true` resources are exactly the ones that fit that shape.
`tcp::Socket` uses only `FILE_OFFSET_FD`/`FILE_OFFSET_CLOSED` in its record
(`src/codegen/builtins/tcp/gen_io.rs` — its other `*_OFFSET` constants are stack-frame
slots, not record slots), and its read/write deadlines live in the **kernel** via
`SO_RCVTIMEO`/`SO_SNDTIMEO` setsockopt, so they ride the fd across the move for free.

The `sendable: false` resources are exactly the ones that do not:

- `tls::Socket` — `TLS_OFFSET_CTX = 32` (`SSL_CTX*`), `TLS_OFFSET_SSL = 40` (`SSL*`); on Windows `TLS_SCHANNEL_OFFSET_BLOCK = 40` (`src/codegen/builtins/tls/gen_shared.rs:38,39,47`).
- `tls::Listener` — `TLS_LISTENER_OFFSET_CTX = 32` (`gen_shared.rs:54`).
- `audio` resources — live slots at 32/40/48 (`gen_alsa_shared.rs:142,144` (offsets 32/48), `gen_macos_shared.rs:40,42,44` and `:128,130,132`), and also `sendable: false`.

So the bit is currently doing double duty: it records a v1 product decision *and*
it keeps a resource whose record extends past offset 24 away from a copy routine
that would silently truncate it. **Flipping the bit alone is a memory-safety
regression**, not a partial fix: the receiver's record is fresh arena memory,
which is not guaranteed zero — the block-grow path deliberately poisons it with
`arena_fill_random` (`src/docs/spec/memory/04_arenas.md:150`) — so
`TLS_OFFSET_SSL` would hold an arbitrary non-null word that the receiver's first
`tls::read` passes to `SSL_read`. That is a wild pointer dereference inside
libssl, not a clean null crash.

`tcp::Listener` is the one case where layer 2 does *not* apply — its record is
plain fd + closed, identical in shape to `tcp::Socket`. Its `sendable: false` is
pure policy ("a listener accepts on its owning thread",
`src/codegen/builtins/tcp/mod.rs:178`) and is genuinely a one-line change plus tests.

## Phase 1 audit results (2026-08-31)

Reproduction re-run verbatim on macos-aarch64 with `target/release/mfb build`:
all three declarations refused with `2-203-0063 TYPE_THREAD_NOT_SENDABLE`, on the
type declaration, exactly as filed. `tcp::Socket`, `udp::Socket` and `fs::File`
in the same file produce no error.

### Live-slot table (the header 0–24 is carried by the existing copy on every row)

| resource | backend | live slots past the header | transfer mode |
| --- | --- | --- | --- |
| `tcp::Listener` | all | *none* — fd@8 + closed@16 only | n/a |
| `tcp::Socket` | all | *none* (deadlines are kernel-side via `SO_RCVTIMEO`/`SO_SNDTIMEO`) | n/a — already sendable |
| `udp::Socket` | all | *none* | n/a — already sendable |
| `fs::File` | all | 32/40/48 write buffer, 56/64/72/80 read cache | deliberately **zeroed** (a cache; unchanged) |
| `tls::Socket` | OpenSSL | 32 `SSL_CTX*`, 40 `SSL*` (`gen_shared.rs:38,39`) | **verbatim** — malloc heap, not arena |
| `tls::Socket` | Schannel | 40 SSPI block ptr (`gen_shared.rs:47`) | **deep-copy** — arena, `st::SIZE` = 320 + 2×0x4400 = **35136** B (`gen_schannel.rs:88`) |
| `tls::Socket` | Network.framework | 32 ctx, 40 dispatch queue, 48 local-host C string (`gen_macos/mod.rs:102,103,112`) | 32/40 **verbatim** (refcounted); 48 is an **arena C string** |
| `tls::Listener` | OpenSSL | 32 `SSL_CTX*` (`gen_shared.rs:54`) | **verbatim** |
| `tls::Listener` | Schannel | 40 WORK block ptr | **deep-copy** — arena, `stl::SIZE` = **288** B (`gen_schannel_server.rs:38`) |
| `tls::Listener` | Network.framework | 32 ctx, 40 queue, 48 lhost (`gen_macos/server.rs:1183-1199`) | as the socket row |

### Verdict 1 — macOS TLS timeout storage: **not in the record**

`lower_tls_set_timeout_macos` (`gen_macos/timeout.rs:52-55`) stores the deadline
on the per-connection **ctx**, at `CTX_RTO`=104 / `CTX_WTO`=112
(`gen_macos/mod.rs:157,158`) — reached through `REC_CTX`@32, not held in the
resource record. It therefore rides the ctx pointer for free, exactly as the
Linux/Windows `setsockopt` deadlines ride the fd. **No extra record slot.**

### Verdict 2 — Schannel block at offset 40: **arena-allocated → deep-copy**

Both Schannel blocks are arena blocks, not malloc/LocalAlloc: the listener's is
allocated at `gen_schannel_server.rs:399-405` ("Allocate the persistent WORK
block (zeroed) that the listener record points at") and the socket's likewise.
Per the `arena-state-is-per-thread` rule no thread may free another's block, so a
verbatim pointer move is unsound and the block must be **byte-copied** into the
receiver's arena.

A byte copy is *sufficient* as well as necessary here, because this is a **move**:
the sender is tombstoned `moved|closed` and its cleanup deactivated, so only the
receiver ever calls `DeleteSecurityContext`/`FreeCredentialsHandle` on the
duplicated SSPI handle values — no double-free. The listener block's
`stl::CONTNAME` key-container name is derived from the *original* WORK pointer
but is stored as bytes, so the copy keeps naming the container that actually
exists.

### Newly found, not in the original filing — macOS `REC_LHOST`@48 is an arena C string

`gen_macos/server.rs:1193-1199` stores the bound host as "an arena copy or the
static `_mfb_tls_anyhost`". A verbatim move would leave the receiver's
`tls::localAddress` reading a **NUL-terminated string in the sender's arena**,
freed at the sender thread's teardown — a use-after-free rather than a
double-free, and a third transfer mode (copy a C string of unknown length) on top
of verbatim and fixed-size-block.

## Goal

- `Thread OF RES tcp::Listener TO T` compiles, and a transferred listener accepts on the receiving thread.
- `Thread OF RES tls::Socket TO T` compiles, and a transferred socket completes `tls::read`/`tls::write` on the receiving thread against a real peer, on all of OpenSSL / Schannel / Network.framework.
- `Thread OF RES tls::Listener TO T` compiles, and a transferred listener completes `tls::accept` on the receiving thread.
- `copy_resource_to_current_arena` carries the whole 96-byte record (or a per-resource-declared live-slot set), so no resource can be added later that silently truncates on transfer.
- The close obligation still fires exactly once, on the receiver, for every case above.

### Non-goals (must NOT change)

- **The `sendable` bit must not be flipped ahead of the copy fix.** Phase order is load-bearing; a commit that flips `tls::Socket` to `true` without Phase 2 lands a wild-pointer dereference. This is the tempting wrong fix — do not take it, and do not "temporarily" flip it to see a test go green.
- **No concurrent use of one handle from two threads.** This bug is about *moving* ownership, not sharing. `thread::transfer` moves; the sender's cleanup is deactivated. OpenSSL `SSL*` objects tolerate use from a different thread but not simultaneous use from two, and nothing here should imply otherwise.
- **`ParameterType::Res(_) => false`** (`src/ir/verify/resources.rs:439`) stays — sharing a resource *collection* across threads remains out of scope per spec §15.6. This bug moves single handles only. (bug-463 covers the parse defect on that same path.)
- No change to any resource record layout, `RESOURCE_RECORD_SIZE`, or the tag/fd/closed/STATE offsets. The fix widens what the copy carries; it must not move what is carried today.
- No change to `fs::File`, `tcp::Socket`, `udp::Socket` transfer behavior — they are already correct and their goldens must not drift.
- No mutual TLS, ALPN, SNI, or session resumption — out of scope, unchanged from plan-06.

## Blast Radius

Found by `grep -rn -B6 "sendable: \(true\|false\)" src/codegen/builtins/*/mod.rs` (the
full builtin-resource census) plus reading `copy_resource_to_current_arena`.

- `src/codegen/builtins/tcp/mod.rs:180` (`tcp::Listener`) — **fixed by this bug.** Record is fd+closed only; policy-only restriction.
- `src/codegen/builtins/tls/mod.rs:162` (`tls::Socket`) — **fixed by this bug.** Needs the widened copy (CTX@32, SSL@40 / Schannel BLOCK@40).
- `src/codegen/builtins/tls/mod.rs:174` (`tls::Listener`) — **fixed by this bug.** Needs the widened copy (CTX@32).
- `src/codegen/memory/arena/builder_arena_transfer.rs:506` `copy_resource_to_current_arena` — **fixed by this bug**; the shared mechanism. Its doc comment ("two-word struct") must be corrected too, or the next reader re-derives the same wrong assumption.
- `src/codegen/builtins/audio/mod.rs:397,409` (2 audio resources) — **latent, same hazard, out of scope.** They share the >@24 record shape, so once Phase 2 widens the copy they *become* mechanically transferable, but audio handles carry backend callbacks bound to a device thread and need their own audit. Do not opportunistically flip them here; file separately if wanted.
- `src/codegen/builtins/process/mod.rs:207` (1 process resource) — **latent, out of scope.** Same reasoning; a child-process handle's waitpid semantics are per-thread on some platforms and need their own analysis.
- `src/codegen/builtins/fs/mod.rs:167` (`fs::File`), `tcp/mod.rs:169` (`tcp::Socket`), `udp/mod.rs:167` (`udp::Socket`) — **unaffected.** Already `sendable: true` and already within the 4-slot shape; they are the regression guards that must not drift.
- macOS `tls::setReadTimeout`/`setWriteTimeout` (`gen_shared.rs:492` → `macos::lower_tls_set_timeout_macos`) — **must be audited in Phase 1.** Linux/Windows route to the shared socket-level `setsockopt` helper (`gen_shared.rs:494`), so their deadlines are kernel-side and ride the fd. Network.framework has no fd, so macOS may hold the deadline in the record; if it lives past offset 24 it is another slot the copy must carry.

## Fix Design

**Phase 2's core change is to stop hardcoding the copied slot set.** Two options
considered:

1. *(rejected)* Copy all 96 bytes blindly with a fixed-size block move. Simple, but it would copy the STATE pointer verbatim into the receiver — re-introducing exactly the sender-arena aliasing that bug-257 fixed. Any blind copy must still special-case @24, at which point the "blind" framing is a lie that invites a later regression.
2. *(recommended)* Give `RegistryResource` a declared **live-slot descriptor**: the byte offsets past the canonical header that hold owned/borrowed words, and for each, whether it moves verbatim (a pointer into a foreign heap, e.g. `SSL*`, `SSL_CTX*`) or needs deep-copy (an arena pointer, like STATE). `copy_resource_to_current_arena` then copies header + STATE (unchanged) + each declared slot. This makes the truncation hazard structurally impossible for future resources instead of relying on a matching `sendable` bit, and it turns "is this resource sendable" back into an honest product decision.

The correctness risk concentrates in the **backend session state**, not the copy:

- **OpenSSL (Linux).** `SSL*` and `SSL_CTX*` are malloc-heap objects, not arena objects, so moving the words is sound. `SSL_CTX` is refcounted and shared with the listener; confirm the accepted socket's borrow (per `tls/mod.rs` — closing an accepted socket never frees the shared context) still holds when socket and listener end up on different threads. OpenSSL 1.1.1+ is thread-safe for *distinct* objects; verify no per-thread error-queue assumption leaks (`ERR_get_error` is per-thread — a handshake error raised on one thread and read on another would report nothing).
- **Schannel (Windows).** `TLS_SCHANNEL_OFFSET_BLOCK = 40` points at a heap block. Determine whether it is arena- or malloc-allocated. **If arena-allocated it must be deep-copied**, per the `arena-state-is-per-thread` memory note — no thread may free another thread's block, so a verbatim pointer move would have the receiver free into the sender's arena at close.
- **Network.framework (macOS).** The listener owns a `dispatch_queue_create("mfb.tls")` serial queue (`gen_macos/server.rs:931`) and connections are bound to it via `nw_connection_set_queue` (`server.rs:1543`). Dispatch queues are refcounted and thread-safe, so a move is expected to be sound, but the synchronous bridge uses `dispatch_semaphore_wait(…, DISPATCH_TIME_FOREVER)` (`server.rs:1152`) — confirm a wait entered on the receiving thread is serviced, and that the queue outlives a socket transferred away from its listener's thread.

Expected output shift: `.ncodesum` drift is expected and intended wherever the
copy routine is emitted, plus new fixtures. Per `AGENTS.md` these are drift
sentinels — regenerate, then prove the delta is only ours.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [x] Add `tests/syntax/threads/thread-plane-tls-socket-sendable/` reproducing the three rejected thread shapes (`tls::Socket`, `tls::Listener`, `tcp::Listener`) with the current `TYPE_THREAD_NOT_SENDABLE` golden. This is the RED test: its golden inverts in Phase 2.
- [x] Add `tests/rt-behavior/threads/thread-transfer-tcp-listener-rt/` — transfer a listener, accept on the worker. Fails to build today.
- [x] Add `tests/rt-behavior/threads/thread-transfer-tls-socket-rt/` — transfer a connected TLS socket, read/write on the worker. Fails to build today.
- [x] Audit the macOS TLS timeout storage (Blast Radius, last item); record whether it occupies a slot past offset 24 and write the verdict into this file.
- [x] Determine whether the Schannel block at offset 40 is arena- or malloc-allocated; write the verdict into this file. This decides verbatim-move vs. deep-copy.
- [x] Confirm the full live-slot set for each of the three resources on each of the five targets, and write the table into this file.

Acceptance: the three new tests fail for the documented reason (`2-203-0063`, not an unrelated parse/resolve error); the slot table and both backend verdicts are recorded here.
Commit: e437f5a12 — all three RED-verified with the exact documented error; audit, the two verdicts, one correction and one new finding (macOS `REC_LHOST` is an arena C string) recorded above.

### Phase 2 — generalize the transfer copy, then opt the resources in

- [x] Add the live-slot descriptor to `RegistryResource` and populate it for every builtin resource (empty for the already-sendable ones — that is the assertion that they fit the header shape).
- [x] Rewrite `copy_resource_to_current_arena` (`src/codegen/memory/arena/builder_arena_transfer.rs:506`) to copy header + STATE + each declared slot, honoring verbatim vs. deep-copy per slot. Correct its "two-word struct" doc comment.
- [x] Only then flip `sendable: true` on `tcp::Listener`, `tls::Socket`, `tls::Listener`, replacing the stale "not thread-sendable in v1" comments with the real rationale.
- [x] Verify the close obligation fires exactly once on the receiver for each, including the accepted-socket-borrows-listener-context case.

Acceptance: Phase 1's three tests pass; `fs::File`/`tcp::Socket`/`udp::Socket` transfer behavior is byte-unchanged except for intended copy-routine drift; Non-goals hold.
Commit: 842ef04f1 (`tcp::Listener`, landed separately per the Open Decision), 0ba0a2285 (live-slot descriptor + the generalized copy + the TLS opt-ins), 258387644 (the `tls::Listener` runtime proof). A resource with no declared slots emits no new instructions, so `fs::File`/`tcp::Socket`/`udp::Socket` transfer codegen is unchanged.

### Phase 3 — regenerate, validate every target, sync docs

- [x] Regenerate `.ncodesum` via `regen-ncodesum.sh` and gate to 0 unexplained diffs with `artifact-gate.sh all`; prove the delta is only the copy routine + new fixtures. **1799 goldens, 0 diffs**; the only pre-existing sums that moved were `tests/byte-identity/thread`'s five, and only because its worker `.mfp` was regenerated (see the golden-delta list in STATUS).
- [x] `cargo test --release --no-fail-fast` (full, not filtered) plus the acceptance harness (`test-accept.sh`) — the latter is not in `cargo test`. **80 x `test result: ok`, 0 failed; acceptance 1319 passed**, both on the merged tree.
- [x] Re-run the reproduction and the two rt fixtures on all five targets; TLS especially must be exercised on OpenSSL, Schannel and Network.framework, since the session-state risk is per-backend and a macOS-only pass proves nothing about the other two. **Done for all three TLS backends** — see the per-backend proof matrix in the STATUS block (macOS locally; Linux/OpenSSL on box 2223; Windows/Schannel on box 2230). Cross-target *lowering* for all five targets is additionally pinned by `tests/byte-identity/resource-xfer-slots`.
- [x] Update `mfb man tls` / `mfb man tcp` prose: both currently state the handles are not thread-sendable (`tls/mod.rs` MODULE_DESC, `tcp/mod.rs:11`).
- [x] Update the `websockets` section of `planning/todo.md` — its `wss://` verdict and its "make `tls::Socket` thread-sendable" recommendation both resolve.

Acceptance: full suite green; artifact gate at 0 unexplained diffs; the reproduction builds and runs on every target it previously failed on; docs no longer claim the old restriction.
Commit: abda0fc47 (docs + the canonical-type-id fix), plus the gate/golden regeneration recorded in the STATUS block below.

## Validation Plan

- Regression tests: the syntax fixture (compile-time acceptance of all three thread shapes) and the two rt-behavior fixtures (runtime proof the transferred handle actually works on the receiver).
- Runtime proof: a transferred `tls::Socket` completes a real request/response on the worker thread, and a transferred listener accepts a real connection — black-box rt fixtures are sufficient here because the failure mode (wild `SSL*`) is loud, but per the `register-slot-import-bugs-need-codegen-inspection` memory note, add a `.ncode` inspection over the copy routine to prove all declared slots are emitted.
- Doc sync: `mfb man tls`, `mfb man tcp` (both assert non-sendability today), `planning/todo.md` websockets section, and the `copy_resource_to_current_arena` doc comment.
- Full suite: `cargo test --release --no-fail-fast`, `test-accept.sh`, `artifact-gate.sh all`.

## Open Decisions

- **Live-slot descriptor vs. per-resource copy hook.** Recommended: the declarative descriptor (Fix Design option 2) — a hook per resource re-scatters the knowledge the descriptor centralizes. (§Fix Design)
- **Should `tcp::Listener` land first, separately?** It needs no copy change and is a genuinely small fix. Recommended: yes — land it as its own commit inside Phase 2, so the risky TLS work is not blocking an independently correct one-line improvement. It also gives the new syntax fixture a partial green early.
- **Audio/process resources.** Recommended: leave `sendable: false` and file separately; they gain the *mechanism* from Phase 2 but need their own device-thread/waitpid analysis. (§Blast Radius)

## Fallout: three PRE-EXISTING `.mfp` defects this uncovered

None of these are about threads or sendability. All three were reproduced on
clean `main` (`fc5c8a6db`) with an attribution binary built via
`git archive main | tar -x -C /tmp/base464 && cargo build --release`, and all
three are fixed here because bug-464 cannot land without the first two.

**F1 — no package could export most built-in resources.**

```
$ cat src/lib.mfb
IMPORT udp
EXPORT FUNC useSock(RES s AS udp::Socket) AS Integer
  RETURN 1
END FUNC
$ /tmp/base464/target/release/mfb build .
Building b464_min (package) for macos-aarch64
error: truncated binary representation
```

`udp::Socket` has been `sendable: true` since it was introduced, so this is
independent of this bug. `TypeTable::type_id` (`src/binary_repr/sections.rs`)
names only `fs.File`, `tcp.Socket` and `tcp.Listener`; every other built-in
resource fell through to the opaque fallback, which wrote a kind-1 (record) entry
with an **empty payload**. A record payload must begin with a `u32` field count,
so the first `checked_u32_at` on read-back overran. bug-390 and bug-436 each hit
this same failure and each fixed only their own case by routing around the arm.
Fixed at the arm: a kind-1 entry gets a zero-field-record payload, exactly how
`add_native` already encodes an opaque LINK resource. Affected `udp::Socket`,
both `tls` handles, `process::Process`, the audio handles and `canvas::Image`.

**F2 — the `RESOURCE_TABLE` had the same three-name allowlist.** Those resources
also got no table row. Added `add_standard_other` plus a
`BUILTIN_RESOURCE_CLOSE_BY_TYPE` id that resolves the close op from the registry
by the row's own type name, rather than needing one sentinel per resource. The
two legacy sentinels are still written and still decoded, so old `.mfp` files
keep loading.

**F3 — the three legacy rows pointed at a phantom type entry, with a wrong
sendable bit.** `add_standard_file/socket/listener` passed the BARE constants
(`fs::FILE_TYPE` = `"File"`) to both `type_id` and `standard_resource_flags`.
Both need the qualified id: `type_id` matches `is_named("fs.File")`, so `"File"`
missed every arm and minted a **phantom type-table entry** instead of returning
the canonical `TYPE_FILE_HANDLE`; and `resolve_type` splits on `'.'`, so a bare
name resolves to `None` and the `SENDABLE` bit was silently clear in all three
rows — including `fs::File`, which has always been sendable. The unit test passed
the qualified id, so it never saw either.

This one had teeth. `src/ir/verify/mod.rs:145-151` seeds `env.resource_sendable`
from an imported package's `RESOURCE_TABLE`, and `is_resource_sendable` consults
that map **before** the built-in registry — so a row keyed by a phantom `"File"`
carrying `sendable: false` was one name-match away from making `fs::File`
non-sendable in every importing project. It stayed inert only because the phantom
key never matched the `fs.File` spelling the check looks up.

Fixed by passing `*_TYPE_ID` at both call sites; `reader.rs:892` already decodes
`TYPE_FILE_HANDLE` back to `"fs.File"`, so the decoded name is unchanged in
meaning and now genuinely matches that key, with the correct bit.

**Golden impact.** F1 and F3 regenerate ~45 committed `.mfp` files. The delta is
the phantom entry disappearing — on `thread_res_sink.mfp`, 2041 → 2013 bytes with
the only string-pool change being the removal of `File`. Verified by attribution:
`scripts/sync-package-mfp.sh` updates **1** file on clean main and **45** here.

## STATUS: FIXED (2026-08-31)

Worked in `.claude/worktrees/464` on `worktree-B-464`, forked from `fc5c8a6db`
and merged up to `ab66ed781` (bug-463) before landing.

**All three thread shapes compile, and all three are proven at RUNTIME on the
receiving thread — not merely at compile time:**

| Goal | Proof | Result |
| --- | --- | --- |
| `Thread OF RES tcp::Listener` accepts on the receiver | `tests/rt-behavior/threads/thread-transfer-tcp-listener-rt` — bind + read the port on main, transfer, dial it; the worker accepts the real connection | `reply=pong:ping` / `worker=1` |
| `Thread OF RES tls::Socket` reads/writes on the receiver | `tests/rt-behavior/threads/thread-transfer-tls-socket-rt` — handshake on main, whole exchange on the worker over the same session, against 8.8.8.8:443 | `moved=TRUE` |
| `Thread OF RES tls::Listener` accepts on the receiver | `tests/rt_tls_listener_thread_transfer.rs` — bind + load the identity on main, transfer, `openssl s_client` completes a real handshake against the worker | `test result: ok` |

The listener test was **RED-checked against the mechanism, not the flag**: with
`tls::Listener`'s `live_slots` temporarily emptied and everything else identical
it fails (`s_client saw ""`), so it gates the carried server context rather than
the registry bit. A test that only gated the bit would have passed either way.

**Gates (post-merge, all uncontended):**

```
cargo test --release --no-fail-fast   80 x `test result: ok`, 0 failed, exit=0
scripts/test-accept.sh                acceptance tests passed (1319 tests ran)
scripts/artifact-gate.sh all          1303 tests, 1465 builds, 1799 goldens, 0 diffs
scripts/sync-package-mfp.sh           updated 0 (no drift after the merge)
```

Reproduction re-run verbatim: `mfb build` on the doc's three-declaration project
now writes an executable, where it previously emitted three `2-203-0063`.

**Golden delta, fully accounted for:**

* 5 `.ncodesum` (`tests/byte-identity/thread`, every target) — its
  `thread_cover_worker.mfp` was regenerated. **No other fixture's sums moved**,
  which is the evidence that the widened copy is inert for a resource with no
  declared slots.
* ~45 `.mfp` — the phantom type entry removed (F3 below);
  `thread_res_sink.mfp` 2041 → 2013 bytes, sole string-pool change `File`.
  Attribution: `sync-package-mfp.sh` updates **1** file on clean main, 45 here.
* `package_cleanup_audit.info` — `types: 1` → `types: 0` (that package declares
  no types; the 1 was the phantom). `resources: 1` unchanged.
* 5 new `.ncodesum` for the new cross-target fixture.

**Deviations from the plan, and one thing it got wrong:**

1. **The doc's Layer-2 mechanism was wrong.** It claimed offsets 32–95 are never
   written, so a naive flip yields poisoned bytes and a wild `SSL*` dereference.
   The copy **zeroes** 32–80 unconditionally, so the naive flip yields a NULL
   session: broken, not memory-unsafe. Corrected in Root Cause. Phase ordering
   and the Non-goal stand.
2. **`SlotBackend` has no "every backend" variant.** The doc's flat live-slot
   descriptor could not express that `tls::Socket`+40 is a foreign-heap `SSL*` on
   OpenSSL but an ARENA block on Schannel — same offset, different ownership. The
   descriptor is backend-tagged instead.
3. **A bug I introduced and caught:** the first draft declared macOS
   `REC_LHOST`@48 live on `tls::Socket`. It is listener-only, so word 48 on a
   socket is uninitialised and the copy ran `strlen` over it — SIGSEGV on the
   first transfer. Rule now written at the site: only declare a slot the
   resource's own constructors write.
4. **Three PRE-EXISTING `.mfp` defects** had to be fixed to land this at all —
   see the Fallout section. F1 blocked bug-464 outright.
5. **Not done, deliberately:** `audio`/`process`/`canvas` stay `sendable: false`
   with `live_slots: &[]` and a comment saying that means *unaudited*, per the
   doc's Non-goals.

**Per-backend runtime proof — all three TLS backends (2026-08-31).** The doc's
Phase 3 required this and it is met in full. Fixtures cross-built on the Mac and
executed on the remote boxes from `.ai/remote_systems.md` (2223 Kali aarch64,
OpenSSL 3.6.3; 2230 Win11 x86_64, Schannel):

| Backend | `tcp::Listener` | `tls::Socket` | `tls::Listener` |
| --- | --- | --- | --- |
| macOS Network.framework | `reply=pong:ping` / `worker=1` | `moved=TRUE` | `test result: ok` (openssl s_client) |
| Linux OpenSSL (box 2223) | `reply=pong:ping` / `worker=1` | `moved=TRUE` | `CLIENT SAW: transferred-listener-ok`, `Verify return code: 0 (ok)` |
| Windows Schannel (box 2230) | `reply=pong:ping` / `worker=1` | `moved=TRUE` | `HANDSHAKE: Tls12`, `CLIENT SAW: transferred-listener-ok` |

This matters most for **Schannel**, whose slots are the only `ArenaBlock`
deep-copies in the descriptor and the one path that could not be reasoned about
from the macOS result: a `tls::Socket` carries a **35136-byte** SSPI block and a
`tls::Listener` a **288-byte** WORK block, each copied into the receiver's arena
rather than aliased. Both now demonstrably survive the move — a Schannel socket
completed a real HTTPS exchange on its receiving thread, and a transferred
listener completed a TLS 1.2 handshake presenting the expected certificate
(thumbprint `76185CB2…E8A29B`, byte-identical to the generated `server.pem`) and
delivered its payload.

The Linux runs equally close the OpenSSL half, where the `SSL_CTX*`@32 /
`SSL*`@40 pair moves verbatim: the transferred listener's chain **verified
against the test CA** (`Verify return code: 0 (ok)`), which a listener carrying a
zeroed server context cannot do.

**What remains harness-level, not coverage-level.** These remote runs were driven
by hand; `scripts/` still has no Windows runner and the acceptance harness still
cannot execute a PE, so nothing in CI re-runs the Schannel proof. The standing
automated guard for the non-macOS backends is
`tests/byte-identity/resource-xfer-slots`, which lowers the live-slot copy for
all five targets and pins it with five `.ncodesum` goldens — enough to catch a
silent change to the emitted copy, not enough to re-prove it at runtime.

## Summary

The real engineering risk is **not** the registry bits — it is
`copy_resource_to_current_arena`, which silently truncates any resource record
past offset 24 and is the actual reason these three handles were fenced off. The
bug is filed as MEDIUM rather than HIGH because the compiler currently refuses the
program: no user can reach the unsafe path today. It becomes HIGH the moment
someone flips a `sendable` bit without Phase 2, which is exactly what the stale
"not thread-sendable in v1" comments invite. `tcp::Listener` is separable and
cheap; the TLS pair carries the per-backend session-state risk and must be proven
on OpenSSL, Schannel and Network.framework independently. Untouched: record
layouts, the resource-collection thread rule (§15.6), and the three resources that
already transfer correctly.
