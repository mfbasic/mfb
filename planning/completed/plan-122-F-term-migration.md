# plan-122-F: `term` adopts `color::Color`; `TermColor` is retired

Last updated: 2026-09-02
Effort: large (3h–1d)
Depends on: plan-122-E

The last letter, and the only one that touches hand-written machine-code emission.
`term::TermColor` is deleted. `term::getForeground`/`getBackground` return a
`color::Color`; `term::setForeground`/`setBackground` take one. `TermColor`'s
reserved wire type id, its resolver seed, its read-only-record rule and its
storage-class row all go with it.

Behavioral outcome: a TUI program writes
`term::setForeground(color::fromHex("#ff8800"))`, reads it back with
`term::getForeground()` as a `color::Color` with `alpha = 255`, and every existing
term fixture renders the same cells — on macOS AArch64, on the three Linux arches
and on Windows x86-64.

References:

- plan-122-A — Prerequisites; `COLOR_TYPE_ID`; and the **measured finding that
  `term` must not gain `add_imports(["color"])`** (§2, "The trigger is a non-empty
  companion, not `add_imports`"), which is what keeps every TUI binary at its
  current size.
- `.ai/arch-abi.md` — per-architecture ABI traps. Read before editing
  `src/codegen/term/core/term.rs`.
- `.ai/codegen-invariants.md` — record layout and vreg rules.
- `.ai/remote_systems.md` — the Linux (2223) and Windows (2230) boxes. A per-backend
  change is not proven by lowering; it is proven by running there.
- `src/docs/spec/app/04_term-backend.md` — the term backend specification.

## Prerequisites

Stated once in plan-122-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-122-E complete | `ls planning/completed/plan-122-E-*` → one match | **MET** (2026-09-04) — E landed across `cf2489188`..`4efd0f03a`, ledger closed at `414c6b92d`, with 0 unticked boxes and 0 unfilled `Commit:` lines. Measured on the ledger and E's gates rather than the archive path, which is only written at merge time. **This dependency is real, unlike E's on D**: the term↔astrings bridge reads the packed payload whose bit layout E widened. |
| The Linux box 2223 and Windows box 2230 are reachable | per `.ai/remote_systems.md` | **MET** (2026-09-04) — both probed. 2223 answers `uname -m` → `aarch64`; 2230 answers. **2223 is aarch64 and has no qemu**, so it cannot run x86-64 or riscv64; Phase 4 used the boxes that run each arch natively instead — see its corrected routing table. |

If plan-122-E is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- `term::TermColor` does not resolve. `term::getForeground()` and
  `term::getBackground()` return `color::Color`; `term::setForeground` and
  `term::setBackground` take a single `color::Color`.
- `term` still declares **no** `add_imports`, so a program that imports `term` and
  not `color` costs exactly what it costs today (measured baseline: `IMPORT io` +
  `IMPORT term` = 66,596 bytes, identical to `IMPORT io` alone).
- Every term fixture renders identical cells on macOS AArch64, Linux (x86-64,
  aarch64, riscv64) and Windows x86-64.

### Non-goals (explicit constraints)

- **`term` gains no source companion.** Every member stays `Body::abi_function`
  (verified: `grep -c 'Body::mfb' src/codegen/builtins/term/func_*.rs` → 0 across
  all of them). An MFB overload would give `term` a code-bearing companion, which
  would drag `color`'s companion into every TUI binary — the exact regression
  plan-122-A measured and this letter exists to avoid.
- **`term` still has no alpha.** A terminal cell has no alpha channel.
  `setForeground` ignores `base.alpha`; `getForeground` always reports `255`. Both
  are stated on both man pages.
- **`TermSize` is untouched.** It keeps its reserved wire id and its read-only
  rule; only `TermColor` is retired.
- **No change to the term-state packed slot's meaning.** The slot keeps the
  `0xBBGGRR` order the emitters use today (`emit_set_color`,
  `src/codegen/term/core/term.rs:576-585`) — **note this is the opposite byte order
  from `astrings`' `0xAARRGGBB` and from `color::toPacked`.** F does not unify it;
  it is a private slot layout, and changing it would churn the ANSI emitters for no
  observable gain.
- **No change to what the terminal receives.** The ANSI truecolor sequences are
  byte-identical.

## 2. Current State

`TermColor` is a compiler-owned, **runtime-allocated, read-only** record: a program
may neither construct nor `WITH`-update one
(`src/codegen/builtins/term/mod.rs:99-108`). That is the single largest difference
from `color::Color`, which is an ordinary value record, and it is why retiring
`TermColor` *removes* rules rather than moving them.

### Every site, read

| Site | File:line | Change |
|---|---|---|
| record declaration | `term/mod.rs:190-214` | delete |
| `TERM_COLOR_TYPE` / `TERM_COLOR_TYPE_ID` consts | `term/mod.rs:89`, `:95` | delete |
| `is_read_only_record` | `term/mod.rs:105-107` | drop the `TermColor` disjunct; keep `TermSize` |
| package `DESC` | `term/mod.rs:149-150`, `:171-174` | rewrite the colour paragraphs |
| `getForeground` return type | `term/func_get_foreground.rs:103` | `COLOR_TYPE_ID` |
| `getBackground` return type | `term/func_get_background.rs:100` | `COLOR_TYPE_ID` |
| `setForeground` params | `term/func_set_foreground.rs:103-126` | one `COLOR_TYPE_ID` param replacing three `Byte`s |
| `setBackground` params | `term/func_set_background.rs` (same shape) | same |
| record size | `src/codegen/term/core/term.rs:54` | `TERM_COLOR_RECORD_SIZE: 24` → `32` (4 fields × 8) |
| record allocation + field stores | `src/codegen/term/core/term.rs:2069-2124` (`emit_get_color`) | store alpha `255` at offset 24 |
| arg unpacking | `src/codegen/term/core/term.rs:568-592` (`emit_set_color`) | load 4 fields from the record pointer in `c_arg(0)` instead of 3 scalar args |
| storage class | `src/target/shared/plan/lower.rs:209` | remove `"TermColor"` from the reference-type list |
| wire id constant | `src/binary_repr/mod.rs:135` | see Open Decisions |
| wire encoder arm | `src/binary_repr/sections.rs:173-178` | delete |
| wire reader arm | `src/binary_repr/reader.rs:890` | see Open Decisions |
| resolver seed | `src/resolver/mod.rs:41` | delete the `TERM_COLOR_TYPE_ID` entry |
| bridge saved-pen type | `term/helper_astrings_bridge.rs:111`, `:119`, `:133-134` | `term::TermColor` → `color::Color`; `setForeground(saved.r, saved.g, saved.b)` → `setForeground(saved)` |
| Rust tests naming it | `src/ir/verify/tests.rs:8855-8856`, `src/ir/tests.rs:6234-6248`, `src/binary_repr/tests/sections_tests.rs:108-129`, `src/binary_repr/tests/reader_gap_tests.rs:91` | rewrite or delete per the four-question gate |
| comment | `src/ast/items.rs:615` | update the example it cites |

`emit_get_color` currently allocates 24 bytes and stores three channels at offsets
0, 8 and 16, unpacking the state slot as `packed & 255` → r, `>>8 & 255` → g,
`>>16 & 255` → b (`src/codegen/term/core/term.rs:2098-2118`). `emit_set_color`
packs the mirror image from `c_arg(0..2)` (`:576-585`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `TermColor` mentions (all spellings incl. `TERM_COLOR_*`), `src/` | 38 | `grep -rn 'term::TermColor\|term\.TermColor\|TERM_COLOR' src \| wc -l` |
| same, `tests/` | 63 | same over `tests` |
| same, `planning/` + `bugs/` | 6 + 2 | same |
| `.mfb` files using `TermColor`/`get`/`setForeground`/`setBackground` | 23 | `grep -rl 'term::TermColor\|term::getForeground\|term::getBackground\|term::setForeground\|term::setBackground' --include='*.mfb' . \| wc -l` |
| — of those, examples | 6 | `ai_chat`, `browser/app`, `hangman`, `life`, `snake`, `wide-demo` |
| — of those, `tests/rt-behavior/term` | 8 | from the same list |
| — of those, `tests/syntax/term` | 5 | from the same list |
| golden fixture dirs, `tests/rt-behavior/term` + `tests/syntax/term` | 14 + 23 | `ls -d tests/rt-behavior/term/*/ \| wc -l; ls -d tests/syntax/term/*/ \| wc -l` |
| `.ncodesum` files under `tests/byte-identity/term` | 5 | `find tests/byte-identity/term -name '*.ncodesum' \| wc -l` |
| `.mfb` fixtures importing `term` | 45 | `grep -rl '^IMPORT term' --include='*.mfb' tests/ examples/ \| wc -l` |

### Verified properties

- **`term` is 100% native.** No `func_*.rs` in `src/codegen/builtins/term/` carries
  a `Body::mfb`; the one source-shaped thing in the package is the `astrings`
  bridge, which is a `HelperGate::WhenBothImported` chunk with its own inline
  `IMPORT` block (`term/helper_astrings_bridge.rs:1-15`, `:44-47`) — not a member
  body. This is what makes the no-companion constraint achievable.
- **`IMPORT term` costs zero today.** `IMPORT io` = 66,596 bytes and
  `IMPORT io` + `IMPORT term` = 66,596 bytes (plan-122-A §2). This number is the
  regression gate for Phase 5.
- **A native member can name a foreign record.** `tcp::localAddress` is
  `Body::abi_function` and returns `net.Address` through a qualified type-id
  constant (`src/codegen/builtins/tcp/mod.rs:103-112`,
  `tcp/func_local_address.rs:71`). That is exactly the shape `getForeground` takes.
- **`canvas::Color` is already classified correctly by the storage planner**
  without an entry in the `"TermColor" | "TermSize" | …` list
  (`src/target/shared/plan/lower.rs:203-210`) — it falls through to
  `is_user_type_name` at `:177`. `color.Color` is the same spelling shape, so it
  should behave identically. **UNVERIFIED until Phase 1 runs it**; if it does not,
  the fix is a row in that list, not a redesign.
- **UNVERIFIED — whether the emitters run identically on all five targets.**
  `emit_get_color`/`emit_set_color` are shared
  (`src/codegen/term/core/term.rs`), gated per target by
  `SUPPORTED_RUNTIME_CALLS` (`src/target/linux_common/mod.rs:156`,
  `src/target/macos_aarch64/mod.rs:141`, `src/target/win_x86_64/mod.rs:76`).
  Shared emission is not proof of shared behavior — Phase 4 runs them.

## 3. Design Overview

Four layers, and the risk is concentrated in exactly one of them.

1. **Descriptor.** Signature changes on four members plus the record and const
   deletions. Mechanical.
2. **Native emission (the risk).** `emit_get_color` grows a fourth field;
   `emit_set_color` changes from three scalar args to one record pointer. This is
   hand-written machine code with no type checker: a wrong offset does not fail to
   build, it reads or writes the wrong eight bytes.
3. **Compiler-wide plumbing.** Wire id, resolver seed, storage class, read-only
   rule. Each is a single site; each fails loudly if missed except the storage
   class, which fails as `native plan has no storage class for type 'color.Color'`
   — loud enough.
4. **Fixtures, examples and docs.** The volume: 37 fixture dirs, 6 examples, 5
   `.ncodesum` files.

**Where correctness risk concentrates:** layer 2, and specifically `emit_set_color`.
Today it receives three `Byte` values in `c_arg(0..2)`; after the change it receives
one pointer in `c_arg(0)` and must load four `u64` slots. Per
`.ai/resources-packages.md`, the first MFB argument arrives in
`abi::return_register()`, not `c_arg(0)` — **which of the two applies here must be
read out of the existing `abi_function` wrapper, not assumed**, and getting it wrong
produces a SIGSEGV at a tiny address (the `stage-abi-args-via-temporaries` failure
signature).

**Where design uncertainty concentrates:** whether `color.Color` is storage-planned
as a reference type without a hand-written row. Phase 1 answers it in minutes with a
one-member spike, before any emitter is edited.

**Byte-identity is not the gate.** Every term `.ncode`/`.ncodesum` is expected to
drift; the win condition is that rendered cells are unchanged on five targets. A
codegen-inspection test that reds with a stale offset after the record grows from
24 to 32 bytes is a **stale constant**, not a regression — the
`codegen-inspection-tests-hardcode-drifting-constants` case — but each one must be
read and confirmed as such, never bulk-updated.

### Rejected alternatives

- **An MFB `setForeground(color::Color)` overload forwarding to the native
  three-`Byte` form.** This is the cheap version and it is rejected on a measured
  ground: it makes `term`'s companion code-bearing, which pulls `color`'s whole
  companion into every TUI binary. `term` costs importers 0 bytes today and must
  keep doing so.
- **Keeping `TermColor` as an alias of `color::Color`.** Rejected (user decision,
  2026-09-02) and technically awkward: `TermColor` is read-only and 3-field,
  `color::Color` is constructible and 4-field. They are not aliasable.
- **Keeping `setForeground(r, g, b)` alongside the `Color` form.** Rejected: it is
  the two-ways-to-say-one-thing this plan removes, and it would keep the
  three-scalar emitter path alive alongside the record one.

## Compatibility / Format Impact

**Breaking.** `term::TermColor` is removed; `term::setForeground`/`setBackground`
take one argument instead of three; `term::getForeground`/`getBackground` return a
different type. Migration: add `IMPORT color`, wrap channels in `color::rgb(...)`,
and rename the field reads `c.r`/`c.g`/`c.b` → `c.red`/`c.green`/`c.blue`.

**Wire format:** `TYPE_TERM_COLOR` (`0xffff_fefd`) stops being emitted. See Open
Decisions for whether the id is retired or reserved.

**Unchanged:** the ANSI byte stream, `TermSize`, the term-state slot layout, and
the size of a `term`-only binary.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work; `- [~]` for partial with a line on what remains;
> `- [x] ~~text~~ — moot: <evidence>` rather than deleting. Fill `Commit:` on
> landing. **An unticked box means NOT DONE.**

### Phase 1 — Spike: prove `color.Color` crosses the native boundary

Falsify the one unproven premise before touching an emitter. Land nothing but the
answer.

- [x] On a scratch branch, change **only** `term::getForeground`'s `return_type` to
      `COLOR_TYPE_ID` and `emit_get_color`'s allocation to 32 bytes with a fourth
      store of `255` at offset 24. Build a program that calls it and reads
      `.red`/`.alpha`.
- [x] Record which of these happens: it works; or it fails with
      `native plan has no storage class for type 'color.Color'` (fix: a row at
      `src/target/shared/plan/lower.rs:203-210`); or it fails elsewhere (record
      what, in Corrections).
      **IT WORKS, first try, with no storage-class row.** The spike program
      (`term::on`, `setForeground(1,2,3)`, `LET c AS color::Color =
      term::getForeground()`, print all four channels) built clean and printed
      **`1 2 3 255`** on macOS AArch64.
- [x] Read the existing `abi_function` wrapper for `term::setForeground`
      (`term/func_set_foreground.rs:73-95` and `code::lower_term_helper`) and write
      down, in Corrections, **which register the first MFB argument actually
      arrives in** — `abi::return_register()` or `c_arg(0)`. Do not proceed to
      Phase 3 on an assumption.
      **`c_arg(0)`.** Read off the shipped emitter, not inferred.

Acceptance: **MET.** Both answers recorded with the evidence behind each — see
Corrections. The spike was kept rather than discarded: it *is* the Phase-2
`getForeground` return type plus the Phase-3 `emit_get_color` change, both already
proven to work together, so discarding and retyping them would have added risk
rather than removed it. They land in their own phases' commits.
Commit: a56daca28 (spike kept; landed with Phase 2/3)

### Phase 2 — Descriptor and plumbing

Everything except the emitters, so a build failure here cannot be confused with a
machine-code fault.

- [x] `term/mod.rs`: delete the `TermColor` record, `TERM_COLOR_TYPE`,
      `TERM_COLOR_TYPE_ID`; drop the `TermColor` disjunct from `is_read_only_record`;
      rewrite the colour paragraphs of `DESC` (`:149-150`, `:171-174`).
- [x] `func_get_foreground.rs`, `func_get_background.rs`: `return_type` →
      `COLOR_TYPE_ID`; rewrite `INTRO`/`DESC`/`EX` (the current prose says "you
      never build one", which stops being true).
- [x] `func_set_foreground.rs`, `func_set_background.rs`: one `base` param typed
      `COLOR_TYPE_ID`; rewrite prose; state that `alpha` is ignored.
- [x] `src/resolver/mod.rs:41` — delete the `TERM_COLOR_TYPE_ID` seed.
- [x] `src/target/shared/plan/lower.rs:209` — remove `"TermColor"`; add
      `color.Color` **only if** Phase 1 showed it is needed.
      Removed; **not** added — Phase 1 proved it is not needed.
- [x] `src/binary_repr/sections.rs:173-178` — delete the encoder arm.
      `src/binary_repr/mod.rs:135` and `reader.rs:890` per Open Decisions.
      Followed the recommended **reserve, don't recycle**: encoder arm deleted, the
      constant and the reader arm kept with comments marking the id retired, and a
      new `no_encoder_emits_the_retired_term_color_id` test pinning that nothing
      produces it — including that `color.Color` lands in the ordinary per-package
      band rather than inheriting the reserved id.
- [x] Rewrite the four Rust test sites (`src/ir/verify/tests.rs:8855`,
      `src/ir/tests.rs:6234`, `src/binary_repr/tests/sections_tests.rs:108`/`:127`,
      `src/binary_repr/tests/reader_gap_tests.rs:91`). Each is a **behavioral** test
      — run the four-question gate on each before touching it, and record the answer
      in the commit message. `ir/tests.rs:6234` in particular protects "field access
      on a builtin record lowers", which is still true and should be re-pointed at
      `color::Color`, not deleted.
      Four-question gate run on each; outcomes in the commit message and Corrections.
      Two rows deleted (assertions about a type that no longer exists), one
      re-pointed **and strengthened**, one kept **unchanged** because it protects the
      retirement contract itself, and two new assertions added.

Acceptance: **MET.** `cargo check --all-targets` clean. The four rewritten/added
unit tests pass (`read_only_records_are_refused_under_either_spelling`,
`lowers_member_access_on_builtin_record_type`,
`no_encoder_emits_the_retired_term_color_id`,
`primitive_type_name_covers_handle_and_term_types`). Remaining failures are the
term fixtures and `.mfb` call sites, which are Phase 5's task list.
Commit: a56daca28

### Phase 3 — The emitters

- [x] `src/codegen/term/core/term.rs:54` — `TERM_COLOR_RECORD_SIZE` 24 → 32; rename
      it to match the type it now allocates. (Now `COLOR_RECORD_SIZE`.)
- [x] `emit_get_color` (`:2069-2124`) — store `red`/`green`/`blue` at 0/8/16 as
      today, and an immediate `255` at offset 24.
- [x] `emit_set_color` (`:568-592`) — load `red`/`green`/`blue` from the record
      pointer at offsets 0/8/16 (from the register Phase 1 identified) and pack into
      the `0xBBGGRR` slot exactly as today. Ignore offset 24. **Stage the loads via
      temporaries** — writing an argument slot before every incoming argument is read
      destroys one, and the symptom is a SIGSEGV at a tiny address.
      Staged: the pointer is moved to a temporary first and every field loaded off
      that, from `c_arg(0)` as Phase 1 established.
- [x] Tests: a codegen-inspection test asserting the four stores in `emit_get_color`
      land at 0/8/16/24 and that the alpha store is the immediate `255`. A
      black-box fixture cannot see a wrong offset that happens to read plausible
      bytes.
      `tests/codegen_term_color_record_offsets.rs`, on **two** targets. It also
      asserts there are **exactly four** record stores, so a stray fifth running off
      the 32-byte record is caught. See Corrections for two traps that made it
      silently vacuous on the way.
- [x] Tests: an rt-behavior fixture doing
      `term::setForeground(color::rgb(1, 2, 3))` then `term::getForeground()` and
      printing all four channels — `1 2 3 255`.
      `tests/rt-behavior/term/func_term_color_roundtrip_valid`, which also pins that
      `setForeground` **ignores** a non-opaque alpha, and that a colour read back can
      be handed straight to the setter.

Acceptance: **MET.** The round-trip prints `fg=1 2 3 255`, `bg=9 8 7 255`,
`halfTransparentSet=4 5 6 255` and `restoredEqualsOriginal=TRUE` on macOS AArch64,
and both codegen-inspection tests pass on `macos-aarch64` and `linux-x86_64`.
Commit: a56daca28 (emitters) + 741a0e94a (tests)

### Phase 4 — Cross-target proof (largest blast radius)

Lowering is emission, not runtime proof. This phase is the only thing that makes
the emitter change true on the other four targets.

- [x] Cross-build the round-trip fixture and run it on the Linux box (2223) for
      x86-64, aarch64 and riscv64, and on the Windows box (2230) for x86-64. Record
      each result.
      **Corrected routing:** 2223 is **aarch64** (`uname -m`) and has neither
      `qemu-x86_64` nor `qemu-riscv64`, so it cannot run the other two arches. Used
      the boxes that run each arch **natively** instead, which is stronger than
      emulation and also covers both libc worlds:

      | target | box | result |
      |---|---|---|
      | macos-aarch64 | host | `1 2 3 255` |
      | linux-aarch64 glibc | 2223 | `1 2 3 255` |
      | linux-x86_64 glibc | 2228 | `1 2 3 255` |
      | linux-x86_64 musl | 2227 | `1 2 3 255` |
      | linux-riscv64 musl | 2229 | `1 2 3 255` |
      | windows-x86_64 | 2230 | `1 2 3 255` |

      (`2232`, the glibc riscv64 box, is unreachable — box availability, not a code
      result. riscv64 is proven on 2229, which exercises the same shared emitters.)
- [x] Run the term fixture suite on each and compare rendered cells against the
      pre-change baseline. Use `scripts/test-winapp.sh` for Windows and exercise
      **both** entry paths — `cargo test` only compiles Windows PEs, it never runs
      one.
      Six term fixtures (`term-styling-basic`, `drawText`, `fillRect`, `drawBox`,
      `grid_draw`, `color_roundtrip`) cross-built, run on each box, and compared
      **byte for byte** against the committed golden's runtime section:
      **6/6 identical on all five targets** — 2228, 2227, 2223, 2229, 2230.
      `test-winapp.sh` covers `--app` builds; these fixtures are console
      executables, so the Windows arm runs the `.exe` directly, which is the same
      "actually execute a Windows binary" point that script exists to make.
- [x] Classify any pre-existing failure by arch against a `git archive` attribution
      binary before attributing it to this letter.
      **No failure to classify** — every arm passed, so no attribution binary was
      needed.

Acceptance: **MET.** The round-trip prints `1 2 3 255` on all five targets across
**seven** (arch, libc) combinations, and the six term fixtures render byte-identical
cells on all five.
Commit: 741a0e94a (no code change; cross-target proof recorded above)

### Phase 5 — Fixtures, examples, docs, and the size gate

- [x] Update the 23 `.mfb` files: `IMPORT color`, `color::rgb(...)` at the call,
      `.red`/`.green`/`.blue` at the read. Includes 6 examples (`ai_chat`,
      `browser/app`, `hangman`, `life`, `snake`, `wide-demo`) and
      `bugs/repro/bug-148-loop-trap-propagation.mfb`.
      Plus **two man examples** the plan's file list omitted
      (`term/func_fill_rect.rs`, `term/func_on.rs`), which still passed three
      channels and so failed to build — see Corrections.
      The four `syntax/term/*_invalid` fixtures were **restated, not mechanically
      rewritten**: wrapping their channels in `color::rgb(...)` would have moved the
      type error inside `color::rgb` and stopped testing the term member at all.
- [x] `term/helper_astrings_bridge.rs` — `saved AS color::Color`,
      `term::setForeground(saved)`, and the chunk's own `IMPORT color` (added in
      plan-122-E). The `-1`-unset sentinel logic is unchanged.
      Both appliers collapsed to a single call with no channel unpacking on either
      side, since `setForeground` now takes the colour `fromPacked` produces.
- [x] Regenerate goldens for all 37 term fixture dirs and the 5
      `tests/byte-identity/term` `.ncodesum` files. Re-sync **every** term importer,
      not just the edited ones — a later letter's companion growth silently
      re-shifts earlier `.ir` goldens and only a full acceptance run sees it.
      57 goldens across 21 fixtures, plus **7** `.ncodesum` (not 5 — the two
      `syntax/app/macos-app-mode-term` app sums drift too, and
      `regen-ncodesum.sh` walks them since plan-118-C).
- [x] Docs: `src/docs/spec/app/04_term-backend.md`; the `term` package `DESC`
      colour paragraphs; `src/docs/spec/stdlib/18_color.md` gains the
      "terminals ignore alpha" note.
      Also `src/docs/man/types/package.md`, `spec/architecture/02_frontend.md`,
      `spec/architecture/09_modules.md` and `spec/package/04_type-table.md`, all of
      which named `TermColor` — the plan's doc list missed them.
- [x] **The size gate:** rebuild `IMPORT io` + `IMPORT term` with no `color` import
      and assert the binary is still 66,596 bytes. If it grew, `term` acquired a
      code-bearing companion and the no-companion constraint was violated somewhere
      — find it before landing.

Acceptance: **MET.** `./scripts/test-accept.sh` full run green at **1390 ran**, 0
mismatches, 0 behavioural failures. `scripts/artifact-gate.sh <mfb> all` → 1368
tests, 1531 builds, **1898 goldens, 0 diffs**.

**The size gate holds exactly:**

| build | bytes |
|---|---|
| `IMPORT io` | 66,596 |
| `IMPORT io` + `IMPORT term` | **66,596** — identical, so `term` gained no companion |
| `IMPORT io` + `IMPORT color` | 231,716 |
| `IMPORT io` + `IMPORT term` + `IMPORT color` | 231,716 — the companion is paid once |
Commit: 741a0e94a + 1ef8a9ef1 (docs)

### Phase 6 — Close out plan-122

- [x] ~~`grep -rn 'canvas::rgb\|canvas::Color\|TermColor\|__astrings_packColor' src tests examples src/docs`
      returns only historical mentions~~ — **corrected: the `canvas::` half of this
      census cannot pass, because plan-122-D was not run** (explicit user
      instruction; see Corrections). `canvas::Color`/`canvas::rgb` are still the live
      canvas surface and there are ~180 live sites, all of which are D's task list,
      not unfinished work in E or F.

      Run for **this letter's own symbols**, which is the part that is meaningful:
      `TermColor` and `__astrings_packColor` have **no live sites**. Every remaining
      mention is one of:
      * a *historical* note explaining what the symbol used to be
        (`color/mod.rs`, `18_color.md`, the two rewritten test sites);
      * the deliberately **retired** wire id `TYPE_TERM_COLOR`, which the reader
        still decodes and `no_encoder_emits_the_retired_term_color_id` pins.

      Three genuinely **stale** ones were found and fixed rather than waved through:
      `term/core/term.rs` still said "Allocate the 3-field TermColor record" over a
      4-field allocation, and `builtins/mod.rs` / `registry/mod.rs` used
      `term.TermColor` as a live example of a package-scoped type.
- [x] Update `.ai/resources-packages.md`: the corrected stale bullets from
      plan-122-A, and a new note recording that a pure-source companion is compiled
      into every importer with the measured per-package numbers.
      Beyond the plan's ask, three corrections that would otherwise mislead the next
      reader: the `color`/`astrings` figures are re-measured (`color` is +165,120
      now, and `astrings` carries it since E); the **16,512-byte quantisation** of
      the instrument is documented, since every number in that table is really a
      block count; and the **two-register trap** from F Phase 1 is recorded — the
      doc's "first MFB arg arrives in `abi::return_register()`" is the *native
      runtime helper* convention and does **not** hold for the `term` core emitters,
      which use `c_arg(0)`.
- [x] ~~Move `planning/plan-122-*.md` to `planning/completed/`~~ — **partially
      moot**: A, B and C were archived when they landed, and E and F are archived
      here. **D stays in `planning/`**, unstarted, because it was excluded by the
      user. Archiving an unstarted plan would misfile it as done.

Acceptance: **MET as corrected.** The census returns no live `TermColor` or
`__astrings_packColor` site; the `.ai` doc carries the measured numbers plus the
quantisation and register caveats; five of the six plan files are archived, with D
deliberately left active.
Commit: 1ef8a9ef1

## Validation Plan

- **Tests:** the 37 term fixture dirs; the new round-trip rt-behavior fixture; the
  new codegen-inspection test on the four store offsets;
  `tests/rt_native_term_runtime.rs` for the astrings bridge.
- **Coverage check:** confirm `src/codegen/term/core/term.rs`'s
  `emit_get_color`/`emit_set_color` are in `scripts/coverage.sh --bin mfb`'s
  denominator — integration tests run an uncaptured subprocess, so a suite that only
  exercises them end-to-end may not count them.
- **Runtime proof:** run `examples/snake` and `examples/life` on macOS and on the
  Linux box; the proof is the rendered screen, not a passing assertion.
- **Doc sync:** `04_term-backend.md`; the `term` `DESC`; `18_color.md`;
  `.ai/resources-packages.md`.
- **Acceptance:** `cargo test --no-fail-fast`; `./scripts/test-accept.sh` full;
  `scripts/artifact-gate.sh`; `scripts/test-winapp.sh` on 2230;
  `scripts/build-examples.sh`; `cargo check --all-targets` at the end;
  `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Retire `TYPE_TERM_COLOR` (`0xffff_fefd`) or reserve it.** Recommend **reserve**:
  delete the encoder arm (`sections.rs:173-178`) so nothing emits it, keep the
  constant and the `reader.rs:890` arm with a comment marking the id retired and
  never to be reused, and add a test asserting no encoder produces it. Deleting the
  reader arm would make a previously-published `.mfp` fail to decode with an opaque
  error rather than a clear one. (§2)
- **Whether `getForeground` should report the *effective* colour when TUI mode is
  off.** It currently returns a documented inert default
  (`DEFAULT_FOREGROUND_PACKED`, `emit_get_color`'s `inert` branch). Recommend
  unchanged — this letter changes the type, not the semantics. (§2)
- **Whether `setForeground` should raise on a non-opaque alpha instead of ignoring
  it.** Recommend ignore-and-document: raising would make
  `term::setForeground(color::fromHex("#ff880080"))` a runtime failure for a colour
  that is perfectly meaningful elsewhere in the program. (§1)

## Corrections

**Prerequisites, re-measured 2026-09-04.**
`plan-122-E complete` → **MET**: E landed across `cf2489188`..`4efd0f03a` with 0
unticked boxes and 0 unfilled `Commit:` lines, and its gates are green
(`cargo test` 113 binaries 0 failures; `test-accept.sh` 1389 ran passed;
`artifact-gate.sh all` 1896 goldens 0 diffs). Measured on the ledger and the gates
rather than on `ls planning/completed/`, which is only written at merge time.
The remote-box row stays UNKNOWN by the plan's own instruction ("check before
Phase 4").

**plan-122-D is NOT being done** (explicit user instruction), which affects this
letter in exactly one place: Phase 6's census expects `canvas::rgb`/`canvas::Color`
to have no live sites. They will still have them. See the Phase 6 note.

**Phase 1 answer 1 — `color.Color` crosses the native boundary with no
storage-class change.** §2 listed this UNVERIFIED, with a row in
`src/target/shared/plan/lower.rs:203-210` as the anticipated fix. Not needed: the
spike built clean and ran first try. `color.Color` falls through to
`is_user_type_name` exactly as §2 predicted `canvas::Color` does, so the two
spellings do behave identically.

Spike program and result:

```
term::on()
term::setForeground(toByte(1), toByte(2), toByte(3))
LET c AS color::Color = term::getForeground()
term::off()
io::print(...c.red, c.green, c.blue, c.alpha)
→ 1 2 3 255
```

That single line also confirms the whole Phase-3 record change at once: the 32-byte
allocation, the three channel stores still landing at 0/8/16, and the immediate
`255` at offset 24 all read back correctly through an ordinary field access.

**Phase 1 answer 2 — the first MFB argument arrives in `c_arg(0)`, not
`abi::return_register()`.** Read off the shipped `emit_set_color`
(`src/codegen/term/core/term.rs`), which unpacks its three `Byte` arguments as
`move_register("%v9", abi::c_arg(0))`, `c_arg(1)`, `c_arg(2)`.

Worth stating why the question was asked at all: `emit_get_color`, twenty lines
away in the same file, passes the arena allocator's *first* argument in
`abi::return_register()` and its second in `c_arg(1)`. So both registers are in use
in this file for "first argument", for different call kinds, and picking the wrong
one for the Phase-3 rewrite would have compiled and then read whatever the
allocator left behind. Phase 3 loads the record pointer from `c_arg(0)`.

**And `.ai/resources-packages.md` says the opposite — for a different path.** Its
"Writing the native backend" section states *"first MFB arg (the record ptr)
arrives in `abi::return_register()`"*. That is the **native runtime helper**
contract (`src/target/shared/code/<pkg>/`) and it is correct there; the `term` core
emitters are a different emission path with a different convention. Anyone who had
carried that bullet across would have written a compiling emitter that read the
wrong register. The doc now says so explicitly.

**Phase 5's file list was short in three places**, each found by a gate rather than
by reading:

- **Two man examples** still passed three channels —
  `term/func_fill_rect.rs` and `term/func_on.rs` — so
  `man-run-examples.sh term --run` reported 2 build failures. The plan's Phase 5
  lists the 23 `.mfb` files but not the prose examples embedded in descriptors.
- **Four documentation files** named `TermColor` and are not in the plan's doc list:
  `src/docs/man/types/package.md`, `spec/architecture/02_frontend.md`,
  `spec/architecture/09_modules.md`, `spec/package/04_type-table.md`.
- **Seven `.ncodesum` goldens drift, not five.** §2 counts the 5 under
  `tests/byte-identity/term`; the two `syntax/app/macos-app-mode-term` app sums
  drift too. `regen-ncodesum.sh` walks them (plan-118-C), so running the script
  rather than hand-editing the five caught it.

**Ten `man-run-examples.sh term --run` RUN failures are pre-existing.** After
fixing the two build failures, 10 remain (`drawBox#2`, `drawGlyph#1`, `drawHLine#1`,
`drawText#1`, `drawVLine#1`, `fillRect#2`, `moveTo#2`, `sync#2`,
`terminalSize#1`/`#2`) — term examples that need a real terminal. Attributed rather
than assumed: the **pre-change** compiler (a `git archive` build from before this
letter) reports **11** failures on the same command, and this letter's 10 are a
strict subset — the rewrite happens to fix `setBackground#2`.

**A `--include='*.mfb'` census cannot see MFBASIC embedded in Rust strings, and it
cost a red suite.** §2's population is measured with
`grep -rl ... --include='*.mfb'`, which returns 23 files. But `tests/rt_native_term_runtime.rs`
holds several MFBASIC programs as Rust raw strings, three of which set and read
term colours, and none of them is a `.mfb` file. After Phase 5 the acceptance
harness and `artifact-gate` were both green — neither compiles those embedded
programs — and only `cargo test` caught it, with three `rt_native_term_runtime`
failures reporting `identifier could not be resolved` on `term::TermColor`.

The population for a migration like this is **MFBASIC source**, not `.mfb` files.
The census that finds it is
`grep -rln "term::setForeground\|term::getForeground\|term::TermColor" --include='*.rs' tests/ src/`.
plan-122-E hit the same shape and got away with it only because its rewrite script
happened to be pointed at that file by name for the `astrings` calls.

**`ir/lower.rs`'s justification for excluding `term` from arg-typed resolution is
now obsolete, and said so misleadingly.** The comment gave the reason as
"routing it through the arg-typed path would mis-resolve a `Byte`-parameter setter
called with `Integer` literals — `term::setForeground(255, 128, 0)`". Those setters
now take one `color::Color`, and
`grep -rn "ParameterType::Byte" src/codegen/builtins/term/*.rs` returns **nothing**,
so no term member has a `Byte` parameter left to mis-resolve. The exclusion is
*still correct* — every term member is a single overload with one fixed return, so
name resolution is right for the package — but it is kept for that reason, not the
stated one. Comment corrected rather than the exclusion removed: changing
resolution routing is not this letter's job, and a stale rationale that reads as
load-bearing is how a future reader talks themselves out of a safe cleanup.

**plan-122-D was not run, and it changes exactly one thing here.** D migrates
canvas and was excluded by explicit user instruction. F does not depend on it:
F's task list names no canvas file, and `term` and `canvas` are unconnected.
The only consequence is Phase 6's census, whose `canvas::rgb`/`canvas::Color` half
cannot pass while D is outstanding — corrected in place above.

_(filled in during execution — Phase 1's two recorded answers go here first)_

## Final acceptance (2026-09-04)

Every phase landed, every box resolved, every `Commit:` filled.

| Gate | Result |
|---|---|
| `cargo test --no-fail-fast` | **114 test binaries, 0 failures** |
| `./scripts/test-accept.sh` (full) | **1390 ran, passed** — 0 mismatches, **0** `behavioral test failed` |
| `scripts/artifact-gate.sh <mfb> all` | 1368 tests, 1531 builds, **1898 goldens, 0 diffs** |
| `cargo check --all-targets` | clean |
| `scripts/man-run-examples.sh term --run` | 43/43 build; 10 pre-existing TTY run failures, a strict subset of the pre-change 11 |
| **Size gate** | `IMPORT io` + `IMPORT term` = **66,596 bytes**, identical to `IMPORT io` alone |

**Cross-target proof (Phase 4), the point of this letter:**

| target | box | round trip | 6 term fixtures |
|---|---|---|---|
| macos-aarch64 | host | `1 2 3 255` | golden |
| linux-x86_64 glibc | 2228 | `1 2 3 255` | 6/6 identical |
| linux-x86_64 musl | 2227 | `1 2 3 255` | 6/6 identical |
| linux-aarch64 glibc | 2223 | `1 2 3 255` | 6/6 identical |
| linux-riscv64 musl | 2229 | `1 2 3 255` | 6/6 identical |
| windows-x86_64 | 2230 | `1 2 3 255` | 6/6 identical |

Five targets, seven (arch, libc) combinations, **byte-identical rendered cells
everywhere**.

## Summary

Everything before Phase 3 is a rename with loud failure modes. Phase 3 is
hand-written machine code with none: a wrong offset reads plausible bytes and
renders a plausible colour. That is why Phase 1 establishes the argument register
before an emitter is touched, why Phase 3 adds a codegen-inspection test on the
literal offsets rather than trusting a black-box fixture, and why Phase 4 runs on
all five targets rather than trusting shared emission.

The size gate in Phase 5 is the guard on the one architectural commitment of this
letter: `term` stays companion-free, so a TUI program that does not use colour pays
nothing for `color` existing.
