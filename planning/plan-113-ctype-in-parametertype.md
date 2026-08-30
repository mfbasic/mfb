# C FFI Type Vocabulary in `ParameterType` Plan

Last updated: 2026-08-30
Effort: large (3h–1d)

The C ABI type vocabulary — `CPtr`, `CString`, `CInt32`, … — is carried as a
bare `String` from the parser to the emitted thunk, and five separate
spelling-keyed authority predicates decide on it. This plan folds it into
`ParameterType` as a single new variant, `ParameterType::C(CAbiType)`, where
`CAbiType` is a closed 16-variant enum in `src/types.rs`.

The behavioral outcome: **a C ABI type is a `ParameterType` variant, the five
spelling-keyed predicates become variant matches, and `ctype: String` no longer
exists anywhere after the AST — while every diagnostic, every wire byte, and
every golden stays identical.**

Two findings from the census drive the design, and both contradict what a reader
would assume from the spec prose:

1. **C ABI types already reach `ParameterType` today — as `Named(Symbol)`.**
   `src/resolver/mod.rs:243-264` `is_c_abi_type` matches
   `ParameterType::Named(name)` against 12 C spellings, and its own doc comment
   says so: *"plan-111-B: every C ABI spelling is a nominal (none has a
   variant), so this is the same list asked of the interned `Symbol` the `Named`
   holds."* So this is **not** an additive change to a disjoint domain — it
   moves spellings out of `Named` into a new variant, which is exactly the
   change class `mfb spec architecture type-name-encoding` warns costs a real
   bug (§3, Risk).
2. **The ABI slot ctype namespace is not closed.** It is the 16 primitives
   **plus** any `CSTRUCT` name declared in the same `LINK` alias
   (`src/ir/verify/link.rs:211-216,256-259`: *"A slot may name a CSTRUCT
   declared in the same LINK alias; the struct rules then apply instead of the
   scalar table"*), and 6 committed goldens exercise it
   (`"ctype": "SfFileInfo"`). That open nominal tail is why `ParameterType` —
   which already pairs closed variants with `Named(Symbol)` — is the right home
   for this vocabulary and a standalone `CType` enum is not: a standalone enum
   would have to grow its own `Named` arm and become a second type grammar.

References:

- `mfb spec architecture type-name-encoding` — the canonical grammar, the
  `parse(name).name() == name` round trip, and the **"adding a variant" hazard**:
  *"every consumer has a `_` arm, so an unwired variant is silently mis-handled
  rather than failing to compile. Adding a variant means auditing the matches
  that need it — the resolver, `ir::shape`, `ir::verify`, monomorph's
  unify/normalize, and the TypeModel builder — not just types.rs."* Read this
  section before Phase 1.
- `mfb spec language native-libraries` (`src/docs/spec/language/17_native-libraries.md`)
  — the ctype table (`:94`), the position restrictions (`:110`, `:331`), and the
  `NATIVE_CPTR_ESCAPE` narrower allow-list. **`:94` is wrong** and Phase 5 fixes
  it (§2, Verified properties).
- `src/types.rs:32-180` — the 23 existing `ParameterType` variants, and the
  `Stateful` doc block (`:65-92`) which is the worked example of this exact
  migration, including what it broke.
- `src/ir/link.rs:21-41` `abi_slot_ctype_is_known` — the authority list;
  `:50,67` the two position predicates; `:119` `ctype_size_align`.
- `src/resolver/mod.rs:243` `is_c_abi_type` — the fifth predicate, already on
  `ParameterType::Named`, and the one with the **narrower** 12-name list.
- `src/codegen/link/thunk/link_thunk.rs:2915` `every_known_ctype_lowers` and
  `src/ir/link.rs:946` `ctype_list_is_exhaustive` — the two existing
  exhaustiveness harnesses that keep the authority list and the lowering arms in
  sync. Both become compiler-enforced.
- `planning/completed/plan-111-A-vocabulary-and-ratchet-gate.md`,
  `planning/plan-112-operator-enum.md` — the two sibling vocabulary migrations.
- `.ai/resources-packages.md`, `.ai/codegen-invariants.md`,
  `.ai/testing-gates.md`, `AGENTS.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Tree compiles clean at HEAD | `cargo check --all-targets` → 0 warnings | UNMEASURED — run at kickoff |
| The type-string gate is green and stays green | `cargo test --test no_type_strings` → 7 passed, 0 failed | MET (measured 2026-08-30) |
| `artifact-gate all` reads 0 diffs before any edit | `scripts/artifact-gate.sh all` | UNMEASURED — run at kickoff, **before** the first edit |

The third row is the cheap substitute for a `git archive` baseline build: gate to
0 first, and every diff after is attributable to this plan.

**plan-112 is not a prerequisite.** It touches `operator`/`op`; this plan touches
`ctype`. They share no file except `src/types.rs` (plan-112 adds
`src/operators.rs`, this plan edits `src/types.rs`) and do not braid. Either may
land first.

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop.
>
> **If you stop, report the current status of *all* prerequisites**, not only the
> one that blocked you.

## 1. Goal

- `src/types.rs` gains `CAbiType` — a closed, `Copy` 16-variant enum — and
  `ParameterType::C(CAbiType)` as its 24th variant, with `parse` and `name` arms
  that keep the round trip byte-exact.
- The five spelling-keyed authorities become variant matches:
  `abi_slot_ctype_is_known` (deleted — the type is the proof),
  `abi_ctype_valid_as_argument`, `abi_ctype_valid_as_return`,
  `ctype_size_align`, `resolver::is_c_abi_type`.
- **0** `ctype`/`param_ctype`/`return_ctype`/`abi_return_ctype` fields or
  parameters typed `String`/`&str` anywhere after `src/ast/`.
- **0** comparisons or match arms against a C type spelling outside
  `src/types.rs` and the two IR-binary decode sites.
- The two hand-maintained exhaustiveness harnesses
  (`every_known_ctype_lowers`, `ctype_list_is_exhaustive`) are replaced by
  exhaustive `match`es that rustc checks.
- `tests/no_type_strings.rs` still passes, and gains a class for the C
  vocabulary so a reintroduced `ctype: String` fails `cargo test`.

### Non-goals (explicit constraints)

- **No behavior change.** Every diagnostic — text, code, ordering, location —
  identical on both corpora. In particular `NATIVE_ABI_UNKNOWN_CTYPE`,
  `NATIVE_CSTRUCT_INVALID` and `NATIVE_CPTR_ESCAPE` must fire on exactly the
  same inputs as today. See §3 Risk 2 for the one place this is easy to get
  wrong.
- **No wire-format change.** `src/ir/binary.rs` keeps writing the ctype as the
  same length-prefixed string (`:310,388,392` `put_str`) and reading it back
  (`:615,653,666` `r.string()`). `name()`/`parse` are the codec; the bytes do
  not move.
- **No golden churn.** All `.ast` and `.ir` goldens byte-identical; the `.ncode`
  corpus byte-identical across every target.
- **No new position rules.** `CVoid` stays return-only, `CString`
  argument-only, `CBuffer` OUT-only. This plan changes how those rules are
  *expressed*, never what they permit.
- **No widening of `NATIVE_CPTR_ESCAPE`.** `is_c_abi_type`'s list is
  deliberately 12 of 16 — it excludes `CBool`, `CByte`, `CVoid` (per
  `17_native-libraries.md:94`) and `CBuffer`. It must **not** become
  `matches!(t, C(_))`. (§3 Risk 2.)
- **No nested-CSTRUCT support.** A struct-valued CSTRUCT field stays rejected
  (`src/ir/link.rs:336-344`). This plan does not touch that rule.
- **No new C types.** `CFloat32`/`CFloat64`/`CIntPtr`/`CUIntPtr`/`CSize` stay
  nonexistent (`17_native-libraries.md:110`). Adding a variant for a name the
  backend cannot marshal would be a regression, not a feature.
- **This plan touches only the C type vocabulary.** Not operators (plan-112),
  not names, not `IrType.kind`/`visibility`, not `Operand::Raw`.

## 2. Current State

A ctype is a `String` from the parser to the thunk emitter, and nothing types it:

- **Parsed** as a bare identifier in `src/ast/link_items.rs`; stored as
  `String` on four AST structs — `CStructField.ctype` (`src/ast/types.rs:299`),
  `FreeSpec.param_ctype`/`return_ctype` (`:364,366`),
  `AbiSpec.return_ctype` (`:378`), `AbiSlot.ctype` (`:388`).
- **Passed through HIR untouched, by design.** `src/hir/mod.rs:150-152`:
  *"The C-side field list is untouched: a `ctype` is a C ABI slot spelling, not
  an MFBASIC type."* That comment is the status quo this plan reverses.
- **Carried in the IR** as `IrCStructField.ctype` (`src/ir/link.rs:97`),
  `IrAbiSlot.ctype` (`:872`), `IrLinkFunction.abi_return_ctype` (`:432`).
- **Decided on** by five spelling-keyed authorities:

  | Authority | Location | Shape |
  |---|---|---|
  | `abi_slot_ctype_is_known` | `src/ir/link.rs:21-41` | `matches!` over 16 spellings |
  | `abi_ctype_valid_as_argument` | `src/ir/link.rs:50` | known && `!= "CVoid"` && `!= "CBuffer"` |
  | `abi_ctype_valid_as_return` | `src/ir/link.rs:67` | known && `!= "CString"` |
  | `ctype_size_align` | `src/ir/link.rs:119` | `match ctype` → `(size, align)` |
  | `resolver::is_c_abi_type` | `src/resolver/mod.rs:243` | `matches!` over **12** spellings, on `ParameterType::Named` |

- **Lowered** in `src/codegen/link/thunk/link_thunk.rs` — 49 of the 86 sites,
  the marshaling arms per ctype.
- **Serialized** to the IR binary as a length-prefixed string
  (`src/ir/binary.rs:310,388,392`) and decoded back (`:615,653,666`). It is
  **not** in `src/binary_repr/**` (`grep -rn "ctype" src/binary_repr/*.rs` → no
  matches), so the `.mfp` type table is untouched by this plan.

### Measured populations

All commands at HEAD, 2026-08-30, tests excluded via
`grep -v _tests.rs | grep -v '/tests.rs'`.

| What | Count | Command |
|---|---|---|
| Total ctype sites (fields + `&str` params + spelling decisions) | **86** | `grep -rnE --include="*.rs" '[!=]= *"C(Ptr\|String\|Buffer\|Int8\|Int16\|Int32\|Int64\|UInt8\|UInt16\|UInt32\|UInt64\|Bool\|Byte\|Float\|Double\|Void)"\|^\s*"C(Ptr\|String\|…)"(\s*\|\s*"[^"]*")*\s*=>\|\b(ctype\|param_ctype\|return_ctype\|abi_return_ctype)\s*:\s*(String\|Option<String>\|&(\x27[a-z]+ )?str)' src/ \| grep -v _tests.rs \| grep -v '/tests.rs' \| wc -l` |
| Distinct files | **8** | same pipeline, `cut -d: -f1 \| sort -u \| wc -l` |
| `ctype` `String`/`Option<String>` fields | **10** | `grep -rnE --include="*.rs" "\b(ctype\|param_ctype\|return_ctype\|abi_return_ctype)\s*:\s*(String\|Option<String>)" src/ \| grep -v _tests.rs \| wc -l` |
| `ctype` `&str` parameters | **13** | same, `&(\x27[a-z]+ )?str` |
| C-spelling compares / match arms | **63** | the spelling half of the 86-site regex |
| Existing `ParameterType` variants | **23** | `grep -n "pub(crate) enum ParameterType" -A 200 src/types.rs \| grep -cE "^\s*[0-9]+-\s{4}[A-Z][A-Za-z]*(\(\|,\| \{)"` |
| `_ =>` arms in the five stages the spec names | **84** | `grep -rn --include="*.rs" '_ =>' src/resolver src/ir/shape.rs src/ir/verify src/monomorph/helpers.rs src/codegen/engine/types \| grep -v _tests.rs \| wc -l` (resolver 4, `ir/shape.rs` 20, `ir/verify` 36, `monomorph/helpers.rs` 6, `codegen/engine/types` 18) |
| Goldens carrying a ctype | **5 `.ir` + 6 `.ast`** | `grep -lE '"ctype"' $(find tests -name "*.ir")` / `-name "*.ast"` |

Site distribution (`cut -d: -f1 | sort | uniq -c | sort -rn`) — **72 of 86 are
in two files**:

| Count | File |
|---|---|
| 49 | `src/codegen/link/thunk/link_thunk.rs` |
| 23 | `src/ir/link.rs` |
| 5 | `src/ast/types.rs` |
| 3 | `src/ir/shape.rs` |
| 2 | `src/ir/verify/link.rs` |
| 2 | `src/codegen/engine/builder/mod.rs` |
| 1 | `src/audit/collect/project.rs` |
| 1 | `src/ast/link_items.rs` |

### The vocabulary (measured, closed at 16)

From `src/ir/link.rs:21-41`, cross-checked against
`link_thunk.rs:2923-2926`'s `CTYPES` array (identical set, held in sync by
`ir::link::tests::ctype_list_is_exhaustive`):

`CPtr`, `CString`, `CBuffer`, `CInt8`, `CInt16`, `CInt32`, `CInt64`, `CUInt8`,
`CUInt16`, `CUInt32`, `CUInt64`, `CBool`, `CByte`, `CFloat`, `CDouble`, `CVoid`.

Spellings actually exercised by the golden corpus
(`grep -ohE '"ctype": "[^"]*"' $(find tests -name "*.ast" -o -name "*.ir") | sort | uniq -c`):
`CPtr` 105, `CInt32` 79, `CString` 40, `CInt64` 23, `CBuffer` 7, plus the four
CSTRUCT names `SfFileInfo` (4), `TimeSpec` (2), `CTm` (1), `CTime` (1).

### Verified properties

- **C spellings already become `ParameterType::Named` on the MFBASIC-facing
  side.** Read `src/resolver/mod.rs:243-264`: `is_c_abi_type` destructures
  `ParameterType::Named(name)` and matches `name.resolve()`. Its test at `:851`
  constructs `ParameterType::named("CPtr")`. So `Named("CInt32")` is a value the
  compiler holds today, and moving it to `C(CAbiType::Int32)` changes what every
  `Named(_)` guard sees. **This is the plan's central risk** (§3).
- **`is_c_abi_type`'s list is 12, not 16, and the gap is deliberate.** It omits
  `CBool`, `CByte`, `CVoid` and `CBuffer`. `17_native-libraries.md:94` confirms
  the intent for the first three: *"A separate, narrower allow-list governs
  which C-ABI type names are rejected from a wrapper's MFBASIC-facing signature
  via `NATIVE_CPTR_ESCAPE`; it does **not** include `CBool`, `CByte`, or
  `CVoid`, and it answers the opposite question — do not confuse the two."*
  Converting this predicate to `matches!(t, C(_))` would silently widen
  `NATIVE_CPTR_ESCAPE` to reject four more spellings.
- **The ABI slot namespace has an open nominal tail.** Read
  `src/ir/verify/link.rs:211-216` (`is_cstruct_slot`, a closure over
  `project.link_cstructs` filtered by alias) and `:256-259` (`continue` before
  the scalar check). A slot naming a CSTRUCT bypasses `abi_slot_ctype_is_known`
  entirely. Six goldens exercise it. **CSTRUCT-named slots therefore stay
  `Named(Symbol)`** — they are nominals and `ParameterType` already models them.
- **CSTRUCT *fields* are closed at 16.** `src/ir/link.rs:336-344` rejects a
  struct-typed field before the scalar check, so a field ctype is always one of
  the 16. Field and slot are different domains; do not unify their validation.
- **`17_native-libraries.md:94` is wrong.** It states the 16 names are *"the
  **only** names an `ABI (...)` slot or return may use"*. `is_cstruct_slot`
  and the `SfFileInfo` goldens disprove it for slots. The return is genuinely
  closed (`src/ir/verify/link.rs:246`, no CSTRUCT bypass). **This is a found
  documentation defect**, recorded here and fixed in Phase 5 — the prose needs
  to say "a slot may additionally name a CSTRUCT declared in the same LINK
  alias".
- **The ctype is not in the `.mfp` type table.**
  `grep -rn "ctype" src/binary_repr/*.rs` → no matches. It rides the IR binary
  LINK section only, so `src/binary_repr/sections.rs`'s wire type ids are
  untouched.
- **UNVERIFIED: whether any `_ =>` arm among the 84 currently receives a
  `Named("C…")` and depends on it.** The count is a scan surface, not a defect
  count. Phase 1 task 5 reviews them and records the real number in Corrections.

## 3. Design Overview

**`ParameterType::C(CAbiType)` — one new variant, not sixteen.**

```rust
/// A C ABI type from a LINK binding's `ABI (...)` clause. Closed: the
/// marshaling backend implements exactly these and no user declaration can
/// add one. A CSTRUCT-named slot is NOT here — it is a `Named`.
pub(crate) enum CAbiType {
    Ptr, Str, Buffer,
    Int8, Int16, Int32, Int64,
    UInt8, UInt16, UInt32, UInt64,
    Bool, Byte, Float, Double, Void,
}
```

`CAbiType::name()`/`parse()` render and read the 16 spellings;
`ParameterType::name` gets one arm (`C(c) => c.name()`), and
`ParameterType::parse`'s bare-token path tries `CAbiType::parse` before falling
through to `Named`. Round trip preserved by construction.

Three pieces, layered: (1) the vocabulary and the `Named` audit; (2) the five
authorities and the front-end carriers; (3) codegen's 49 marshaling arms and the
wire seam.

**Where design uncertainty concentrates — schedule FIRST.** The `Named(_)`
audit. `mfb spec architecture type-name-encoding` is explicit that this is the
quiet failure mode, and names the precedent: *"Stateful is the worked example,
and it cost a real bug. … Two guards elsewhere asked `matches!(t, Named(_))`
meaning 'is this a nominal?', and both silently changed answer … Neither failed
to compile; the signal was one acceptance fixture that no longer built."* Phase
1 runs that audit as its own deliverable, before any carrier is retyped, because
it is the cheapest thing that could invalidate the design.

**Where correctness risk concentrates — schedule LAST.** Two places:

- **Risk 1 — the 49 marshaling arms** in `link_thunk.rs`. A swapped arm moves
  the wrong width into a register and byte-identity catches it; a swapped arm
  between two same-width types (`CInt64`/`CUInt64`) does **not** move a byte and
  byte-identity is blind to it. Per `register-slot-import-bugs-need-codegen-inspection`,
  convert arm-for-arm in source order and never merge two arms in the commit
  that retypes them.
- **Risk 2 — `resolver::is_c_abi_type` widening.** The obvious conversion
  (`matches!(t, ParameterType::C(_))`) is **wrong**: it silently adds `CBool`,
  `CByte`, `CVoid`, `CBuffer` to `NATIVE_CPTR_ESCAPE`'s rejection set. The
  correct conversion enumerates the same 12 `CAbiType` variants. Phase 2 carries
  a dedicated task and a negative test for this.

**Byte-identity is this plan's correctness gate, and it is the right one.** This
is provably-neutral work: the same decisions on a variant instead of a spelling.
No target is expected to diff, and no golden should churn. Per `AGENTS.md`, a
diff means objdump one fixture, localize, and fix — never that the design is
dead.

**Rejected alternatives:**

- *16 flat variants on `ParameterType`* (`ParameterType::CInt32`, …). Rejected:
  grows a 23-variant enum to 39, forces 16 new arms at every exhaustive match
  instead of 1, and buries the fact that these form one closed sub-vocabulary.
  `C(CAbiType)` gives the same expressiveness with one arm and one containment
  predicate (`matches!(t, C(_))`).
- *A standalone `CType` enum outside `ParameterType`* (the plan-112 shape).
  Rejected on the census: the slot namespace has an open CSTRUCT tail
  (§2 Verified properties), so a standalone enum needs its own `Named` arm and
  becomes a second type grammar — the exact duplication
  `mfb spec architecture type-name-encoding` and plan-111 exist to prevent.
- *Leaving `ctype` a `String` and only typing the five authorities.* Rejected:
  the authorities are 5 of 86 sites; the other 81 keep the spelling alive and
  the gate cannot read zero.
- *Adding the missing `CFloat32`/`CIntPtr`/`CSize` spellings while here.*
  Rejected: non-goal. They do not exist and the backend cannot marshal them.

## Compatibility / Format Impact

| Contract | Change |
|---|---|
| IR binary LINK section | **None on the wire.** `put_str(o, &f.ctype)` becomes `put_str(o, f.ctype.name())`; decode becomes `CAbiType::parse(&r.string()?)` for a field, and `ParameterType::parse` for a slot (which may be a CSTRUCT nominal). |
| `.mfp` type table | **Untouched** — ctype is not in `src/binary_repr/**`. |
| `.ast` / `.ir` goldens | **Byte-identical.** `name()` renders the same spellings. |
| `.ncode` / `.ncodesum` | **Byte-identical**, all targets. |
| Diagnostics | **Identical.** `NATIVE_ABI_UNKNOWN_CTYPE`, `NATIVE_CSTRUCT_INVALID`, `NATIVE_CPTR_ESCAPE` fire on the same inputs with the same text. |
| `mfb spec language native-libraries` | **Corrected** — `:94`'s "only names" claim gains the CSTRUCT-slot case (§2, a found defect). |
| `mfb man` | Untouched. |

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; mark
> moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill `Commit:` when a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — `CAbiType`, the variant, and the `Named` audit

Lands the vocabulary with no carriers retyped, and settles the one thing that
could invalidate the design. Safe alone: nothing constructs a `C(_)` yet, so no
`Named(_)` guard can yet see one.

- [ ] Add `CAbiType` (16 variants, `Clone, Copy, PartialEq, Eq, Hash, Debug`)
      to `src/types.rs`, with `name(self) -> &'static str` and
      `parse(&str) -> Option<Self>` over the exact 16 spellings from
      `src/ir/link.rs:21-41`.
- [ ] Add `ParameterType::C(CAbiType)`; add its `name` arm and its `parse` arm
      (tried in the bare-token path **before** the `Named` fallback, so
      `parse("CInt32")` yields `C(Int32)` and `parse("SfFileInfo")` still yields
      `Named`).
- [ ] Add `ParameterType::c_abi(self) -> Option<CAbiType>` as the single
      containment accessor. Do **not** add a broad `is_c()` helper that invites
      the Risk-2 mistake.
- [ ] **Audit every `Named(_)` guard** for one that means "is this a nominal?"
      and would change answer now that C spellings are not `Named`. Start from
      the 84 `_ =>` arms and every `matches!(…, Named(_))` in the five stages the
      spec names (resolver, `ir::shape`, `ir::verify`, `monomorph::helpers`,
      the TypeModel builder). Record the real count and each verdict in
      Corrections — this is a deliverable, not a background check.
- [ ] Tests: `src/types.rs` `#[cfg(test)]` — (a) `parse(name(c)) == Some(c)` for
      all 16, enumerated explicitly; (b) `parse("SfFileInfo")` is `Named`, not
      `C`; (c) `parse("CPtrX")`, `parse("cptr")`, `parse("")` are not `C`;
      (d) a golden-corpus test asserting every `"ctype"` string in
      `tests/**/*.ast` and `tests/**/*.ir` round-trips byte-exactly through
      `ParameterType::parse`/`name`.

Acceptance: `cargo test --no-fail-fast` green; the round-trip and corpus tests
pass; `cargo test --test no_type_strings` still 7 passed / 0 failed; the
`Named(_)` audit is written into Corrections with a verdict per site.
Commit: —

### Phase 2 — the five authorities and the front-end carriers

Converts the decision points and the AST/IR carriers. 23 of 86 sites
(`src/ir/link.rs`) plus the resolver and the AST structs.

- [ ] Retype the AST carriers to `ParameterType`: `CStructField.ctype`
      (`src/ast/types.rs:299`), `FreeSpec.param_ctype`/`return_ctype`
      (`:364,366`), `AbiSpec.return_ctype` (`:378`), `AbiSlot.ctype` (`:388`);
      fix the mint site in `src/ast/link_items.rs`.
- [ ] Retype the IR carriers: `IrCStructField.ctype` (`src/ir/link.rs:97`),
      `IrLinkFunction.abi_return_ctype` (`:432`), `IrAbiSlot.ctype` (`:872`).
- [ ] **Delete `abi_slot_ctype_is_known`** (`src/ir/link.rs:21-41`) — a
      `CAbiType` *is* the proof of knownness. Its three production callers are
      `:51` and `:68` (the two position predicates, which absorb it) and `:345`
      (the CSTRUCT-field check, which becomes "parse failed"), preserving the
      exact `NATIVE_ABI_UNKNOWN_CTYPE` text. Note `:580` emits the same rule
      code for the `CBuffer`-as-ABI-return position rule but does **not** call
      this predicate — leave it alone beyond retyping its `== "CBuffer"`.
- [ ] Delete the unreachable arm at
      `src/codegen/link/thunk/link_thunk.rs:1872`, whose own comment says it is
      gated by `abi_slot_ctype_is_known`. With a `CAbiType` scrutinee the state
      is unrepresentable, so the arm is not merely unreachable but ill-typed.
- [ ] Convert `abi_ctype_valid_as_argument` (`:50`) and
      `abi_ctype_valid_as_return` (`:67`) to `CAbiType` matches. Keep the
      position rules identical: argument = all but `Void` and `Buffer`; return =
      all but `Str`.
- [ ] Convert `ctype_size_align` (`:119`) to a `CAbiType` match. It returns
      `None` for the storage-less types today; keep that, now as explicit arms.
- [ ] **Convert `resolver::is_c_abi_type` (`src/resolver/mod.rs:243`) to a
      12-variant `CAbiType` match — NOT `matches!(t, C(_))`.** (§3 Risk 2.) The
      excluded four are `Bool`, `Byte`, `Void`, `Buffer`.
- [ ] Tests: extend `is_c_abi_type_recognizes_and_rejects`
      (`src/resolver/mod.rs:851`) with **negative** assertions that `CBool`,
      `CByte`, `CVoid` and `CBuffer` are NOT C-ABI types for this predicate. That
      test is the regression guard for Risk 2; write it before the conversion.
- [ ] `src/ir/verify/link.rs`: `is_cstruct_slot` (`:211`) now compares a
      `Named`'s symbol against the alias's CSTRUCT names; the scalar path takes
      a `CAbiType`.

Acceptance: `cargo test --no-fail-fast` green, including the new negative
`is_c_abi_type` assertions; all `.ast` and `.ir` goldens byte-identical
(`scripts/test-accept.sh <target> /tmp/accept-113` → 0 mismatches, same `N ran`
as the kickoff baseline); the six CSTRUCT-slot goldens still build.
Commit: —

### Phase 3 — codegen marshaling and the wire seam (largest blast radius)

The 49 `link_thunk.rs` sites plus the IR binary codec. Last: this is where a
wrong arm moves wrong bytes.

- [ ] Convert `src/codegen/link/thunk/link_thunk.rs`'s marshaling arms to
      `CAbiType`, **one arm at a time in source order**. Do not merge arms in
      the same commit that retypes them (§3 Risk 1).
- [ ] Convert `src/codegen/engine/builder/mod.rs` (2 sites) and
      `src/audit/collect/project.rs` (1 site).
- [ ] `src/ir/binary.rs`: encode via `name()` at `:310,388,392`; decode at
      `:615,653,666` via `CAbiType::parse` for a CSTRUCT **field** (closed) and
      `ParameterType::parse` for an ABI **slot** (may be a CSTRUCT nominal).
      A decode failure must be an error, never a default — mirror
      `AbiDirection`'s wire contract (`src/ir/link.rs:842-844`).
- [ ] **Delete the two hand-maintained exhaustiveness harnesses** now that rustc
      enforces them: `every_known_ctype_lowers`
      (`link_thunk.rs:2915`) and `ctype_list_is_exhaustive` (`src/ir/link.rs:946`).
      Replace with a comment naming the exhaustive `match` that supersedes each.
      Per `AGENTS.md`, state the proof in the commit: the test protected "a name
      in the authority list has a lowering arm", which an exhaustive match on a
      closed enum makes a compile error.
- [ ] Verify no `_ =>` arm was left on a `CAbiType` match:
      `grep -rn -B 20 '_ =>' src/codegen/link/thunk/link_thunk.rs src/ir/link.rs`
      reviewed for `CAbiType` scrutinees.

Acceptance: `scripts/artifact-gate.sh all` → **0 diffs** (same reading as the
Prerequisites row, so any diff en route is attributable and gets root-caused,
not regenerated); `scripts/test-accept.sh <target> /tmp/accept-113` → 0
mismatches; `cargo test --no-fail-fast` green; the `rt-behavior/native/**`
fixtures (`libsnd-open-file-info-rt`, `libsnd-read-samples-rt`,
`native-struct-scalar-rt`, `native-struct-cstring-rt`) **execute** correctly —
a same-width arm swap is invisible to byte-identity and only running catches it.
Commit: —

### Phase 4 — lock the gate

- [ ] Extend `tests/no_type_strings.rs` with a C-vocabulary class: **0**
      `ctype`/`param_ctype`/`return_ctype`/`abi_return_ctype` typed
      `String`/`Option<String>`/`&str` outside `src/ast/`, and **0** comparisons
      or match arms on a C spelling outside `src/types.rs` and the two decode
      sites. Assert hard zero, no budget row.
- [ ] Re-run every §Measured populations command and paste the new counts into
      Corrections. Every line must read 0 except the vocabulary itself.

Acceptance: `cargo test --test no_type_strings` passes with the new class at
hard zero; the re-measured census reads 0.
Commit: —

### Phase 5 — docs and archive

- [ ] **Fix `src/docs/spec/language/17_native-libraries.md:94`**: the 16 names
      are the only names for an ABI *return* and for a CSTRUCT *field*, but a
      *slot* may additionally name a CSTRUCT declared in the same LINK alias
      (`src/ir/verify/link.rs:211-216,256-259`; goldens
      `libsnd-open-file-info-rt`, `native-struct-scalar-rt`, +4). Keep the
      citation markers (`[[src/ir/link.rs:abi_slot_ctype_is_known]]` must be
      repointed — that function is deleted in Phase 2).
- [ ] Sweep for other citations of the deleted `abi_slot_ctype_is_known`:
      `grep -rn "abi_slot_ctype_is_known" src/docs .ai planning` → repoint each.
- [ ] Update `mfb spec architecture type-name-encoding` to list `C(CAbiType)`
      among the variants and to record that a C ABI spelling is no longer a
      `Named` (the section's own "audit every `Named(_)`" guidance gains a second
      worked example).
- [ ] `.ai/resources-packages.md`: record that the ctype vocabulary is a
      `ParameterType` variant and that CSTRUCT-named slots remain `Named`.
- [ ] Move `planning/plan-113-ctype-in-parametertype.md` to `planning/completed/`.

Acceptance: `mfb spec language native-libraries` renders the corrected prose;
no dangling citation to `abi_slot_ctype_is_known`
(`grep -rn "abi_slot_ctype_is_known" src/ .ai planning` → 0 outside this plan's
Corrections); `cargo test --no-fail-fast` green.
Commit: —

## Validation Plan

- **Tests:** `src/types.rs` round-trip + corpus tests (Phase 1); the negative
  `is_c_abi_type` assertions for `CBool`/`CByte`/`CVoid`/`CBuffer` (Phase 2 —
  the Risk-2 regression guard); `tests/no_type_strings.rs`'s new class (Phase 4).
  The two deleted exhaustiveness harnesses are replaced by rustc, and the commit
  must say so with the proof.
- **Coverage check:** `mfb` is a binary crate — measure with `--bin mfb`
  (`coverage-measurement-mechanics`). Confirm `CAbiType`'s arms are in the
  denominator; a `const`-shaped table needs a `black_box` runtime test.
- **Runtime proof:** the four `tests/rt-behavior/native/**` fixtures must
  execute with correct output, not merely build. This is the only check that
  catches a same-width arm swap (`CInt64`↔`CUInt64`), which byte-identity cannot
  see. Also run one `libsnd` fixture end to end.
- **Doc sync:** `src/docs/spec/language/17_native-libraries.md` (the `:94`
  defect), `mfb spec architecture type-name-encoding`,
  `.ai/resources-packages.md`, and every citation of the deleted
  `abi_slot_ctype_is_known`.
- **Acceptance:** `cargo test --no-fail-fast` (never bare `cargo test` — it
  fail-fasts at `golden.rs` and silently skips every `rt_*` test);
  `scripts/artifact-gate.sh all`; `scripts/test-accept.sh <target>
  /tmp/accept-113` (**never** a real directory as the second argument — it is
  `rm -rf`'d); `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup
  run 1.96.0 cargo fmt)`.

## Open Decisions

1. **One variant `C(CAbiType)` or 16 flat variants?** — **Recommended:
   `C(CAbiType)`.** One arm per consumer instead of sixteen, keeps
   `ParameterType` at 24 variants, and gives a single containment predicate.
   The flat form's only advantage is shorter patterns at the ~63 decision sites,
   which is not worth 16 new arms at every exhaustive match. (§3)
2. **Does `CAbiType` live in `src/types.rs` or its own module?** —
   **Recommended: `src/types.rs`.** `tests/no_type_strings.rs` exempts exactly
   one grammar file (`the_grammar_file_is_exactly_one`), and a C spelling is a
   type spelling; putting the vocabulary anywhere else forces a second exemption
   and re-opens the "how many grammar files are there" question plan-111-A
   closed. Note this differs from plan-112, where operators are *not* types and
   correctly get their own file. (§3)
3. **Should Phase 2 delete `abi_slot_ctype_is_known` or keep it as a thin
   `Option`-returning wrapper?** — **Recommended: delete.** Keeping it preserves
   a spelling-keyed entry point the gate would have to exempt, and its two
   callers read more clearly as "parse failed". (Phase 2)
4. **Does the `NATIVE_ABI_UNKNOWN_CTYPE` message text survive verbatim when the
   check becomes "parse failed"?** — **Recommended: yes, verbatim**, since the
   non-goals forbid a diagnostic change and the fixtures pin it. Flagged because
   the natural rewrite tempts a reword. (Phase 2)

## Corrections

<!-- Filled in DURING execution. Every place this plan turned out to be wrong:
     the claim, what was actually true, and the evidence. Phase 1's `Named(_)`
     audit result goes here. -->

## Summary

The real engineering risk is not the 86-site conversion — 72 of those sites are
in two files and rustc finds every one. It is that C spellings are `Named` today
(`src/resolver/mod.rs:243`), so this plan moves values *out* of `Named`, which is
the precise change class the spec documents as costing a real bug with
`Stateful`. Phase 1 therefore ships the `Named(_)` audit as a deliverable before
any carrier is retyped, and Phase 2 carries a dedicated negative test for the one
conversion that is wrong in the obvious way — `resolver::is_c_abi_type`, whose
12-of-16 list is deliberate and must not become `matches!(t, C(_))`.

Left untouched, deliberately: CSTRUCT-named ABI slots stay `Named(Symbol)`
(they are nominals, and the namespace is genuinely open); nested CSTRUCTs stay
rejected; no C type is added or removed; and every other post-AST string
category — operators (plan-112), binding and function names, `IrType.kind`,
`Operand::Raw` — is a separate plan.
