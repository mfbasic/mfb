# bug-496: `g = <op>(g, f(…))` reads a freed block when `f` reassigns global `g` (UAF; also the `&` operator)

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (memory safety — use-after-free)

Status: FIXED

STATUS: FIXED (a075a589b, ratchet follow-up 316a5185c). `src/codegen/engine/value/
operand_snapshot.rs`, hooked at the one seam every operand passes through
(`lower_value`): on entry to a multi-operand node it records each operand that lowers
to a pointer into storage a LATER operand's user-code call can reassign (a global, a
by-ref / address-taken local, a resource's STATE, or a member/extract of one) and,
when that operand is lowered, `copy_flat_block`s it into a statement-scope pending
temp that the op consumes. Matching is by node address (fail-safe: a rewritten
operand falls back to the old behavior, never to a wrong copy). Narrowness pinned by
`tests/rt_operand_snapshot.rs` via the `operand_snapshot` stack-slot count: plain
locals (`x = append(x, f())`, `s & f()`) and a global followed only by pure native
builtins emit no snapshot, so the in-place `x = append(x, <pure>)` fast path is
untouched. Fixture: `tests/rt-behavior/collections/bug496_operand_snapshot_rt`.
Verified at -O0 and -O3 on the MEM-12 spike (`same -> [abcdefghtail] len=12`) and
its `collections::append` global-evictor variant (`1,2,3,0,1,2,`); full `cargo test
--no-fail-fast` exit 0; `artifact-gate all` 0 diffs (no committed fixture has the
shape). No MFBASIC syntax or observable behavior of a correct program changed: the
copy realizes the semantics `.ai/collections.md` already specified — operand 0 is
the pre-call value, the nested write is still (correctly) lost, and `GS` reads `XY`
afterwards. bug-487's in-place `RES … STATE` arms (half 1) are unchanged: they never
pass a `Call` node through `lower_value`, and its repro fails identically before and
after (7-701-0001).

Regression Test: add an rt fixture asserting `GS & same()` returns operand 0's original bytes (`len=12`), and a `collections::append` global-evictor variant.

## Summary

Operand 0 of a binary op / builtin call is lowered to a pointer into a value's
current block. If a *later* operand calls a function that reassigns the same
location, that assignment's owning store frees operand 0's block, and the op then
reads through the stale pointer. Demonstrated for a plain **global** (no `RES`, no
`STATE`) through both the `&` string-concat operator and `collections::append` —
a use-after-free reachable from ordinary source in a language whose spec
advertises memory safety through owned values. This is the general
argument-aliasing case that open **bug-487** scoped out as "half 2".

## Mechanism

```rust
// src/codegen/engine/builder/mod.rs:2841 (lower_string_concat_helper)
abi::load_u64("%v22", "%v20", 0), // left byteLength   <- freed block, offset 0
abi::load_u64("%v23", "%v21", 0), // right byteLength
abi::add_registers("%v24", "%v22", "%v23"),
```

`arena_free` preserves `[ptr, ptr+16)` as the free-node overlay
(`arena.rs:1112-1114`, scrub starts at `ptr+16`), so for `&` the read at offset 0
silently returns the quick-bin `next` link instead of the real `byteLength`. The
`collections::append` path (`list_mutate.rs:31+`, `lower_list_insert_collection`)
reads `COUNT`/`DATA_LENGTH` out of the recycled words and asks the arena for a
nonsense size.

## Reproduction (lead-run, live)

`spikes/audit-3/MEM-12/` — `mfb build spikes/audit-3/MEM-12 &&
./spikes/audit-3/MEM-12/build/mfb_project.out`. Observed at -O0 and -O3:

```
other -> [abcdefghtail] len=12     # other() reassigns a DIFFERENT global -> correct
same  -> [tail] len=4              # same() reassigns GS (left operand) -> UAF, bytes lost
```

Expected: both `abcdefghtail` / `len=12` (losing the nested write is correct
value semantics per `.ai/collections.md`; losing operand 0's bytes is not). The
`collections::append` global-evictor variant aborts with `7-701-0001 Allocation
failed`. Sequencing the call into a `LET` first fixes it, pinning the mechanism.

## Best fix

When lowering a call/binop whose argument 0 lowers to a pointer into a global's
block (or any location a later operand's call can reach) and any later argument
contains a `Call`/`CallResult`, materialize argument 0 into an owned temporary
(existing `lower_value_owned` / `copy_flat_block` path) before lowering the rest,
freed at statement scope. This is bug-487's "half 2"; the global case is simpler
than the `RES … STATE` case because there is no write-back to republish.

## Non-goals

Do not change evaluation order or make the nested write survive (bug-487 is
explicit that losing it is correct); do not add a copy to the common
`x = append(x, <pure expr>)` shape the in-place path optimizes.

## Prior art

**bug-487** (`bugs/bug-487-state-mutating-operand-uaf.md`, OPEN) — same mechanism
through `RES … STATE`; it scopes the general argument-aliasing case out as "half
2… outside that plan's blast radius". This finding is the evidence half 2 is
reachable with no resource at all, through a plain global, and that it also hits
`&`, not just `collections::append`. Also noted as a footgun (not a defect) in
`.ai/collections.md:362`. No prior bug doc for the global / `&` variant (searched
`append(global`, `global = append`, `operand`, `aliasing`, `use-after-free`).
