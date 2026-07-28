# bug-152 — `FAIL e` re-raise leaks the deep-copied Error transient

**FIXED 2026-07-11** (commits 14de6012 "error-block-in-slot / design b" +
549e1d25 "free consumed nested-record arg blocks"). Verified RSS-flat and correct
on all four remotes (Kali aarch64, Alpine x86_64 musl, Ubuntu x86_64 glibc, Alpine
riscv64 musl): a two-level `FAIL e` re-raise loop holds ~1 MB flat at 40k/160k
iterations with the propagated code/message intact. Regression:
`tests/rt-behavior/trap/bug152_reraise_adopt`.

Fixed by two complementary changes rather than the once-feared 140-site ABI sweep:
1. **Design "b" adopt** — a propagating error travels as ONE owned flat Error block
   parked in a new per-thread `ARENA_CURRENT_ERROR_OFFSET` slot (tag
   `RESULT_ERR_BLOCK_TAG`); the catching trap route ADOPTS that block (freed once)
   instead of rebuilding a fresh one and orphaning the source. Only the FAIL/return
   producers that hold an owned block set the slot; the ~140 loose-register
   producers are untouched (dual-tag → catcher rebuilds for them), so it is not a
   140-site change. This removed the re-raise deep-copy orphan.
2. **Constructor self-containment** — the `ErrorLoc` block that `error(...)` builds
   and byte-inlines into the Error was orphaned on the FAIL control transfer (which
   clears, not frees, pending temps); freeing consumed nested-record temps at
   construction closed that (a separate, general cross-call propagation leak that
   also predated design b).

Historical description follows.

Discovered 2026-07-11 while fixing bug-151 (the general caught-`Error` leak). Once
bug-151 registers the caught `e` for scope-drop, re-raising it with `FAIL e` from a
handler leaks a *different* block — the deep copy made on the way out — so a
re-raise-in-a-loop still grows RSS ~1 KB/iteration.

## Symptom

A handler that re-raises the caught error:

```
FUNC reraise(n AS Integer) AS Integer
  LET v AS Integer = inner(n)   ' inner FAILs
  RETURN v
  TRAP(e)
    FAIL e                       ' re-raise
  END TRAP
END FUNC
```

called in a loop leaks linearly:

| N (re-raises) | max RSS |
|---------------|---------|
| 10,000        | 10.9 MB |
| 20,000        | 20.7 MB |
| 40,000        | 40.4 MB |

Output stays correct (the propagated error's `code`/`message` are intact) — this is
a pure leak, not a miscompile or double-free.

## Root cause

`emit_error_value_exit` runs the scope-drop cleanups before propagating. When the
handler has live owned cleanups (after bug-151, the caught `e` is always one),
`store_pending_error_from_value` → `lower_value_owned(error)` **deep-copies** the
`FAIL`ed error into a fresh standalone block B so the subsequent frees cannot scrub
the message/source pointers it propagates (plan-02 Phase 8). The original `e` (block
A) is then correctly freed by the cleanup, and B's *fields* (code + block-relative
message/source pointers) travel to the caller in registers — but block B itself is
never freed. The caller's trap route rebuilds its own `e` block from those
registers, so B is orphaned the moment the propagation copies out of it.

Before bug-151 the handler usually had *no* live cleanups, so `FAIL e` took the
`emit_direct_error_route_to_trap` path (no deep copy) and instead leaked block A —
same net one-block-per-`FAIL e` leak, just a different block. So this is not a
regression from bug-151; it is a pre-existing leak in the deep-copy propagation path
that bug-151 makes the common trigger for.

## Fix (needs design)

The deep-copy transient B is a caller-relative propagation buffer whose lifetime
ends as soon as the receiving trap route (or the top-level exit handler) has copied
its fields out. Freeing it requires either:
- propagating the error as an *owned block pointer* the receiver frees after
  rebuilding (a Result-ABI change — the error currently travels as three loose
  registers: code, message-ptr, source-ptr), or
- having the deep copy target a caller-owned slot the receiver adopts, rather than a
  fresh arena block orphaned at the send site.

Both touch the error-Result ABI and the trap-route rebuild, so this is an
error-propagation redesign, not a spot free. Same block-lifetime family as bug-151
and the bug-147.5(b) thread-send copied-message leak.

Confirmed structural blocker (code audit 2026-07-11): the propagated message/source
registers hold *interior* pointers — `emit_load_error_fields` computes
`message = error_block_base + block[8]` (block-relative offset), never carrying the
block base — and `route_current_result_to_trap` (builder_codegen_primitives.rs:1946)
always REBUILDS a fresh `e` block via `emit_build_error_inline` from those interior
pointers. So the receiver structurally cannot free the propagated block (it never
learns the base), and every hop orphans one block. A correct fix must change the
Result ABI to carry the Error block base (so the receiver can adopt or free it),
which also forces rodata-message errors (`FAIL error(code, "literal")`, today a
no-alloc path pointing message at rodata) to always allocate a block. That is a
change to every `FAIL`, error return, trap route, and error-producing builtin across
all three backends.

Feasibility measured (2026-07-11): **140 sites across 19 files** set `RESULT_ERR_TAG`
(every error producer — fs/io/os/net/tls/crypto/datetime/term/thread/link/app plus
the shared assemblers). Carrying an Error-block-base signal means each of those 140
sites must set it (0 for the rodata-message majority, the block base for the few that
own a block); a single missed site leaves a garbage/stale base that the trap route
would `arena_free`, crashing error handling on that path. Functional tests + the four
remotes cannot exercise all 140 error paths, so this change is NOT provably
regression-free by the validation available here — a missed producer would be a
latent crash on an untested error path. The correct execution is a dedicated effort
that routes ALL error construction through one choke point (so the base is set in a
single place) plus exhaustive error-path coverage, not a 140-site sweep.

## Scope

All targets (shared codegen). Correctness-critical only for programs that re-raise
(`FAIL e`) in a loop; the common catch-and-handle path is fixed by bug-151. No
double-free, no miscompile — pure memory growth.
