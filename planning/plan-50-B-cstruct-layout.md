# plan-50-B: `CSTRUCT` declaration and the C layout computer

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-50-A (the slot-ctype allow-list — `CStruct` joins that namespace)

Introduces the declaration a binding uses to describe a C struct, and the layout
computer that turns it into exact size/alignment/field offsets. Declaration and
layout only: **no ABI slot may use it yet, no codegen, no callers.** That is what
makes this landable alone — it is a pure, unit-testable primitive whose output is
checkable against a C compiler.

The single behavioral outcome: `CSTRUCT SfInfo { CInt64 frames; CInt32
samplerate; … }` declared in a `LINK` block computes size `32`, align `8`, and
field offsets `0/8/12/16/20/24` — byte-for-byte what `gcc` reports for the real
`SF_INFO` on the target — and a struct the compiler cannot lay out correctly is
rejected rather than guessed.

References (read first):

- `src/ast/items.rs:parse_link_block` (`:598`) — the block body loop; today it
  accepts only `FUNC` and rejects anything else with
  `MFB_PARSE_UNEXPECTED_STATEMENT` (`:772-776`).
- `src/ast/types.rs:LinkBlock` (`:271`), `AbiSlot` (`:339`) — the AST shapes.
- `src/ir/link.rs` — `IrLinkFunction` (`:16`), `IrAbiSlot` (`:60`); the whole file
  is 84 lines and owns the LINK IR.
- `src/ir/lower.rs:link_functions` (`:292`) — AST→IR for the LINK block.
- `src/target/shared/code/type_utils.rs:align` (`:221`) — `value.div_ceil(a) * a`,
  the reusable primitive.
- `src/target/shared/code/builder_collection_layout.rs:4-7` — proof that MFBASIC
  records are **not** C structs: `Some(8 * fields.len())`, one word per field, no
  per-field size, alignment, or padding. This is why a C struct needs its own
  layout model and cannot reuse `TypeModel.record_fields`.
- Hardware ground truth, probed with `gcc` against real `sndfile.h` on the aarch64
  box (port 2223) during planning:
  ```
  SF_INFO        size=32 align=8 frames=0 samplerate=8 channels=12 format=16 sections=20 seekable=24
  SF_FORMAT_INFO size=24 align=8 format=0 name=8 extension=16
  ```
- `.ai/compiler.md`, `.ai/specifications.md`.

## 1. Goal

- A `LINK` block may declare `CSTRUCT <CName> AS <MfbType> … END CSTRUCT` with
  `<field> <ctype>` members. `<CName>` is local to that LINK alias; `<MfbType>` is
  the ordinary MFBASIC record type it presents as.
- **A `CSTRUCT` name never escapes its `LINK` block** (`NATIVE_CSTRUCT_ESCAPE`,
  §4.5): it may appear only in its own declaration, an `ABI (...)` slot's ctype
  position, and `SIZEOF`. A wrapper's MFBASIC-facing signature names `<MfbType>`,
  never `<CName>` — exactly as `CPtr` is confined by `NATIVE_CPTR_ESCAPE`.
- The compiler computes, for each declared struct: total size, alignment, and
  every field's byte offset, using standard C struct layout rules.
- The computed layout for the two driving structs **equals `gcc`'s** on every
  supported target (the numbers above are the test fixture).
- A struct the compiler cannot lay out faithfully is **rejected**, not
  approximated: unknown field ctype, `CVoid` field, zero fields, duplicate field
  name, nested struct, or a total size past the cap (§4.4).
- `CStruct` is *declarable* but not yet *usable* in an `ABI (...)` slot — a slot
  naming a struct is still rejected by plan-50-A's `NATIVE_ABI_UNKNOWN_CTYPE`
  until plan-50-E. This phase adds no marshaling.

### Non-goals (explicit constraints)

- **No MFBASIC `TYPE` is touched.** A `CSTRUCT` is a native-side layout
  descriptor and shares nothing with the record model
  (`builder_collection_layout.rs:4-7`). The spec's rule — *"Native ABI types are
  separate from MFBASIC source types"* — is the guardrail; do not map a `TYPE`
  onto a C struct or give records a C layout.
- No `.mfp` format change (that is plan-50-C). A `CSTRUCT` that reaches encoding
  in this phase would be dropped; therefore this phase **must** reject a struct
  used by any ABI slot, which plan-50-A already does.
- No nested structs, no fixed-size arrays, no unions, no bitfields. `SF_INFO` and
  `SF_FORMAT_INFO` need none of them. Reject explicitly; do not silently flatten.
- No packing/`#pragma pack` control. Natural alignment only.
- `CPtr` must not escape (`NATIVE_CPTR_ESCAPE` stays intact); a struct is not a
  way to hand a raw pointer to source code.

## 2. Current State

There is **no C-struct layout computer anywhere in the compiler.** Confirmed by
survey:

- MFBASIC records: one word per field, unconditionally
  (`builder_collection_layout.rs:4-7`); field *i* lives at `8*i`. No size, no
  alignment, no padding — structurally incapable of expressing `SF_FORMAT_INFO`
  (whose `name` sits at offset 8 *after* 4 bytes of padding, not at `8*1` by
  coincidence — the coincidence is real for that struct but breaks for `SF_INFO`,
  where `channels` is at 12, not 16).
- `TypeModel.record_fields: HashMap<String, Vec<(String, String)>>`
  (`src/target/shared/code/mod.rs:~360`) carries name/type pairs only — no offsets.
- Every C struct offset the compiler *does* know is **hand-coded per target**, not
  computed: `termios_lflag_offset`/`termios_cc_offset`
  (`src/target/macos_aarch64/code.rs:55,63`), `stat_mode_offset` (`:347`),
  `dirent_name_offset` (`:670`), `addrinfo_addr_offset` (`:731`). This plan
  deliberately does **not** retrofit those — they are platform structs with
  layouts that vary per OS, and a binding-declared `CSTRUCT` is a different
  contract. They are cited only as evidence that "hardcode the offsets" is the
  status quo this phase replaces *for binding-declared structs*.
- The LINK ABI is scalar-only: every slot is one register-width value via
  `abi::load_u64`/`store_u64` (`src/target/shared/abi.rs:780-840`). Nothing in
  `link_thunk.rs` knows about aggregates.

Reusable: `type_utils.rs:align:221`.

Parser shape to mirror: `parse_free_block` (`src/ast/items.rs:815`) is the
existing example of a nested, `END`-terminated sub-block inside a LINK function;
`CSTRUCT` is the same pattern one level up, inside the LINK *block*.

## 3. Design Overview

Three pieces, layered:

```
  CSTRUCT source
        │  src/ast/items.rs:parse_cstruct         (new, mirrors parse_free_block)
        ▼
  ast::types::CStructDecl { name, fields: Vec<CStructField>, line }
        │  src/ir/lower.rs:link_cstructs          (new, beside link_functions:292)
        ▼
  ir::link::IrCStruct { name, fields: Vec<IrCStructField> }
        │  src/ir/link.rs:compute_c_layout        (new — THE primitive)
        ▼
  CLayout { size, align, offsets: Vec<usize> }   <-- unit-tested against gcc
```

`compute_c_layout` is a pure function over `(field ctypes, target)`. Purity is the
point: it is the only place C layout knowledge lives, it has no I/O, and its
correctness is checkable by comparing against a real C compiler.

**Where the risk concentrates:** entirely in `compute_c_layout`. A wrong offset is
a silent memory-corruption bug at runtime — libsndfile would `memcpy` 24 bytes
over a buffer we believe is laid out differently. It is mitigated by (a) the
function being pure and exhaustively unit-testable, (b) the gcc-probed fixtures
above as ground truth, and (c) plan-50-G's hardware validation on all three
architectures.

**Layout is computed, never declared.** A binding author writes field ctypes in C
declaration order and *nothing else* — no offsets, no size, no padding. Offsets
are derived. This is the single most important security property of the design and
it is what plan-50-C's package-path gate relies on: there is no attacker-supplied
offset to trust, because offsets are never transported (see plan-50-C §3).

Rejected alternative: **map an MFBASIC `TYPE` directly onto a C struct** (infer
the C layout from a declared record, with no `CSTRUCT` at all). Rejected on three
counts: it conflates two unrelated layouts (records are `8*i`, arena-allocated,
with String headers); MFBASIC field types (`String`, `List OF Byte`) have no
single C representation; and it would make every record's layout an ABI contract,
freezing an internal detail. The spec's "native ABI types are separate from
MFBASIC source types" rule already forbids it. `CSTRUCT … AS <MfbType>` gets the
convenience of a declared correspondence **without** fusing the two layouts: the C
side is described explicitly, and `AS` is a mapping, not an equivalence.

Rejected alternative: **let the `CSTRUCT` and the record share a name and resolve
by position** (ctype position → `CSTRUCT` table; type position → package types).
It works — the positions never overlap — but it is resolve-by-context magic where
`AS <MfbType>` is one word of explicitness, and it gives the reader two things
called `SfFormatInfo` that are not the same thing. `AS` also makes
`NATIVE_CSTRUCT_ESCAPE` (§4.5) natural: with the mapping declared once, no other
site ever needs to name the C struct.

Rejected alternative: **let the author declare offsets/size explicitly**
(`frames CInt64 AT 0`). Rejected: it moves layout authority to hand-written
source, it is exactly the field a crafted `.mfp` would forge, and it makes the
binding wrong-by-default on any target with different rules. Computing is both
safer and less work for the author.

Rejected alternative: **derive layout by parsing the C header.** Rejected: no C
parser in-tree, and it would make the compiler depend on host headers at build
time.

## 4. Detailed Design

### 4.1 Surface

```
TYPE AudioFormat            ' the public face — an ordinary MFBASIC record
  format    AS Integer
  name      AS String
  extension AS String
END TYPE

LINK "libsnd" AS sndLink
  CSTRUCT SfFormatInfo AS AudioFormat     ' 24 bytes: format@0, name@8, extension@16
    format     CInt32
    name       CString
    extension  CString
  END CSTRUCT
END LINK
```

Fields are `<name> <ctype>`, one per line, in **C declaration order** — order is
load-bearing (it drives offsets), unlike a `TYPE`'s field order.

`AS <MfbType>` names the MFBASIC record this struct presents as. It is **required**,
not optional: a `CSTRUCT` with no public face could never cross the boundary, and
making the mapping explicit is what lets `<CName>` stay private (§4.5). The two
names are free to differ (`SfFormatInfo` ↔ `AudioFormat`) or coincide.

`CSTRUCT` names live in the LINK alias's namespace, alongside its `FUNC` names but
in a separate table (a struct and a function may share a name without conflict;
they are never resolved from the same position). A duplicate `CSTRUCT` name within
one LINK block is rejected.

The field **correspondence is by name, and coverage must be total** — every
`CSTRUCT` field must appear in `<MfbType>` with a compatible type, and vice versa
(plan-50-E §3 owns the type-compatibility table and its rule). This phase records
the mapping; E enforces and marshals it.

Parsed by `parse_cstruct`, dispatched from `parse_link_block`'s body loop
(`src/ast/items.rs:598`) beside the existing `FUNC` arm, so an unknown clause
still falls through to `MFB_PARSE_UNEXPECTED_STATEMENT` (`:772-776`).

### 4.2 The layout algorithm

Standard C natural alignment:

```
offset = 0; struct_align = 1
for field in fields (declaration order):
    (fsize, falign) = ctype_size_align(field.ctype)
    offset          = align(offset, falign)      // type_utils.rs:align:221
    field.offset    = offset
    offset         += fsize
    struct_align    = max(struct_align, falign)
size = align(offset, struct_align)
```

`ctype_size_align`, for every ctype in plan-50-A's allow-list:

| ctype | size | align |
|---|---|---|
| `CInt8`, `CUInt8`, `CByte`, `CBool` | 1 | 1 |
| `CInt16`, `CUInt16` | 2 | 2 |
| `CInt32`, `CUInt32`, `CFloat` | 4 | 4 |
| `CInt64`, `CUInt64`, `CDouble`, `CPtr`, `CString` | 8 | 8 |
| `CVoid` | — | rejected as a field |

`CBool` is C `_Bool` (1 byte); `CByte` is `unsigned char` (1 byte); `CString` is a
`const char*` field (8 bytes) — its *pointer*, not its bytes. Reading that pointer
into an owned `String` is plan-50-F.

### 4.3 Target dependence

All four supported targets are LP64 with identical rules for these ctypes:
x86-64, aarch64, riscv64 (all LP64), and Windows x64 (LLP64 — `long` is 4 bytes
there, but the ABI vocabulary has no `CLong`; every type is fixed-width, so the
table is identical). `ctype_size_align` therefore has **no target parameter
today**.

It must still be written to take one (or be trivially extensible), because the
table is an ABI contract and a future 32-bit or ILP32 target (e.g. arm64_32,
riscv32) would need 4-byte pointers. Recommend: thread the target through the
signature now and assert LP64, rather than bake the assumption in silently.

Verification against gcc, worked by hand and matching the probe:

- **SfInfo**: `frames`@0 (+8) → `samplerate` align 4 → @8 (+4) → `channels`@12
  (+4) → `format`@16 (+4) → `sections`@20 (+4) → `seekable`@24 (+4) → offset 28,
  struct_align 8 → **size 32**. gcc: `size=32 align=8 … seekable=24`. ✓
- **SfFormatInfo**: `format`@0 (+4) → `name` align 8 → **pad 4** → @8 (+8) →
  `extension`@16 (+8) → offset 24, align 8 → **size 24**. gcc: `size=24 align=8
  format=0 name=8 extension=16`. ✓

The `SfFormatInfo` padding is the case that proves the computer is necessary: a
naive `8*i` record layout lands `name` at 8 by luck, but `SfInfo` puts `channels`
at 12 where `8*i` says 16.

### 4.4 Validation

Rejected at both syntaxcheck and `ir::verify` (the package path is the security
gate — see plan-50-A §3 and `src/ir/verify/mod.rs:2602-2609`):

| Condition | Rule |
|---|---|
| field ctype not in the plan-50-A allow-list | `NATIVE_ABI_UNKNOWN_CTYPE` (reuse) |
| field ctype is `CVoid` | `NATIVE_CSTRUCT_INVALID` |
| zero fields | `NATIVE_CSTRUCT_INVALID` |
| duplicate field name in one struct | `NATIVE_CSTRUCT_INVALID` |
| duplicate `CSTRUCT` name in one LINK block | `NATIVE_CSTRUCT_INVALID` |
| field ctype names a `CSTRUCT` (nesting) | `NATIVE_CSTRUCT_INVALID` |
| computed size > `MAX_CSTRUCT_SIZE` (**1024** bytes) | `NATIVE_CSTRUCT_TOO_LARGE` |

The size cap matters because plan-50-E places the struct buffer in the thunk's
stack frame; an unbounded size from a crafted `.mfp` is a frame-overflow
primitive. 1024 is far above any real binding struct (`SF_INFO` is 32) and far
below anything that threatens the frame. Enforce it **here**, at layout, so it is
impossible to construct an oversized layout in the first place.

### 4.5 `NATIVE_CSTRUCT_ESCAPE` — the C name stays inside the LINK

A `CSTRUCT` name is a **native-side layout descriptor, not a type**. It may appear
only in:

1. its own `CSTRUCT <CName> AS <MfbType>` declaration,
2. an `ABI (...)` slot's ctype position (`info INOUT SfFormatInfo`),
3. `SIZEOF <CName>`.

Anywhere else — a wrapper parameter type, a wrapper return type, a `TYPE` field, a
source-level binding — is `NATIVE_CSTRUCT_ESCAPE`. This mirrors
`NATIVE_CPTR_ESCAPE` (`src/ir/verify/mod.rs:2633-2652`) and carries the same
argument: the C representation is an ABI-boundary detail, and letting it leak into
ordinary MFBASIC would make a private layout part of the public API.

The rule is what earns `AS <MfbType>`: because the mapping is declared once at the
struct, no other site ever needs to name `<CName>`.

New rules in `src/rules/table.rs` (`2-203`, next free after plan-50-A's `0123`):

| Code | Name | Severity |
|---|---|---|
| `2-203-0124` | `NATIVE_CSTRUCT_INVALID` | Error |
| `2-203-0125` | `NATIVE_CSTRUCT_TOO_LARGE` | Error |
| `2-203-0126` | `NATIVE_CSTRUCT_ESCAPE` | Error |

No runtime error code, so `src/docs/spec/diagnostics/02_error-codes.md` (the
`build.rs:178` build input) is untouched. **Both rules need a row in
`src/docs/spec/diagnostics/01_rule-codes.md`** in the same change: the
`every_rule_is_documented_in_the_spec` guard (`src/rules/mod.rs`, added
`afdcceb6`) asserts every `RULES` entry's code and name are documented there.

## Compatibility / Format Impact

- **Changes:** `CSTRUCT` becomes a reserved clause inside a `LINK` block. A
  binding that previously used `CSTRUCT` as… nothing — the block body accepted
  only `FUNC`, so there is no source that could break.
- **Unchanged:** `.mfp` byte format (`BINARY_REPR_VERSION` stays `4`; plan-50-C
  bumps it); all existing marshaling; the MFBASIC record model.
- A `CSTRUCT` declared in this phase is inert: unusable in an ABI slot, and
  dropped at `.mfp` encoding. That is intentional and is why this lands safely
  alone — but it means **a binding must not ship a `CSTRUCT` until plan-50-C**.

## Phases

One landable unit.

### Phase 1 — declaration, layout, validation

- [ ] `src/ast/types.rs`: add `CStructDecl { name, maps_to, fields:
      Vec<CStructField>, line }` and `CStructField { name, ctype, line }`; add
      `cstructs: Vec<CStructDecl>` to `LinkBlock` (`:271`).
- [ ] `src/ast/items.rs`: add `parse_cstruct` incl. the required `AS <MfbType>`
      (mirror `parse_free_block:815`); dispatch it from `parse_link_block`'s body
      loop (`:598`).
- [ ] `NATIVE_CSTRUCT_ESCAPE` (§4.5) in both `src/syntaxcheck/mod.rs` and
      `src/ir/verify/mod.rs`, beside the existing `NATIVE_CPTR_ESCAPE` scans
      (`src/ir/verify/mod.rs:2633-2652` / `src/syntaxcheck/mod.rs:414-441`) — a
      `CSTRUCT` name in any wrapper param/return position is rejected.
- [ ] `src/ast/serialize.rs`: extend the LINK serialization (`:297-462`) to carry
      `cstructs` — the AST golden path.
- [ ] `src/ir/link.rs`: add `IrCStruct`/`IrCStructField`; add
      `ctype_size_align(ctype, target)` and `compute_c_layout(fields, target) ->
      Result<CLayout, ...>` with `CLayout { size, align, offsets }`; add
      `MAX_CSTRUCT_SIZE = 1024`.
- [ ] `src/ir/mod.rs`: add `link_cstructs: Vec<IrCStruct>` to `IrProject`
      (`:28`, beside `link_functions`); re-export the new types (`:153`).
- [ ] `src/ir/lower.rs`: add `link_cstructs` beside `link_functions:292`.
- [ ] `src/rules/table.rs`: add `NATIVE_CSTRUCT_INVALID` (`2-203-0124`),
      `NATIVE_CSTRUCT_TOO_LARGE` (`2-203-0125`), and `NATIVE_CSTRUCT_ESCAPE`
      (`2-203-0126`), **plus a row for each in
      `src/docs/spec/diagnostics/01_rule-codes.md`** (the
      `every_rule_is_documented_in_the_spec` guard, `src/rules/mod.rs`).
- [ ] Validation per §4.4 in **both** `src/syntaxcheck/mod.rs` (slot-level spans)
      and `src/ir/verify/mod.rs` (function-level, beside
      `check_link_functions:2610`).
- [ ] Spec: new `CSTRUCT` subsection in
      `src/docs/spec/language/17_native-libraries.md` — the surface, the layout
      algorithm, the size/align table, the natural-alignment and no-nesting
      rules, the 1024-byte cap, and the two new rules. Cite
      `[[src/ir/link.rs:compute_c_layout]]`.
- [ ] Tests: `src/ir/link.rs` unit tests asserting **exactly** the gcc-probed
      numbers — `SfInfo` → size 32, align 8, offsets `[0,8,12,16,20,24]`;
      `SfFormatInfo` → size 24, align 8, offsets `[0,8,16]`. Plus edge cases: a
      single `CInt8` field (size 1, align 1); `CInt8` then `CInt64` (offsets
      `[0,8]`, size 16 — trailing-pad case); all-1-byte fields (no padding).
- [ ] Tests: `tests/syntax/native/native-cstruct-invalid/` covering each §4.4
      rejection (mirror `tests/syntax/native/native-abi-unbound-slot-invalid/`).
- [ ] Tests: `src/ir/verify/tests.rs` — package-path rejection of an oversized
      and a zero-field struct.

Acceptance: the unit tests reproduce gcc's `SF_INFO` (32/8, `[0,8,12,16,20,24]`)
and `SF_FORMAT_INFO` (24/8, `[0,8,16]`) exactly; each §4.4 condition is rejected
with its rule on both the source and package paths; `scripts/test-accept.sh` is
green with zero golden churn for every existing binding.
Commit: —

## Validation Plan

- Tests: as listed above — the gcc-parity unit tests are the core; the invalid
  suite covers every rejection. Per `.ai/compiler.md`, both valid and invalid
  sides are required.
- Runtime proof: **none is possible in this phase and that is expected** — a
  `CSTRUCT` has no callers until plan-50-E. Per `.ai/compiler.md`'s Hard
  Completion Gate, this sub-plan therefore cannot be called "struct support
  works"; its verifiable claim is narrower and fully checkable: *the computed
  layout equals the C compiler's*. Ground truth is regenerable with the probe
  used during planning (`gcc -I include`, `offsetof`/`sizeof`/`_Alignof` on the
  2223 box). Re-run it on 2227 (x86_64) and 2229 (riscv64) when finalizing to
  confirm the table is genuinely target-invariant.
- Doc sync: `src/docs/spec/language/17_native-libraries.md`; `cargo build`,
  `cargo test --bin mfb spec`, no leaked `[[` markers.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  (zero churn) plus `scripts/artifact-gate.sh` (codegen unaffected).

## Open Decisions

- **Does `ctype_size_align` take a target parameter now?** Recommend **yes** —
  thread it and assert LP64 — even though all four targets agree today. The
  alternative (add it later) means auditing every call site under time pressure
  when an ILP32 target lands, and the table is an ABI contract. (§4.3)
- **`MAX_CSTRUCT_SIZE = 1024`.** Recommend 1024: ~32× the largest real struct
  here, ~1/8 of a conservative frame budget. Alternative: 256 (tighter, still
  8× headroom). Revisit if plan-50-E's frame math wants a smaller number — E is
  the consumer and may lower it, but must not raise it. (§4.4)
- **Where do `CSTRUCT` names resolve?** Recommend a per-alias table separate from
  the FUNC table, mirroring how `link_aliases` (`src/ir/lower.rs:link_aliases:360`)
  already keys a second namespace off the alias. Alternative: one shared table —
  rejected, as it would make a struct and a function collide for no reason.

## Summary

All the risk is in `compute_c_layout`: a wrong offset is silent memory corruption
at runtime, not a compile error. It is contained by making the function pure,
pinning its expected output to numbers probed from a real C compiler on real
target hardware, and refusing to lay out anything the rules do not cover exactly
(nesting, arrays, unions, bitfields, oversized).

Untouched: the MFBASIC record model, the `.mfp` format, every existing binding,
and all current marshaling. Nothing calls this code yet — by design.
