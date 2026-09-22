# plan-146-D: Grow arms, 6 `String` rows

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-146-C

Prerequisites: see plan-146-A.

Six `String` builtins return their first argument with bytes added (findings §3.4
"grow"):

- **append growth:** `strings::padRight`, `strings::padRightToWidth`,
  `strings::repeat`;
- **prefix growth:** `strings::padLeft`, `strings::padLeftToWidth`,
  `os::resourcePath` (`base + "/" + relative`).

Append growth is the self-append's shape: reserve, then write past the end.
Prefix growth also moves `s`'s bytes right. After D, none of the six allocates per
statement at S1 or S2. A loop that grows `s` allocates `O(log n)` times, as
`s = s & t` does.

## 1. Goal

- `ArmId::StrGrow`, one arm for the six rows through a table
  `STRING_GROW_FNS: &[(&str, RangeInclusive<usize>, GrowFn)]`. Its marker is
  `inplace_str_grow`.
- The 6 rows flip `Pending("D")` → `Arm([StrGrow])`, and their `cases.tsv` lines
  flip `pending:D` → `arm`.
- Failure atomicity: each row computes its final length and raises every error it
  can (Phase 1 lists them) before `emit_string_reserve`. The reserve allocates
  before any write, so `ErrOutOfMemory` also leaves `s` unchanged.
- A differential runtime case per row compares `LET t = f(s, …)` with
  `s = f(s, …)`: width already met (no change), multi-byte `padChar`, `times = 0`
  and `1`, and wide (East Asian) characters for the `*ToWidth` pair.

### Non-goals

- The copying lowerings stay for non-self-update calls.
- No change to any result, including `*ToWidth`'s column rule.
- S9 stays declined (letter G).

## 2. Current State

| row | lowering (findings Appendix B.2) | how the result is built |
|---|---|---|
| `padLeft`, `padRight` | `Body::abi_inline` → `gen_pad::lower_strings_pad` | `emit_arena_alloc_call` of the padded length |
| `repeat` | `Body::abi_inline` → `func_repeat::lower` | alloc of `len × times + 9` |
| `padLeftToWidth`, `padRightToWidth` | `Body::Rewrite` → MFBASIC helper (`helper_pad_to_width.rs`) | the helper builds and returns a new `String` |
| `os::resourcePath` | `Body::abi_function` → `lower_resource_path` (`os/func_resource_path.rs`) | `base + "/" + relative` into an owned arena `String` |

The `Rewrite` and `abi_function` rows become visible to the seam in B Phase 1
(`self_update_builtin` spellings).

## 3. Design

**The arm.** `try_inplace_string_grow_assign`:

1. `resolve_string_self_update(…, needs_shadow = true)`.
2. The row's *measure* step computes `newLen` and whatever it needs to write (the
   pad count, the base path) and raises every error. It is split out of the
   copying lowering, as C split the window.
3. `emit_string_reserve(site, newLen)`: it fits in `len + shadow`, or it regrows
   geometrically. The regrow copies `len` bytes and frees the old block.
4. The row's *write* step:
   - `padRight` / `padRightToWidth`: write the pad bytes at `+8+len`.
   - `repeat`: copy bytes `[8, 8+len)` to `+8+len·k` for `k = 1 … times−1`.
     `times = 0` sets the length to 0, and `times = 1` writes nothing.
   - `padLeft` / `padLeftToWidth` / `resourcePath`: move `[8, 8+len)` right by
     `d = newLen − len` with a backward byte loop (the regions overlap), then
     write the `d` prefix bytes at `+8`.
5. `emit_string_set_len(site, newLen)`.

**`*ToWidth`.** Their helper computes a column count from `s`, and Phase 1 records
exactly how. The measure step computes the same count. It calls the same
width routine, which only reads `s`, or a native twin if one exists. It must not
re-implement the column rule, so the differential case is the only proof needed
that the two agree.

**`resourcePath`.** The measure step obtains the base path the copying lowering
uses (Phase 1 records its source and failure modes). Then the prefix is
`base + "/"`.

**The shadow predicate** adds the six bare names.

Risk: the backward move for prefix growth, and `repeat`'s self-overlapping copy
(the source region is part of the growing destination; copying from the fixed
prefix `[8, 8+len)` keeps it correct). The differential cases cover both at edge
lengths.

Rejected alternatives:

- **Route `padRightToWidth` through its MFBASIC helper with a `MUT` parameter.**
  It needs a by-reference parameter convention the language does not have.
- **Treat prefix growth as `Exempt`.** The result is `s` plus bytes, so a copy of
  `s` exists to avoid. plan-142 put `prepend` and `insert` in arms for the same
  reason.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. `- [~]` partial. Moot tasks struck through with evidence, never
> deleted. **An unticked box means NOT DONE.**

### Phase 1: Read the six lowerings

- [ ] Record per row: every raise point (`file:symbol`), and how `newLen` is
      computed. For `*ToWidth`, the width routine the helper calls and whether a
      native twin exists. For `resourcePath`, the base path's source and failure
      modes.

Acceptance: recorded here with citations (est. 30 min).
Commit: —

### Phase 2: Split measure from build, byte-identical

- [ ] `gen_pad.rs`, `func_repeat.rs`, `func_resource_path.rs`: a measure half the
      copying lowering calls. `helper_pad_to_width.rs` is MFBASIC and is not split.
      Its measure step is new code in the arm, per §3.

Acceptance: `cargo build --release && cargo test --test golden` → 0 `.ncode` diffs
(est. 20 min, for the same reason as C Phase 2).
Commit: —

### Phase 3: The arm

- [ ] `ArmId::StrGrow`, `STRING_GROW_FNS`, `try_inplace_string_grow_assign`, the
      marker, and the `SELF_UPDATE_ARMS` entry after `StrWindow`.
- [ ] Add the six names to `is_string_self_update`.
- [ ] The 6 rows → `Arm([StrGrow])`; 6 `cases.tsv` lines → `arm`. Each line pairs
      the grow with a C shrink that restores `x`
      (`x = strings::padRight(x, 8) ; x = strings::left(x, 3)`).
- [ ] Runtime: `tests/rt-behavior/strings/self-update-grow-valid/`, with the
      differential cases and a trapped failure per raising row, at a local and a
      global.
- [ ] A growth-amortization case: `s = strings::padRight(s, len(s) + 1)` 10,000
      times allocates fewer than 64 blocks (`arena.<k>.alloc_calls` from `--debug`).
      That is the geometric-regrow claim, measured.
- [ ] RED proof: make `padLeft`'s move run forward instead of backward. The
      differential case must fail. Restore.

Acceptance: `cargo test --bin mfb self_update`, the harness over the six lines, and
`scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/strings/self-update-grow-valid'`
→ pass (est. 8 min).
  Expected golden diffs: none from in-tree fixtures (the fixture grep in B's
  Measured populations finds no grow-row self-update in `tests/`, `examples/` or a
  helper body). Run `cargo test --test golden`. Any diff is a bug to root-cause.
Commit: —

## Validation Plan

- Tests: the differential, failure and amortization fixtures, 6 harness lines, 6
  matrix rows.
- Per-letter gate: `cargo test --bin mfb`.

## Open Decisions

None beyond plan-146-A's.

## Corrections

## Summary

D lands one arm for the six grow-shaped builtins on B's reserve/set-length
helpers: it measures and raises first, reserves (allocating only when capacity
runs out), then writes. Prefix growth is the only new move shape.
