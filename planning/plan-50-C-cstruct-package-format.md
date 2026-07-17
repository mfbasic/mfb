# plan-50-C: `CSTRUCT` in the `.mfp` format, and the package-path gate

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-50-B (`IrCStruct` and `compute_c_layout` must exist)

Carries `CSTRUCT` declarations and struct-slot direction through the `.mfp`
Binary Representation, bumps `BINARY_REPR_VERSION` `4` → `5`, and hardens the
package-path verifier so a **crafted `.mfp` cannot forge a struct layout**.

This lands before any codegen (plan-50-E) on purpose: it defines the complete
wire format once, so D consumes it without a second format break. A struct slot
still fails to compile after this phase — `ir::verify` accepts and round-trips it,
codegen does not yet lower it.

The single behavioral outcome: a binding's `CSTRUCT` table survives an
encode→decode round-trip byte-identically, and a hand-corrupted link table
claiming an impossible struct is **rejected by `ir::verify`** rather than reaching
the marshaling backend.

References (read first):

- `src/ir/binary.rs:BINARY_REPR_VERSION` (`:18`) — currently `4`. The decoder is
  an **exact-match** check, not a floor (`:229-233`):
  ```rust
  let version = r.u16()?;
  if version != BINARY_REPR_VERSION {
      return Err(format!("Binary Representation version {version} unsupported (expected {BINARY_REPR_VERSION})"));
  }
  ```
- `src/ir/binary.rs:encode_project` (`:240`) / `decode_project` (`:344`) — the
  link trailer is presence-gated on write (`:259-264`) and EOF-gated on read
  (`:358`, via `IrReader::at_end:195-197`), so a `LINK`-free package stays
  byte-identical.
- `src/ir/binary.rs:encode_link_function` (`:268`) / `decode_link_function`
  (`:385`) — the 14-field positional record; ABI slots at `:279-283` are
  `(str name, str ctype, bool is_out)` with **no tag and no length prefix**, so
  any new field is a hard format break.
- `src/ir/verify/mod.rs:check_link_functions` (`:2610`) — the package-path gate;
  its header comment (`:2602-2609`) states the rationale: *"a crafted .mfp's link
  table drives raw C calls, so these are marshaling-safety gates."*
- `src/ir/verify/mod.rs:2756-2768` vs `src/syntaxcheck/mod.rs:592-619` — **the
  anti-pattern this plan must not repeat.** `IrFree` (`src/ir/link.rs:51`) keeps
  only `slot`+`symbol`; the deallocator's ctypes are dropped at lowering
  (`src/ir/lower.rs:346-349`), so the package path can only check
  `free.symbol.is_empty()` while the source path enforces the real signature. A
  crafted `.mfp` therefore gets a **weaker** FREE check than source.
- `src/binary_repr/mod.rs` — `SECTION_BINARY_REPR = 16` (`:48`),
  `MFPC_MAJOR_VERSION = 2` (`:52`), `ABI_FORMAT_VERSION = 1` (`:54`).
- `planning/audit-2-package-decode.md` and the `audit-1-package-decode-impl`
  memory (PKG-01..07: depth caps, cycle guards, bounded allocation) — the
  established hardening posture for this decoder.

## 1. Goal

- `IrProject.link_cstructs` encodes and decodes, round-tripping byte-identically.
- `IrAbiSlot` carries a direction (`In`/`Out`/`InOut`) instead of a bare
  `is_out: bool`, encoded as a `u8`.
- Every other planned wire change rides this one bump (§4.2b): `IrLinkExpr::Var`'s
  name payload (plan-50-I) and `bind_in` (plan-50-E). **No `result_slot`** — the
  accepted `RESULT`→`RETURN` unification reuses the existing `result` field.
- `BINARY_REPR_VERSION` is `5`; a `v4` package is rejected with the existing
  clear message, and `bindings/sqlite3/sqlite3.mfp` is regenerated.
- `ir::verify` **fully re-validates** every decoded `CSTRUCT` — every check
  plan-50-B applies at source, applied again on the package path, with no
  weaker-on-the-package-path gap of the kind `IrFree` has today.
- A crafted link table cannot influence a struct's memory layout **by
  construction**: offsets and sizes are never transported (§3).

### Non-goals (explicit constraints)

- No codegen. `link_thunk.rs` is untouched; a struct slot still fails to lower.
- No change to `MFPC_MAJOR_VERSION` (`2`) or the container's section framing —
  the link trailer rides **inside** `SECTION_BINARY_REPR` (`16`), so no new
  section id is allocated and section 10 (native library table) and section 11
  (resource table) are untouched.
- The link trailer's presence gating (`:259`/`:358`) does not change: a package
  with no `LINK` block must stay byte-identical to today's encoding apart from
  the version word.
- No relaxation of any existing decode hardening (`MAX_DECODE_DEPTH = 256`,
  `src/ir/binary.rs:87`).

## 2. Current State

The link table is not its own `.mfp` section — it is a trailer inside the IR
payload of `SECTION_BINARY_REPR` (`16`). Payload header: `MFBR` magic
(`src/ir/binary.rs:4`) + `u16` LE version. Primitives: `put_str` = `u32` LE length
+ UTF-8 bytes (`:49`); `put_vec` = `u32` LE count + elements (`:65`); `put_bool` =
one byte (`:34`).

`encode_link_function` (`:268`) writes 14 fields positionally; encoder and decoder
are exact mirrors. ABI slots (`:279-283`):

```rust
put_vec(out, &f.abi_slots, |o, slot| {
    put_str(o, &slot.name);
    put_str(o, &slot.ctype);
    put_bool(o, slot.is_out);
});
```

Because slots are positional with no per-slot tag, adding a field is a hard break
— hence the version bump. Note `consts` are serialized as **decimal strings**
(`:286-289`, parsed back at `:403-410`), as is `IrLinkExpr::Int` (`:315-318`);
variable-width but harmless, and out of scope here.

Today's verifier (`check_link_functions:2610`) checks: `NATIVE_CPTR_ESCAPE` over
wrapper params/return (`:2633-2652`), slot binding (`:2661` loop → `NATIVE_CONST_OUT`
`:2676`, `NATIVE_ABI_UNBOUND_SLOT` `:2687`/`:2697`), result markers (`:2707`), and
a `free.symbol.is_empty()` stub (`:2758-2768`).

The `IrFree` asymmetry is the cautionary precedent, and it is documented as
deliberate at `src/ir/verify/mod.rs:2756` — *"The IR's FREE form keeps only
slot+symbol (the deallocator's signature check stays in syntaxcheck)"*. The
consequence is that the package path, the one an attacker controls, is the weaker
of the two. **This plan inverts that default for structs.**

## 3. Design Overview

The security property is achieved by what the format **does not** carry:

```
  transported:      field NAMES + field CTYPES, in declaration order
  NOT transported:  size, alignment, field offsets, padding
```

The decoder re-runs `compute_c_layout` (plan-50-B §4.2) over the decoded ctypes.
There is no attacker-supplied offset to validate, because an offset is never a
wire value — it is always derived. A crafted `.mfp` can therefore only choose
*ctypes*, and every ctype is drawn from plan-50-A's closed allow-list with a
known size/align. The worst a crafted table can do is declare a struct that is
valid but wrong for the real C library, which is exactly the same authority a
binding author already has by writing the `CSTRUCT` by hand — no privilege
escalation.

This is why plan-50-B computes rather than declares layout (B §3): the
package-path gate is a *consequence* of that choice, not an extra check bolted on.

Layered:

```
  IrProject.link_cstructs ─── encode_cstruct ──► [.mfp SECTION_BINARY_REPR trailer]
                                                            │
                                                  decode_cstruct
                                                            ▼
                                       ir::verify::check_link_cstructs   <-- full re-validation
                                                            │
                                                  compute_c_layout       <-- offsets DERIVED here
```

**Where the risk concentrates:** the version bump. `BINARY_REPR_VERSION` is an
exact-match gate, so an unbumped field addition makes every existing `v4` package
decode garbage into the new field rather than erroring — silently, into a
struct-layout input. The bump is mandatory and is the first task, not the last.

Rejected alternative: **transport the computed layout** (size/align/offsets) so
the decoder need not recompute. Rejected on two counts: it hands an attacker
direct control of the offsets the thunk will read/write — the single most
dangerous value in the feature — and it creates a consistency problem (what if the
transported offsets disagree with the ctypes?) whose only safe resolution is to
recompute and compare, i.e. do the work anyway. Recomputing is strictly less code
and strictly safer.

Rejected alternative: **a new `.mfp` section for the struct table**. Rejected:
struct declarations are meaningless without the link functions that use them, and
the trailer is already the established home for LINK data; a new section id would
also touch the container spec (`src/docs/spec/package/01_container-format.md`) for
no benefit.

Rejected alternative: **keep `is_out: bool` and append `is_inout: bool`.** Two
bools with an illegal `(true, true)` combination is a smell, and every reader
would have to know the precedence. See §Open Decisions.

## 4. Detailed Design

### 4.1 Direction

`IrAbiSlot.is_out: bool` (`src/ir/link.rs:63`) becomes:

```rust
pub enum AbiDirection { In, Out, InOut }   // encoded u8: 0, 1, 2
```

Wire: replace `put_bool(o, slot.is_out)` with `put_u8(o, slot.direction as u8)`.
Decode rejects any byte outside `0..=2` (an unknown direction must be an error,
never a default — the `eval_link_const` catch-all `_ => 0`
(`src/ir/lower.rs:398`) is the in-tree example of why a silent default in this
layer is a hazard).

Call sites to migrate (`is_out` → `direction`): `src/ir/link.rs:63`,
`src/ir/lower.rs:321-325`, `src/ir/binary.rs:279-283`/`:394-400`,
`src/ir/verify/mod.rs:2676`/`:2687`, `src/target/shared/code/link_thunk.rs:337`/
`:353`/`:406`, `src/target/shared/nir/json.rs:376`, `src/ast/types.rs:339`
(`AbiSlot.is_out`) and `src/ast/items.rs:926` (the `match_identifier_ci("OUT")`
parse). Test fixtures: `src/ir/coverage_tests.rs:330-336`,
`src/ir/verify/tests.rs:2567,2592,2614-2620,2635-2641,2655-2656`.

`InOut` is unreachable from source in this phase — plan-50-E adds the `INOUT`
keyword to `parse_abi_spec`. Encoding it now is deliberate: it makes E a pure
codegen phase with no second format break.

### 4.2 The struct table

Appended to the link trailer, after `link_functions` and `link_aliases`, so the
presence gate (`:259-264`) and EOF gate (`:358`) keep their current shape:

```rust
// encode_project, after link_aliases
put_vec(out, &project.link_cstructs, encode_cstruct);

fn encode_cstruct(out: &mut Vec<u8>, s: &IrCStruct) {
    put_str(out, &s.alias);          // owning LINK alias
    put_str(out, &s.name);           // the C-side name (never escapes the LINK block)
    put_str(out, &s.maps_to);        // the AS <MfbType> record it presents as
    put_vec(out, &s.fields, |o, f| { put_str(o, &f.name); put_str(o, &f.ctype); });
}
```

Field order is the wire order — it **is** the layout input, so it must round-trip
exactly. No offsets, no size (§3).

The trailer's presence condition widens to
`!link_functions.is_empty() || !link_aliases.is_empty() || !link_cstructs.is_empty()`.

### 4.2b The other fields riding this bump

The slot record is positional with no tags, so **every** field addition is a hard
format break (§2). Carrying every planned field here means **one** version bump for
the whole feature instead of four, and leaves the later phases pure
parser/codegen work. Each is inert until its phase parses the surface that sets it.

- **`IrLinkExpr::Var` gains a `String` payload** (plan-50-I §4.3). Tag 0 currently
  encodes as a bare `put_u8(out, 0)` (`src/ir/binary.rs:314`); it becomes
  `put_u8(0); put_str(name)`. This is the one that makes `RESULT`/`SUCCESS_ON`
  expressions able to name a slot rather than one nameless variable.
- **`bind_in`** (plan-50-E §4.1) — per-slot `field = <param|literal>` bindings.

**No `result_slot` field.** An earlier draft added one for plan-50-H's `RETURN
<name>`. The accepted `RESULT`→`RETURN` unification (plan-50-H) makes it
unnecessary: `RETURN <expr>` reuses the **existing**
`IrLinkFunction.result: Option<IrLinkExpr>` (`src/ir/link.rs:43`), already encoded
at `src/ir/binary.rs:291`. `RETURN db` is `result = Var("db")`. The unification
removed a planned field rather than adding one.

`ir::verify` must reject a `Var(name)` naming no slot even though source cannot yet
produce one: the package path does not get to assume the frontend ran.

### 4.3 The gate

New `ir::verify::check_link_cstructs`, called from `verify_semantics` beside
`check_link_functions`, re-applying **every** plan-50-B §4.4 rule on decoded IR:

| Condition | Rule |
|---|---|
| field ctype outside the allow-list | `NATIVE_ABI_UNKNOWN_CTYPE` |
| `CVoid` field / zero fields / duplicate field / duplicate struct name / nested | `NATIVE_CSTRUCT_INVALID` |
| `compute_c_layout` size > `MAX_CSTRUCT_SIZE` | `NATIVE_CSTRUCT_TOO_LARGE` |
| a slot's ctype names a `CSTRUCT` not declared in that slot's LINK alias | `NATIVE_CSTRUCT_INVALID` |

The last row is new to the package path: source resolution guarantees it, decode
does not.

**No new rule codes.** This phase reuses plan-50-A's `2-203-0123` and plan-50-B's
`2-203-0124`/`0125`. `src/rules/table.rs` is hand-maintained and not generated;
`src/docs/spec/diagnostics/02_error-codes.md` (the `build.rs:178` build input) is
untouched — no runtime error code is added.

### 4.4 Bounding the decode

Consistent with the PKG-01..07 posture (`audit-1-package-decode-impl`):

- Cap `link_cstructs` count and per-struct field count. `put_vec` reads a
  `u32` count, so an unbounded count is an allocation primitive. Recommend
  **256** structs per package and **64** fields per struct — both far above any
  real binding, both cheap to enforce at decode.
- `MAX_CSTRUCT_SIZE` (plan-50-B, 1024 bytes) is enforced again here via
  `compute_c_layout`. Enforcing at both layers is intentional: B stops a
  bad *source* binding, C stops a bad *package*.

### 4.5 Regenerating checked-in packages

`bindings/sqlite3/sqlite3.mfp` is committed and encodes `BINARY_REPR_VERSION 4`.
The bump makes it undecodable, so it **must be rebuilt in the same commit** or
`tests/rt-behavior/native/native-link-import-sqlite-rt` (which imports the built
binding) breaks. Grep for any other committed `.mfp` before landing.

## Compatibility / Format Impact

- **Changes:** `BINARY_REPR_VERSION` `4` → `5`. Every `v4` `.mfp` is rejected with
  the existing message from `:229-233`. This is a **breaking format change** for
  any package built by a prior compiler — acceptable under the project's
  version-locked-spec model (the embedded spec always matches the binary), and
  the `v2→v3→v4` history shows the established convention: bump, add a
  `/// Version 5 …` doc line, regenerate.
- **Changes:** the ABI-slot wire record is `(str, str, u8)` where it was
  `(str, str, bool)`.
- **Unchanged:** `MFPC_MAJOR_VERSION` (`2`), `ABI_FORMAT_VERSION` (`1`), the
  container's section framing, sections 10/11, `MAX_DECODE_DEPTH`, and the
  byte-identity of a `LINK`-free package's trailer (still absent).
- Registry impact: a published `v4` package cannot be consumed by a `v5`
  compiler. Confirm with the plan-48/49 blob work whether any published binding
  exists that would need a republish — likely none, but check.

## Phases

One landable unit.

### Phase 1 — format, gate, regeneration

- [ ] **First:** bump `BINARY_REPR_VERSION` to `5` in `src/ir/binary.rs:18` with a
      `/// Version 5: CSTRUCT table + ABI slot direction` doc line, following the
      v2/v3/v4 convention.
- [ ] `src/ir/link.rs`: replace `IrAbiSlot.is_out: bool` with
      `direction: AbiDirection`; add `IrCStruct.alias` and `IrCStruct.maps_to`
      (the `AS <MfbType>` target, plan-50-B §4.1); add the `bind_in` table (§4.2b).
      (`IrLinkExpr::Var(String)` is plan-50-I's edit — coordinate so both land
      under this single bump.)
- [ ] Migrate every `is_out` call site listed in §4.1 (compiler + test fixtures).
- [ ] `src/ir/binary.rs`: `put_u8`/`u8` direction in `encode_link_function:279-283`
      / `decode_link_function:394-400`, rejecting a byte outside `0..=2`; encode the
      `bind_in` table as tagged field 15 (§4.2b); add `encode_cstruct`/`decode_cstruct`
      (incl. `maps_to`); extend the trailer in `encode_project:259-264` /
      `decode_project:358`; widen the presence gate.
- [ ] `src/ir/binary.rs`: enforce the §4.4 count caps at decode.
- [ ] `src/ir/verify/mod.rs`: add `check_link_cstructs` per §4.3; call it from
      `verify_semantics`.
- [ ] Regenerate `bindings/sqlite3/sqlite3.mfp` (§4.5); grep for other committed
      `.mfp` files and regenerate those too.
- [ ] Spec: update `src/docs/spec/package/02_binary-representation.md` — the
      version, the slot record's `u8` direction, and the `CSTRUCT` trailer entry.
      Note in `src/docs/spec/language/17_native-libraries.md` that offsets are
      never transported and are recomputed at decode. Cite
      `[[src/ir/binary.rs:encode_cstruct]]`.
- [ ] Tests: round-trip byte-identity for a project with `CSTRUCT`s and each
      direction, beside the existing assertions at
      `src/ir/coverage_tests.rs:505-513`.
- [ ] Tests: `src/ir/verify/tests.rs` — a decoded struct with an unknown field
      ctype, an oversized struct, a slot naming an undeclared struct, and a
      direction byte of `3` are each rejected.
- [ ] Tests: a `v4` payload is rejected with the version message.

Acceptance: a `CSTRUCT`-bearing project encodes and decodes byte-identically; each
§4.3 corruption is rejected by `ir::verify` with its rule; a `v4` `.mfp` is
rejected; the regenerated `bindings/sqlite3` still passes
`tests/rt-behavior/native/native-link-sqlite-rt` **at runtime**;
`scripts/test-accept.sh` is green.

Commit: —

## Validation Plan

- Tests: round-trip byte-identity (`src/ir/coverage_tests.rs`); the four
  corruption rejections and the `v4` rejection (`src/ir/verify/tests.rs`,
  `src/ir/binary.rs` unit tests). Per `.ai/compiler.md`, the invalid side is the
  substance of this phase.
- Runtime proof: `tests/rt-behavior/native/native-link-import-sqlite-rt` and
  `native-link-sqlite-rt` must **execute** correctly against the regenerated
  `.mfp` — this is the phase's real proof, since the version bump could silently
  break the one binding that exercises the whole path. `CSTRUCT` itself still has
  no runtime behavior to demonstrate (no codegen until plan-50-E), and this
  sub-plan must not claim otherwise.
- Doc sync: `src/docs/spec/package/02_binary-representation.md`,
  `src/docs/spec/language/17_native-libraries.md`. Then `cargo build`,
  `cargo test --bin mfb spec`, no leaked `[[` markers.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
  Expect churn **only** in `.mfp`-byte goldens from the version bump; prove each
  is the bump and not a regression by diffing a regenerated payload against a
  pre-change checkout (`.ai/compiler.md`: do not assume a mismatch is a test
  issue).

## Open Decisions

- **`AbiDirection` enum vs. appending `is_inout: bool`.** Recommend the enum: the
  format breaks either way, two bools admit an illegal `(true, true)` state, and
  the migration is ~10 call sites (§4.1). Alternative (append a bool) is a smaller
  diff but leaves a representable-illegal state in the IR forever.
- **Should `check_link_cstructs` also close the `IrFree` asymmetry** (`:2756-2768`)
  while it is in this code? Recommend **no — file it separately** via
  `/write-bug`. It is a real package-path weakness and it is adjacent, but it is
  not this feature, and AGENTS.md forbids bundling unrelated changes into a
  commit. Note it explicitly so the next reader sees it was seen, not missed.
- **Count caps: 256 structs / 64 fields.** Recommend as stated; both are ~10× any
  plausible binding. Alternative: derive from `MAX_CSTRUCT_SIZE` — rejected as
  needlessly clever.

## Summary

The risk is the version bump: `BINARY_REPR_VERSION` is an exact-match gate, so
forgetting it turns every `v4` package into garbage silently fed to a
layout computer. It is task #1, and the regenerated `sqlite3` binding running at
runtime is the proof.

The design's security rests on a negative: offsets are never on the wire, so a
crafted `.mfp` has no offset to forge. That, plus re-validating every source-side
rule on the package path, deliberately avoids the weaker-on-the-package-path gap
`IrFree` has today.

Untouched: codegen, the container framing, sections 10/11, and the existing decode
hardening.
