# bug-601: a `MUT` copy of a non-flat list ALIASES its source, and in-place mutation then corrupts or frees the source

Last updated: 2026-09-13
Effort: the pointer-`String` half is done (plan-132); the recursive-type half needs the
bug-536 shape C design
Severity: HIGH — a 17-line program SIGSEGVs, and a 10-line one silently computes a wrong value
Class: Memory-safety / Correctness (value semantics)

Status: **Open — FIXED for the pointer-`String` records by plan-132** (Phase 1 `510a361ad`,
Phase 2 `da7f876d5`). `net::Address`, `udp::Datagram` and `audio::AudioDevice` are on the
ordinary flat layout, so they are `memcpy`-copyable and `MUT ys = xs` makes a real copy. The
three `List OF net::Address` repros below flip on macOS, Linux (box 2223) and Windows (box
2230): `removeAt` on the copy prints `ys=0 xs=1`, the two `append` shapes print `ys=2 xs=1`
with every host intact, and nothing crashes.
**Still open: the recursive-type row.** A `List OF Tree` (a recursive user union), `MUT ys =
xs`, 5 in-place appends printed `ys=6 xs=112` — the same aliasing, in a class flattening does
not touch (a recursive value cannot be flat). It rides on copy-insertion for recursive types,
bug-536 shape C's prerequisite. `Error` and `ErrorLoc` are split out as bug-602.
Regression Test: `a_mut_copy_of_an_address_list_is_independent_of_its_source` in
`tests/net/rt_net_address_record_layout.rs` (the pointer-`String` half). None yet for the
recursive row.

## How it was found

Found while working bug-599 (`List OF net::Address` is never freed). Evaluating whether a
deep drop could be added to that list type needed to know whether two bindings can share
one list block. They can, and the sharing is already unsound today, before any drop.

Defect search before filing: `grep -rliE "in-place|inplace" bugs/` → each hit read for a
bind-copy shape; bug-538 (`get` of a recursive element aliases storage) and bug-142
(`FOR EACH` + in-place append) are different shapes; bug-536 shape C records that
recursive values are *shared* across owning stores, but only as the reason a drop cannot be
added — not that the in-place arms already mutate the shared buffer. Number: filed first
as bug-600, which raced — a peer commit `bba16923d` ("fix(bug-600): a timed-out test run
kills everything its program started") holds it. bug-601 verified free with
`git log --all -E --grep='bug-6(0[1-9]|1[0-9])'` (empty), no `bug-60[1-9]*` file in `bugs/`,
`planning/` or any worktree, and no mention in `planning/bug-backlog.md` / `todo.md`.

## Reproduction (base `8435c09ca`, macOS aarch64, release)

```basic
IMPORT io
IMPORT net
IMPORT collections
FUNC main AS Integer
  LET xs = net::lookup("127.0.0.1")
  MUT ys = xs
  ys = collections::removeAt(ys, 0)
  io::print("ys=" & toString(len(ys)))
  io::print("xs=" & toString(len(xs)))
  RETURN 0
END FUNC
```

| probe | output | expected |
| --- | --- | --- |
| `MUT ys = xs` then in-place `removeAt(ys, 0)`, `List OF net::Address` | `ys=0 xs=0`, exit 0 | `ys=0 xs=1` |
| same, `ys = append(ys, a)` once | `ys=2 xs=96`, then **SIGSEGV** (exit 139) | `ys=2 xs=1` |
| same, `ys = append(ys, get(xs, 0))` once | **SIGSEGV** before any output | `xs=1 ys=2 …` |
| `List OF Tree` (a recursive user union), `MUT ys = xs`, 5 in-place appends | `ys=6 xs=112` | `ys=6 xs=1` |
| **contrast** `List OF String`, `MUT ys = xs`, in-place `removeAt` | `ys=0 xs=1` | correct |
| **contrast** `List OF net::Address`, `ys = append(append(ys, a), a)` (copying path) | `ys=3 xs=1` | correct |

Command per probe: `mfb build <dir>` then `/usr/bin/time -l build/<name>.out`.

## Root cause

Two halves that are each deliberate, and unsound together:

1. **No owning copy at the bind.** `lower_value_owned`
   (`src/codegen/engine/value/builder_values.rs`) deep-copies an aliasing source only when
   `is_freeable_flat_value(type)`, which requires `type_is_memcpy_copyable`. That is false
   for any collection whose payload is a recursive type or a pointer-`String` record
   (`is_pointer_string_record`: `net::Address`, `udp::Datagram`, `audio::AudioDevice`). So
   `MUT ys = xs` stores `xs`'s block pointer in `ys`. Measured in the emitted code: the
   `List OF net::Address` probe's `_mfb_fn_main` calls `_mfb_arena_alloc` **0** times; the
   identical `List OF String` probe calls it **2** times (the copy) plus
   `_mfb_rt_drop_owned_collection` 6 times.
2. **The in-place arms assume unique ownership and never check it.**
   `resolve_inplace_plain_local` / `InPlaceGate::admits_with`
   (`src/codegen/collection/assign/inplace_dest.rs`) gate on `by_ref`, a live `FOR EACH`
   and the collection layout. Uniqueness is supplied by copy-insertion ("Soundness rests on
   value semantics + copy-insertion (no live alias)", `.ai/collections.md`). For this class
   copy-insertion does not happen, so `removeAt` compacts the shared buffer (a silent wrong
   `xs`) and `append`'s grow arm reallocs it and frees the old block
   (`emit_free_pre_grow_buffer`) under `xs` (use-after-free).

The contract broken: `mfb spec language memory-semantics` §14.1 ("Copy creates an
independent value … Mutating the destination cannot affect the source") and §14.6 ("No two
live mutable bindings may refer to the same collection buffer").

`removeAt` over a *recursive* element type is already declined (`G24`), which is why the
`List OF Tree` repro uses `append`; `G24` exists for a different hazard (payload relocation
under a fetched element) and does not cover this one.

## Options

**A. Fail closed in the in-place gate:** decline every in-place arm when the collection
type is not `type_is_memcpy_copyable`. Declining is always correct (`.ai/collections.md`).
Emits a free nowhere and moves no lifetime. **Cost:** `json::parse` builds every array with
`acc = collections::append(acc, item)` over a `List OF Json`
(`src/codegen/builtins/json/helper_parse_array_items.rs`), and that is exactly the
in-place append this declines — array parsing becomes quadratic, on the untrusted-input
decoder bug-510 / bug-536 already care about. It would churn the `byte-identity/json`
goldens at least (other packages: guess, not measured). Needs a benchmark before it lands.

A narrower A — decline only when the `MUT` binding was ever initialised or assigned from
an aliasing source — is a shape-coupled escape analysis and must also see the reverse
direction (`LET zs = ys` after `ys` is fresh, then an in-place op on `ys`). Not recommended
as a bug fix.

**B. Copy-insertion for the class at owning stores**, using the deep copy that already
exists (`copy_value_to_current_arena`: `thread_copy_symbol` for recursive types,
`fix_collection_transfer_payload` → `copy_record_fields_into_existing` for pointer-`String`
records). This is bug-536 shape C's prerequisite, which the USER DECISION of 2026-09-06
moved to a design plan. Doing only the bind site closes the three repros but not a record
field or collection element that aliases the same buffer.

## Interaction with bug-599

bug-599's deep-drop option depends on B: without copy-insertion every drop is a double free
of the shared block. Option A does not help bug-599 (the leak is unchanged).
