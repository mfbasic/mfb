# bug-565: the inline-`TRAP` ERROR path leaks ~780 B per trapped error

Last updated: 2026-09-07
Effort: medium
Severity: **HIGH** (unbounded leak on any loop whose fallible call fails)
Class: Memory / correctness

Status: **FIXED** (2026-09-07, `<fix-hash>`)
Regression Test: `tests/runtime/rt_scope_drop_leaks.rs` —
`a_trap_whose_call_always_fails_runs_at_constant_rss`,
`a_trapped_error_read_by_its_handler_runs_at_constant_rss`,
`a_failing_call_inside_a_collection_walk_runs_at_constant_rss`,
`an_auto_propagated_error_out_of_a_loop_body_runs_at_constant_rss`, the POSITIVE
RSS-**equality** pin `an_inline_builtins_own_domain_error_still_leaks_its_error_loc`,
and the 25-run behaviour pin
`every_trapped_error_shape_still_produces_the_right_value`;
`tests/codegen/codegen_trap_error_free.rs` — five owner counts, two of them
negative; and the two totality assertions
`src/codegen/memory/arena/builder_arena_transfer.rs::tests::the_result_tag_partition_is_total`
and `::exactly_one_tag_is_adoptable_and_the_emitter_uses_it`.

Found while fixing bug-561, which fixed the *success* half of the same lowering.

## The finding

```
FUNC always(n AS Integer) AS String
  IF n >= 0 THEN
    FAIL error(7, "always")
  END IF
  RETURN toString(n)
END FUNC
SUB main()
  … WHILE i < N
    LET s AS String = always(i) TRAP(e)
      RECOVER "fallback"
    END TRAP
  … END WHILE
END SUB
```

| N | peak RSS before | after |
| --- | --- | --- |
| 200 000 | **149.8 MB** | **1.0 MB** |
| 400 000 | **298.6 MB** | **1.0 MB** |

~780 B per trapped error, and the shape is not narrow — three more programs
measured the same defect:

| program | N | 2N | before | after |
| --- | --- | --- | --- | --- |
| handler READS the error (`e.code`, `RECOVER e.message`) | 200k | 400k | 149.8 → 298.6 MB | 1.0 → 1.0 MB |
| failing call inside a `List OF String` walk | 50k | 100k | 298.6 → 596.2 MB | 1.0 → 1.0 MB |
| error AUTO-PROPAGATED out of the middle of a loop body | 50k | 100k | 38.2 → 75.4 MB | 1.0 → 1.0 MB |

The last two are the shapes bug-571 measured and could not fix ("the error paths
out of a `FOR EACH` body still grow … that is bug-565, still open"). All are
`/usr/bin/time -l` peak RSS, macOS arm64, release, at two counts — a one-shot
cannot tell a leak from an allocator high-water mark.

The control that makes the numbers attributable is the SAME program with the
producer's condition inverted — the identical inline `TRAP` over the identical
callee, whose error branch is simply never taken: **1.06 MB at 200 000 and
1.06 MB at 400 000 on the base compiler, 1.03/1.03 MB after**. That isolates the
leak to the error path rather than to the `TRAP`, the callee or the loop. (It is
flat on the base compiler because bug-561 fixed the Ok half of the same
lowering.) It is pinned as
`the_same_trap_whose_call_never_fails_runs_at_constant_rss`.

## Root cause — TWO orphaned blocks per trapped error, not one

The `NirValue::CallResult` lowering (`builder_values.rs`, both the direct
user/`.mfb` callee and the indirect `FUNC`-value callee) built its error branch
like this:

```
error_register = emit_build_error_inline(value_slot, message_slot, source_slot)
store error_register -> payload_slot
emit_build_result_inline(tag_slot, "Error", payload_slot)   ' COPIES the block in
store -> result_slot
```

**Orphan 1 — the parked block.** `FAIL error(...)` does not return a loose error.
`emit_direct_error_return` / `store_pending_error_from_value` build ONE owned flat
`Error` block, PARK its base in the per-thread `ARENA_CURRENT_ERROR_OFFSET` slot,
and tag the result `RESULT_ERR_BLOCK_TAG` so the catcher ADOPTS it (design "b",
plan-error-block-in-slot). This lowering never looked at the tag: it rebuilt a
fresh `Error` from the loose registers unconditionally, leaving the parked block
owned by nothing until the next `FAIL` overwrote the slot.

**Orphan 2 — the rebuilt block.** `emit_build_result_inline` deep-copies the block
at `payload_slot` into the freshly allocated `Result`, after which the source has
no owner either. This is the exact shape bug-561 fixed on the Ok branch.

`materialize_current_result`'s error assembly (the inline-builtin / runtime-helper
path) had orphan 1 solved — it adopts, and frees the adopted block after the copy
— but its REBUILD branch had orphan 2, plus a third: the `ErrorLoc` it stamps for
itself with `_mfb_build_error_loc` is inlined into the flat `Error` and then
abandoned, as is the caller-arena copy of a worker error's message.

## The fix

One shared emitter, `emit_trapped_error_result`, for all three trapped-`Result`
lowerings. It ADOPTS on the parked tag and REBUILDS otherwise, wraps the payload
in the `Result` once at a join point, and then frees what it is holding.

The three lowerings differ on exactly one axis, which is now a type rather than a
comment — `TrappedErrorSource`:

| variant | lowering | rebuild's `ErrorLoc` | freed? |
| --- | --- | --- | --- |
| `CalleeRegister` | direct user/`.mfb` callee, indirect `FUNC` value | the CALLEE's, arriving in `x3` and preserved verbatim | **no** — this frame allocated nothing |
| `CurrentLocation` | inline builtin / runtime helper trapped here | a fresh `_mfb_build_error_loc` for this expression | yes, guarded |
| `WorkerArena` | inline-trapped `thread::waitFor` | a caller-arena deep copy of the worker's origin, plus a copy of its message | yes, guarded |

### Spec

`mfb spec language memory-semantics` §14 preamble: "Each live value is owned by
exactly one binding, container slot, temporary, closure environment, thread
message, or return slot. Values are reclaimed by deterministic drop at the end of
the owning scope." §14.7 lists `FAIL` and auto-propagated errors among the scope
edges at which live bindings are dropped — and both of those edges are where this
leak was, because both are how a trapped error arrives. Neither orphan had an
owner at all: the parked block is not the `Result`'s (that is a copy, §14.1:
"Copy creates an independent value with no shared mutable state") and not the
handler's `e` (that is another copy, out of the `Result`). The fix names one owner
— this expression's temporary — and drops it at the end of the branch that made
it. `_mfb_arena_alloc` counts are unchanged or LOWER everywhere; only frees are
added.

### Why this is not a double free

Adding a free is the double-free direction, so the free does not rest on the
emitter's compile-time belief about who owns what.

* **The payload.** The one block here another owner may still hold is the PARKED
  one: it lives in the per-thread current-error slot until somebody adopts it, and
  `route_current_result_to_trap` adopts the same slot for a function-level `TRAP`.
  `emit_free_trapped_error_payload` loads that slot AT RUN TIME and frees only if
  the payload is not it. `emit_adopt_current_error_block` zeroes the slot as it
  hands the block over, so a real adoption always differs and is always freed; a
  block still parked is never freed, however it got there. Fail-closed: an
  unforeseen path leaks a block rather than freeing one twice.
* **The rebuild's components.** `emit_free_borrowed_guarded` frees the `ErrorLoc`
  (and the worker message copy) only when the pointer differs from the RAISER's,
  which is the value the caller names as not-ours. For `CalleeRegister` the two
  are literally the same stack slot, so the emitter skips the compare entirely
  rather than emitting one that could go the wrong way — and
  `codegen_trap_error_free.rs::only_the_lowering_that_stamps_its_own_error_loc_frees_one`
  asserts that contrast in the instruction stream. A single unconditional "free
  the ErrorLoc" would have been a use-after-free of the callee's origin.

### The enumeration, and its totality

The predicate that decides "this `Error` block is dead after the `TRAP`" is keyed
on the raw result TAG, so the tag vocabulary is partitioned explicitly —
`TrappedErrorTagClass` — instead of letting an unlisted tag fall to a default:

| tag | class | consequence |
| --- | --- | --- |
| `RESULT_OK_TAG` (0) | `NotAnError` | the wrap-error branch is not taken |
| `RESULT_ERR_TAG` (1) | `LooseRegisters` | rebuild from the registers |
| `RESULT_PROGRAM_EXIT_TAG` (2) | `LooseRegisters` | rebuild from the registers |
| `RESULT_ERR_BLOCK_TAG` (3) | `ParkedBlock` | ADOPT the parked block |

`the_result_tag_partition_is_total` reads `error_constants.rs` with `include_str!`
and asserts the file declares exactly these four and that each has an explicit
class — a fifth tag reds the test rather than silently rebuilding.
`exactly_one_tag_is_adoptable_and_the_emitter_uses_it` asserts the adopt set has
exactly one member and that the emitted compare is derived from the partition, not
spelled again beside it, so the generated code cannot drift from the table above.
`every_trapped_error_source_is_accounted_for` does the same for the three source
kinds.

## Golden delta

`artifact-gate.sh all`: **35 of 1 973 goldens moved** — 7 packages x 5 targets:
http, json, net, strings, tcp, tls, udp, the same set bug-561 moved, because they
are the packages whose `.mfb` bodies inline-`TRAP` a call. Every other fixture —
audio, bits, collections, crypto, csv, datetime, encoding, fs, general, io, math,
money, os, regex, term, thread, vector, and all of `syntax/**` and
`rt-behavior/**` — is byte-identical. Regenerated with
`bash scripts/regen-ncodesum.sh target/release/mfb`; the 35 files it rewrote are
the same 35 the gate flagged, and the re-run gate reports **1 412 tests, 1 578
builds, 1 973 goldens, 0 diffs**.

The delta is localized rather than assumed. The `-ncode` dump of each of the 7
fixtures was diffed function by function against the pre-change compiler (a
detached worktree at the same commit, 0-diff baseline): **0 functions added, 0
removed, `dataObjects` byte-identical in all seven**, and 21 changed function
instances / 17 distinct functions. **Every one of them gained at least one
payload-free guard AND at least one `_mfb_arena_free`, and not one changed
without gaining an owner** — 21/21:

| function | assembly | payload guard | ErrorLoc guard | `_mfb_arena_free` | `_mfb_arena_alloc` |
| --- | --- | --- | --- | --- | --- |
| `http::buildResponse` | 0 → 1 | 0 → 1 | — | 36 → 37 | 12 → 12 |
| `http::frameAdvance` | 0 → 3 | 0 → 3 | — | 15 → 18 | 27 → 27 |
| `http::frameComplete` | 0 → 1 | 0 → 1 | — | 1 → 2 | 4 → 4 |
| `http::handleRequest` | 2 → 5 | 0 → 5 | 0 → 2 | 64 → 69 | 32 → **30** |
| `http::handleRequestSSL` | 2 → 5 | 0 → 5 | 0 → 2 | 64 → 69 | 32 → **30** |
| `http::invokeHandler` | 1 → 1 | 0 → 1 | 0 → 1 | 5 → 6 | 15 → **14** |
| `http::lingerNet` | 1 → 1 | 0 → 1 | 0 → 1 | 3 → 4 | 10 → **9** |
| `http::lingerTls` | 1 → 1 | 0 → 1 | 0 → 1 | 3 → 4 | 10 → **9** |
| `http::parseRequest` | 0 → 1 | 0 → 1 | — | 15 → 16 | 25 → 25 |
| `http::readNet` | 1 → 1 | 0 → 1 | 0 → 1 | 4 → 5 | 12 → **11** |
| `http::readTls` | 1 → 1 | 0 → 1 | 0 → 1 | 4 → 5 | 12 → **11** |
| `http::readRequestNet` | 1 → 1 | 0 → 1 | 0 → 1 | 11 → 12 | 13 → **12** |
| `http::readRequestTls` | 1 → 1 | 0 → 1 | 0 → 1 | 11 → 12 | 13 → **12** |
| `http::respondPath` | 1 → 1 | 0 → 1 | 0 → 1 | 11 → 12 | 10 → **9** |
| `net::decodeQueryComponent` (in http, net, tcp, tls, udp — 5 instances) | 0 → 1 | 0 → 1 | — | 1 → 2 | 7 → 7 |
| `json::parseHexQuad` | 1 → 1 | 0 → 1 | 0 → 1 | 8 → 9 | 10 → **9** |
| `strings::fromScalars` | 0 → 1 | 0 → 1 | — | 3 → 4 | 9 → 9 |

Three things in that table are the semantics proof:

* **`arena_alloc` never rises, and falls in nine of the seventeen.** The adopt
  branch stops rebuilding an `Error` that was already built, so the fix removes
  an allocation from every function whose trapped error can arrive parked. Only
  frees are added.
* **The `ErrorLoc` guard appears in exactly the functions that trap an inline
  builtin or a runtime helper** (`TrappedErrorSource::CurrentLocation`) and in
  none of the ones that trap a user callee (`CalleeRegister`), where the origin
  belongs to the raiser. `_mfb_build_error_loc` call counts are unchanged
  everywhere — the fix frees that block, it does not build more of them.
* **`assembly` rises where a lowering GAINED the adopt/rebuild join** — the two
  `builder_values.rs` paths, which previously had no join at all — and stays put
  where `materialize_current_result` already had one.

Each of these seventeen was leaking an `Error` block per trapped error inside the
standard library.

## Residual — filed as bug-573, not fixed

An error raised by an **inline builtin's own domain check** still leaks, and this
change deliberately declines to touch it:
`collections::get(xs, 9) TRAP(e) RECOVER "zz"` holds **39.1 MB at 200 000 and
77.2 MB at 400 000 both before and after**. The leak is on the RAISE side, before
any `TRAP` is involved: `_mfb_make_error_result` allocates an `ErrorLoc`,
`_mfb_rt_park_error` builds the owned `Error` block by inlining a COPY of it, and
the original is orphaned. The evidence that it is the `ErrorLoc` and not the trap
is that it scales with the recorded FILENAME length — ~200 B per raise for
`src/main.mfb`, ~750 B for a 131-character source path (149.9 MB at 200 000).
Freeing it here would be freeing a block the non-trapped propagation path still
hands to its caller in `x3`, so it needs its own change and its own gate —
**bug-573**, which carries the long-path measurement and the enumeration of raise
sites a fix would need. It is pinned here as an RSS **equality** (`an_inline_builtins_own_domain_error_still_leaks_its_error_loc`,
asserted to GROW) so a later fix reds this document instead of quietly passing.
