# bug-333: the string/conversion and collection builders carry ~1,400 lines of algorithmic duplication, and one duplicated pair has already drifted into a reproducible compile failure

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup / duplication)

Status: Open
Regression Test: none new for the duplication items — the guarantee is **byte-identical generated output**, enforced by `scripts/artifact-gate.sh` plus `scripts/test-accept.sh`. Item **C1 is the exception**: it needs a real acceptance test (see Validation Plan).

## Update (2026-07-20): what plan-57 changed underneath this report

plan-57 rewrote much of the collection-builder surface, so the C-item line
numbers below are stale. The findings themselves stand; read them by name, not by
line. What actually changed:

- **Every C item's duplication got *worse*, not better, and deliberately.** Each
  entry-table site now carries a second arm for the entry-free (`kind = 2`)
  representation. That raises the cost of leaving these duplicated: a change to
  the shared skeleton must now be made at six sites **times two arms**. plan-57-D
  found four corruption-class bugs, two of which (`net`'s byte-list builders,
  `push_collection_data_base_from_capacity`) were exactly this — one copy of a
  duplicated formula updated and another missed.
- **C5 (entry linear scan, six sites)** is the highest-value item after C1 for
  this reason. Any extracted `emit_entry_scan` must take the stride from
  `list_entry_stride`/`kind2_payload_size` rather than `COLLECTION_ENTRY_SIZE`,
  and must select it from the *block kind*, never from the payload type — a
  `Map OF Scalar TO T` has a fixed-width key and still keeps its entries.
- **C3 (three payload-compare emitters)** — all three gained an explicit
  `stride_type` parameter (`""` for a map). Their 7-arm dispatch is still
  triplicated; the extraction is now slightly larger but unchanged in shape.
- **S6 (two open-coded header writes)** — both sites still bypass
  `emit_write_list_header_from_registers`, and both now also need the kind byte
  from `list_block_kind`. A third open-coded-header class was found and fixed
  separately in plan-57-D: seven runtime byte-list builders were stamping
  `kind = 0` on blocks with no entry table.
- **What plan-57-A/B actually collapsed** is narrow: three byte-list
  constructors folded into `audio/mod.rs::emit_alloc_byte_list`, and two of
  eleven element-access sites. plan-57-A's own findings record that "38 indexed
  read sites" turned out to be 2 convertible ones. Re-measure before scheduling
  any C item — this report's counts were estimated, and every plan-57 estimate of
  the same surface was wrong by a large factor in both directions.

Two independent cleanup reviewers (Agent 02 — string/conversion builders; Agent 01
— collection builders) converged on the same shape of finding in
`src/target/shared/code/`: the same algorithm is emitted two, three, or eleven
times, each copy diverging only in label prefixes, slot names, and one scale
constant. Measured on this worktree: `emit_fixed_to_string_value` and
`emit_money_to_string_value` share 210 of 234/283 lines; `lower_list_append_in_place`
and `lower_list_prepend_in_place` share 352 of 417/413; `emit_string_to_int_value`
shares 99 of its 124 lines with the radix version that "Generalizes" it. None of
this is a user-visible defect on its own — every one of these paths compiles
correctly today.

**Except one.** Item C1 below is a three-way parallel implementation of static
type inference whose two hand-written builtin tables have already drifted apart,
and the drift is not theoretical: `io::print(typeName(strings::upper(s)))` — a
valid MFBASIC program — fails to compile with `error: native code cannot determine
typeName argument type`, on all fifteen `strings::` builtins tested. That item is
a correctness defect wearing a duplication costume, and this document requires it
be **reconciled first, as its own landable change with its own test**, before any
collapse touches it.

The single correct outcome a fix produces: each of the algorithms below exists
once, parameterized where the copies genuinely differ; the two static-type tables
are reconciled to a documented union (and `typeName(strings::upper(s))` compiles);
and **every artifact the compiler emits for every target is byte-identical before
and after**, except for the C1 reconciliation and the two items explicitly flagged
below as output-shifting.

References:

- Cleanup review, Agent 02 findings #1, #2, #3, #5, #6, #7, #12 and Agent 01
  findings #1, #3, #5, #6, #9, #10, #11 (`/tmp/cleanup-findings/index.md:33-66`).
- `spec/architecture/06_native.md` (the code-builder seam),
  `spec/language/06_collections.md` and the collection ABI constants at
  `src/target/shared/code/error_constants.rs:752-816`.
- **bug-322** (arena-allocation / internal-call / error-tail boilerplate). Every
  emitter cited here also contains open-coded alloc blocks. Those are bug-322's
  and are **not restated or re-scoped here**; this document covers only the
  algorithmic body between the allocs.
- **bug-327** (file splits / module re-organization). The `builder_strings*.rs`
  re-split, the `builder_collection_query.rs` ↔ `builder_collection_queries.rs`
  rename, and the `builder_collection_layout.rs` reordering belong there. Items
  S5, C6, and C7 below record the **measured evidence only** and hand the fix to
  bug-327. (Forward reference: bug-327 is a sibling document from the same review
  and may not exist yet at the time of reading.)
- Severity note: this document is filed LOW as a duplication cluster per the
  review's classification. Item C1, measured, is a reproducible compile-time
  failure on valid source and would stand alone at MEDIUM/Correctness. If the
  cluster is not scheduled promptly, split C1 out.
- **bug-354** (`bugs/bug-354-static-type-name-drift-typename-failure.md`) — the
  correctness counterpart to item **C1**, filed separately and independently
  re-verified. It owns the table reconciliation and its regression test; this
  document retains only the eventual collapse (C1 step 3). **It corrects C1's
  framing in three ways:** the failure is all **32** `strings::` builtins (not the
  15 sampled here) plus `math::abs`/`min`/`max` and
  `collections::find`/`contains`/`hasKey`; the predicate for failure is solely
  `static_type_name` returning `None`, so the "two tables disagree" framing below
  is imprecise; and the reverse direction (`no data object`) does **not** fire in
  practice. Read bug-354's matrix before touching C1. Severity there: HIGH.
- **bug-355** (`bugs/bug-355-map-getor-missing-hash-probe.md`) — the correctness
  counterpart to item **C4**'s hash-probe finding, and the resolution of Open
  Decision #1 below (affirmative: measured 33× at 4096 entries). It owns adding
  the probe to `lower_map_get_or`; this document retains only the `Miss`
  parameterization, which must land *after* it. Severity there: MEDIUM.

## Current State

This is a cleanup bug, so most items have no failing run — the evidence is
measurement. All commands below were run in the worktree root at base `25c38ba1`;
all `path:line` citations were opened and confirmed.

### Baseline sizes

```
$ wc -l src/target/shared/code/builder_{strings,strings_builtins,strings_package,conversions,search}.rs \
        src/target/shared/code/builder_collection_{query,queries,mutate,layout,compare}.rs \
        src/target/shared/code/builder_value_semantics.rs \
        src/target/shared/code/private/unicode.rs
    1796 builder_strings.rs
    2939 builder_strings_builtins.rs
     448 builder_strings_package.rs
    1499 builder_conversions.rs
    1152 builder_search.rs
     674 builder_collection_query.rs
    2073 builder_collection_queries.rs
    4471 builder_collection_mutate.rs
    1960 builder_collection_layout.rs
     497 builder_collection_compare.rs
     890 builder_value_semantics.rs
     962 private/unicode.rs
```

### The one item that actually fails today (C1)

```
$ cat src/main.mfb
IMPORT io
IMPORT strings

SUB main
  LET s AS String = "abc"
  io::print(typeName(strings::upper(s)))
END SUB

$ mfb build
Building tnrepro (executable) for macos-aarch64
error: native code cannot determine typeName argument type while lowering eval call io.print
```

- Observed: hard compile failure. Reproduced identically for all fifteen tested
  `strings::` builtins: `upper`, `lower`, `caseFold`, `normalizeNfc`, `trim`,
  `trimStart`, `join`, `split`, `graphemes`, `mid`, `replace`, `find`, `byteLen`,
  `contains`, `startsWith`, `endsWith`.
- Expected: prints `String` (or `List OF String` for `graphemes`/`split`,
  `Integer` for `find`/`byteLen`, `Boolean` for the predicates) — the types
  `data_objects.rs:1084-1114` already knows.

Contrast cases that build correctly, which bound the defect to the table
disagreement and become the regression guards:

| Program | Table that answers | Result |
| --- | --- | --- |
| `typeName(strings::upper(s))` | only `static_type_name_with_types` | fails ✗ |
| `typeName(math::sqrt(f))` | only `static_type_name` | works ✓ |
| `typeName(s)` where `s` is a local | both | works ✓ |
| `typeName(toString(1))` | both | works ✓ |

### One verbatim duplicated pair, quoted in full

Two scalar-count loops inside the **same function**,
`lower_strings_pad` (`builder_strings_builtins.rs:2395`). The first is
`:2464-2487`, the second `:2507-2530`. The only differences are the four label
prefixes (`strings_pad_scalars_*` vs `strings_pad_value_*`) and the mask register
(`scratch16` vs `scratch17`):

```rust
// builder_strings_builtins.rs:2464-2487
self.emit(abi::add_immediate(&scratch11, &scratch17, 8));
self.emit(abi::move_immediate(&scratch12, "Integer", "0")); // byte index
self.emit(abi::move_immediate(&scratch14, "Integer", "0")); // scalar count
self.emit(abi::move_immediate(&scratch16, "Integer", "192"));
self.emit(abi::label(&loop_label));
self.emit(abi::compare_registers(&scratch12, &scratch9));
self.emit(abi::branch_ge(&done));
self.emit(abi::add_registers(&scratch15, &scratch11, &scratch12));
self.emit(abi::load_u8(&scratch13, &scratch15, 0));
self.emit(abi::and_registers(&scratch13, &scratch13, &scratch16));
self.emit(abi::compare_immediate(&scratch13, "128"));
self.emit(abi::branch_ne(&not_cont));
self.emit(abi::branch(&after));
self.emit(abi::label(&not_cont));
self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
self.emit(abi::label(&after));
self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
self.emit(abi::branch(&loop_label));
self.emit(abi::label(&done));

// builder_strings_builtins.rs:2507-2530 — identical but for `scratch17` on the
// mask line and the `strings_pad_value_*` label prefix.
```

Three of the five `bug-175 E` alignment guards in `builder_collection_mutate.rs`
are byte-for-byte identical between append and prepend — comment and code:
`:869-873` ≡ `:1588-1592`, `:929-933` ≡ `:1646-1650`, `:1194-1198` ≡ `:1895-1899`.
The other two (`:838` / `:1556`, `:1123` / `:1831`) say the same thing in
different words, which is worse: a reader cannot tell whether the rewording
encodes a real difference. It does not.

---

## String / conversion

### S1 — `toString` for scaled integers is written three times (~340 redundant lines)

- `builder_strings.rs:1233` `emit_fixed_to_string_value` (234 lines)
- `builder_strings.rs:1476` `emit_money_to_string_value` (283 lines)
- `builder_strings.rs:833` `emit_integer_to_string_value` (150 lines)

Measured, after normalizing `fixed`/`Fixed`/`money`/`Money` to a common token:
fixed and money share **210 lines**; 24 lines are unique to fixed, 73 to money.
Comparing only the emitted `abi::` op sequences: 121 ops vs 143 ops with 32
differing entries — **111 shared ops**. The second function's own doc comment
concedes it (`builder_strings.rs:1472-1475`):

> Structurally mirrors `emit_fixed_to_string_value` with the scale changed from
> `2^32` to `100000` plus the half-away pre-round.

The third copy is not a paraphrase either. `emit_integer_to_string_value` emits a
79-op sequence of which **75 ops appear in the same order** in the 121-op fixed
sequence (4 unique). It differs only in register idiom — `allocate_register()`
rather than `temporary_vreg()`, for the documented x86 `div`/`msub` reason at
`builder_strings.rs:846-851` — and in having no fractional part.

Fix: extract `emit_scaled_decimal_to_string(scale, precision_default, rounding)`
covering the shared digit loop, buffer allocation, sign handling, and copy-out.
The genuinely distinct parts are money's half-away-from-zero pre-round and the
`2^32` vs `100000` scale. The integer path folds in as `scale = 1`.

### S2 — `emit_string_to_int_value` is a special case of `emit_string_to_int_value_base`

- `builder_conversions.rs:171` (base-10, 124 lines) vs `:306` (radix, 161 lines).

After normalizing the `string_to_int_base_` label prefix to `string_to_int_`:
**99 shared lines**, 25 unique to base-10, 62 to radix. The radix version's doc
comment states the relationship (`builder_conversions.rs:296-300`):

> Generalizes `emit_string_to_int_value`'s base-10 digit accumulation to an
> arbitrary `base` in `2..=36`…

Both carry a near-identical explanation of the same unsigned-compare hazard —
`:254-260` (7 lines, cites bug-144) and `:425-432` (8 lines, cites bug-49) — in
front of an **identical eight-instruction guard** (`compare_registers(acc,
cutoff)` / `branch_hi(overflow)` / `branch_eq(cutoff_equal)` / `branch(digit_ok)`
/ `label(cutoff_equal)` / `compare_registers(digit, cutlim)` /
`branch_hi(overflow)` / `label(digit_ok)`), at `:261-268` and `:433-440`.

Fix: delete the base-10 emitter and have `lower_to_int`'s 1-arg form call the
radix emitter with a constant base of 10 — or, if the constant-base specialization
is worth keeping for output size, extract the shared accumulate-with-cutoff body
so the hazard comment exists once.

### S3 — `builder_conversions.rs` hand-rolls a second UTF-8 codec

`private/unicode.rs` is the designated codec (its decoder carries a spec-anchored
self-defending contract at `:63-77`). `builder_conversions.rs` ignores it and
open-codes both directions.

**Encoder — a straight deletion.** `emit_scalar_to_string_value`'s 4-arm encoder
(`builder_conversions.rs:716-793`, ~78 lines) is instruction-for-instruction the
same shape as `emit_utf8_encode_next` (`private/unicode.rs:349-430`): identical
`shift_right`/`or 0xC0|0xE0|0xF0`/`and 0x3F`/`or 0x80`/`store_u8` ladder, identical
`128`/`2048`/`65536` thresholds. The only structural difference is bookkeeping:
`unicode.rs` advances a `cursor` register; `conversions` sets a `len` register.
`emit_utf8_encoded_width` (`private/unicode.rs:317-347`) computes exactly that
`len` from the same thresholds. So the replacement is a two-call sequence —
`emit_utf8_encoded_width(cp, len)` then `emit_utf8_encode_next(buf, cp)` — and the
~78 lines go.

**Decoder — needs a variant, not a swap.** `emit_string_to_scalar_value`
(`builder_conversions.rs:576-688`) and `emit_utf8_decode_next`
(`private/unicode.rs:78-219`) are **not interchangeable**, in two ways:

1. Malformed input: `unicode.rs` substitutes U+FFFD with width 1 and resyncs
   byte-wise (documented `:70-71`). `conversions` branches to `invalid` →
   `emit_invalid_argument_return()` (`:684`). `toScalar` must trap, not
   substitute.
2. Validation depth: `conversions` classifies only the **lead** byte
   (`:610-619`) and then masks continuation bytes unchecked; it does not reject
   overlongs, surrogates, or out-of-range codepoints the way `unicode.rs:63-77`
   does. It is protected today only by the ingress invariant (every `String` is
   valid UTF-8) plus its exact-length check at `:678-680`.

Fix: add a trap-on-malformed variant to `private/unicode.rs` (a `Malformed::Trap`
/ `Malformed::Substitute` mode parameter, or a thin
`emit_utf8_decode_next_strict`), and route `toScalar` through it. Point (2) is a
**strengthening** of the current behavior and will shift generated output for
`toScalar`; land it separately from the encoder deletion.

### S4 — the UTF-8 scalar-boundary walk is open-coded eleven times

The idiom is always the same six ops — `load_u8(byte, cursor, 0)`,
`and_registers(byte, byte, mask/*192*/)`, `compare_immediate(byte, "128")`,
`branch_ne(<done>)`, `add_immediate(cursor, cursor, 1)`,
`subtract_immediate(remaining, remaining, 1)` — wrapped in a loop, with the mask
`192` and the target `128` spelled as bare string literals every time.

Sites (all confirmed):

| File | Lines |
| --- | --- |
| `builder_search.rs` | `:183-192`, `:227-236`, `:693-702`, `:717-726` |
| `builder_strings_builtins.rs` | `:2185-2195`, `:2218-2228`, `:2464-2487`, `:2507-2530`, `:2844-2857`, `:2891-2903` |
| `builder_strings_package.rs` | `:185-197` |

Two corrections to the reviewer's framing:

- The lead said the literal `"192"` appears at 8 sites. It appears at **22 code
  sites** across `src/target/shared/code/` (`grep -rn '"192"'`, excluding
  `tls/macos.rs:83` where `LCTX_SIZE = "192"` is an unrelated struct size). Four
  of those are inside `private/unicode.rs` itself (`:121`, `:138`, `:175`, `:373`)
  — i.e. even the canonical codec respells it.
- The worst pair is intra-function, not cross-file: `builder_search.rs`
  `lower_find` (`:4`) contains the advance loop twice, at `:183-192` and
  `:227-236`, differing only in label names; and `lower_strings_pad`
  (`builder_strings_builtins.rs:2395`) contains the count loop twice, quoted
  verbatim in Current State above.

Fix: add `emit_scalar_advance` / `emit_scalar_retreat` / `emit_scalar_count` to
`private/unicode.rs`, and give `192` / `128` names
(`UTF8_CONTINUATION_MASK` / `UTF8_CONTINUATION_TAG`) there.

### S5 — the three `builder_strings*.rs` files have no principled split (evidence only; fix belongs to bug-327)

Recorded here because it is what makes S1 hard to find. Enumerating every
`pub(super) fn`:

- `builder_strings.rs` (1796 lines) contains **zero `strings::` builtins**. It is
  `lower_replace` (`:4`) + `lower_list_replace` (`:317`) — the bare `replace`
  builtin — and then the entire `toString` family (`:717`, `:812`, `:833`,
  `:984`, `:1233`, `:1476`, `:1765`). It is a conversion file wearing a strings
  name, which is precisely why S1's three copies sit unnoticed next to each other.
- `builder_strings_builtins.rs` (2939 lines) holds all 20 `lower_strings_*`
  entry points.
- `builder_strings_package.rs` (448 lines) holds the `strings::` dispatcher plus
  six unrelated helpers.

No fix is proposed here. **bug-327 owns the re-split.** The only constraint this
document places on it: do S1 and S3 either strictly before or strictly after the
split, never interleaved, so the byte-identical diff stays readable.

### S6 — two ~62-line open-coded collection-header writes, duplicating a helper 8 sites already call

`emit_write_list_header_from_registers`
(`builder_collection_mutate.rs:3340`) has eight callers:
`builder_strings_builtins.rs:145`, `:358`, `:1765`;
`builder_collection_layout.rs:413`; `builder_collection_mutate.rs:646`, `:3196`,
`:3949`, `:4284`.

Two sites bypass it and write the header field-by-field instead:

- `builder_strings.rs:471-537`, inside `lower_list_replace` (`:317`)
- `builder_search.rs:991-1052`, inside `lower_list_mid` (`:804`)

Both emit the same seven fields in the same order (`KIND`, `KEY_TYPE`,
`VALUE_TYPE`, `FLAGS_VERSION`, `COUNT`, `CAPACITY`, `DATA_LENGTH`,
`DATA_CAPACITY`) with `count` doubling as `capacity` — exactly the "tight header"
contract the helper documents at `:3337-3339`.

**This swap is NOT output-preserving, and that is a finding in its own right.**
The helper's callee `emit_write_collection_header_full`
(`builder_collection_mutate.rs:3356`) also writes
`COLLECTION_OFFSET_BUCKETS_READY = 0` at `:3403-3409`, with the comment "Fresh,
grown, and copied collections all reset it here." The two open-coded copies
**never write that byte** — they leave whatever the arena block happened to hold.
Both sites are list-only paths (`lower_list_replace`, `lower_list_mid`) where the
field is a documented no-op, so no live defect follows; but the helper's comment
overstates its own coverage, and adopting the helper adds two instructions at each
site.

Fix: adopt the helper at both sites, accept the two-instruction shift, regenerate
goldens, and confirm the delta is exactly those two instructions × 2 sites.
Correct the helper's "all reset it here" comment either way.

### S7 — the float exponent-decode preamble is written three times

The six-op sequence `move/float_move → shift_right_immediate(exponent, bits, 52)
→ move_immediate(mask, "2047") → and_registers → compare_immediate(exponent,
"2047") → branch_eq(<invalid|overflow>)` appears at:

- `builder_conversions.rs:131-136` (`emit_float_to_int_value`, `:110`)
- `builder_conversions.rs:1216-1221` (`emit_float_bits_to_fixed_value`, `:1193`)
- `builder_conversions.rs:1491-1497` (`emit_double_overflow_check`, `:1488`) —
  which already encapsulates exactly this and has three callers (`:850`, `:907`,
  `:1035`)

The duplication runs deeper than the reviewer noted. The **edge-case block** that
follows is also duplicated near-verbatim between the first two: `:141-151` vs
`:1226-1235` — `shift_right(sign, bits, 63)` / `compare 1` / branch /
`move_immediate(mask, "4503599627370495")` / `and_registers(mantissa, bits, mask)`
/ `compare 0` / `branch_ne(overflow)`. The **only** semantic differences across
the whole ~20-line region are the range threshold (`1086` for Integer at `:137`
vs `1054` for Fixed at `:1222`) and the label names.

One seam note for the implementer: `emit_double_overflow_check` takes its input
from an FP register (`float_move_x_from_d`, `:1491`) while the other two start
from a GPR already holding the bits (`move_register`, `:131`/`:1216`). The
extracted helper needs to take the bits register, with the FP move at the call
site.

Fix: extract `emit_float_exponent_range_guard(bits, threshold, overflow_label,
invalid_label)` covering both the preamble and the edge block; re-express
`emit_double_overflow_check` as the `threshold = None` case.

---

## Collection

### C1 — three parallel static-type resolvers, two of which have drifted into a compile failure (**do this one first, and separately**)

> **Split out to bug-354** (`bugs/bug-354-static-type-name-drift-typename-failure.md`),
> which owns the reconciliation, the full enumerated failure matrix, and the
> regression test. This section is retained for the duplication context and for
> step 3 (the collapse) only. bug-354 supersedes the mechanism description below
> where the two differ — in particular, the failure set is 38 calls, not 15, and
> it is driven solely by `static_type_name`.

The same question — "what is the static type of this `NirValue`?" — is answered by
three independent implementations:

| Implementation | Site | Type source | Builtin resolution |
| --- | --- | --- | --- |
| `CodeBuilder::static_type_name` | `builder_value_semantics.rs:650` | `self.locals` / `self.globals` | **hand-written table**, `:677-707` |
| `static_type_name_with_types` | `data_objects.rs:1066` | a `types: HashMap` | **hand-written table**, `:1084-1114` |
| `static_nir_value_type` | `type_utils.rs:3` | a `locals: HashMap` | delegates to `builtins::general/collections/strings::resolve_call` (`:32-42`) |

The third one is the correct design and already exists. The first two are ~110
lines each of hand-maintained builtin dispatch, and **they know disjoint sets of
builtins**:

Known only to `static_type_name` (`builder_value_semantics.rs`):
`replace`, `find`, `mid` (bare forms); `get`, `getOr`, `collections.get`,
`collections.getOr`; `math.floor`, `math.ceil`, `math.round`, `math.sqrt`,
`math.exp`, `math.log`, `math.log10`, `math.sin`, `math.cos`, `math.tan`,
`math.asin`, `math.acos`, `math.atan`, `math.pow`, `math.atan2`.

Known only to `static_type_name_with_types` (`data_objects.rs`):
`collections.find`, `strings.find`, `strings.mid`, `strings.replace`,
`strings.trim`, `strings.trimStart`, `strings.trimEnd`, `strings.upper`,
`strings.lower`, `strings.caseFold`, `strings.normalizeNfc`, `strings.join`,
`strings.graphemes`, `strings.split`, `strings.startsWith`, `strings.endsWith`,
`strings.contains`, `strings.byteLen`.

Known to both: `typeName`, `toString`, `len`, `toInt`, `toFloat`, `toFixed`,
`toByte`, `toMoney`, `toScalar`, `isNumeric`.

That is: the code-builder table knows **zero** `strings.*` targets; the
data-objects table knows **zero** `math.*` targets and nothing about `get`/`getOr`.

**The mechanism that turns this into a compile failure.** For `typeName(x)`, the
data-objects pre-pass folds the call to a string constant and interns it in the
literal pool (`data_objects.rs:1042-1048` → `static_type_name_with_types` →
`push_string_value`). The code builder independently folds the same call and then
**looks the result up in that pool** (`builder_values.rs:708-712` →
`load_string_constant` → `emit_load_string_constant`,
`builder_emit_helpers.rs:98-107`). The two folds must agree or the program does
not compile:

- Builder resolves, pre-pass did not → `native code string literal '<T>' has no
  data object` (`builder_emit_helpers.rs:106`).
- Pre-pass resolves, builder did not → `native code cannot determine typeName
  argument type` (`builder_values.rs:710`, also `:959`, `:1586`).

The second direction is live today; see Current State. All fifteen `strings::`
builtins reproduce it.

**There is no comment warning the two must agree.** The reviewer's lead said one
exists; I searched (`grep -rni 'in sync|must agree|agree with|same table'` over
`src/target/shared/code/`) and the only hits are `link_thunk.rs:1536` and
`os.rs:38`, both about unrelated tables, plus `mod.rs:1132` ("must agree EXACTLY")
which is about `CString` struct slots. The invariant is **entirely undocumented**,
which is why it broke.

**Required sequencing — this is the non-negotiable part of this document.** Do
*not* fold this into the collapse:

1. **Reconcile first.** Produce the union of the two tables, reconciled against
   `builtins::general::resolve_call` / `builtins::collections::resolve_call` /
   `builtins::strings::resolve_call` — the authoritative resolvers that
   `type_utils.rs:32-42` already uses. Write the union, and the justification for
   every entry, into this file. Where the two tables disagree on a *type* (not
   merely on presence), the `builtins::*` resolver wins.
2. **Land the reconciliation on its own**, with the acceptance test from the
   Validation Plan. This step *will* change generated output — programs that
   previously failed to compile now emit code, and the string pool gains entries.
   That is the intended delta.
3. **Only then collapse.** With the tables proven equal, delete both hand-written
   tables in favor of the `builtins::*`-delegating form, keeping the two thin
   wrappers that differ only in where locals' types come from
   (`self.locals` vs a passed `HashMap`). That step must be byte-identical.

### C2 — `lower_list_append_in_place` vs `lower_list_prepend_in_place`: 352 shared lines of 417/413

- `builder_collection_mutate.rs:799` (append, 417 lines)
- `builder_collection_mutate.rs:1517` (prepend, 413 lines)

Measured after normalizing `append`/`prepend` to a common token: **352 shared
lines**; 65 unique to append, 61 to prepend. Three of the five `bug-175 E`
alignment guards are byte-identical (quoted in Current State); the other two are
reworded paraphrases of the same rule.

Everything before the write phase is common: element-size/alignment computation,
capacity check, geometric growth of both `capacity` and `dataCapacity`, allocation,
verbatim copy of entries and data, free of the pre-grow buffer, install. Only the
final write genuinely differs — append writes at `slot[count]`, prepend shifts
entries right by one and writes `slot[0]` (`:266-276` of the normalized prepend
body).

Two implementation notes the diff surfaced:

- A partial extraction **already exists and append does not use it.**
  `emit_free_pre_grow_buffer` (`builder_collection_mutate.rs:238`) is called by
  prepend (`:1785`) and by the two map paths (`:2571`, `:2892`); append open-codes
  the equivalent at `:1070-1108`. The two are not instruction-equivalent — the
  helper delegates to `free_intermediate_collection` with a freshly allocated
  register, while append's inline copy sizes the block by hand into
  `scratch8`/`scratch9`/`scratch10`/`scratch11`/`scratch16`. **Collapsing append
  onto the helper will shift output.** Either keep append's inline copy inside the
  extracted `emit_list_grow_for_one`, or land the helper adoption as a separate
  golden-regenerating step.
- Prepend's comment at the normalized `:266-268` cites "`append` at `:957-991`".
  That line reference is stale — append's copy is at `:1070-1108`. Delete the
  reference rather than repairing it; the extraction makes it moot.

Fix: extract `emit_list_grow_for_one(list_slot, element_type, need_data)` covering
everything up to the write phase; leave two ~60-line write bodies.

### C3 — three payload-compare emitters duplicate the same 7-arm dispatch, and reimplement the String arm three times

- `builder_collection_compare.rs:194` `emit_collection_payload_match_branch`
- `builder_collection_compare.rs:288` `emit_collection_payload_matches_value_branch`
- `builder_collection_compare.rs:387` `emit_collection_payloads_match_branch`

All three match on the identical arm set in the identical order:
`"Boolean" | "Byte"` → `load_u8`; `"Scalar"` → `load_u32`;
`"Integer" | "Float" | "Fixed" | "Money"` → `load_u64`; `"String"`;
`is_pointer_collection_payload_type` → `load_u64`;
`type_model.record_fields.contains_key` → `emit_comparable_values_match_branch`;
`inline_collection_payload_size(..).is_some()` → `emit_compare_bytes_branch`;
`other` → the same error string shape.

The String arm hand-rolls a byte-compare loop in each: `:229-252`, `:334-351`,
`:442-459`. All three are the same loop —
`load_u8`/`load_u8`/`compare_registers`/`branch_ne`/two `add_immediate`/
`subtract_immediate`/`branch` — differing only in register names, the label
prefix, and where the length equality check reads its operands.

`emit_compare_bytes_branch` exists **in the same file at `:4`**, and each of these
three functions **already calls it** from the inline-payload arm (`:271-278` and
siblings). So the helper is present, proven, and skipped by the arm that most
needs it.

One divergence to preserve when collapsing: `emit_compare_bytes_branch` walks
private scratch copies of the pointers (documented at `:13-17`, "bug-175 D: … must
leave the caller's key pointer untouched"), whereas the inline String loops
advance the caller's `data`/`cur`/`lcur` registers in place. The first two
emitters do not read those registers afterward; verify per site before swapping,
and expect a small output shift from the two extra `move_register` ops.

Fix: collapse the three to two (`:194` and `:288` differ only in whether the data
pointer is computed internally), and route the String arm through
`emit_compare_bytes_branch` after the length check.

### C4 — `get`/`getOr` and `append`/`prepend` dispatcher pairs — and `getOr` on a map silently lost the hash-probe fast path

Four copy-paste pairs, all measured:

| Pair | Sites | Shared / total |
| --- | --- | --- |
| `lower_collection_get` / `lower_collection_get_or` | `builder_collection_queries.rs:25` / `:189` | differ in 3 blocks, all default-argument plumbing |
| `lower_list_get` / `lower_list_get_or` | `builder_collection_query.rs:4` / `:475` | 67 of 76 / 88 |
| `lower_map_get` / `lower_map_get_or` | `builder_collection_query.rs:337` / `:563` | 90 of 138 / 112 |
| `lower_collection_append` / `lower_collection_prepend` | `builder_collection_mutate.rs:4` / `:48` | differ in the index (`COUNT` vs `0`), one extra guard, and label/slot prefixes |

**The map pair is not "only the miss path", and this is a real finding the lead
did not carry.** `lower_map_get` opens with a 42-line hash-probe fast path
(`builder_collection_query.rs:345-386`): `if Self::map_key_probe_eligible(key_type)
{ … emit_map_probe(…) … return … "[hash]" }`. `lower_map_get_or`
(`:563`) **has no such block at all** — it goes straight to the linear entry scan.
So `collections::getOr` on a probe-eligible map is O(n) where `collections::get` on
the identical map is O(1). That is a silent performance divergence, not a
correctness one, and it is exactly the kind of omission a `Miss` parameterization
would have made impossible.

> **Split out to bug-355** (`bugs/bug-355-map-getor-missing-hash-probe.md`), which
> confirmed the reading, measured the divergence (33× at 4,096 entries; `hasKey`
> and `set` both have the probe, `contains` is a list op and needs none), and owns
> adding the probe. It lands **before** this item; C4 here remains the
> byte-identical `Miss` collapse and must still not add the probe.

Fix: parameterize each pair on a `Miss` enum (`Miss::Trap` vs
`Miss::Default(slot)`). Deciding whether `getOr` **should** get the hash probe is
a behavior question, not a refactor one — resolve it explicitly (see Open
Decisions) and land it separately from the collapse.

### C5 — the entry linear scan is hand-rolled at six sites

The skeleton is identical every time: load collection and needle from slots, load
`COLLECTION_OFFSET_COUNT`, zero an index, point `entry` at
`collection + COLLECTION_HEADER_SIZE`, then loop `compare_registers(index, count)`
/ `branch_ge(not_found)` / load the entry's offset+length pair / call a payload
compare emitter / advance by `COLLECTION_ENTRY_SIZE` / `branch(loop)`.

Confirmed sites (cited by the call into the compare emitter, which anchors the
loop body):

| File | Loop / call | Entry field scanned |
| --- | --- | --- |
| `builder_collection_query.rs` | `:400-437` (call `:430`) | `KEY_OFFSET` |
| `builder_collection_query.rs` | `:585-622` (call `:615`) | `KEY_OFFSET` |
| `builder_collection_queries.rs` | `:109-167` (call `:159`) | `VALUE_OFFSET` |
| `builder_collection_queries.rs` | `:325-357` (call `:355`) | `KEY_OFFSET` |
| `builder_collection_mutate.rs` | `:2268-2302` (call `:2290`) | `KEY_OFFSET` |
| `builder_collection_mutate.rs` | `:4166-4200` (call `:4197`) | `VALUE_OFFSET` |

Note this is not strictly a *map key* scan — `builder_collection_queries.rs:109`
(`lower_collection_contains`) runs the identical loop over `VALUE_OFFSET`. The
extraction must therefore be parameterized on the entry field, which makes it
strictly more valuable. Three further sites use the same compare emitters without
the surrounding scan (`builder_collection_mutate.rs:4324`,
`builder_search.rs:333`, `builder_strings.rs:391`/`:612`) and are out of scope.

Fix: extract `emit_entry_scan(collection_slot, needle_slot, entry_field, type_,
found_label, not_found_label)`.

### C6 — `builder_collection_query.rs` vs `builder_collection_queries.rs` (evidence only; fix belongs to bug-327)

Two files whose names differ by one letter, declared adjacently at
`src/target/shared/code/mod.rs:3105-3106`, with no conceptual seam. There is no
rule a reader can apply to guess which file a function is in. Worse, a single
logical operation is split across both: `lower_collection_get`
(`builder_collection_queries.rs:25`) is the dispatcher that calls `lower_list_get`
and `lower_map_get` (`builder_collection_query.rs:4` and `:337`) — the entry point
and its two implementations live in different files distinguished only by a
plural.

`builder_collection_query.rs` (674 lines) holds list/map get, the map key probe,
and the `_or` variants. `builder_collection_queries.rs` (2073 lines) holds the
public dispatchers plus zip, slice, and the whole callback family (`for_each`,
`transform`, `filter`, `reduce`, `sum`) — three unrelated subsystems.

No fix proposed here. **bug-327 owns the re-split** (a plausible cut is
`lookup` / `slice_zip` / `callbacks`). C4 and C5 above should land *after* it, so
the collapsed helpers land in their final home.

### C7 — `builder_collection_layout.rs` sandwiches its public API between two helper blocks (evidence only; fix belongs to bug-327)

The file reads: helpers `:4-1080` → public builtin lowerings `:1082-1334`
(`lower_len`, `lower_empty_collection`, `lower_list_literal`, `lower_map_literal`,
`lower_collection_values`) → helpers again `:1336-1959`.

**Three** emitters — not two, as the lead said — are defined after their only
caller, all inside this file:

| Emitter | Defined at | Sole caller(s) |
| --- | --- | --- |
| `emit_write_collection_header` | `:1336` | `:1310` |
| `emit_write_collection_entry` | `:1418` | `:1321` |
| `emit_add_payload_length` | `:1554` | `:1245`, `:1261` |

(Verified by `grep -rn 'self\.<name>(' src/target/shared/code/`: one, one, and two
call sites respectively, all above the definition, none outside the file.)

No fix proposed here beyond the reordering, which **bug-327 owns**.

---

## Root Cause

Two mechanisms, and they reinforce each other.

**Additive growth without extraction.** Every one of these pairs began as "copy
the working emitter, change the constant." `emit_money_to_string_value` says so in
its own doc comment (`builder_strings.rs:1472-1475`);
`emit_string_to_int_value_base` says so in its (`builder_conversions.rs:296-300`).
Nothing in review or CI penalizes the copy, because the copy is *correct* — the
cost is deferred entirely to the next person who has to change both.

**Absent or bypassed seams.** In four of these items the helper already exists and
the duplicate simply does not call it: `emit_compare_bytes_branch`
(`builder_collection_compare.rs:4`) is called by a sibling arm three lines from
each hand-rolled String loop; `emit_write_list_header_from_registers`
(`builder_collection_mutate.rs:3340`) has eight callers and two bypassers;
`emit_free_pre_grow_buffer` (`builder_collection_mutate.rs:238`) has three callers
and one bypasser; `private/unicode.rs` is the designated UTF-8 codec and
`builder_conversions.rs` rebuilt it. The file organization (S5, C6, C7) is what
keeps these seams invisible: a `toString` emitter living in `builder_strings.rs`
does not look like it belongs next to `builder_conversions.rs`, so nobody looks.

C1 is the same mechanism reaching its natural end state. Two hand-maintained
tables answering the same question, with no test relating them and no comment
naming the invariant, grew in opposite directions — one file gained `strings.*`
support, the other gained `math.*` — until a valid program stopped compiling. No
single commit is at fault; the *absence of a shared source of truth* is.

## Goal

- `emit_scaled_decimal_to_string` exists once and serves Fixed, Money, and
  Integer `toString`; the ~340 redundant lines in `builder_strings.rs` are gone.
- The base-10 integer parse is expressed as the radix parse with `base = 10`; the
  unsigned-cutoff hazard is explained in exactly one place.
- `builder_conversions.rs` contains no UTF-8 encoder or decoder; both directions
  route through `private/unicode.rs`, with the trap-vs-substitute policy as an
  explicit parameter.
- `emit_scalar_advance` / `emit_scalar_retreat` / `emit_scalar_count` exist in
  `private/unicode.rs` and are the only place `192` and `128` are spelled.
- `emit_write_list_header_from_registers` has ten callers and zero bypassers.
- One float exponent/range guard serves all three call sites.
- `emit_list_grow_for_one` exists; `lower_list_append_in_place` and
  `lower_list_prepend_in_place` are each under ~120 lines.
- Two payload-compare emitters remain, and none reimplements a byte compare.
- `get`/`getOr` and `append`/`prepend` are each one function with a `Miss` /
  index parameter.
- `emit_entry_scan` exists and serves all six scan sites.
- **C1:** the two static-type tables are replaced by delegation to
  `builtins::*::resolve_call`; the union is documented in this file with a
  justification per entry; `io::print(typeName(strings::upper(s)))` compiles and
  prints `String`; an acceptance test covers `typeName` over at least one builtin
  from each of `strings::`, `math::`, and `collections::`.
- Every artifact is byte-identical across the whole change **except** the three
  deltas named in Non-goals.

### Non-goals (must NOT change)

- **Generated-code semantics.** Every emitted instruction sequence must be
  byte-identical, with exactly three intended exceptions, each landed as its own
  commit with its own golden regeneration:
  1. **C1's reconciliation** — programs that previously failed to compile now
     emit code; the string pool gains entries.
  2. **S6's helper adoption** — two extra `BUCKETS_READY` instructions at two
     sites.
  3. **S3's strict decoder** — `toScalar` gains overlong/surrogate/range
     rejection it does not perform today.
  If any *other* golden moves, the refactor is wrong. Do not regenerate goldens
  to make a diff go away.
- **The collection ABI.** Header layout, entry layout, offsets, growth policy
  (`error_constants.rs:752-816`) are untouched. This is a code-organization
  change, not a data-format one.
- **`toScalar`'s trap-on-malformed contract.** S3 must not import
  `private/unicode.rs`'s U+FFFD substitution into `toScalar`. Substituting instead
  of trapping is the tempting wrong fix; it is forbidden.
- **`emit_compare_bytes_branch`'s scratch-copy discipline** (bug-175 D,
  `builder_collection_compare.rs:13-17`). C3 must not "simplify" it into advancing
  the caller's registers.
- **The `bug-49` / `bug-144` unsigned-compare guard** (S2) and the `bug-175 E`
  alignment guards (C2) must survive extraction unchanged in behavior. Losing one
  to a merge is the realistic failure mode here.
- **File splits and renames** (S5, C6, C7) are bug-327's. Do not do them
  opportunistically inside this work.
- **Arena-alloc, internal-call, and error-tail boilerplate** inside every emitter
  cited here is bug-322's. Do not collapse those blocks as part of this change;
  doing so makes both diffs unreviewable.

## Blast Radius

Searched, not recalled. Every site classified.

**Fixed by this bug:**

- `builder_strings.rs:833`, `:1233`, `:1476` (S1) — the three `toString` copies.
- `builder_conversions.rs:171`, `:306` (S2) — the two integer parses.
- `builder_conversions.rs:576-688`, `:694-808` (S3) — the second UTF-8 codec.
- `builder_search.rs:183`, `:227`, `:693`, `:717`;
  `builder_strings_builtins.rs:2185`, `:2218`, `:2464`, `:2507`, `:2844`,
  `:2891`; `builder_strings_package.rs:185` (S4) — eleven boundary walks.
- `builder_strings.rs:471-537`, `builder_search.rs:991-1052` (S6) — two header
  bypassers.
- `builder_conversions.rs:131`, `:1216`, `:1488` (S7) — three exponent decodes.
- `builder_value_semantics.rs:650`, `data_objects.rs:1066` (C1) — the two drifted
  tables.
- `builder_collection_mutate.rs:799`, `:1517` (C2).
- `builder_collection_compare.rs:194`, `:288`, `:387` (C3).
- `builder_collection_queries.rs:25`, `:189`; `builder_collection_query.rs:4`,
  `:337`, `:475`, `:563`; `builder_collection_mutate.rs:4`, `:48` (C4).
- `builder_collection_query.rs:430`, `:615`; `builder_collection_queries.rs:159`,
  `:355`; `builder_collection_mutate.rs:2290`, `:4197` (C5).

**Latent, same hazard, out of scope:**

- `type_utils.rs:3` `static_nir_value_type` — the *third* static-type
  implementation. It is not defective (it delegates to `builtins::*`), so it is
  the reconciliation target, not a fix target. Its callers
  (`module_analysis.rs:350`, `:351`, `:390`) are unaffected.
- `private/unicode.rs:121`, `:138`, `:175`, `:373` — the canonical codec also
  respells `"192"`. Naming the constant there is part of S4; the four sites are
  already inside the right module.
- `builder_collection_mutate.rs:4324`, `builder_search.rs:333`,
  `builder_strings.rs:391`, `:612` — call the payload-compare emitters without the
  surrounding entry scan. Unaffected by C5's extraction; they benefit from C3
  automatically.
- `builder_collection_layout.rs:1098` — a `"192"` site in a payload path, not a
  scalar-boundary walk. Out of scope for S4.
- The 22 open-coded arena-alloc blocks inside these same emitters — **bug-322**.
- `builder_values.rs:200-1443` (`lower_value_inner`, one 1244-line function) —
  reported by the same reviewer; it is a file-split item, **bug-327**.

**Unaffected:**

- `builtins::general/collections/strings::resolve_call` — C1 reads these as the
  source of truth; it does not modify them.
- All four backends (`aarch64`, `x86_64`, `riscv64`, and the platform `target/*`
  modules) — every change here is above the MIR seam and emits the same ops.
- The `.mfp` format, the linker, and the runtime helper catalog.

## Fix Design

The shape is uniform: for each pair, normalize away the cosmetic difference
(labels, slot names, scratch numbering), confirm the remainder is empty or is a
named parameter, and extract. The correctness risk is **not** in any individual
extraction — it is in three specific places:

1. **C1's reconciliation.** This is the only step that changes what the compiler
   accepts. Getting the union wrong in the permissive direction produces a
   `no data object` failure on a *different* program; getting it wrong in the
   restrictive direction leaves a `cannot determine typeName` failure in place.
   Both are loud, which is the saving grace. Deriving the union from
   `builtins::*::resolve_call` rather than by hand-merging the two tables is what
   makes this tractable.
2. **Preserved hazard guards.** The `bug-49`/`bug-144` unsigned-cutoff comparison
   (S2) and the five `bug-175 E` alignment guards (C2) each encode a real,
   previously-shipped defect. An extraction that silently drops one produces a
   miscompile on a narrow input (i64::MIN magnitude; an unaligned variable-length
   element) that the golden suite may not cover. Diff the *emitted op sequence*,
   not the source, at each of these sites.
3. **The three intended output shifts** (Non-goals). Each must be its own commit.
   A combined diff makes it impossible to tell an intended shift from a broken
   extraction.

**Rejected alternatives**, recorded so they are not re-litigated:

- *Collapse C1's two tables into one shared hand-written table without
  reconciling first.* Rejected: it silently picks a winner. Whichever table is
  kept, the other's builtins vanish, converting the current failure into a
  different failure. The union must be derived and justified before either table
  is deleted.
- *Swap `toScalar`'s decoder for `emit_utf8_decode_next` directly.* Rejected: it
  converts a trap into a U+FFFD substitution, silently changing `toScalar`'s
  documented failure contract.
- *Adopt `emit_write_list_header_from_registers` at the two bypassers and suppress
  the resulting golden diff.* Rejected: the diff is real (the `BUCKETS_READY`
  store). Regenerate and verify it is exactly those two instructions.
- *Do the file splits (S5/C6/C7) as part of these extractions.* Rejected: a diff
  that both moves 3,000 lines between files and rewrites emitters cannot be
  reviewed for byte-identity. bug-327 first or last, never interleaved.
- *Regenerate all goldens once at the end.* Rejected: it defeats the entire
  verification strategy. Run `scripts/artifact-gate.sh` after every extraction.

## Phases

### Phase 1 — C1 reconciliation (the only behavior change)

- [ ] Add an acceptance test exercising `typeName` over `strings::upper`,
      `strings::split`, `strings::contains`, `math::sqrt`, `math::round`, and
      `collections::get`. Confirm the `strings::` cases fail today with
      `native code cannot determine typeName argument type`.
- [ ] Derive the union of `builder_value_semantics.rs:677-707` and
      `data_objects.rs:1084-1114` against `builtins::general/collections/strings::resolve_call`.
      Write the table and a per-entry justification into this file.
- [ ] Replace both hand-written tables with delegation to the `builtins::*`
      resolvers, mirroring `type_utils.rs:32-42`.
- [ ] Add a unit test asserting the two resolvers return the same answer for every
      builtin name the `builtins::*` catalog knows — the missing invariant, made
      executable.

Acceptance: the Phase 1 acceptance test passes; the parity unit test passes; the
golden delta consists only of newly-compiling programs and new string-pool
entries.
Commit: `—`

### Phase 2 — byte-identical extractions

Land each as its own commit, running `scripts/artifact-gate.sh` after every one.

- [ ] S1 — `emit_scaled_decimal_to_string`; retire the three copies.
- [ ] S2 — collapse the base-10 parse onto the radix parse.
- [ ] S4 — `emit_scalar_advance`/`_retreat`/`_count` in `private/unicode.rs`;
      named `UTF8_CONTINUATION_MASK`/`_TAG`; convert all eleven sites.
- [ ] S7 — `emit_float_exponent_range_guard`; convert all three sites.
- [ ] C2 — `emit_list_grow_for_one`; keep append's inline free block as-is.
- [ ] C3 — collapse three payload-compare emitters to two; route the String arm
      through `emit_compare_bytes_branch`.
- [ ] C4 — `Miss` parameter for `get`/`getOr`; index parameter for
      `append`/`prepend`. Do **not** add the hash probe to `getOr` here.
- [ ] C5 — `emit_entry_scan`; convert all six sites.
- [ ] S3 (encoder half) — delete `builder_conversions.rs:716-793` in favor of
      `emit_utf8_encoded_width` + `emit_utf8_encode_next`.

Acceptance: `scripts/artifact-gate.sh` reports **zero** golden diffs after each
commit in this phase. Any diff means the extraction is wrong.
Commit: `—`

### Phase 3 — the two remaining intended shifts

- [ ] S6 — adopt `emit_write_list_header_from_registers` at both bypassers;
      regenerate goldens; confirm the delta is exactly two `BUCKETS_READY` stores
      × two sites. Fix the helper's "all reset it here" comment.
- [ ] S3 (decoder half) — add the trap-on-malformed variant to
      `private/unicode.rs`; route `toScalar` through it; regenerate goldens;
      confirm the delta is confined to `toScalar` lowering.
- [ ] Resolve Open Decision #1 (`getOr` hash probe) and land it if affirmative,
      separately.

Acceptance: goldens shift only as described; `scripts/test-accept.sh` green.
Commit: `—`

### Phase 4 — full validation

- [ ] `scripts/test-accept.sh` green on macOS.
- [ ] `scripts/artifact-gate.sh` clean against the Phase 3 goldens.
- [ ] Re-run the C1 reproduction end-to-end; confirm it prints `String`.
- [ ] Confirm no `builder_*` file gained an arena-alloc or file-split change that
      belongs to bug-322 / bug-327.

Acceptance: full suite green; the total golden delta across the whole bug equals
the union of the three intended shifts.
Commit: `—`

## Validation Plan

- **Regression test (C1 only):** a new acceptance case under
  `tests/rt-behavior/` printing `typeName` of a `strings::`, a `math::`, and a
  `collections::` call. It fails today with `native code cannot determine typeName
  argument type` and passes after Phase 1. Plus a unit test asserting the two
  static-type resolvers agree across the `builtins::*` catalog.
- **The guarantee for every other item is byte-identical output**, not a new test.
  `scripts/artifact-gate.sh <mfb>` regenerates the deterministic `-ast`/`-ir`/plan
  dumps and diffs them against committed goldens with no link or run step (~5 min),
  and is the per-commit gate for all of Phase 2.
- **Runtime proof:** `scripts/test-accept.sh` — the full acceptance suite,
  compiling and running every fixture. This is what catches a dropped `bug-49` or
  `bug-175 E` guard that the artifact dumps would not.
- **Coverage caveat:** both scripts run one host target. The extractions here are
  all above the MIR seam and emit identical ops on every backend, so host-only
  coverage is adequate — but S3's decoder change and C1's new string-pool entries
  should be spot-checked on a Linux target before closing.
- **Doc sync:** `spec/architecture/06_native.md` if the `private/unicode.rs`
  public surface grows (S3, S4). The `emit_write_collection_header_full` comment
  claiming "Fresh, grown, and copied collections all reset it here"
  (`builder_collection_mutate.rs:3403-3405`) is currently false and must be
  corrected in S6 regardless of outcome.

## Open Decisions

1. ~~**Should `collections::getOr` on a map use the hash probe?**~~ **RESOLVED —
   yes.** Measured in **bug-355**: at 4,096 entries, 20,000 `getOr` lookups take
   ~67 ms against ~2 ms for the identical `get` lookups, a 33× gap that doubles
   with every doubling of the map, while `hasKey` (which has the probe) stays
   flat. The "document the asymmetry as intentional" alternative is rejected.
   bug-355 owns the probe addition and lands **before** C4's collapse; C4 remains
   byte-identical and must still not add the probe itself. (§C4)
2. **Should `emit_string_to_int_value` survive as a constant-base specialization?**
   Collapsing it onto the radix emitter is the clean answer but may grow emitted
   code for the common 1-arg `toInt`. Recommended: collapse, measure, and
   re-specialize only if the artifact gate shows meaningful growth. (§S2)
3. **How strict should `toScalar` become?** S3's strict decoder rejects overlongs
   and surrogates that the current lead-byte-only classifier accepts. Since every
   `String` is valid UTF-8 by the ingress invariant, this is unreachable in
   practice — so the choice is between defense-in-depth and byte-identity.
   Recommended: strict, matching `private/unicode.rs`'s documented posture
   (`:63-77`). (§S3)

## Summary

Roughly 1,400 lines of algorithmic duplication across the string/conversion and
collection builders, in thirteen itemized clusters, every one measured rather than
estimated. Twelve of them are pure cleanup whose correctness is guaranteed by
byte-identical output under `scripts/artifact-gate.sh` and `scripts/test-accept.sh`.

The thirteenth is not. **C1 — the three-way parallel static-type inference — has
already drifted into a reproducible compile failure**: `typeName(strings::upper(s))`
does not build, on any of fifteen `strings::` builtins, because two hand-written
tables that must agree have grown in opposite directions with no comment, no test,
and no shared source of truth relating them. That item must be reconciled against
`builtins::*::resolve_call`, with the union documented and justified and a parity
test added, **before** anything collapses it — and it should be scheduled first
regardless of what happens to the rest of this cluster.

Left untouched: the collection ABI, all four backends, and — deliberately — the
arena-allocation boilerplate (bug-322) and the file splits (bug-327) that
interleave with every item here and would make this diff unreviewable.
