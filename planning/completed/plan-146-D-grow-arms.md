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

- [x] Record per row: every raise point (`file:symbol`), and how `newLen` is
      computed. For `*ToWidth`, the width routine the helper calls and whether a
      native twin exists. For `resourcePath`, the base path's source and failure
      modes.

      - **`padLeft` / `padRight`** (`strings/gen_pad.rs:lower_strings_pad`). One
        `invalid` label → `raise_error("strings.padLeft"/"strings.padRight",
        "ErrInvalidArgument")`, from five conditions, all before the allocation:
        `width < 0`; `byteLen(padChar) == 0`; `padChar` is not exactly one scalar
        (`emit_scalar_count_loop`); `padChar` is not well-formed UTF-8 (decode
        width ≠ its byte length, or `emit_utf8_encoded_width` ≠ it — audit-unicode
        #7); and a size overflow in `emit_checked_size_multiply` /
        `emit_checked_size_add` / `…_add_immediate(total, 9)` (audit-unicode #2).
        Plus `raise_error_bare("ErrOutOfMemory")` on the alloc branch.
        `newLen = byteLen(value) + padCount × byteLen(padChar)`, where
        `padCount = max(0, width − scalarCount(value))` — SCALARS, not columns.
      - **`repeat`** (`strings/func_repeat.rs:lower`). `invalid` →
        `raise_error("strings.repeat", "ErrInvalidArgument")` from `times < 0`,
        from `emit_checked_size_multiply(len × times)` and from
        `…_add_immediate(total, 9)`; `ErrOutOfMemory` on the alloc branch.
        `newLen = byteLen(value) × times` exactly (`times = 0` → 0).
      - **`padLeftToWidth` / `padRightToWidth`** (`Body::Rewrite` to
        `strings/helper_pad_to_width.rs`, `__strings_padToWidthCopies`). Three
        MFBASIC `FAIL error(77050002, …)` = `ErrInvalidArgument` (declared):
        `columns < 0`; `len(padChar) <> 1` (the MFBASIC `len` — a SCALAR count, and
        unlike `gen_pad` it does not check UTF-8 well-formedness); and
        `strings::displayWidth(padChar) < 1` (a zero-column padChar). Then
        `copies = 0` when `have >= columns`, else `(columns − have) / unit`
        truncating (the undershoot rule), and the result is `value` unchanged when
        `copies = 0`. **The width routine is `strings::displayWidth`, and it has a
        native twin**: `strings/func_display_width.rs:lower` (`Body::abi_inline`,
        `errors: vec![]`) — the arm measures with it, so it does not re-implement
        the column rule. Note `unit` is a COLUMN count while the multiplier applied
        to bytes is `byteLen(padChar)`: the two differ for a wide padChar.
      - **`os::resourcePath`** (`os/func_resource_path.rs:lower_resource_path`,
        `Body::abi_function`). `{symbol}_bad_arg` → `ErrInvalidPath` from
        `os/gen_paths.rs:emit_reject_dot_component` for a `.` or `..` component of
        `relative` (component boundary: `/`, and `\` on Windows); `{symbol}_fail` →
        `ErrUnsupported` from the executable-path acquisition
        (`emit_executable_path_into`: `_NSGetExecutablePath` / `readlink("/proc/self/exe")`
        / `GetModuleFileNameW`) and from the backward separator scan running out;
        `{symbol}_alloc_error` → `ErrOutOfMemory`. The base is NOT a global or a
        constant: it is computed per call from the host executable path, then
        `strip` components are removed from its end, where
        `(strip, suffix) = os/gen_paths.rs:resource_base_offset(build_mode, module_name)`
        — `(1, "")` for Console/WindowsApp, `(2, "Resources")` for MacApp,
        `(2, "share/<module>")` for LinuxApp. `newLen = prefix_len + byteLen(relative)
        + extra`, `extra = 1` for an empty suffix else `suffix.len() + 2`.
Commit: 3fcd54a2c

### Phase 2: Split measure from build, byte-identical

- [x] `gen_pad.rs`, `func_repeat.rs`, ~~`func_resource_path.rs`~~: a measure half
      the copying lowering calls. `helper_pad_to_width.rs` is MFBASIC and is not
      split. Its measure step is new code in the arm, per §3.
      `gen_pad::pad_measure` (taking the caller's registers, `PadScratch`) and
      `func_repeat::repeat_measure` (returning the slots, labels and registers the
      copying tail uses) keep both paths' allocation order, so the split is
      byte-identical. `func_resource_path.rs` is NOT split: it builds a whole
      `CodeFunction` out of raw instructions and its own `Vregs`, which a
      `CodeBuilder` arm cannot call into — Correction D1 records what the arm does
      instead. `func_display_width.rs` gained `display_width_of` (the same walk,
      minus the constant fold) so the `*ToWidth` measure can call it.

Acceptance: `cargo build --release && cargo test --test golden` → 0 `.ncode` diffs
(est. 20 min, for the same reason as C Phase 2).
Result: `artifact-gate.sh target/release/mfb strings` → `1 tests, 6 build(s), 7
golden(s) checked, 0 diff(s)` after each split; the full gate ran green at the end
of Phase 3 (below).
Commit: 3fcd54a2c

### Phase 3: The arm

- [x] `ArmId::StrGrow`, `STRING_GROW_FNS`, `try_inplace_string_grow_assign`, the
      marker, and the `SELF_UPDATE_ARMS` entry after `StrWindow` (plus its
      `FIELD_NEVER` row at all 15 field sites, plan-146-B Correction B4).
- [x] Add the six names to `is_string_self_update` (`STRING_SHADOW_ARMS`).
- [x] The 6 rows → `Arm([StrGrow])`; 6 `cases.tsv` lines → `arm`. Each line pairs
      the grow with a C shrink that restores `x`
      (`x = strings::padRight(x, 8) ; x = strings::left(x, 3)`).
- [x] Runtime: `tests/rt-behavior/strings/self-update-grow-valid/`, with the
      differential cases and a trapped failure per raising row, at a local and a
      global. 28 differentials (width already met, multi-byte padChar, wide
      two-column values and padChars, `times` 0/1/3, the empty string, and
      `os::resourcePath` at a local, a global, empty and in a loop), 3
      grow-then-use cases, and 6 trapped failures — every line `ok` /
      `raised=TRUE s=[abc]`.
- [x] A growth-amortization case: ~~`s = strings::padRight(s, len(s) + 1)`~~
      `s = strings::padRight(s, i)` 10,000 times allocates fewer than 64 blocks
      (`arena.<k>.alloc_calls` from `--debug`). That is the geometric-regrow claim,
      measured. **12 allocations** for 10,000 growing statements (`arena.0.alloc_calls
      12`, the whole program). The plan's own statement cannot be an arm — it reads
      `s` in a later argument, which `G21-string` declines (Correction D2).
- [x] RED proof: make `padLeft`'s move run forward instead of backward. The
      differential case must fail. Restore.
      `padLeftToWidthWide MISMATCH [  ���] want [  漢字]` — the only differentials
      that fail are the ones whose regions actually overlap (a 6-byte value moved
      right by 2); the ASCII cases pad by more than they hold, so they do not.
      That is why the fixture carries a wide-character case. Restored.

Acceptance: `cargo test --bin mfb self_update`, the harness over the six lines, and
`scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/strings/self-update-grow-valid'`
→ pass (est. 8 min).
  Expected golden diffs: none from in-tree fixtures (the fixture grep in B's
  Measured populations finds no grow-row self-update in `tests/`, `examples/` or a
  helper body). Run `cargo test --test golden`. Any diff is a bug to root-cause.
Result: `cargo test --bin mfb self_update` → the matrix fires `StrGrow` for all six
rows at S1 and S2; the harness over the six lines at S1/S2 → every filter `test
result: ok`; `scripts/test-accept.sh … 'rt-behavior/strings/self-update-grow-valid'`
→ `acceptance tests passed`. Golden: `artifact-gate [all]: 1487 tests, 1662
build(s), 2102 golden(s) checked, 0 diff(s)` — no fixture holds a grow-row
self-update, exactly as predicted.
Commit: 3fcd54a2c

## Validation Plan

- Tests: the differential, failure and amortization fixtures, 6 harness lines, 6
  matrix rows.
- Per-letter gate: `cargo test --bin mfb`.

## Open Decisions

None beyond plan-146-A's.

## Corrections

- **D1 — `os::resourcePath`'s arm asks the host once, through `os::executablePath`.**
  §3 says the measure step "obtains the base path the copying lowering uses", but
  that lowering is a `Body::abi_function` helper built out of raw
  `Vec<CodeInstruction>` with its own `Vregs` (`os/func_resource_path.rs`), which a
  `CodeBuilder` arm cannot call into, and calling the helper itself allocates a
  block per statement (the harness bound rejects that). What the arm does instead,
  all of it builder-side:
  1. rejects a `.`/`..` component of the binding with the same rule
     (`emit_reject_dot_components`, `ErrInvalidPath`) before anything is written;
  2. calls `os::executablePath()` — no arguments, so no string literal — ONCE per
     call of the function holding the self-update, caching the block in a frame slot
     (`prescan_string_resource_base`, freed by the scope drop like the self-update
     scratch). A running process's own executable path cannot change, which is why
     once is enough; the copying lowering still asks per call.
  3. rebuilds the prefix (`<exe dir>` + `/` + the app mode's suffix + `/`) in the
     function's self-update scratch — so `resourcePath` joins `SCRATCH_ARMS` — with
     `strip`/`suffix` from the same `os/gen_paths.rs:resource_base_offset`; the
     suffix bytes are compile-time constants written as immediates, since a
     synthesized literal has no data object ("native code string literal '' has no
     data object", the first attempt);
  4. moves the binding's bytes right and copies the prefix in front of them.
  Two supporting changes: `CodeBuilder` now carries the module name (the LinuxApp
  suffix is `share/<module>`), and `runtime_symbols` (`target/shared/plan/symbols.rs`)
  pulls in `os.executablePath` for a module holding a `resourcePath` self-update —
  a runtime call no NIR op names, so the plan would otherwise not emit the helper
  ("internal relocation target `_mfb_rt_os_os_executablePath` is not defined").
  Verified against the copying path at a local, a global, an empty relative path and
  in a loop, plus the `..` rejection (fixture `self-update-grow-valid`).
- **D2 — the plan's amortization statement is not an arm.**
  `s = strings::padRight(s, len(s) + 1)` reads the binding in a LATER argument, which
  `G21-string` declines by design (the arm would be measuring bytes it is about to
  rewrite). Measured with `s = strings::padRight(s, i)` instead — same growth, one
  byte per iteration: 10,000 statements, `arena.0.alloc_calls 12`.
- **D3 — an `abi_function` member's call site is a `RuntimeCall`, not a `Call`.**
  `s = os::resourcePath(s)` reaches NIR as `{"kind": "runtimeCall", "target":
  "os.resourcePath"}` (`grep -n 'os.resourcePath' p146nir.nir`), so every gate that
  matched `NirValue::Call` saw nothing: the resolver's G2, `is_string_self_update`,
  `is_self_update_call`/`is_global_self_update_call` (which build the S2/S9
  destinations) and `ops_hold_self_update` (the prescans). They now go through one
  `self_update_call_parts(value)`, which answers for both node shapes. Without it
  the arm never fired (`os::resourcePath at Local: fired none of [StrGrow]`).
- **D4 — the default padChar is a stack block in the arm.** `strings::padRight(s, n)`
  without a padChar materializes a one-byte `" "` ARENA block in the copying
  lowering (`gen_pad.rs`, bug-536 shape B) — an allocation per statement, which the
  arm cannot pay. The arm builds the same `[len][bytes][NUL]` shape in a 16-byte
  frame object instead (`string_pad_char_slot`); every reader of a `String` only
  ever wants that shape at the pointer.

## Summary

D lands one arm for the six grow-shaped builtins on B's reserve/set-length
helpers: it measures and raises first, reserves (allocating only when capacity
runs out), then writes. Prefix growth is the only new move shape.
