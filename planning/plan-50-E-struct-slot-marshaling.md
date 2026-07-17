# plan-50-E: struct slot marshaling — scalar fields, `INOUT`, `BIND IN`

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-50-A (closed ctype namespace), plan-50-B (`compute_c_layout`,
`CSTRUCT … AS <MfbType>`), plan-50-C (wire format + `AbiDirection` +
`result_slot`), plan-50-D (`store_u16`), plan-50-H (`RETURN <name>`)

The phase where a C struct actually crosses the boundary. Teaches the thunk to
stage a struct buffer in its frame, write bound input fields **in** before the
call, and read the C struct's fields back **out** into a fresh record after — for
scalar fields only. Adds `INOUT` and the `BIND IN` clause; the result marker is
plan-50-H's `RETURN <name>`, unchanged.

`CString` struct fields (`const char *name`) are plan-50-F; this phase rejects
them. That split is what keeps this sub-plan medium: the scalar path is pure
sized load/store against a stack buffer, while the pointer path drags in arena
allocation, UTF-8 validation, and the copy-and-leave ownership rule.

The single behavioral outcome: an MFBASIC program calls
`sf_command(handle, SFC_GET_CURRENT_SF_INFO, &info, 32)` through a `LINK` binding
and reads back the **real** `samplerate`, `channels`, and `frames` of an open
audio file as an ordinary MFBASIC record — proving a 32-byte C struct round-trips
correctly on real hardware.

References (read first):

- `src/target/shared/code/link_thunk.rs:lower_link_thunk` (`:333`) — the frame:
  `STATUS_OFF=8` (`:339`), `CRET_OFF=16` (`:340`), string-return scratch at 24
  (`:341`), `param_base=32` (`:342`), `cslot_base=param_base+n_params*8` (`:343`),
  `out_base=cslot_base+m_slots*8` (`:344`), `cretd_off=out_base+n_out*8` (`:347`),
  `frame=align(cretd_off+8+24,16)` (`:348`).
- `src/target/shared/code/link_thunk.rs:404-471` — the per-slot staging loop. The
  `OUT` arm (`:406-417`) is the shape this phase generalizes:
  ```rust
  abi::store_u64(abi::ZERO, abi::stack_pointer(), out_off),   // zero the buffer
  abi::add_immediate("%v9", abi::stack_pointer(), out_off),   // take its address
  abi::store_u64("%v9", abi::stack_pointer(), cslot_off),     // pass the address
  ```
- `src/target/shared/code/link_thunk.rs:448-458` — the `CInt32` range check
  (`ErrOverflow`, `range_fail`), and `:453-454`, the shift-pair sign-extension
  idiom this phase reuses for narrow signed fields.
- `src/target/shared/code/link_thunk.rs:475-493` — C args loaded into AAPCS64
  registers by `int_idx`/`flt_idx`. Only **8** integer argument registers exist
  (`abi::argument_register`).
- `src/target/shared/code/builder_collection_layout.rs:4-7` — MFBASIC record
  layout: `Some(8 * fields.len())`, field *i* at `8*i`. This is how the mapped
  record is read/written; it is **not** the C layout.
- `src/target/shared/code/mod.rs:~360` — `TypeModel.record_fields:
  HashMap<String, Vec<(String, String)>>`, the name→index source for the record side.
- `.ai/compiler.md` §"Native Codegen Register Lifetimes" and the memory
  `arena-alloc-clobbers-x14-x15`: **`_mfb_arena_alloc` destroys all of `x0`–`x17`.
  There is no survivor set.** Spill to stack slots across it.
- `src/docs/spec/language/17_native-libraries.md:129-146` — the deferred
  `RETURN_OUT` design and its "Implementation status" note, both rewritten here.
- Ground truth probed during planning (gcc, real `sndfile.h`, aarch64 box 2223):
  `SF_INFO size=32 align=8 frames=0 samplerate=8 channels=12 format=16 sections=20 seekable=24`
- `libsndfile-1.2.2/src/sndfile.c:1066-1070` — `SFC_GET_CURRENT_SF_INFO = 0x1002`
  (`sndfile.h:145`), the all-scalar single-output call this phase's proof uses:
  ```c
  case SFC_GET_CURRENT_SF_INFO :
      if (data == NULL || datasize != SIGNED_SIZEOF (SF_INFO))
          return (sf_errno = SFE_BAD_COMMAND_PARAM) ;
      memcpy (data, &psf->sf, sizeof (SF_INFO)) ;
  ```

## 1. Goal

- An `ABI (...)` slot may name a declared `CSTRUCT` as its ctype, with direction
  `IN` (default), `OUT`, or `INOUT`.
- `BIND IN <slot> … END BIND` writes named struct fields from wrapper parameters
  (or literals) before the call; every unbound field is zero.
- `RETURN <slot>` (plan-50-H) on a struct slot makes its **post-call** contents the
  wrapper's result: a freshly allocated record of the `CSTRUCT`'s `AS <MfbType>`,
  populated from the C struct's fields.
- Scalar fields marshal correctly in both directions with exact widths and correct
  signedness: `CInt8/16/32/64`, `CUInt8/16/32/64`, `CBool`, `CByte`, `CDouble`,
  `CFloat`.
- The struct buffer is **fully zeroed** before every call — no uninitialized stack
  bytes are ever handed to a C library.
- A `CString` or `CPtr` struct field in a mapped slot is **rejected** (plan-50-F
  lifts the `CString` restriction).

### Non-goals (explicit constraints)

- **No `CString` field marshaling.** Rejected here with a clear diagnostic;
  plan-50-F implements it. This is a scope boundary, not an "unsupported"
  stand-in for requested behavior — the requested behavior (`getFormats`) lands in
  F/G, and this sub-plan does not claim it.
- **No multi-output result.** `RETURN` names exactly **one** slot. The spec's
  deferred `RETURN_OUT DivModResult[quotient, remainder]` multi-slot form stays
  deferred (§Open Decisions). One slot is all `SFC_GET_CURRENT_SF_INFO` and
  `SFC_GET_SIMPLE_FORMAT` need.
- **No `CPtr` escape.** A `CPtr` struct field must never surface in a record;
  `NATIVE_CPTR_ESCAPE` still holds.
- No change to scalar (non-struct) slot marshaling. Existing bindings must emit
  byte-identical thunks.
- No change to the `.mfp` format — plan-50-C already carries everything.

## 2. Current State

Every ABI slot is exactly one 8-byte value. `cslot_base + slot_idx*8` gives each
slot one word (`:405`); `out_base + out_seq*8` gives each `OUT` slot one zeroed
word (`:407`). Args are loaded from those words straight into argument registers
(`:475-493`). Nothing in the 900-line thunk knows about aggregates.

`OUT` is the only mechanism that passes an *address* to C: it zeroes a stack word,
`add_immediate`s its address, and stores that address in the cslot (`:409-413`).
A struct slot is the same three steps over a *sized, aligned* buffer instead of a
word — which is why this phase is a generalization rather than a new mechanism.

The result marker is plan-50-H's `RETURN <name>` + `result_slot`; this phase adds
no result mechanism of its own — a struct slot is simply another thing `RETURN` can
name. `BIND IN` is new and joins the clause dispatch beside H's `RETURN`.

The spec has documented `RETURN_OUT` as the intended multi-output design, and its
own Implementation-status note (`17_native-libraries.md:146`) says plainly it does
not exist. **`RETURN_OUT` is not implemented and should not be** — plan-50-H's
`RETURN <name>` subsumes its single-slot case entirely, and the multi-slot case
stays deferred. Rewrite those passages to say so rather than leaving them
describing a form that will never ship in that spelling.

## 3. Design Overview

```
  wrapper param (record T)            C struct buffer (frame)          result record (T')
        │                                     │                                ▲
        │  marshal_in: per field              │                                │
        │    load record[8*i]                 │                                │
        │    range-check / convert            │                                │
        │    sized store -> buf+offset ──────►│                                │
        │                                     │  call ──►  C library           │
        │                                     │  (writes fields)               │
        │                                     │                                │
        │                       marshal_out: per field                         │
        │                         sized load  buf+offset                       │
        │                         sign-extend if signed  ──────────────────────┘
```

Two independent halves (`marshal_struct_in`, `marshal_struct_out`) over one shared
buffer-staging step. Direction selects which halves run: `IN` → in only; `OUT` →
out only; `INOUT` → both.

### The mapping is declared once, at the `CSTRUCT`

plan-50-B's `CSTRUCT <CName> AS <MfbType>` carries the correspondence, and
`NATIVE_CSTRUCT_ESCAPE` (B §4.5) keeps `<CName>` inside the `LINK` block. So this
phase needs **no** name resolution scheme of its own: a struct slot's ctype names
the `CSTRUCT`, and the record type to build is whatever that `CSTRUCT` maps to.

```
FUNC getFormat(index AS Integer) AS AudioFormat         ' ← MfbType, the public face
  SYMBOL "sf_command"
  ABI (sndfile CPtr, command CInt32, info INOUT SfFormatInfo, datasize CInt32) AS status CInt32
                                          '       ↑ ctype position → the CSTRUCT
  BIND IN info
    format = index
  END BIND
  RETURN info
```

**Fields correspond by name, and coverage must be total** (enforced here; B §4.1
records the mapping). Every `CSTRUCT` field must appear in `<MfbType>` with a
compatible type, and vice versa. Partial coverage is rejected: a silently-unmapped
field is exactly the typo a binding author cannot see, and it would be zeroed on
the way in and dropped on the way out — a wrong-answer bug with no diagnostic.

### `BIND IN` — input fields without a dummy record

A struct slot's input fields come from `BIND IN`, not from a record-typed wrapper
parameter:

```
BIND IN info
  format = index        ' <struct field> = <wrapper param | literal>
END BIND
```

Every field not named by `BIND IN` is **zero** (§4.3 zeroes the whole buffer
first). The caller therefore writes `getFormat(3)` — not
`getFormat(SfFormatInfo[format := 3, name := "", extension := ""])`, a record whose
other two fields are junk that libsndfile immediately overwrites.

This also means an `IN`/`INOUT` struct slot needs **no** `CString` input
marshaling for fields the binding does not bind — which is why the `getFormats`
path never allocates a throwaway C string (cf. plan-50-F §4.3).

| C field ctype | MFBASIC record field | note |
|---|---|---|
| `CInt8/16/32/64`, `CUInt8/16/32`, `CByte` | `Integer` | sign/zero-extend per signedness |
| `CUInt64` | `Integer` | reinterpreted; values > `i64::MAX` wrap (document it) |
| `CBool` | `Boolean` | nonzero→TRUE |
| `CFloat`, `CDouble` | `Float` | finiteness enforced on the way out |
| `CString` | `String` | **plan-50-F** — rejected here |
| `CPtr` | — | always rejected (`NATIVE_CPTR_ESCAPE`) |

**Where the risk concentrates:** two places.

1. **Register lifetime.** `marshal_out` must allocate a record
   (`_mfb_arena_alloc`), and that call **destroys every caller-saved register,
   `x0`–`x17`, with no survivor set** (`.ai/compiler.md`; memory
   `arena-alloc-clobbers-x14-x15`). Any field value held in a scratch register
   across it is silently corrupted. The design's answer is structural: the struct
   buffer is `sp`-relative, so it survives the call by construction — allocate the
   record **first**, then read each field from the stack buffer one at a time,
   never holding a field value across an allocation. This is the exact bug class
   the memory `copy-record-register-aliasing` documents.
2. **Offsets.** A wrong offset writes into a neighbouring field. Contained by
   plan-50-B's gcc-parity tests and this phase's hardware proof.

Rejected alternative: **infer the C layout from the MFBASIC record** and skip
`CSTRUCT`. Rejected in plan-50-B §3 — records are `8*i` with no padding and cannot
express `SF_INFO` (`channels` at 12, not 16).

Rejected alternative: **bind input fields by passing a record-typed wrapper
parameter** (`getFormat(info AS AudioFormat) AS AudioFormat`, marshaling the whole
record in). This was the original design; `BIND IN` replaced it. The record form
forces the caller to construct a full record when only one field is an input, and
every junk field then costs a `CString` allocation on the way in for a value the C
library overwrites. `BIND IN` states exactly the inputs and zeroes the rest.

Rejected alternative: **make an `INOUT` slot implicitly the result marker.**
Rejected — and `sf_open` is the concrete counterexample, in this very library:
```
ABI (path CString, mode CInt32, info INOUT SfInfo) AS handle CPtr
RETURN handle
```
`info` is genuinely `INOUT` (libsndfile fills it), but the result is the handle. A
direction cannot carry "is the result"; a name must. This is the same reasoning
that produced plan-50-H, and it is why `RETURN <name>` — not the slot's direction
and not a magic slot name — selects the result.

## 4. Detailed Design

### 4.1 Surface

```
FUNC currentInfo(RES snd AS Sndfile) AS SfInfo
  SYMBOL "sf_command"
  ABI (snd CPtr, cmd CInt32, info OUT SfInfo, size CInt32) AS status CInt32
  CONST cmd = 4098               ' SFC_GET_CURRENT_SF_INFO (0x1002)
  CONST size = SIZEOF SfInfo     ' 32 — see §Open Decisions
  RETURN info
  SUCCESS_ON status = 0
END FUNC
```

`INOUT` joins `OUT` in `parse_abi_spec` (`src/ast/items.rs:926`), matched
case-insensitively beside the existing `match_identifier_ci("OUT")`:

```rust
let name = self.parse_abi_slot_name()?;
let direction = self.parse_abi_direction();   // OUT | INOUT | (absent -> In)
let ctype = self.parse_c_type_name()?;
```

`BIND IN <slot> … END BIND` is a new clause in `parse_link_function`'s dispatch
(`:705-779`), storing `bind_in: Vec<(String, Vec<(String, Expression)>)>` — per
slot, a list of `field = <param|literal>`. Mirror `parse_free_block:815` for the
nested `END`-terminated shape.

`RETURN <slot>` is plan-50-H's clause, used verbatim. This phase adds **no**
result-marker mechanism; `result_slot` already exists (plan-50-C §4.2b) and
already means "which name is the result". A struct slot is simply a name it can
hold.

### 4.2 Frame layout

```
  …existing… cretd_off = out_base + n_out*8
  struct_base = align(cretd_off + 8, max_struct_align)      // NEW
    slot k buffer at align(running, layout[k].align), size layout[k].size
  struct_end
  frame = align(struct_end + 24, 16)                        // the +24 tail scratch is preserved
```

`max_struct_align` is ≤ 8 for every ctype in the table (plan-50-B §4.2), so
`struct_base` is 8-aligned in practice; compute it rather than assume. Total struct
bytes are bounded by `MAX_CSTRUCT_SIZE` × slots — with the 1024-byte cap
(plan-50-B §4.4) and 8 integer argument registers capping usable slots, the frame
cannot be pushed past ~8 KB by a crafted `.mfp`. **Verify this bound explicitly
when implementing** and note it in the spec; it is the reason the cap exists.

### 4.3 Staging (all directions)

Mirrors `:409-413`, generalized:

```rust
// zero the whole buffer — never hand uninitialized stack bytes to C
for off in (0..layout.size).step_by(8) { store_u64(ZERO, sp, struct_off + off); }
// (tail < 8 bytes: use store_u32/u16/u8 to avoid writing past the buffer)
abi::add_immediate("%v9", abi::stack_pointer(), struct_off);
abi::store_u64("%v9", abi::stack_pointer(), cslot_off);   // the C arg is the address
```

Zeroing is **mandatory and security-relevant**, not hygiene: `sf_open` requires a
zeroed `SF_INFO` for non-RAW reads, and an unzeroed buffer leaks the thunk's stack
contents into a C library. The tail must not over-write — a 4-byte-tail struct
zeroed with `store_u64` would clobber the next buffer.

### 4.4 `marshal_struct_in`

Only fields named by `BIND IN` are written; the rest are already zero from §4.3.
For each `BIND IN <slot> { field = <param> }`:

```
load_u64 %v10, [sp + param_off]       ; the bound wrapper parameter (or an immediate, for a literal)
<convert per ctype>                   ; CInt32: reuse the :448-458 range check -> ErrOverflow
<sized store> %v10 -> [sp + struct_off + field.offset]   ; store_u8/u16/u32/u64
```

Note this reads a **wrapper parameter**, not a record field — there is no record
on the input side at all, so no record pointer to load and none of the
register-lifetime hazard that §4.5's output path has.

Width selection is `ctype_size_align` (plan-50-B §4.2) → `store_u8` / `store_u16`
(plan-50-D) / `store_u32` / `store_u64`. **Every narrow integer gets the `CInt32`
treatment**: a 64-bit MFBASIC `Integer` that does not fit the C field's width must
fail with `ErrOverflow`, not truncate — the same rule `:448` already applies to
`CInt32` args, applied per field and per width. Silently truncating a field is the
`bug-238` class of error (a `CInt32` OUT surfacing `-1` as `4294967295`) and must
not be reintroduced.

### 4.5 `marshal_struct_out`

Runs after the `SUCCESS_ON` gate (a failed call produces no record):

```
alloc the result record FIRST (arena_alloc; clobbers x0-x17)   <-- see §3 risk #1
spill the record pointer to a stack slot
for each field i -> record index j:
    <sized load> [sp + struct_off + field.offset] -> %v9     ; buffer is sp-relative: survives
    <sign-extend if signed>                                   ; shift pair, per :453-454
    <normalize CBool (nonzero->TRUE) / CDouble (reject NaN/Inf, per :800-819)>
    load_u64 %v10, [sp + record_slot]
    store_u64 %v9, [%v10 + 8*j]
```

`ldr_u16`/`ldr_u32`/`ldr_u8` zero-extend, so a signed narrow field **must** be
sign-extended explicitly — otherwise a `CInt32 sections = -1` surfaces as
`4294967295`, exactly `bug-238`. `CDouble` fields reuse the finiteness rejection
at `:800-819`: an MFBASIC `Float` is always finite.

The record allocation ordering is the load-bearing detail. Reading fields into
registers and *then* allocating would lose them all.

### 4.6 Result and `BIND IN` rules

The result-marker rule is plan-50-H's, unchanged: exactly one of `RETURN <name>` or
`RESULT <expr>`. This phase only adds the case that `RETURN` may name a struct
slot — and then the slot must be `OUT` or `INOUT` (a `RETURN` on an `IN` struct
slot would return the zeroed buffer, which is never intended).

New rules (`2-203`, next free after plan-50-B's `0126`):

| Code | Name | Severity |
|---|---|---|
| `2-203-0127` | `NATIVE_STRUCT_FIELD_MISMATCH` | Error — the `AS <MfbType>` record and the `CSTRUCT` differ by field name, type, or coverage |
| `2-203-0128` | `NATIVE_BIND_IN_INVALID` | Error — `BIND IN` names an unknown slot/field, a non-struct slot, an `OUT` slot, a duplicate field, or binds an unknown parameter |

`RETURN` on an `IN` struct slot reuses H's `NATIVE_ABI_RESULT_MARKER`.

Both are enforced on the **source and package paths** (plan-50-C §4.3's posture).
No runtime error code is added, so `src/docs/spec/diagnostics/02_error-codes.md`
(the `build.rs:178` build input) is untouched. `ErrOverflow` (`77050010`) and
`ErrFloatNaN`/`ErrFloatInf` are reused as-is.

**Both rules need a row in `src/docs/spec/diagnostics/01_rule-codes.md`** in the
same change — the `every_rule_is_documented_in_the_spec` guard (`src/rules/mod.rs`,
added `afdcceb6`) asserts every `RULES` entry's code and name are documented there.

## Compatibility / Format Impact

- **Changes:** `INOUT` and `BIND IN` become reserved clauses in a `LINK` block; a
  struct slot compiles. The spec's deferred `RETURN_OUT` design is retired in
  favour of plan-50-H's `RETURN <name>`.
- **Unchanged:** the `.mfp` format (plan-50-C did it); scalar-slot marshaling;
  every existing thunk's bytes; `MAX_CSTRUCT_SIZE`.

## Phases

One landable unit.

### Phase 1 — struct slots end-to-end for scalar fields

- [ ] `src/ast/items.rs`: parse `INOUT` in `parse_abi_spec` (`:926`); add the
      `BIND IN <slot> … END BIND` clause to `parse_link_function`'s dispatch
      (`:705-779`), mirroring `parse_free_block:815`'s nested `END`-terminated shape.
- [ ] `src/ast/types.rs` / `src/ir/link.rs`: add the `bind_in` table (§4.1);
      `src/ir/lower.rs:292` carries it; `src/ir/binary.rs` encodes it — **fold this
      field into plan-50-C's bump**, do not add a second one. `result_slot` already
      exists (C §4.2b + H); this phase only lets `RETURN` name a struct slot.
- [ ] Resolution + checks (§3, §4.6) in `src/syntaxcheck/mod.rs` **and**
      `src/ir/verify/mod.rs`: slot ctype → `CSTRUCT`; total field coverage against
      the `CSTRUCT`'s `AS <MfbType>`; per-field type compatibility; `CString`/`CPtr`
      field rejection; `BIND IN` validity; `RETURN` on an `IN` struct slot.
- [ ] `src/rules/table.rs`: `NATIVE_STRUCT_FIELD_MISMATCH` (`2-203-0127`),
      `NATIVE_BIND_IN_INVALID` (`2-203-0128`), **plus a row for each in
      `src/docs/spec/diagnostics/01_rule-codes.md`** (the
      `every_rule_is_documented_in_the_spec` guard, `src/rules/mod.rs`).
- [ ] `link_thunk.rs`: extend the frame (§4.2); add the struct arm to the staging
      loop (§4.3) with a non-overwriting tail zero; implement `marshal_struct_in`
      (§4.4) and `marshal_struct_out` (§4.5), **allocating the record before
      reading any field**.
- [ ] Tests: `tests/syntax/native/native-struct-slot-invalid/` — field-name
      mismatch, type mismatch, partial coverage, `CString` field, `CPtr` field,
      `BIND IN` on an unknown slot/field/param, `BIND IN` on an `OUT` slot,
      `RETURN` on an `IN` struct slot.
- [ ] Tests: `src/ir/verify/tests.rs` — the same rejections on the package path.
- [ ] Tests: `tests/rt-behavior/native/native-struct-scalar-rt/` — the runtime
      proof below.
- [ ] Spec: rewrite `17_native-libraries.md:129-146` — `RETURN_OUT` is **retired**,
      subsumed by plan-50-H's `RETURN <name>`; the multi-slot form stays deferred.
      Document `INOUT`, struct slots, `BIND IN`, total coverage against
      `AS <MfbType>`, the field type table, mandatory zeroing, and the per-field
      overflow rule. Cite
      `[[src/target/shared/code/link_thunk.rs:lower_link_thunk]]`.

Acceptance: the runtime proof below returns the true `samplerate`/`channels`/
`frames` of a real WAV on **aarch64, x86_64, and riscv64**; every §4.6 misuse is
rejected on both paths; `scripts/test-accept.sh` green with churn only where
struct tests were added; existing thunks byte-identical via
`scripts/artifact-gate.sh`.
Commit: —

## Validation Plan

- Tests: the invalid suite above (source + package paths) — per `.ai/compiler.md`,
  both valid and invalid sides are mandatory and non-skippable.
- **Runtime proof** (`.ai/compiler.md`'s Hard Completion Gate — compiler output is
  not proof): a binding over libsndfile that
  1. `sf_open(path, SFM_READ, info)` with `info` an **`IN`** struct slot (zeroed —
     libsndfile requires it for non-RAW reads), result `return CPtr` → `RES Sndfile`;
  2. `sf_command(snd, 0x1002 /* SFC_GET_CURRENT_SF_INFO */, info, 32)` with `info`
     an **`OUT`** struct slot + `RETURN info` → an `SfInfo` record;
  3. asserts `samplerate`/`channels`/`frames` equal a known-good WAV fixture's.

  This exercises both directions with only scalar fields, and needs **no
  multi-output**: `SFC_GET_CURRENT_SF_INFO` `memcpy`s the whole `SF_INFO` and is a
  single output (`sndfile.c:1066-1070`). Run on aarch64 (2223), x86_64 (2228), and
  riscv64 (2229) — the vendored libraries for all three landed earlier this
  session under `bindings/libsnd/vendor/`. A wrong offset or width fails loudly
  here (e.g. `channels` read at 16 instead of 12 returns `format`).
- Doc sync: `src/docs/spec/language/17_native-libraries.md`; `cargo build`,
  `cargo test --bin mfb spec`, no leaked `[[` markers.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` +
  `scripts/artifact-gate.sh`.

## Open Decisions

- **`CONST size = 32` is a hand-written `sizeof`.** libsndfile validates
  `datasize != SIGNED_SIZEOF(SF_INFO)` and fails the call, so a wrong constant is
  caught loudly at runtime rather than corrupting memory — but it is a magic
  number the compiler already knows. Recommend adding **`CONST <slot> = SIZEOF
  <CStructName>`**, folded by the compiler from `compute_c_layout`. It is a small
  extension to `eval_link_const` (`src/ir/lower.rs:387`) — but note that function's
  catch-all `_ => 0` (`:398`) would silently pin **0** for an unrecognized const
  expression, so `SIZEOF` must be handled explicitly *and* the catch-all should
  become an error. Alternative: hardcode `32` in plan-50-G and file `SIZEOF`
  separately. Recommend doing `SIZEOF` here — hardcoding a `sizeof` in every
  binding is exactly the fragility `compute_c_layout` exists to remove.
- **`CUInt64` → `Integer` wraps** for values above `i64::MAX`. Recommend
  documenting it (MFBASIC has no unsigned 64-bit type) rather than erroring;
  `SF_INFO` has no such field. Revisit if a binding needs it.
- **Multi-slot results** (the spec's deferred `RETURN_OUT DivModResult[quotient,
  remainder]`) stay deferred. `sf_open` genuinely wants it (handle **and** filled
  `SF_INFO`), and plan-50-G sidesteps that with `SFC_GET_CURRENT_SF_INFO`.
  Recommend retiring the `RETURN_OUT` spelling outright in the spec — `RETURN
  <name>` covers the single-slot case — and re-describing the multi-slot gap in
  terms of `RETURN`, rather than leaving a deferred design nobody will implement
  under that name.

## Summary

Two risks. **Register lifetime**: `_mfb_arena_alloc` destroys `x0`–`x17` with no
survivors, so `marshal_out` allocates the record before touching any field and
reads each field from the `sp`-relative buffer — the mistake here is invisible in
small tests and corrupts data in large ones. **Offsets**: a wrong one silently
writes a neighbouring field; plan-50-B's gcc-parity tests plus this phase's
three-architecture hardware proof are the defense.

Untouched: scalar slots, the `.mfp` format, and every existing binding.
`CString` fields — and therefore `getFormats` — remain out of reach until
plan-50-F.
