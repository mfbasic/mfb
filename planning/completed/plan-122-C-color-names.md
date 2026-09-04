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
| plan-122-B complete | `ls planning/completed/plan-122-B-*` → one match | **MET** (2026-09-03) — B landed in this same `worktree-P-122` run across `bf4324544`..`61b998b0b`, with the doc-sync and citation repair at `d3451d8a9`. Measured directly rather than by the archive path, which is only written at merge time and is a *proxy* for completeness: `grep -c '^- \[ \]' planning/plan-122-B-color-perceptual.md` → **0** unticked, `grep -c '^Commit: —$'` → **0** unfilled, and B's gates are green (`cargo test --no-fail-fast` exit 0, 100 binaries; `test-accept.sh` 1384 ran, passed; all `rt_canvas_*` suites green with the six reference PNGs unregenerated). |
| The `color` companion's unused-import byte cost is recorded | `grep -n 'byte cost' planning/completed/plan-122-A-*.md` | **MET** — recorded in plan-122-A's Corrections (§"Phase 6 — MEASURED byte cost"). **Read the correction below before using it**: that number is quantised to 16,512-byte blocks, which plan-122-A did not know. |

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

- [x] Transcribe the CSS Color Level 4 `<named-color>` table into
      `src/codegen/builtins/color/helper_name_table.rs` as
      `FUNC __color_nameTable() AS Map OF String TO Integer` plus
      `LET __COLOR_NAMES`, keys lower-cased, values `0xAARRGGBB` with alpha `255`.
      **Record the entry count in Corrections** — it is the number this letter's
      cost is proportional to.
      **Not transcribed — extracted.** `curl`ed `https://www.w3.org/TR/css-color-4/`
      and parsed its own `named-color-table` rows, because the plan says to take the
      list from the specification and 148 rows of hex is exactly what a hand
      transcription corrupts silently. **148 entries.**
- [x] Add a Rust unit test in that file that parses the literal out of the const
      and pins: the entry count, that every key is lower-case ASCII, that every
      value has alpha `255`, and a spot-check of the six values most often
      misremembered (`green`, `lime`, `gray`, `grey`, `purple`, `rebeccapurple`).
      This is the `srgb_table_matches_the_transfer_function` lesson: a length test
      catches a truncated paste, only a value test catches a wrong one.
      Landed as four tests, not one: count, lower-case-ASCII keys, alpha-`255`
      values, and the spot-checks — plus assertions that the six duplicate-spelling
      pairs agree with each other.
- [x] `func_from_name.rs` — trim, `strings::lower`, `collections::get`, raise
      `FAIL error(77050004, …)` on a miss.
- [x] **Measure and record** the `color` companion's unused-import byte cost again
      with the plan-122-A §2 command, and record the delta this table added.

Acceptance: **MET.** The table's four Rust tests pass. `fromName` resolves the
spot-checked names — `green` → `#008000ff` (**not** `#00ff00`), `lime` →
`#00ff00ff`, `RebeccaPurple` → `#663399ff` case-insensitively, `"  TEAL  "` →
`#008080ff` after trimming, `grey` and `gray` → the same `#808080ff` — and raises
**by code** (`err.code = errorCode::ErrNotFound` → `TRUE`) for both an unknown name
and the empty string. Byte numbers below, with the quantisation caveat.
Commit: ec44a0f73

### Phase 2 — Go/no-go on the table

A decision point, not a task list. Land it as a written note in Corrections either
way.

- [x] Compare the Phase-1 delta against the rest of the `color` companion. If the
      table is a minority of the package's cost, keep it and continue to Phase 3.
      **It is not a minority — it is 50%**, so this branch did not apply.
- [x] If it dominates, raise it with the user before proceeding — the fallback is
      constants-only, and that is their call, not the implementer's. Do not
      silently narrow the scope.
      **Raised. Decision: keep the full 148-colour table** (user, 2026-09-03),
      consistent with their recorded 2026-09-02 decision to ship the full set. The
      constants-only fallback and a trimmed-table middle option were both put
      explicitly, with the numbers; neither was taken.

Acceptance: **MET.** Recorded decision with the numbers behind it:

| | bytes | over baseline |
|---|---|---|
| `IMPORT io` (baseline) | 66,600 | — |
| `IMPORT io` + `IMPORT color`, A+B | 149,160 | +82,560 |
| `IMPORT io` + `IMPORT color`, A+B+C | 231,720 | +165,120 |

**The table plus `fromName`/`nameOf` costs +82,560 — exactly 50% of `color`'s whole
companion**, and about **4.8%** of the 1,707,404-byte canvas app measured in
plan-122-B Phase 3. The 16 record constants contribute **0** to this figure (see
Corrections). Escalated at 50% rather than deciding it, because 50% is not the
"minority" this phase requires to proceed on its own authority.
Commit: ec44a0f73

### Phase 3 — Reverse lookup and constants

- [x] `func_name_of.rs` — walk `__COLOR_NAMES`, compare `toPacked(base)`, raise
      `ErrNotFound` when alpha is not `255` or no entry matches.
      Keeps the **alphabetically smallest** match rather than the first found, so
      the answer does not depend on map iteration order — see Corrections.
- [x] Add **one** constant (`color::black`) via `add_constant` and measure the
      byte delta, resolving the UNVERIFIED property in §2. Then add the remaining
      15 only if a constant is genuinely free; if it is not, say so in Corrections
      and ship the ones that earn their place.
      **Resolved by reading the code instead of measuring**, which is stronger:
      `RegistryPackage::get_mfb` renders imports, records, unions, enums, helpers
      and member bodies — constants are **not** in that list, so a record constant
      contributes exactly **0** bytes to the companion and folds into a `Color`
      constructor at its call site. A constant is therefore genuinely free, and all
      16 shipped. (See Corrections; a size diff could not have beaten a fact the
      assembly order already fixes, and given the 16,512-byte quantisation it could
      not even have resolved one constant.)
- [x] Tests: `nameOf(fromName(n)) = n` over the spot-checked names; `nameOf` raises
      for a colour not in the table and for a table colour with alpha `< 255`.
- [x] Man prose: the CSS `green` vs `lime` trap (§4), and the exact-match rule on
      `nameOf` — a reader must not expect nearest-colour behavior.

Acceptance: **MET**, in `tests/rt-behavior/color/color_names_rt`.

Round trips hold over the spot-checked names (`rtLime`→`lime`, `rtTeal`→`teal`,
`rtPurple`→`purple`, `nameRebecca`→`rebeccapurple`, `nameGreen`→`green`).
Duplicate spellings resolve to the alphabetically first — `nameGrey` and
`nameGray` both `gray`, `nameCyan` and `nameAqua` both `aqua`, `nameMagenta`
`fuchsia`.

**Both** `nameOf` rejection classes assert the raised code: `offByOne`
(`#ff0001`, one step off `red`) and `translucent`/`transparent` (RGB matches `red`
exactly, alpha does not) all report `notFound TRUE`. `fromName` likewise rejects
four classes by code — unknown name, empty string, a hex string, and a name with an
inner space.

Constants agree with the table rather than merely existing:
`agreeGreen`/`agreeGray`/`agreeTeal`/`agreeMaroon` all `TRUE`, and `constGreen` is
`#008000` — the CSS value, not `#00ff00`.

12 `color` Rust unit tests pass (4 name-table, 4 constants, 4 sRGB).
`man-census.sh --fill color` → **28 pages, 100% every column, 47/47 param-desc,
7/7 types**; `--memory-scope color` **0**; `--scope color` **0**;
`man-run-examples.sh color --run` → **57 examples, 57 built, 57 ran, 0 failed**.
Runtime proof: `color::toHex(color::fromName("RebeccaPurple"))` → `#663399`.
Commit: ec44a0f73

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

**MEASUREMENT CORRECTION, affecting plan-122-A and plan-122-B too: the built
`.out` size is QUANTISED to 16,512-byte blocks.** Every byte figure in this plan
series comes from `stat -f%z` on a built executable, and that instrument is
coarser than it looks.

Caught because two independent deltas came out *exactly* equal — A+B's companion
and C's table both `+82,560` to the byte. That is too neat, so I tested the
instrument instead of believing it: adding 1, 2, then **20** extra
`io::print` statements to the probe changed the built size **not at all**
(`231720` four times). Widening the sweep:

| extra prints | bytes |
|---|---|
| 0, 100 | 231,720 |
| 500 | 297,768 |
| 1000 | 347,304 |
| 2000 | 512,424 |
| 4000 | 859,176 |

Every observed delta — including plan-122-A's `+33,024` and plan-122-B's canvas-app
`+33,024` — is a multiple of **16,512**. So sizes move in blocks with slack inside
a block, and any single measurement carries up to ±16,512 bytes of error.

Consequences, stated rather than quietly left:

- **plan-122-A Phase 6's "+33,024 bytes"** for the `color` companion is 2 blocks,
  true to ±16,512 — so "an order of magnitude below every other pure-source
  package" still holds comfortably (the next smallest was +462,336, 28 blocks),
  but the figure is not byte-exact.
- **plan-122-B Phase 3's canvas-app "+33,024 (+1.97%)"** is likewise 2 blocks. I
  called its exact agreement with plan-122-A's number "perfect confirmation"; it is
  still real confirmation — identical content lands in identical blocks — but the
  agreement is to block granularity, not to the byte.
- **This letter's 50%** is 5 blocks against 10, so ±1 block is ±10 percentage
  points. The go/no-go was escalated with that tolerance stated.

None of the conclusions drawn from these numbers change; the precision claimed for
them does.

**Record constants cost an unused importer exactly ZERO, and that is settled by
reading the code rather than by a size diff** — which resolves §2's UNVERIFIED
property ("whether a record constant folds at the call site or emits a companion
FUNC"). `RegistryPackage::get_mfb` assembles the injected companion from imports,
**records, unions, enums, helpers and member bodies** — constants are not in that
list and are never rendered into it. A record constant folds into a `Color`
constructor at its call site via `RegistryConstant::components`. So all 16 are free
to a program that does not name one, and a size-diff measurement would only have
confirmed a fact the assembly order already fixes. Phase 3's "add one and measure"
step was therefore answered directly.

**`nameOf` must not depend on map iteration order, and the first implementation
did.** Six CSS colours have two spellings (`gray`/`grey`, `darkgray`/`darkgrey`,
`lightgray`/`lightgrey`, `slategray`/`slategrey`, `aqua`/`cyan`,
`fuchsia`/`magenta`), so for those the answer depends on which key the reverse walk
meets first. `mfb man collections keys` is explicit that order is *"implementation-
defined but stable for a given unchanged map"* — insertion order today, but
"treat insertion order as the current implementation's behavior rather than a
guarantee to rely on across versions".

Returning the first match would have made `nameOf(fromName("grey"))` an artefact of
the order the generator happens to emit the table in, and would have flaked this
member's golden the day that changed. Rewritten to keep the **alphabetically
smallest** match, which is order-independent and picks the expected spelling in all
six cases (`gray` < `grey`, `aqua` < `cyan`, `fuchsia` < `magenta`, …). Verified
`String` `<` works in MFBASIC before relying on it.

## Final acceptance (2026-09-03)

Every phase landed, every box resolved.

| Gate | Result |
|---|---|
| `color` Rust unit tests | **12 passed** — 4 name-table, 4 constants, 4 sRGB |
| `scripts/man-run-examples.sh color --run` | 57 examples, 57 built, 57 ran, **0 failed** |
| `man-census.sh --fill color` | **28 pages**, 100% every column, 47/47 param-desc, 7/7 types |
| `--memory-scope color` / `--scope color` | **0** / **0** |
| Runtime proof | `color::toHex(color::fromName("RebeccaPurple"))` → `#663399` |
| `cargo check --all-targets` | clean |

Whole-plan `cargo test`, `test-accept.sh` and `artifact-gate.sh` results are
recorded once at the end of plan-122-A, since A/B/C landed on one branch.

## Summary

The engineering question in C is not "is the table right" — a unit test settles
that. It is "should the table ship at all", and the honest answer needs the
measurement Phase 1 produces. Phase 2 exists so that decision is made with numbers
and escalated to the user rather than quietly made by whoever is implementing.

Both halves of that played out, and a third thing the plan did not anticipate.
The table is right, and it is right because it was **extracted from the
specification rather than transcribed** — the extractor's first attempt misaligned
3 names against 181 hexes and refused to emit, which is precisely the corruption a
hand transcription commits in silence. The size question came out at exactly 50%,
neither the "minority" that permits proceeding nor an obvious dominance, so it went
to the user with all three options and their numbers.

The unanticipated one: the *instrument* was wrong. Built `.out` sizes are quantised
to 16,512-byte blocks, so every byte figure in this plan series — including the two
that plans A and B had already drawn conclusions from — carried up to ±16,512 bytes
of error that nobody had noticed. Two deltas coming out exactly equal to the byte
is what gave it away. The conclusions survived; the claimed precision did not, and
all three plans now say so.

Untouched: canvas, term, astrings, and every colour-space path.
