# plan-146-B: The `String` seam, one shadow rule, and `s = toString(s)`

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-146-A

Prerequisites: see plan-146-A (they gate every letter).

plan-142's seam serves `String` badly in three places (findings §3.5). This letter
fixes all three and then lands the smallest `String` arm, which proves the seam
end to end:

1. **G10.** `resolve_self_update` gates on `CollectionTypeLayout::from_type`, which
   has no `String` case, so a `String` arm cannot use it.
2. **Spellings.** `self_update_builtin` names only `Body::abi_inline`/`Intrinsic`
   natives and `#collections_*` monomorphs. The `Rewrite` rows
   (`padLeftToWidth`, `padRightToWidth`) and the `abi_function` row
   (`os::resourcePath`) never build an S2 or S9 site.
3. **The shadow.** A capacity shadow exists only for `&` targets, found by two
   prescans that each key on a `&` chain (`prescan_string_self_appends`,
   `add_global_string_capacities`). A shrink, grow or rewrite arm leaves the block
   at a size `byteLength + 9` no longer describes (findings B.3 fact 1), so every
   such target needs a shadow too.

After B, `s = toString(s)` on a `String` emits nothing at S1 and S2. Since bug-667,
the statement copies `s` and frees the old block. `toString` of a `String` is the
identity, so the in-place form is a no-op.

## 1. Goal

- `resolve_string_self_update(site, value, name, arity, needs_shadow)` exists and every `String`
  arm in C–G gates through it (§3).
- One predicate, `is_string_self_update(value, root)`, decides which `String`
  bindings get a shadow. Both prescans call it. At the end of Phase 2 it accepts
  exactly the `&` chains it accepts today, so the refactor is byte-identical. Each
  arm letter widens it by adding that letter's shapes.
- `self_update_builtin` answers the bare name for every call-target spelling of
  the 26 rows C–E arm, and for `toString`. The spellings are recorded in Phase 1.
- `ArmId::StrIdentity` fires for `s = toString(s)` at S1 and S2. Its `cases.tsv`
  line flips `pending:B` → `arm`.

### Non-goals

- No arm for any row except `toString`.
- The concat arm's behavior is unchanged. Its shadow code moves into shared
  helpers byte-identically.
- S9 stays declined for `String` (letter G).
- `toString` of any other type is untouched.

## 2. Current State

- The concat arm (`builder_inplace_assign.rs:1609`) runs its own gates inline: G1
  (`site.by_ref`), shadow present (frame slot or hidden global), the `&` chain
  (G20), no later operand reads `s` (G21), and `G-global-operand` (no operand
  stores to the global). `lower_string_self_append_one` (`:1688`) holds the
  regrow: geometric capacity, copy, free the old block at `len + 9 + shadow`,
  repoint the slot, and update the shadow.
- `prescan_string_self_appends` (`builder_control.rs:2389`) claims `strcap_<name>`
  for a local that is a `&` target and is not in `address_taken_locals`.
  `add_global_string_capacities` (`self_update.rs:285`) declares `$strcap$<name>`
  for a global `String` that is a `&` target. Both are keyed by the `&` shape
  (`string_self_append_operands[_of]`).
- `self_update_builtin` (`self_update.rs:214`): `native_builtin_target` bare names,
  then `#collections_*`/`collections.*`. It answers `None` for everything else.
- `is_self_update_call` / `is_global_self_update_call` (`:255`, `:262`) decide
  whether an S9/S2 statement gets a `Ref`/`Global` destination. Both test
  `self_update_builtin(target).is_some()`.
- `lower_to_string`'s `String` arm (`builder_strings.rs:837`, the bug-667 hunk at
  `:924-944`) copies unless the argument is the statement's own pending
  temporary.

### Measured populations

| What | Count | Command |
|---|---|---|
| In-tree fixtures holding a `String` self-update of a plan-146 row | 25 lines in 8 files (`tests/`: 2 files, both bug-667's; `examples/`: 6) | `bash /tmp/p146list.sh` (Appendix) → the `.mfb` half |
| MFBASIC helper bodies in `src/` holding one | 8 lines in 4 files: `encoding/func_html_escape.rs:43-47` (`out = strings::replace(out, …)` ×5), `json/helper_round_digits.rs:59` (`strings::left`), `http/helper_multipart_boundary.rs:22` (`strings::trim`), `string/repr/builder_strings.rs:931` (a comment) | same script, the `.rs` half |
| Call-target spellings of the 26 arm rows and `toString` after lowering | 4 kinds, 26 rows: 23 `<pkg>.<member>` natives (`strings.left`, …, `fs.pathNormalize`), 2 `#strings_<member>` (`padLeftToWidth`, `padRightToWidth`), 1 `os.resourcePath`, and the bare `toString` | `/tmp/p146_nir.py` (Phase 1): one project holding all 27 statements at S1 and S2, `mfb build --nir`, `grep -oE '"target": ?"[^"]*"' p146nir.nir | sort | uniq -c` |

The helper-body lines matter for golden churn. Once C and E land, every program
that calls `encoding::htmlEscape`, formats a JSON number through
`helper_round_digits`, or builds a multipart boundary runs an arm inside the
helper. Those goldens are expected to shift in C (`left`, `trim`) and E
(`replace`).

## 3. Design

**`resolve_string_self_update`** (new, in `self_update.rs`, beside
`resolve_self_update`). It returns the parsed call, or declines, having emitted
nothing, in this order:

| gate | declines when |
|---|---|
| `G-string-dest` | `site.dest` is not `Direct`, `Global` or `Ref` (a field destination) |
| `G-string-type` | `site.type_ != ParameterType::String` |
| G2 | `value` is not a `Call` |
| G3 / G4 | `self_update_builtin(target) != Some(name)`, or the arity is wrong |
| G5 / G6 | `args[0]` is not the binding (`site.is_self`) |
| G21-string | a later argument reads the binding (`site.read_by`). The arm would read bytes it is rewriting. Today's copying path reads every argument before the write. |
| G1 | `site.by_ref` and the destination is not `Ref` (letter G adds the shared shadow; until then, G1 declines every `Ref`) |
| `G-shadow` | `needs_shadow` and the binding has no shadow (no frame slot and no hidden global) |
| `G-global-operand` | S2 only: an argument reaches a store to the global (`values_reach_store`, as the concat arm does) |

The concat arm keeps its own gates. It is an operator, not a call, and G20 is its
shape test. It moves onto the shared shadow helpers below.

**Shared shadow helpers** (moved out of `lower_string_self_append_one`, same
emission):

- `string_shadow_slot(site)`: the frame slot, or the hidden global loaded into
  `concat_global_strcap`, as the concat arm does today. A `publish` step stores a
  global's shadow back after the arm.
- `emit_string_reserve(site, need_len_slot)`: make the block hold at least
  `need_len` bytes. It either fits in `len + shadow`, or it regrows geometrically
  (allocate, copy `len` bytes, free the old block at `len + 9 + shadow`, repoint).
  The allocation comes before any write, so `ErrOutOfMemory` leaves `s` unchanged.
- `emit_string_set_len(site, new_len_slot)`: `shadow += oldLen − newLen`, store the
  length, write the NUL.

**One shadow predicate.** `is_string_self_update(value, root: &dyn Fn(&NirValue) -> bool) -> bool`
answers true for a `&` chain rooted at the binding (today's rule). Each arm letter
adds its call shapes, `Call` with `self_update_builtin(target) ∈ STRING_ARM_NAMES`
and `args[0]` the root. `prescan_string_self_appends` and
`add_global_string_capacities` both call it, so a local and a global get a shadow
under the same rule. `STRING_ARM_NAMES` starts as `["toString"]`. `toString` needs
no shadow, so it is excluded from the shadow predicate. The list exists for C to
extend.

**The identity arm.** `try_inplace_string_identity_assign`: through the resolver
with name `toString`, arity 1, and no `G-shadow` (it changes no length). Then it
emits a marker slot (`inplace_str_identity`) and nothing else. It returns `true`.
The old block stays, and nothing is freed or stored. At S2 the seam's
`open_inplace_ref_dest` load and `close_inplace_dest` store are the only code,
and the store writes back the same pointer.

Risk: low. The refactor is byte-identical by construction, and the identity arm
removes code. The correctness question is only whether `toString(s)` of a
`String` is ever not the identity. The spec and `.ai/codegen-invariants.md` both
say it is ("`toString(String)` is the IDENTITY arm"). Phase 3's runtime case
checks it.

Rejected alternatives:

- **Teach `resolve_self_update` a `String` case.** It is built around
  `CollectionTypeLayout` (G7, G10, the iterator gates). A `String` case would be a
  second resolver inside the first.
- **A shadow for every `MUT` `String`.** Every local and global `String` would gain
  a slot and a reset on every store, which shifts nearly every golden for no
  benefit where no arm runs.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. `- [~]` partial. Moot tasks struck through with evidence, never
> deleted. **An unticked box means NOT DONE.**

### Phase 1: Spellings (measure first)

- [x] One `mfb build --nir` probe holding `s = f(s, …)` for each of the 26 rows C–E
      arm (the Pending C/D/E rows) and `s = toString(s)`, at S1 and S2. Record the
      call-target spellings (`grep -oE '"target": ?"[^"]*"'`) here, one per row.
      Each appears twice (S1 + S2), which is the prediction that both sites lower
      the same target:

      | spelling | rows |
      |---|---|
      | `strings.<member>` | `left right mid stripPrefix stripSuffix trim trimStart trimEnd trimChars graphemeAt padLeft padRight repeat upper lower caseFold normalizeNfc replace` (18) |
      | `fs.<member>` | `pathBaseName pathDirName pathExtension pathNormalize` (4) |
      | `#strings_padLeftToWidth`, `#strings_padRightToWidth` | the two `Body::Rewrite` rows (2) |
      | `os.resourcePath` | the `Body::abi_function` row (1) |
      | `toString` | the identity row |

      (The dump also holds `strings.displayWidth`, `strings.repeat` ×2 more and
      `#strings_padToWidthCopies` from inside the `*ToWidth` helper bodies — not
      call targets of a self-update statement.)
- [x] Extend `self_update_builtin` to answer each recorded spelling's bare name
      (expected: `Rewrite` targets such as `#strings_padLeftToWidth`, and
      `abi_function` targets such as `os.resourcePath`). Extend
      `self_update_builtin_names_every_spelling` with one assertion per new
      spelling, plus a negative (`#strings_nope`).
      Only 4 of the 27 needed one (`STRING_SELF_UPDATE_SPELLINGS`): the other 23 are
      `Body::abi_inline`/`Intrinsic` natives `native_builtin_target` already
      dequalifies. The test now asserts all 26 rows plus `toString`, and two
      negatives: `#strings_nope` and `#strings_padToWidthCopies` (a `Rewrite`
      helper that is not a row).

Acceptance: `cargo test --bin mfb self_update_builtin_names_every_spelling` passes
with the new assertions (est. 5 min).
Commit: —

### Phase 2: Resolver and shadow helpers, byte-identical

- [x] `resolve_string_self_update` with the gates in §3 (no caller yet besides a
      unit test that runs each gate's decline on a hand-built `NirValue`).
      `resolve_string_self_update_runs_every_gate` (`string_self_update.rs`) builds
      a `CodeBuilder` through `BuilderHarness` and asserts one decline per gate
      (`G-string-dest`, `G-string-type`, G2, G3, G4, G5/G6, `G21-string`, G1,
      `G-shadow` at a local and at a global) plus the all-pass case.
- [x] Move the shadow code out of the concat arm into `string_shadow_slot`,
      `emit_string_reserve` and `emit_string_set_len`. The concat arm calls them.
      Landed as `string_shadow_exists` / `string_shadow_slot` /
      `publish_string_shadow` and `emit_string_regrow` (the regrow the concat arm
      has always emitted, now shared and parameterized by the caller's slots,
      registers, labels and a `fill` hook). `emit_string_reserve` and
      `emit_string_set_len` are built on `emit_string_regrow` but have no caller
      until letter C, so they land with it (Correction B1).
- [x] `is_string_self_update`, called by both prescans. It accepts exactly today's
      `&` shapes. (`STRING_SHADOW_ARMS` is empty until letter C; the unit test
      `is_string_self_update_accepts_exactly_the_self_append_today` pins that.)

Acceptance: codegen is unchanged.
  Check: `cargo build --release && cargo test --test golden` → pass, 0 `.ncode`
  diffs (est. 20 min: the artifact gate is the only check that sees every concat
  site's emission, and every function's prescan). A diff is a refactor bug:
  objdump one fixture and fix it.
Result: `artifact-gate [all]: 1485 tests, 1660 build(s), 2098 golden(s) checked,
0 diff(s)`; `test result: ok. 1 passed` (191.08 s).
Commit: —

### Phase 3: The identity arm

- [ ] `ArmId::StrIdentity`, `try_inplace_string_identity_assign`, its marker
      `inplace_str_identity`, and its entry in `SELF_UPDATE_ARMS` after the
      collection arms.
- [ ] The `toString` row: `Pending("B")` → `Arm([StrIdentity])`. Its `cases.tsv`
      line: `pending:B` → `arm`.
- [ ] Runtime: `tests/rt-behavior/general/tostring_string_owning_store` (bug-667's
      fixture) passes unchanged. Its statements are now arm sites at S1 and S2. Add
      one case to it that prints `s` after `s = toString(s)` in a loop of 3 at a
      global.
- [ ] RED proof: make the arm return `false` and confirm the matrix and the harness
      line fail. Restore.

Acceptance: `cargo test --bin mfb self_update && MFB_SELF_UPDATE_FILTER=toString cargo test --test rt_inplace_self_update && scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/general/tostring_string_owning_store'`
→ pass (est. 5 min).
  Expected golden diffs: only fixtures holding `s = toString(s)`. That is
  `tostring_string_owning_store` alone (Measured populations). Check it with
  `cargo test --test golden` after this phase only if that fixture has a committed
  `.ncode` golden (`ls tests/rt-behavior/general/tostring_string_owning_store/`).
  Otherwise the acceptance run above covers it.
Commit: —

## Validation Plan

- Tests: the resolver's gate unit test, the spelling assertions, the identity arm
  in the matrix and the harness, bug-667's fixture.
- Coverage check: the matrix enumerates every `Arm` row, so `toString` is in its
  denominator from Phase 3.
- Per-letter gate: `cargo test --bin mfb`.

## Open Decisions

None beyond plan-146-A's.

## Corrections

- **B1 — `emit_string_reserve` / `emit_string_set_len` land with letter C, their
  first caller.** Phase 2 has no caller for them, and an unused method is dead code
  AGENTS.md forbids ("never 'consumed by a later phase'"). They are written and
  built on `emit_string_regrow` (this phase's shared regrow); letter C lands them
  with the window arm that calls them. Nothing about the seam's shape changes.
- **B2 — the concat arm's regrow is shared through a caller-owned frame.**
  §3's "move the shadow code into `emit_string_reserve`" cannot be literal and
  byte-identical at once: the concat regrow interleaves the operand copy between
  the old-bytes copy and the free, and allocates its slots/registers/labels in its
  own order, all of which the `.ncode` records. So the shared routine
  (`emit_string_regrow`) takes a `StringRegrow` of the caller's slots, registers
  and labels plus a `fill` hook for what follows the copied bytes; the concat arm
  passes its own (unchanged order) and `emit_string_reserve` passes its own. The
  artifact gate confirms it: 2098 goldens, 0 diffs.
- **B3 — Phase 1's new spellings move no golden.** Naming `toString`,
  `#strings_pad*ToWidth` and `os.resourcePath` makes S2/S9 build a self-update site
  for them (observation O1's dead load) even before an arm exists. The only in-tree
  fixture with such a statement is `tests/rt-behavior/general/tostring_string_owning_store`
  (`grep -rnoE '\b([a-zA-Z_][a-zA-Z0-9_]*) = (toString|strings::padLeftToWidth|strings::padRightToWidth|os::resourcePath)\(\1\b' --include='*.mfb' tests examples benchmark` → no hit; the fixture's `g = toString(g)` and its lambda are found by the narrower `= *toString\(` grep), and it carries no `.ncode`/`.ncodesum` golden. Phase 2's gate agrees: 0 diffs.

## Summary

B builds the `String` half of the seam: a resolver that does not depend on a
collection layout, shadow helpers shared with the concat arm, one rule for who
gets a shadow, and the call-target spellings. The first arm is a no-op for
`s = toString(s)`. The refactor is byte-identical, and the only behavior change is
the removal of a copy.

## Appendix: the fixture grep (`/tmp/p146list.sh`)

```sh
re='strings::(left|right|mid|stripPrefix|stripSuffix|trim|trimStart|trimEnd|trimChars|graphemeAt|upper|lower|caseFold|normalizeNfc|replace|padLeft|padRight|repeat)|fs::(pathBaseName|pathDirName|pathExtension|pathNormalize)|encoding::[a-zA-Z]+|net::percentDecode|regex::replace|fs::readText|io::input|os::getEnv|toString'
grep -rnoE "\b([a-zA-Z_][a-zA-Z0-9_]*) = ($re)\(\1\b" --include='*.mfb' tests examples
echo ---
grep -rnoE "\b([a-zA-Z_][a-zA-Z0-9_]*) = ($re)\(\1\b" --include='*.rs' src
```

Run 2026-09-21 at `2fcfda48e`: 25 `.mfb` lines (8 files), 8 `.rs` lines (4 files).
