# plan-89-A: the opaque `AttributedString` type + text I/O

Last updated: 2026-08-08
Overall Effort: huge (>3d) — the whole plan-89 feature (sub-plans A–E)
Effort: large (3h–1d)
Depends on: nothing

Adds a new opaque, value-semantic built-in type `AttributedString` and the shell of a new
built-in package `astrings`, wired end to end (front end → IR → codegen → `.mfp` wire), plus the
three text-emitting seams that ignore attributes: `toString(AttributedString)`, `io::print`, and
`io::write`. No attribute *storage behavior* yet (that is plan-89-B) — this sub-plan delivers the
type as an opaque wrapper around a `String` whose attribute overlay is always empty.

**Single behavioral outcome for A:** a program can `astrings::fromString("hi")`, bind it
(`MUT`/`LET`), print it with `io::print`/`io::write` (emitting exactly `hi`), and round-trip it back
with `toString` — all built and run headless, with the value fully opaque (no field access
compiles).

References (read first):

- **The machinery map for this feature** — the authoritative citation set the whole of plan-89 is
  written against. Reproduced inline below (§2); the key precedents are `toString`
  (`src/builtins/general.rs:318`, `src/target/shared/code/builder_strings.rs:757` `lower_to_string`)
  and the `Error`/`ErrorLoc` hardcoded-type family.
- `src/binary_repr/mod.rs:102-134` — wire type-id constants; ids **11–19 are RESERVED for future
  primitives**, `FIRST_TABLE_TYPE_ID = 20`.
- `src/docs/spec/memory/03_heap-values.md` — heap value layout (String object; inline-in-record
  String blocks) that `AttributedString`'s heap object mirrors.
- `bugs/bug-434-collections-defaultable-when-empty.md` — **related, not a prerequisite** (see
  Prerequisites).

## Prerequisites

These are a precondition on the whole plan-89 feature; stated once here (B–E point back).

| Must be true | Command | Status 2026-08-08 |
|---|---|---|
| Working tree builds green at HEAD | `cargo build --bin mfb` → ok | **MET** 2026-08-08 (built green in worktree, 34.9s) |
| The reserved wire-id band 11–19 is still free | `rg -n 'TYPE_[A-Z]+ *= *1[1-9]' src/binary_repr/mod.rs` → no primitive claims 11–19 | **MET** 2026-08-08 (no matches) |
| **plan-85 complete** (ABI-token + error-Result convention migration) | `ls planning/plan-85-*.md` → no matches (all sub-plans archived to `planning/completed/`) | **MET** 2026-08-08 (all plan-85-A/B/C/D in `planning/completed/`; `/tmp/p85-clean2` is a stale detached worktree, not an active P-85 branch) |

**plan-85 is a hard prerequisite — if it is not complete, plan-89 cannot start, full stop.**
plan-89 adds *new* native emission sites in `src/target/shared/code/` (`fromString` and the Tier-C
functions emit error-Results for inclusive-bounds violations; the `toString`/transform overloads add
operand emission). plan-85 is mid-migration of exactly that machinery — the ABI token vocabulary
(~4,900 sites) and the error-Result convention (884 operands across 56 files, `RESULT_*_REGISTER` →
`%retMFB`). New native sites written during that migration would be missed by plan-85's census
(`planning/plan-85-census.md`) or emit legacy/in-flux tokens. plan-89 must therefore be authored
against the **post-plan-85** conventions (typed `Operand::Abi`, aligned `%retMFB`), and must not
absorb, prefer, or route around plan-85's state. (Only the native sub-plans are gated: A's type
registration, C/D's overload codegen, and B's Tier-C native functions all emit into that ABI. The
`.mfb` source-companion work in B/E — the `Attribute` model and a `.mfb` `toMarkdown` — touches no
emission site, but the letters are landed in order, so the whole feature waits on plan-85 regardless.)

**bug-434 is deliberately NOT a prerequisite.** Under this plan's opaque-*primitive* design (§3),
`AttributedString` is a hardcoded built-in type with an explicitly-granted empty default; its
internal attribute list is codegen-internal and never surfaces as a user-facing `List OF Attribute`
binding. So the collection-defaultability rule bug-434 fixes never gates this feature. bug-434
remains a worthwhile independent fix, but landing plan-89 does not wait on it, and plan-89 must not
absorb, prefer, or route around it.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every command
> before continuing and before deciding to stop; report all rows if you stop.

## 1. Goal

- A new type `AttributedString` exists as an always-in-scope built-in, opaque and value-semantic:
  copyable, droppable, defaultable to empty, encodable to/decodable from `.mfp`.
- `astrings::fromString(s AS String) AS AttributedString` constructs one whose visible text is `s`
  and whose attribute overlay is empty.
- `toString(a AS AttributedString) AS String` returns the visible text (empty overlay in A, but the
  arm is the general text-extraction path B–E reuse).
- `io::print(a AS AttributedString)` and `io::write(a AS AttributedString)` emit the visible text.
- **Opacity is enforced:** `a.text`, `a.anything`, `AttributedString[...]` construction, and
  `WITH a { ... }` all fail to compile.

### Non-goals (explicit constraints)

- **No attribute storage or query behavior in A.** `addAttribute`/`removeAttribute`/`toMarkdown` are
  later letters. A ships the type with a permanently-empty overlay.
- **`AttributedString` is not comparable or orderable** (it wraps a list overlay, mirroring `List`):
  no `=`/`<>`/ordering, not a `Map` key, not a `Set` element. Do not add equality.
- **No implicit `AttributedString → String` coercion** anywhere. Text is reached only via `toString`
  or an explicit overload.
- **Do not change `String`** or any existing `strings::`/`io::` behavior for `String` arguments.
- **No new user-visible field-hiding mechanism** — opacity is achieved by the type having *no
  user-visible fields at all* (primitive-like), not by extending record visibility.

## 2. Current State

The feature rides two precedents, both mapped to source:

**Type-based builtin overloading already works — `toString` is the template.** A builtin callable
resolves its return type statically from its argument type strings, and codegen emits a per-type
arm:
- `src/builtins/general.rs:318` — `TO_STRING` accepts `Integer|Float|Fixed|Money|Boolean|String|
  Byte|Scalar` and `List OF Byte`; `GeneralResolver` delegates `resolve_return_type` here
  (`general.rs:157`).
- `src/target/shared/code/builder_strings.rs:757` — `lower_to_string` — `match value.type_.as_str()
  { "String" => …, "Boolean" => …, … other => Err("native toString does not accept …") }`. Adding
  an `"AttributedString"` arm here is the extraction seam.
- Package/user override routing: `src/builtins/mod.rs:146` `general_override_target`.

**Hardcoded (Family-B) built-in types are the opacity precedent — `Error`/`ErrorLoc`.** They never
appear in `project.types`; each stage hardcodes them:
- Name registration: `src/resolver/mod.rs:14` `BUILTIN_TYPES`.
- `Type` variant + `to_string`: `src/syntaxcheck/types.rs:87`, `src/syntaxcheck/mod.rs:1625`.
- Field tables (we register **no** user-visible fields for `AttributedString`): `src/ir/verify/
  mod.rs:1219` `builtin_type_fields`; `src/target/shared/code/validation.rs:337` codegen
  `record_fields`.
- Defaultable/comparable base deltas: `src/ir/verify/mod.rs:182`
  `is_comparable_defaultable_primitive`.
- Wire id: `src/binary_repr/mod.rs:102` (claim a reserved id 11–19); encode
  `src/binary_repr/sections.rs:78` `type_id`; decode `src/binary_repr/reader.rs:882`
  `primitive_type_name`.
- Layout classification: `src/target/shared/code/builder_collection_layout.rs:587/611/634`.

**`astrings` package registration — 10 sites** (from the map): `src/builtins/mod.rs:1` (`mod`), a new
`src/builtins/astrings.rs` (`BuiltinModule` + `BuiltinFunction` table), `src/builtins/
descriptor.rs:631` (`REGISTRY`), `src/builtins/mod.rs:81` (`is_builtin_import`), `mod.rs:1067`
(`ALL_BUILTIN_PACKAGES` test), `src/syntaxcheck/builtins.rs:38` (`BUILTIN_ARG_MODES` — `Read`),
`src/docs/man/mod.rs:29` (`PACKAGE_ORDER`) + `build.rs:27` (`MAN_PACKAGES`), plus a source companion
`src/builtins/astrings_package.mfb` (B–E) and, if return types are arg-computed, the allow-list at
`src/target/shared/code/type_utils.rs:49`.

**`io::print`/`io::write`**: `src/builtins/io.rs:6` (`PRINT`/`WRITE`, both `Parameter::required(
"value","String")` → `Nothing`); helper specs `src/target/shared/runtime/io_specs.rs:3`;
`expected_arguments` arm `io.rs:135`.

**String runtime rep**: `src/docs/spec/memory/03_heap-values.md:27` — arena `StringObject { U64
byteLength; utf8Bytes; U8 nul }`; inline-in-record String is a block-relative offset (`:55`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `strings::` functions (overload surface for C/D) | 39 | `grep -cE '^const [A-Z_]+: &str = "strings\.' src/builtins/strings.rs` → 39 |
| Importable builtin packages (adding `astrings` → 24) | 23 | `sed -n '1067,1091p' src/builtins/mod.rs` (`ALL_BUILTIN_PACKAGES`) |
| Family-B registration sites for a new hardcoded type | 12 | enumerated in the machinery map §3 |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| A builtin can dispatch on static arg type | CONFIRMED | `toString` reads `arg_types[0]` in the resolver and `value.type_` in `lower_to_string` |
| Full record opacity (no field *read*) does not exist today | CONFIRMED | map §3: read-only-record blocks construct/WITH only; fields stay readable — so opacity requires a no-fields (primitive-like) type |
| Reserved wire ids 11–19 are free | UNVERIFIED — Prereq | `src/binary_repr/mod.rs:118` comments 11–19 reserved; confirm none claimed |

## 3. Design Overview

**`AttributedString` is an opaque primitive-like built-in type**, not a record. Rationale: the map
shows full record opacity (suppressing field *reads*) does not exist, and the user requirement is
hard opacity (`a.text` must not compile). Modeling it like `String`/`Scalar` — a named type with a
heap representation and *no user-visible fields* — gives opacity for free (any `.field` is an unknown
field) and reuses the `Error`/`ErrorLoc` hardcoded-type wiring.

**Runtime representation:** an arena-allocated heap object holding two values — the visible
`String` and an attribute `List OF Attribute` (always empty in A). Value semantics: copy = deep copy
of both; drop = drop both. Model the layout on the inline-block scheme in `03_heap-values.md` (a
fixed-size header with block-relative offsets to the String block and the list). Exact byte layout is
Detailed Design §4.1.

**Where correctness risk concentrates (schedule last within A):** copy/drop of the compound heap
object across scope exit and into collections — a shallow copy or missed drop is a UAF/leak, invisible
to a happy-path test. The type is introduced with a **no-op empty overlay** first so copy/drop can be
proven before any attribute logic exists.

**Where design uncertainty concentrates (schedule first):** the 12-site Family-B registration — get
the type to *exist and round-trip* (parse a `MUT a AS AttributedString`, default it, `.mfp`
encode/decode) before adding functions.

**Byte-identity is NOT this plan's gate.** This adds behavior; acceptance is rt-behavior + unit
tests. Expect `.ir`/`.ncode` goldens of every fixture importing `astrings` to shift once B–E add the
source companion — that is the feature working, not a regression (see B–E).

**Rejected alternatives:**
- *Opaque record with new "hide all field reads" machinery.* Rejected: adds a general new visibility
  axis to the type system for one type, and still needs a granted default; the primitive-like model
  is smaller and reuses `Error`'s path.
- *`String` + side table (a `String` that sometimes carries spans).* Rejected (same as plan-13-B):
  makes every existing `strings::` function's behavior conditional on hidden state.
- *Inline private-use-block markers (plan-13-B's model).* Rejected by the superseding decision:
  it trades the Tier-B lockstep burden for a visible⇄raw mapping + a `toString` that must strip id
  carriers + marker pollution of every raw-string path. The separate-overlay model keeps the visible
  text a clean `String` and localizes the position-remap work to Tier B.

## 4. Detailed Design

### 4.1 Heap layout
A fixed header (8-aligned) with: an offset/pointer to the visible `String` block and an
offset/pointer to the attribute `List OF Attribute`. Follow `is_pointer_string_record` /
`type_is_flat` conventions (`builder_collection_layout.rs:587/634`) so copy is a deep copy and drop
reclaims both sub-values. Reserve the wire id from the 11–19 band (call it `TYPE_ATTRIBUTED_STRING`).

### 4.2 Registration (the 12 Family-B sites)
Wire `AttributedString` into each site listed in §2 (resolver name table, `Type` variant + display,
inference, `builtin_type_fields` with an **empty** user-field list, defaultable/comparable deltas
= defaultable + NOT comparable, codegen `record_fields`, layout classification, wire encode/decode).

### 4.3 `astrings::fromString`
Native-direct function (like `strings::` members). Codegen: allocate the header, deep-copy the
argument `String` into the String slot, initialize the list slot to an empty `List OF Attribute` via
`lower_empty_collection`.

### 4.4 `toString` / `io::print` / `io::write` overloads
- `toString`: add `"AttributedString"` to the accept set in `general.rs:318` and an arm in
  `lower_to_string` (`builder_strings.rs:757`) that loads the String slot and returns it (deep copy,
  since `toString` yields an owned `String`).
- `io::print`/`io::write`: add an `AttributedString` overload (`io.rs` OV table + `expected_arguments`)
  whose codegen extracts the String slot and reuses the existing String→helper path.

## Compatibility / Format Impact

- **New:** the `AttributedString` type + one reserved wire id; the `astrings` package shell;
  `astrings::fromString`; `AttributedString` overloads of `toString`/`io::print`/`io::write`.
- **Unchanged:** `String`, every existing function's behavior on `String`, all existing wire ids.

## Phases

> **Keep the checkboxes current — tick `- [x]` in the same commit as the work.**

### Phase 1 — the type exists and round-trips (design-uncertainty first)

- [x] Register `AttributedString` at all 12 Family-B sites (§2/§4.2), user-visible field list EMPTY,
      defaultable = yes, comparable = no. Reserve `TYPE_ATTRIBUTED_STRING` in 11–19.
      (Wire id 11 `binary_repr/mod.rs`; encode `sections.rs`; decode `reader.rs`; resolver
      `BUILTIN_TYPES`; frontend `Type::AttributedString` variant + parse/display/walk/comparable/
      copyable/sendable/construct/with-update guards; ir/verify defaultable-only delta + read-only
      update + provably_data; codegen `record_fields` 2-field internal layout `[text:String,
      spans:List OF Integer]`.) Defaultability is a *defaultable-only* delta (`ir/verify/resources.rs
      is_defaultable`), NOT `is_comparable_defaultable_primitive`, so the type stays non-comparable.
      Built green: `cargo build --bin mfb`.
- [x] Confirm opacity: fixtures where `a.text`, `AttributedString["x"]`, and `WITH a {}` each fail to
      compile — `a.text` → `TYPE_UNKNOWN_VALUE`, `AttributedString[...]` →
      `TYPE_READ_ONLY_RECORD_CONSTRUCTOR`, `WITH a {}` → `TYPE_READ_ONLY_RECORD_UPDATE`.
- [x] Tests: `tests/syntax/astrings/opacity-invalid/` (the three rejections, golden captured); Rust
      unit tests in `syntaxcheck::types::types_tests` (`attributed_string_annotation_accepts`,
      `_record_literal_rejected`, `_field_read_rejected`, `_not_comparable`). NOTE: "default is empty"
      is a runtime property proven in Phase 3 (`toString` of a default `AttributedString` == `""`),
      not decidable at type-check time.

Acceptance: MET. `MUT a AS AttributedString` builds and runs to native (exit 0); the three opacity
rejections fire with the documented diagnostics; `.mfp` encode+decode round-trips — committed package
fixture `tests/syntax/astrings/roundtrip-package/` (exports `wrap(a AS AttributedString) AS
AttributedString`; `.info` decode golden), and a consumer importing it builds+runs (verified). Both
fixtures pass `test-accept.sh`.
Commit: —

### Phase 2 — construction + copy/drop (correctness risk last)

- [ ] Register the `astrings` package shell (10 sites, §2) with `fromString` as its only function.
- [ ] Implement `astrings::fromString` codegen (§4.3).
- [ ] Prove value semantics: a fixture that copies an `AttributedString` into a `List`, drops the
      original, and reads the copy back via `toString` (Phase 3) — run under the leak/UAF checks the
      rt-behavior harness applies.

Acceptance: `fromString` builds an `AttributedString`; copy-into-collection + drop-original leaves
the copy valid; no leak/UAF in the rt-behavior run.
Commit: —

### Phase 3 — text seams (`toString`, `io::print`, `io::write`)

- [ ] `toString(AttributedString)` overload (§4.4): accept-set + `lower_to_string` arm.
- [ ] `io::print`/`io::write` `AttributedString` overloads (§4.4).
- [ ] Tests: `tests/rt-behavior/astrings/fromstring-print-rt/` — `io::print(astrings::fromString(
      "hi"))` emits `hi`; `toString(astrings::fromString("hi"))` equals `"hi"`; both `io::print` and
      `io::write` covered.

Acceptance: the rt-behavior fixture prints exactly the visible text and `toString` round-trips.
Commit: —

## Validation Plan

- Tests: syntax goldens (opacity), rt-behavior (fromString/print/toString), Rust unit tests
  (defaultable/not-comparable, resolver arms).
- Coverage check: the rt-behavior fixture exercises the real codegen arms (not just the resolver).
- Runtime proof: the fromString→print fixture emitting `hi`.
- Doc sync: new `astrings` man package skeleton (`src/docs/man/builtins/astrings/`) + a spec section
  stub for the type (fully written in E); `src/docs/spec/language/04_types.md` primitives note if the
  type is listed there.
- Acceptance: `cargo test --bin mfb`; `artifact-gate.sh <exe> all` (expect `astrings`-importer golden
  shifts once the package shell lands — regenerate and confirm the delta is only the new package).

## Open Decisions

1. **Opaque primitive vs opaque record.** Recommended **primitive-like** (this plan) — full record
   opacity doesn't exist and the primitive path reuses `Error`'s wiring. Alternative (record + new
   field-hiding machinery) is larger and still needs a granted default.
   Decision: primitive-like
2. **Wire id: reserved primitive band (11–19) vs `FIRST_TABLE_TYPE_ID` (≥20).** Recommended the
   reserved primitive band, matching the type's primitive-like modeling; falls back to a table id if
   the band is contested.
   Decision: 11

## Corrections

<!-- Filled in during execution. -->

## Summary

A stands up the opaque type and its text seams with an always-empty overlay, so the risky compound
copy/drop is proven before any attribute logic exists. The engineering risk is the heap object's
value semantics; everything attribute-shaped is deferred to B–E. Untouched: `String` and all existing
`String` behavior.
