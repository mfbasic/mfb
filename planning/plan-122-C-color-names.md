# plan-122-C: Named colours — `fromName`, `nameOf`, and the basic constants

Last updated: 2026-09-02
Effort: medium (1h–2h)
Depends on: plan-122-B

The last piece of the `color` package: CSS named colours
(`color::fromName("rebeccapurple")`), the reverse lookup `color::nameOf`, and a
small set of package constants for the colours a program reaches for without
thinking (`color::black`, `color::white`, `color::red`, …).

This is the only letter in plan-122 whose content is **bulk data**, and the
measurement in plan-122-A Phase 6 is what decides how it ships. A pure-source
package's whole companion is compiled into every importing binary
(measured: `IMPORT astrings` alone costs 1,073,280 bytes; see plan-122-A §2), so a
name table is not free — it is paid by every program that imports `color`, which
after D–F means every canvas program and every program that colours an
`AttributedString`.

Behavioral outcome: `color::fromName("RebeccaPurple")` returns `#663399ff`,
`color::nameOf(color::fromHex("#ff0000"))` returns `"red"`, and
`color::fromName("nosuchcolour")` raises `ErrNotFound`.

References:

- plan-122-A — Prerequisites, `Color`, `fromHex`, and the **measured byte cost of
  the `color` companion** recorded in its Corrections. Read that number before
  starting.
- CSS Color Module Level 4, §"Named Colors" — the authoritative `<named-color>`
  list and its hex values. **Take the list from the specification, not from
  memory**; both the membership and the exact hex of the greys are easy to
  misremember.
- `src/codegen/builtins/vector/mod.rs` — the `add_constant` record-constant
  precedent (`vector::zeroFloat3`), and `src/codegen/registry/mod.rs:885-900` for
  `RegistryConstant`'s `components` field, which is how a record constant inlines
  its per-field literals.

## Prerequisites

Stated once in plan-122-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-122-B complete | `ls planning/completed/plan-122-B-*` → one match | NOT MET |
| The `color` companion's unused-import byte cost is recorded | `grep -n 'byte cost' planning/completed/plan-122-A-*.md` | NOT MET |

If plan-122-B is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- `color::fromName(name AS String) AS Color` resolves every CSS Color Level 4
  `<named-color>`, case-insensitively, and raises `ErrNotFound` (`77050004`) for
  anything else.
- `color::nameOf(base AS Color) AS String` returns the CSS name of an **exact**
  match (alpha `255` and RGB equal to a table entry) and raises `ErrNotFound`
  otherwise.
- A set of record constants exists for the basic colours, so
  `color::black` needs no call and no string.

### Non-goals (explicit constraints)

- **No nearest-colour search.** `nameOf` is an exact reverse lookup. "Closest
  named colour" is a different function with a different cost and a contestable
  metric; it is not in this plan.
- **No `transparent` keyword.** CSS's `transparent` is `#00000000`, which
  `color::rgba(0, 0, 0, 0)` already spells; adding a name whose alpha is not `255`
  would break `nameOf`'s stated exact-match rule.
- **No new error code.** `ErrNotFound` already exists
  (`grep -n ErrNotFound src/codegen/builtins/errorcode/mod.rs` → `77050004`), so no
  `data_objects.rs` row is needed.
- **No named-colour maths.** Nothing here reads or writes the sRGB seam.

## 2. Current State

`color` after plan-122-B has the `Color` record, the constructors, the packed
bridge, hex text, the sRGB seam and the perceptual/HSL layers. It has no notion of
a colour *name*, and no constants at all.

`RegistryConstant` (`src/codegen/registry/mod.rs:885-900`) supports two shapes: a
scalar constant folding to a literal (`value`), and a **record** constant inlining
per-field literals into a constructor of `type_name` (`components`). The second is
what `color::black` needs; `vector::zeroFloat3` is the shipped example.

### Measured populations

| What | Count | Command |
|---|---|---|
| CSS Color Level 4 `<named-color>` entries | UNMEASURED | Take from the CSS Color 4 §"Named Colors" table at implementation time. **Phase 1's first act.** |
| `add_constant` call sites in the tree | 21 | `grep -rn 'add_constant' src/codegen/builtins/ \| wc -l` |
| Existing `ErrNotFound` uses in builtin bodies | (record at implementation time) | `grep -rn '77050004' src/codegen/builtins/ \| wc -l` |

### Verified properties

- **`RegistryConstant` supports record constants**, verified by reading
  `src/codegen/registry/mod.rs:885-900` (the `components: Option<&'static [&'static str]>`
  field, documented as "the ordered per-field literals a **record** constant
  inlines into a constructor of `type_name`") and the round-trip test
  `package_constants_and_overrides_round_trip_through_the_builders`
  (`src/codegen/registry/mod.rs:3996`).
- **UNVERIFIED — whether a record constant folds at the call site or emits a
  companion FUNC.** This decides whether the 16 basic-colour constants cost
  anything at all. Phase 3 measures it before adding more than one.

## 3. Design Overview

Two independent pieces:

1. **The name table and its two lookups.** One private helper holding the table,
   `fromName` reading it forward, `nameOf` reading it backward.
2. **The basic-colour constants.** Independent of (1) — a constant does not consult
   the table, it inlines its four literals.

**Where the risk is:** not correctness — it is *size*. A wrong hex value fails a
test; a table shipped in every binary fails nobody's test and shows up as a
megabyte. Phase 1 therefore measures the cost of the table **before** the reverse
lookup or the constants are written, and Phase 2 is explicitly allowed to conclude
"this is too expensive, ship the constants only".

### Table representation

Recommended: a single `Map OF String TO Integer` from lower-cased name to
`0xAARRGGBB` packed value, built once by a private
`FUNC __color_nameTable() AS Map OF String TO Integer` and hoisted to a module-level
`LET`, the same shape `canvas` uses for its sRGB table
(`canvas/helper_color.rs:27`, now `color/helper_srgb.rs`).

`fromName` is then `collections::get` after `strings::lower`, and `nameOf` is a
walk of the same map comparing packed values. A second reverse map would double the
data for a lookup nobody calls in a loop.

### Rejected alternatives

- **A `Named` enum with one variant per colour.** Rejected: an enum variant set is
  a closed, breaking-change-prone surface (canvas's `DrawItem` freeze rule,
  `canvas/mod.rs:160-165`), and CSS's list has grown before (`rebeccapurple`). A
  string lookup that raises `ErrNotFound` grows without breaking a `MATCH`.
- **Two maps, name→packed and packed→name.** Rejected: doubles the shipped data
  to speed up a call that is not in any hot path.
- **Shipping only the 16 basic colours and no table.** Held in reserve as Phase 2's
  documented escape hatch if the measurement says the table is too expensive; not
  the default, because the user asked for the full set (decision, 2026-09-02).

## 4. Member surface

| Member | Signature | Notes |
|---|---|---|
| `fromName` | `(name AS String) AS Color` | case-insensitive; leading/trailing whitespace trimmed; raises `ErrNotFound` |
| `nameOf` | `(base AS Color) AS String` | exact match only, alpha must be `255`; raises `ErrNotFound` |

Constants (record constants via `add_constant`, all alpha `255`): `black`,
`white`, `red`, `green`, `blue`, `yellow`, `cyan`, `magenta`, `gray`, `silver`,
`maroon`, `olive`, `navy`, `teal`, `purple`, `orange`.

`green` is a trap worth stating on the man page: the CSS keyword `green` is
`#008000`, **not** `#00ff00` (that is `lime`). The constant follows CSS, and the
page says so, because a caller who assumes otherwise gets a colour that looks
merely "wrong" rather than failing.

## Compatibility / Format Impact

- **Public surface:** `color` gains 2 members and 16 constants. Nothing is removed
  or renamed.
- **Binary size:** every program importing `color` grows by the table. Measured in
  Phase 1 and recorded in Corrections.
- **Expected golden drift:** `.ir`/`.ast` for every `color` importer (the companion
  grows). No `build.log` or `.run` change for any fixture that does not call the
  new members.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work; `- [~]` for partial with a line on what remains;
> `- [x] ~~text~~ — moot: <evidence>` rather than deleting. Fill `Commit:` on
> landing. **An unticked box means NOT DONE.**

### Phase 1 — The table, measured

- [ ] Transcribe the CSS Color Level 4 `<named-color>` table into
      `src/codegen/builtins/color/helper_name_table.rs` as
      `FUNC __color_nameTable() AS Map OF String TO Integer` plus
      `LET __COLOR_NAMES`, keys lower-cased, values `0xAARRGGBB` with alpha `255`.
      **Record the entry count in Corrections** — it is the number this letter's
      cost is proportional to.
- [ ] Add a Rust unit test in that file that parses the literal out of the const
      and pins: the entry count, that every key is lower-case ASCII, that every
      value has alpha `255`, and a spot-check of the six values most often
      misremembered (`green`, `lime`, `gray`, `grey`, `purple`, `rebeccapurple`).
      This is the `srgb_table_matches_the_transfer_function` lesson: a length test
      catches a truncated paste, only a value test catches a wrong one.
- [ ] `func_from_name.rs` — trim, `strings::lower`, `collections::get`, raise
      `FAIL error(77050004, …)` on a miss.
- [ ] **Measure and record** the `color` companion's unused-import byte cost again
      with the plan-122-A §2 command, and record the delta this table added.

Acceptance: `color::fromName` resolves the spot-checked names in an rt-behavior
fixture and raises `ErrNotFound` for a miss (asserted by **code**, not by absence
of output); the Rust table test passes; both byte numbers are in Corrections.
Commit: —

### Phase 2 — Go/no-go on the table

A decision point, not a task list. Land it as a written note in Corrections either
way.

- [ ] Compare the Phase-1 delta against the rest of the `color` companion. If the
      table is a minority of the package's cost, keep it and continue to Phase 3.
- [ ] If it dominates, raise it with the user before proceeding — the fallback is
      constants-only, and that is their call, not the implementer's. Do not
      silently narrow the scope.

Acceptance: a recorded decision with the two numbers behind it.
Commit: —

### Phase 3 — Reverse lookup and constants

- [ ] `func_name_of.rs` — walk `__COLOR_NAMES`, compare `toPacked(base)`, raise
      `ErrNotFound` when alpha is not `255` or no entry matches.
- [ ] Add **one** constant (`color::black`) via `add_constant` and measure the
      byte delta, resolving the UNVERIFIED property in §2. Then add the remaining
      15 only if a constant is genuinely free; if it is not, say so in Corrections
      and ship the ones that earn their place.
- [ ] Tests: `nameOf(fromName(n)) = n` over the spot-checked names; `nameOf` raises
      for a colour not in the table and for a table colour with alpha `< 255`.
- [ ] Man prose: the CSS `green` vs `lime` trap (§4), and the exact-match rule on
      `nameOf` — a reader must not expect nearest-colour behavior.

Acceptance: the round-trip and both `nameOf` rejection cases pass;
`man-census.sh --fill color` 100%; `--memory-scope color` and `--scope color` 0;
`scripts/man-run-examples.sh color --run` green.
Commit: —

## Validation Plan

- **Tests:** `tests/rt-behavior/color/` for `fromName`/`nameOf` including both
  rejection classes; the Rust table test for the data itself.
- **Coverage check:** confirm `helper_name_table.rs`, `func_from_name.rs` and
  `func_name_of.rs` are in `scripts/coverage.sh --bin mfb`'s denominator.
- **Runtime proof:** a scratch program printing
  `color::toHex(color::fromName("RebeccaPurple"))` → `#663399`.
- **Doc sync:** `src/docs/spec/stdlib/18_color.md` gains the named-colour section
  and states the source (CSS Color Level 4) rather than listing the table twice.
- **Acceptance:** `cargo test --no-fail-fast`; `./scripts/test-accept.sh` full run;
  `scripts/artifact-gate.sh`; `cargo check --all-targets`; `cargo fmt`.

## Open Decisions

- **`grey` as well as `gray`.** CSS accepts both spellings for four colours
  (`gray`, `darkgray`, `lightgray`, `slategray` and their `grey` forms). Recommend
  carrying both in `fromName` (they are in the CSS table) and returning the `gray`
  spelling from `nameOf`, documented. (§4)
- **Whether `nameOf` should exist at all.** Recommend yes — it is what makes a
  colour round-trip through a config file readable — but it is the member to cut
  first if Phase 2 goes badly. (§4)

## Corrections

_(filled in during execution)_

## Summary

The engineering question in C is not "is the table right" — a unit test settles
that. It is "should the table ship at all", and the honest answer needs the
measurement Phase 1 produces. Phase 2 exists so that decision is made with numbers
and escalated to the user rather than quietly made by whoever is implementing.

Untouched: canvas, term, astrings, and every colour-space path.
