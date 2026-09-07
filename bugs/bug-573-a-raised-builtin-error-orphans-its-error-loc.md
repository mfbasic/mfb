# bug-573: every error raised through `_mfb_make_error_result` orphans the `ErrorLoc` it just built

Last updated: 2026-09-07
Effort: medium (the free is small; proving who owns `x3` on the propagate path is the work)
Severity: **HIGH** (unbounded leak on any loop whose builtin call raises)
Class: Memory / correctness

Status: **Fixed** (this branch)
Regression Test: bug-565's negative pin
(`an_inline_builtins_own_domain_error_still_leaks_its_error_loc`, which asserted
the RSS GROWS) is flipped to `assert_flat` and renamed
`an_inline_builtins_own_domain_error_runs_at_constant_rss`; joined by
`a_raised_error_runs_at_constant_rss_however_long_its_filename_is` (the decisive
form), three more raise shapes, a no-error control, and
`every_raised_error_still_reports_its_true_origin`. The emitted shape is
`tests/codegen/codegen_parked_error_source_free.rs`.

Found while fixing bug-565, and measured to be a different defect: bug-565 is on
the TRAP side and is fixed; this one is on the RAISE side and is byte-identical
before and after it.

## Failing reproduction

```
IMPORT io
IMPORT collections
SUB main()
  LET xs AS List OF String = ["aa", "bb", "cc"]
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < N
    LET g AS String = collections::get(xs, 9) TRAP(e2)
      RECOVER "zz"
    END TRAP
    acc = acc + len(g)
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
```

Peak RSS (`/usr/bin/time -l`, macOS arm64, release), **identical before and after
bug-565's fix**:

| N | peak RSS |
| --- | --- |
| 200 000 | 39.1 MB |
| 400 000 | 77.2 MB |

~200 B per raised error. The `TRAP` is only there to keep the program running;
the leak is upstream of it.

## The decisive evidence: it scales with the FILENAME

The same program built so its recorded source path is 131 characters instead of
`src/main.mfb`'s 12:

| N | peak RSS, long path |
| --- | --- |
| 200 000 | 149.9 MB |
| 400 000 | 298.9 MB |

~750 B per raised error. Nothing in the program changed but the path recorded in
the `ErrorLoc`, whose `filename` is inlined into the block. That is what
identifies the orphan.

## Root cause

`emit_error_register_return` assembles a raised error in two steps:

1. `_mfb_make_error_result` (plan-16) allocates an **`ErrorLoc`** — filename,
   line, column — and returns it in `RESULT_ERROR_SOURCE_REGISTER` (`x3`).
2. `_mfb_rt_park_error` (`emit_park_error_block_from_registers`, plan-118-E
   phase 2) allocates the single owned flat `Error` block and INLINES copies of
   the message and that `ErrorLoc` into it, parks the block, and RESTORES the
   loose registers — including the original `x3`.

After step 2 the `ErrorLoc` from step 1 has no owner: the parked `Error` holds a
copy, not it. Nothing frees it, on any path.

## Why it is not a two-line fix

The obvious free — inside `_mfb_rt_park_error`, right after the block is built —
is wrong. The helper is a single synthesized function shared by every raise site,
and its `x3` input is not always a fresh block:

* on the `_mfb_make_error_result` path it IS fresh (this is the leak);
* on a **propagated** error the same registers carry a `source` that is an
  interior pointer INTO the caller's parked `Error`, which
  `route_current_result_to_trap`'s rebuild branch and
  `emit_trapped_error_result`'s `CalleeRegister` source both read after the park;
* on the OOM-degraded path (`building_error_block`) `x3` may be null.

So the fix needs the per-site ownership answer, not a helper-local one — either a
second entry point taking "this `ErrorLoc` is mine", or the same runtime
pointer-identity guard bug-565 and bug-571 use, comparing `x3` against the parked
block's own inlined `source` pointer. Either way it ADDS a free on the path every
error in the language takes, so it wants its own change, its own enumeration of
raise sites, and its own gate.

## What a fix must produce

The reproduction above flat at both counts, with the long-path variant flat too
(that is the sensitive form), and every error-message / `e.source` behaviour
fixture unchanged — the origin must survive, because a raised error's
`ErrorLoc` is what `mfb`'s top-level error printer reports.


---

## What the fix is

`_mfb_rt_park_error` releases the `ErrorLoc` it was handed, immediately after
`emit_build_error_inline` has copied it into the owned `Error` block it parks —
and leaves `RESULT_ERROR_SOURCE_REGISTER` pointing at the parked block's OWN
inlined copy rather than at the released original.

```
  bl _mfb_build_error_loc / copy_value_to_current_arena   ← the fresh ErrorLoc
  ...
  <park> build the flat Error, inlining a COPY of it; store its base in the slot
  cmp   source, 0
  b.eq  park_error_source_kept                            ← no origin: nothing to free
  <size it with the SAME formula _mfb_build_error_loc allocated with>
  bl    _mfb_arena_free
park_error_source_kept:
  ldr   off, [base + 16]                                  ← re-point x3 into the
  cbz   off -> x3 = 0                                        parked block's copy
  add   x3, base, off
```

Three things make it safe:

* **A runtime null guard.** An error with no origin arrives with `x3 == 0` —
  every `LINK` thunk returns one, and so does a raise whose own
  `_mfb_build_error_loc` hit OOM. Nothing is freed there. The other no-park path
  (`building_error_block`) never reaches the release at all:
  `emit_build_error_inline`'s OOM diverges through `raise_error_bare` first.
* **The size is the allocation's own.** `emit_record_block_size_to_slot` for
  `ErrorLoc` is the same formula `_mfb_build_error_loc` sized its `arena_alloc`
  with — 24 fixed bytes plus the inlined filename block — read back off the
  block. A size that is not the allocation's returns it to the wrong bin
  (bug-560's mechanism, not "doesn't free").
* **`x3` is left VALID, not dangling.** The origin the loose register names is
  now the parked block's own copy: byte-identical, and owned by the block the
  catcher adopts.

## The doc's "why it is not a two-line fix" did NOT survive

This document said the free is not helper-local because "on a **propagated**
error the same registers carry a `source` that is an interior pointer INTO the
caller's parked `Error`, which `route_current_result_to_trap`'s rebuild branch
and `emit_trapped_error_result`'s `CalleeRegister` source both read after the
park".

Both halves are wrong about the PARK, and the contrast that disproves them is
that `_mfb_rt_park_error` has exactly three call sites, and all three hand it a
block the frame just allocated:

| site | where its `x3` comes from |
| --- | --- |
| `emit_error_register_return` | `_mfb_make_error_result` → `_mfb_build_error_loc`, fresh |
| `emit_stamp_current_error_source` | `emit_build_error_loc`, fresh |
| `emit_finalize_worker_error_source` | `copy_value_to_current_arena` of the worker's `ErrorLoc` into THIS arena, or `emit_build_error_loc` — fresh either way |

Interior `ErrorLoc` pointers are real — `emit_load_error_fields` produces
`errorBase + offset`, and `emit_direct_error_return` puts exactly that in `x3`
for a `FAIL <Error value>` — but that path parks the block DIRECTLY
(`emit_store_current_error`) and never calls the shared helper. And the two
readers the document names are on the REBUILD branch of their respective
routers, which is reached only when the tag is not `ERR_BLOCK` — that is, only
when no park happened, so there is nothing freed to read.

The mechanism the document states — `_mfb_make_error_result` allocates an
`ErrorLoc`, the park inlines a copy and orphans the original — is exactly right,
and its evidence (the scaling with the filename) reproduced to within a percent.

## Measured

Peak RSS at 200 000 / 400 000 iterations, before = this branch's parent (which
already carries bug-574, so the argument leak is not in these numbers):

| shape | before | after |
| --- | --- | --- |
| `collections::get` raise, `src/main.mfb` (12 chars) | 39.1 → 77.2 MB, **199 B/raise** | 1.0 → 1.0 MB, **0** |
| the same at `src/<100 chars>/main.mfb` (117 chars) | 131.2 → 261.4 MB, **682 B/raise** | 1.0 → 1.0 MB, **0** |
| contrast: a loop that raises nothing | 1.0 → 1.0 MB | 1.0 → 1.0 MB |

4.6 B per filename byte — the slope that identifies the block as the `ErrorLoc`.

Three shapes the report implicated turn out never to have leaked, and are kept
as POSITIVE pins rather than claimed as fixes:

| shape | before | after |
| --- | --- | --- |
| `FAIL error(...)` from a user `FUNC`, trapped | 0 B/call | 0 B/call |
| the same, `PROPAGATE`d through a middle frame | 0 B/call | 0 B/call |
| `fs::readText` on a missing file under an inline `TRAP` | 0 B/call | 0 B/call |

`FAIL error(...)` builds one `Error` block with its origin already inside it and
parks it directly, so there is no separate `ErrorLoc` to orphan; and a runtime
helper under an INLINE `TRAP` takes the raw path, which never stamps an origin
and never calls the park at all.

## Correction to bug-574's commit message

That message attributed the ~1 KB per call that `net::lookup`'s resolve-failure
loop still grows to "bug-573's orphaned `ErrorLoc`, measured on the same binary".
It is not: the same two programs measure 1 089 → 1 056 B/call and 1 163 →
1 040 B/call across this fix, i.e. unchanged. What that commit measured
correctly, and what stands, is that the residue no longer scales with the host
name's length (4.4 B per byte → 0.15). The remaining growth is flat in the
argument, survives both fixes, and is not attributed here.

## The enumeration, and its totality

`ParkedErrorSource` (`src/codegen/error/emission/park_error_helper.rs`) is the
seat: `emit_park_error_call` takes one, each of the three sites names its own
variant with the provenance above, and the helper matches **exhaustively with no
wildcard** — a fourth park site, or a site whose source is borrowed, is a build
error until somebody answers the question for it. That is bug-567's shape, and
it is the right one here because the free lives in a single shared function that
cannot ask its caller anything at run time.

The third site — `WorkerArenaCopy`, the one where a mistake is cross-arena
corruption rather than a leak, because `x19` is per-thread — has an existing
end-to-end pin: `tests/rt-behavior/threads/thread-error-source-rt` asserts a
worker's terminal error still reports the WORKER's own `src/lib.mfb:192:8` after
`thread::waitFor` has carried it across the boundary. It is green on this branch
(`test-accept.sh`, 1 434 ran). The block released there is the deep COPY
`copy_value_to_current_arena` made into the calling thread's arena, never the
worker's original.

`tests/codegen/codegen_parked_error_source_free.rs` is its emitted-code
counterpart: exactly one guarded release and exactly one `_mfb_arena_free` in
`_mfb_rt_park_error` (a second would be freeing the parked block itself), the
guard located by the branch that targets its OWN label and asserted to be a
compare against 0, the re-point asserted to follow the release, the park's body
asserted byte-identical across all three raise shapes, and no other function in
the plan carrying a `park_error_source*` label.

## Golden attribution

144 goldens refreshed; 27 `codegen_cover` fixtures × 5 targets plus 10
full-dump `.ncode`/`.mir` goldens. Per function, on `linux-x86_64`, across all 27
packages:

**Exactly ONE function changed in each — `_mfb_rt_park_error` — gaining exactly
one `_mfb_arena_free` and no `_mfb_arena_alloc` (1/0/202 instructions → 1/1/240),
with `dataObjects` byte-identical in every package and not one other symbol
touched.** The seven `.ncode`/`.mir` dumps carry the same single-function diff
(the frame grows 144 → 192 bytes for the sizing slots); no data object and no
other symbol appears in any of them.

## Gates

`cargo test --release --no-fail-fast`, `scripts/artifact-gate.sh` (1 412 tests,
1 578 builds, 1 973 goldens, 0 diffs), `scripts/test-accept.sh` (1 434 ran),
`rustup run 1.96.0 cargo fmt --all --check`, `cargo check --all-targets`.
