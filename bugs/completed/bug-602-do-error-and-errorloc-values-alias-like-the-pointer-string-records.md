# bug-602: do `Error` and `ErrorLoc` values alias the way the pointer-`String` records did?

Last updated: 2026-09-13
Effort: small (tested)
Severity: none — not a defect
Class: Memory-safety / value semantics — investigation

Status: **Closed — not a defect, measured 2026-09-13.** Every way a second name can reach an
`Error` gives an independent copy, and bug-601's in-place list shapes on a `List OF Error`
leave the source intact. This was already true before plan-132: the same probes print the
same correct output on the compiler before the flatten. `Error` and `ErrorLoc` were never
pointer-`String` records.
Regression Test: `an_error_value_copy_is_independent_of_its_source` in
`tests/runtime/rt_error_value_copies.rs` (a positive pin: it fails if `Error` ever leaves
the flat layout).

## Why this is filed

bug-601 showed that `MUT ys = xs` on a list of `net::Address` shares one block: the
compiler makes no copy of a value that is not `type_is_memcpy_copyable`, and the
pointer-`String` records (`net::Address`, `udp::Datagram`, `audio::AudioDevice`) are not.
An in-place `removeAt` on the copy empties the source; a growing `append` frees it. The owner
decided (2026-09-12) to flatten those three records onto the ordinary inline-`String` layout.

`Error` and `ErrorLoc` were suspected to be on the same pointer layout, because the doc
comment on `CodeBuilder::is_pointer_string_record`
(`src/codegen/collection/layout/builder_collection_layout.rs`) lists them: "`Error`/`ErrorLoc`:
the fallible-call ABI, trap materialization, `FAIL`".

**The sources disagree, and the spec plus the code say that comment is stale:**

- The free function `is_pointer_string_record` names only `net::Address`, `udp::Datagram`
  and `audio::AudioDevice`. It does not list `Error` or `ErrorLoc`.
- `mfb spec memory` (`src/docs/spec/memory/03_heap-values.md`, "`Error` and `ErrorLoc`")
  says they are **flat built-in records**. Their `String`/sub-record fields are inlined by
  block-relative offset, "so the whole value is a single pointer-free block", and "copying an
  `Error` is one `memcpy`". A null `source` is an offset-0 sentinel.

If that holds, an `Error` copy is a real copy and bug-601's aliasing cannot happen, whatever
the ownership model. **That is a reading, not a measurement** — this bug exists to run the
probes below and confirm it, and to correct the stale comment either way.

## The owner's hypothesis (to test, not to assume)

> there is only 1 of each so I *think* they are treated more like a RES than a LET or MUT.

That is: an `Error` value is produced once (by a raise, `FAIL`, or a `TRAP` binding) and then
moved rather than copied, so there may be no second binding that could share its block —
which would make the bug-601 shape unreachable for them even though the layout is the same.

## Results (2026-09-13)

### 1. Layout: `Error` and `ErrorLoc` are ordinary flat records

`TypeModel::module_tables` (`src/codegen/engine/validation/validation.rs`) inserts them into
`record_fields` as plain records — `Error` = `code AS Integer`, `message AS String`,
`source AS ErrorLoc`; `ErrorLoc` = `filename AS String`, `line AS Integer`, `char AS Integer`
— with the comment "laid out as ordinary 3-field records so construction, field access,
copying, and cleanup reuse the generic record machinery". Nothing in
`builder_collection_layout.rs` special-cases either name
(`grep -n '"ErrorLoc"\|"Error" =>' src/codegen/collection/layout/builder_collection_layout.rs`
is empty), so the generic rule applies: every field is a scalar, a `String` or a flat record,
which makes both `type_is_memcpy_copyable`. There was never a second route onto the pointer
layout. The stale doc comment went away with `CodeBuilder::is_pointer_string_record` itself
(plan-132 Phase 3, `301c27462`).

### 2. The owner's hypothesis does not hold, and the question is still answered "safe"

An `Error` is **not** RES-like. The language accepts every second-binding shape: `LET`/`MUT`
copies of a `TRAP` binding, `List OF Error`, an `Error` record field, a parameter and return,
a closure capture, and `FAIL` of a binding still in scope all compile. It is safe for the
reason the spec gives — the value is one flat block, so each of those makes a copy — not
because a second binding is impossible.

### 3. Probes

Programs under `/tmp/b602/p*` (runner `/tmp/b602/run.sh`), built with `mfb build` and run on
macOS AArch64. "Before" is the compiler at `e04ecae8f` (plan-132 Phase 0, no layout change
yet); "after" is main at `d6d43aed3`.

| Probe | Shape | Output (before and after, identical) | Expected |
| --- | --- | --- | --- |
| p1 | `TRAP(e)`, `LET e2 = e`, `MUT e3 = e`, `e3 = error(8, …)` | `e=7/first@src/main.mfb:6`, `e2=` the same, `e3=8/second@…:18` | source unchanged |
| p2 | bug-601 shape 1: `MUT ys = xs`, `ys = removeAt(ys, 0)` over `List OF Error` | `ys=0 xs=1`, `xs0=1/one` | `ys=0 xs=1` |
| p3 | bug-601 shape 2: in-place `append(ys, error(2, "two"))` | `ys=2 xs=1`, `xs0=1/one ys1=2/two` | `ys=2 xs=1` |
| p4 | bug-601 shape 3: `append(ys, get(xs, 0))` | `ys=2 xs=1`, every element `one@src/main.mfb` | no crash |
| p5 | `Holder { err AS Error }` copied, `WITH`-updated, field copied out; `List OF Holder` `removeAt` | `h=5/held n=1`, `h2=6/replaced n=2`, `e4=5/held`, `ys=0 xs=1 xs0=5/held` | source unchanged |
| p6 | parameter and return; closure capture used after its scope and after 32 000 bytes of string churn; `FAIL` of a live binding | `a=9/nine b=9/nine c=10/ten`, `f=cap12`, `g=escaped1`, `g=escaped2`, `trapped=13/orig a13=13/orig`, `a13-after=13/orig` | source unchanged |
| p7 | every shape above, 20 000 iterations, checking the source each step | `bad=0`, exit 0 | `bad=0` |

Every probe exits 0.

### 4. Emitted code: the copy exists

bug-601 measured `_mfb_fn_main` of the `List OF net::Address` probe calling `_mfb_arena_alloc`
**0** times — no copy at the bind. The `List OF Error` probe p2
(`mfb build --ncode /tmp/b602/p2_list_remove_at`, counting relocations from `_mfb_fn_main`)
calls `_mfb_arena_alloc` **7** times, `_mfb_arena_free` 5 times and
`_mfb_rt_drop_owned_collection` 9 times: the bind copies, and the copies are dropped.

## Relation to other bugs

- **bug-601 / bug-599**: the original aliasing and leak for the pointer-`String` records.
- **bug-573**: an unrelated leak — a raised error orphaned its `ErrorLoc` (fixed
  `982a52e17`); not aliasing.
- **bug-593**: an inline `TRAP`'s `Result` wrapper now has an owner; relevant because `TRAP`
  is one of the paths that binds an `Error`.
