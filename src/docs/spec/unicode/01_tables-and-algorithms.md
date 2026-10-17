# Unicode Tables and Algorithms

MFBASIC's `strings::` package implements grapheme segmentation, normalization,
and case mapping in the compiled native binary, with
no external library dependency. To do this the compiler embeds a set of
read-only Unicode property and mapping tables into a program whose generated code
performs a Unicode-aware `strings::` or `regex::` operation at run time. Calls
handled by the static folding paths can instead embed their results. Runtime
routines walk the tables as
architecture-neutral `abi::` operations, which each native backend lowers. This
topic specifies the embedded tables (layout, source, emission) and the runtime
algorithms that consume them.

The compile-time and runtime paths use the data as follows:

- **Compile-time folding.** When a Unicode-aware call (`strings::upper`,
  `lower`, `caseFold`, `normalizeNfc`, `graphemes`) has a statically known
  string argument, the compiler evaluates it in-process using the Rust crates
  `unicode_segmentation`, `unicode_normalization`, and `unicode_casefold`, and
  the result is baked in as a literal. No table lookup happens at runtime.
  [[src/target/shared/nir/constfold.rs:native_strings_package_static_string_value]]
  [[src/codegen/engine/types/type_utils.rs:strings_package_static_string_value]]
  [[src/unicode/backend.rs:graphemes]]
  [[src/codegen/builtins/strings/func_graphemes.rs:lower]]
- **Terminal measurement.** `term::measureText` always executes its runtime
  function, even for a statically known string, so its presentation-mode and
  active-terminal gates cannot be folded away. Its grapheme walk reads the same
  packed property trie as terminal drawing.
  [[src/codegen/builtins/term/func_measure_text.rs:lower_measure_text]]
  [[src/codegen/builtins/term/func_measure_text.rs:measure_text_of]]
- **Runtime tables.** When the argument is dynamic, the compiler emits code that
  uses the algorithms below against the embedded tables: a two-stage lookup for
  packed properties, binary search for mappings and category/Script ranges, and
  per-starter scans for composition pairs. The property trie and composition
  tables come from utf8proc; NFD and case mappings are generated through Rust
  mappings; category and Script come from separate pinned tables. [[src/codegen/string/unicode_props.rs:emit_unicode_property_lookup]]

The string memory model these routines operate on (UTF-8 length-prefixed heap
values) is `./mfb spec unicode strings-model`. `mfb man strings` owns how each
`strings::` function is called (arguments, return type, error codes); this package
owns the runtime model and the semantic contracts a faithful reimplementation must
reproduce.

## Pinned general-category and Script tables

General category and Script do **not** come from the utf8proc property trie. They
are answered from two separate range tables generated from **Unicode 16.0.0** and
emitted as their own read-only data. [[src/unicode/range_tables.rs:gencat]]
[[src/unicode/range_tables.rs:script]]

Their consumers are `strings::isLetter`, `isDigit`, `isWhitespace`, `isUpper` and
`isLower` (through the internal `strings::genCat`), and `regex`'s Unicode property
classes, `\p{Script}` tests, shorthand classes and word test (through the internal
`regex::genCat` and `regex::scriptOf`).
[[src/codegen/builtins/strings/func_gen_cat.rs:lower]]
[[src/codegen/builtins/regex/func_gen_cat.rs:lower]]
[[src/codegen/builtins/regex/func_script_of.rs:lower]]
[[src/codegen/builtins/regex/helper_prop_test.rs:__regex_propTest]]
[[src/codegen/builtins/regex/helper_script_test.rs:__regex_scriptTest]]
[[src/codegen/builtins/regex/helper_shorthand_match.rs:__regex_shorthandMatch]]
[[src/codegen/builtins/regex/helper_is_word.rs:__regex_isWord]]

These sources are versioned independently. Grapheme properties, combining classes,
composition pairs and display widths come from vendored utf8proc. Decomposition
and case mapping come from the Rust dependencies and Rust's `char` mappings.
General category, Script and Line_Break (below) remain pinned to Unicode 16.0.0
independently of those sources. Updating one source does not update the others; the compiler does
not enforce a shared Unicode version.
[[src/unicode/runtime_tables.rs:parse_tables]]
[[src/unicode/range_tables.rs:GENCAT_RANGES]]
[[src/unicode/range_tables.rs:SCRIPT_RANGES]]

Each category/Script range record is `(u32 lastCodepoint, u32 nameIndex)`,
little-endian, with an 8-byte stride. Binary search selects the first range whose
inclusive end is at least the scalar. Names are fixed-stride String records:
`u64 byteLength`, UTF-8 bytes, NUL, then padding to the table's stride, which is
`round_up(8 + longest_name_bytes + 1, 8)`. The query returns a read-only pointer
into the names table without allocating.
[[src/unicode/range_tables.rs:runs_bytes]]
[[src/unicode/range_tables.rs:names_bytes]]
[[src/unicode/range_tables.rs:name_stride]]
[[src/codegen/string/unicode_props.rs:emit_unicode_range_lookup]]

## Provenance: utf8proc

The packed property trie and composition tables are parsed out of a vendored C source
file, `third_party/utf8proc/utf8proc_data.c`, included into the compiler at
build time with `include_str!`. utf8proc is the Julia-project Unicode library
(MIT/expat license), vendored at version 2.11.3 (the
`UTF8PROC_VERSION_MAJOR`/`_MINOR`/`_PATCH` macros); its embedded data tracks the
corresponding Unicode release.
[[src/unicode/runtime_tables.rs:UTF8PROC_DATA]]

The compiler does **not** link or call utf8proc. `parse_tables` scrapes the C
array initializers textually — `array_body` finds `static const <name>[] = {`
and slices to the matching `};` — then reparses each comma-separated element.
Symbolic constants (`UTF8PROC_BOUNDCLASS_*`, `UTF8PROC_INDIC_CONJUNCT_BREAK_*`,
`UINT16_MAX`, `true`/`false`) are mapped to integers by hand-written match tables;
bidi-class references collapse to `0` (unused). The general-category and
decomposition-type columns are never consumed, so their constants are not mapped.
[[src/unicode/runtime_tables.rs:parse_value]] The NFD, uppercase, lowercase, and casefold
mapping tables are **not** taken from utf8proc — they are generated by iterating
every codepoint `0..=0x10FFFF` through the Rust normalization/case crates
(`build_mapping_tables`). [[src/unicode/runtime_tables.rs:parse_tables]]
[[src/unicode/runtime_tables.rs:array_body]]
[[src/unicode/runtime_tables.rs:build_mapping_tables]]

## Two-stage property lookup

The packed properties below come from a two-level trie keyed by the
high and low byte of the codepoint: `stage1[cp >> 8]` yields a base index into
`stage2`, and `stage2[base + (cp & 0xFF)]` yields the index of a shared
`PackedProperty` record. Many codepoints share one property record, so the
trie is far smaller than 0x110000 entries. [[src/unicode/runtime_tables.rs:property_for_codepoint]]

```text
cp ──▶ stage1[cp >> 8] ─────────────▶ base
                                       │
       (cp & 0xFF) ───────────────────┼──▶ stage2[base + low] ─▶ prop_index
                                            │
                                            ▶ properties[prop_index]  (12-byte record)
```

Both stage tables are `Vec<u16>`. The runtime emitter `emit_unicode_property_lookup`
reproduces this exactly as architecture-neutral `abi::` operations that each
native backend lowers: shift `cp` right 8, scale by 2 (u16 stride),
load the stage-1 entry, add `cp & 0xFF`, scale by 2, load the stage-2 entry,
multiply by the 12-byte record size, and add to the base of the properties
table. The result register holds a pointer to the live `PackedProperty` record;
individual fields are then read with `load_u16` at fixed offsets.
[[src/codegen/string/unicode_props.rs:emit_unicode_property_lookup]]
[[src/codegen/string/unicode_props.rs:emit_unicode_property_u16]]

Reference table sizes (asserted by tests): stage1 = 4352 u16, stage2 = 46336
u16, properties = 8385 records, combinations = 961 entries each.
[[src/unicode/runtime_tables.rs:parses_utf8proc_runtime_tables]]

## PackedProperty record

`PackedProperty` is the Rust-side representation; it carries six `u16` fields.
On emission each record is serialized little-endian, in field order, producing a
**12-byte** on-disk record (6 × 2). [[src/unicode/runtime_tables.rs:PackedProperty]]
[[src/unicode/runtime_tables.rs:encode_le]]

| Off | Field                  | Source (utf8proc field idx) | Runtime offset const                            |
|-----|------------------------|-----------------------------|-------------------------------------------------|
| 0   | `combining_class`      | 1                           | `UNICODE_PROPERTY_OFFSET_COMBINING_CLASS` = 0   |
| 2   | `comb_index`           | 9                           | `UNICODE_PROPERTY_OFFSET_COMB_INDEX` = 2        |
| 4   | `comb_length`          | 10                          | `UNICODE_PROPERTY_OFFSET_COMB_LENGTH` = 4       |
| 6   | `flags`                | 11/13/14/15/16/17 (packed)  | `UNICODE_PROPERTY_OFFSET_FLAGS` = 6             |
| 8   | `boundclass`           | 19                          | `UNICODE_PROPERTY_OFFSET_BOUNDCLASS` = 8        |
| 10  | `indic_conjunct_break` | 20                          | `UNICODE_PROPERTY_OFFSET_INDIC_CONJUNCT_BREAK`=10|

Decomposition and case mappings use the separate flattened u32 tables below;
the packed record contains no mapping sequence indices. The compiler-side offset
constants must agree with `PackedProperty::encode_le`.
[[src/codegen/string/unicode_props.rs:UNICODE_PROPERTY_SIZE]]

`flags` is a bitfield packed from four utf8proc booleans plus the display-width
fields:

| Bit  | Flag                              | utf8proc field |
|------|-----------------------------------|----------------|
| 0    | `COMB_IS_SECOND` (composition 2nd)| 11             |
| 1    | `COMP_EXCLUSION`                  | 13             |
| 2    | `IGNORABLE`                       | 14             |
| 3    | `CONTROL_BOUNDARY`                | 15             |
| 4–5  | `charwidth` (0–3, 2 bits)         | 16             |
| 6    | `AMBIGUOUS` (East Asian ambiguous)| 17             |

The runtime reads the display width as `(flags >> 4) & 0b11` (`CHARWIDTH_SHIFT`=4,
`CHARWIDTH_MASK`); a value of 2 is East-Asian-Wide (two terminal columns), 1 is
one column, and 0 is zero-width. How per-scalar widths combine into a grapheme's
width — including the one-cell clamp for a grapheme made only of zero-width
scalars — is owned by `./mfb spec unicode strings-model`. The `AMBIGUOUS` bit
marks East Asian Ambiguous width; terminal measurement and drawing treat
ambiguous as width 1.

[[src/unicode/runtime_tables.rs:COMB_IS_SECOND]]
[[src/unicode/runtime_tables.rs:charwidth]]
[[src/unicode/runtime_tables.rs:parse_properties]]

`combining_class`, `comb_index`, `comb_length`, `flags`, `boundclass`, and
`indic_conjunct_break` are the only fields actually consumed at runtime, by NFC
canonical ordering / composition, grapheme segmentation and display-width measurement.

## Composition (combinations) tables

NFC recomposition uses two parallel `Vec<u32>` tables of equal length
(`combinations_second`, `combinations_combined`), scraped directly from
utf8proc. Given a starter's `comb_index`/`comb_length`, the run scans
`combinations_second[comb_index .. comb_index+comb_length]` for the second
codepoint; on a hit the same slot in `combinations_combined` is the composed
result. [[src/unicode/runtime_tables.rs:combinations_second]]

## Mapping tables (NFD, upper, lower, casefold)

NFD decomposition and the three case maps each use a pair of generated tables:
an *entries* table of fixed records plus a flattened *sequences* table of u32
codepoints. They are generated identically by `build_mapping_tables`, which
walks every codepoint, applies a mapping closure, and records an entry only when
the mapping differs from the identity (i.e. is not just `[cp]`). The entries are
therefore emitted in ascending codepoint order, which the runtime relies on for
binary search. [[src/unicode/runtime_tables.rs:build_mapping_tables]]

- NFD: closure `value.nfd()`. [[src/unicode/runtime_tables.rs:build_nfd_tables]]
- Uppercase: `char::to_uppercase`. Lowercase: `char::to_lowercase`. Casefold:
  `case_fold()`. [[src/unicode/runtime_tables.rs:parse_tables]]

Each entry is the `NfdEntry` record, serialized little-endian as **16 bytes**
(three u32 fields plus one u32 of zero padding):

| Off | Field             | Runtime offset const                         |
|-----|-------------------|----------------------------------------------|
| 0   | `codepoint`       | `UNICODE_NFD_ENTRY_OFFSET_CODEPOINT` = 0     |
| 4   | `sequence_offset` | `UNICODE_NFD_ENTRY_OFFSET_SEQUENCE_OFFSET`=4 |
| 8   | `sequence_length` | `UNICODE_NFD_ENTRY_OFFSET_SEQUENCE_LENGTH`=8 |
| 12  | (zero pad)        | —                                            |

[[src/unicode/runtime_tables.rs:NfdEntry]]
[[src/unicode/runtime_tables.rs:nfd_entries_hex]]
[[src/codegen/string/unicode_props.rs:UNICODE_NFD_ENTRY_SIZE]]

`sequence_offset` is a u32 index into the corresponding flattened sequences
table (each element a u32 codepoint); `sequence_length` is the element count.
The runtime lookup is a binary search keyed on `codepoint`, scaling the midpoint
by the 16-byte record stride (`<< 4`), with a `not_found` exit that returns
length 0. On a hit it scales `sequence_offset` by 4 to a byte address into the
sequences table. The same helper serves NFD and all three case maps; only the
entries/sequences symbols and the entry count differ.
[[src/codegen/string/unicode_props.rs:emit_unicode_u32_mapping_lookup]]

## Embedding: data objects and symbols

A compiler pass emits up to seventeen raw, read-only `CodeDataObject`s: the
thirteen utf8proc and mapping tables below, plus the four pinned general-category
and Script range and name tables.
Which tables a given program carries is an **implementation detail, not a
language or binary-format guarantee**: the selection below can change without
changing any Unicode result. Today emission is **per-table**, driven by the
relocations the generated code actually carries: a table is emitted iff some
function relocates against its
`_mfb_unicode_*` symbol. So a program that reaches only part of the Unicode
surface carries only the tables it reads — `strings::graphemes` and
`term::measureText` pull just the base trie (`stage1`/`stage2`/`properties`);
`strings::caseFold`
pulls just the two casefold tables (the case-mapping path never indexes the
property trie); `strings::normalizeNfc` pulls the base trie plus the
`combinations_*`/`nfd_*` tables. The relocation scan is the ground truth; a
coarse NIR usage heuristic remains only as a fallback that emits the full set
when the NIR usage scan detects Unicode use but no table relocation is named.
Each table's bytes come from a hex serializer; sizes and
alignments are fixed per table (u16 tables and the 12-byte `properties` records
align 2; u32 tables and the 16-byte mapping-entry records align 4). [[src/codegen/memory/data/data_objects.rs:unicode_runtime_data_objects]]
[[src/codegen/engine/analysis/module_analysis.rs:module_uses_unicode_runtime_tables]]
[[src/codegen/engine/builder/mod.rs:referenced_unicode_tables]]

| Symbol (`_mfb_unicode_*`)        | Element / record       | Align |
|----------------------------------|------------------------|-------|
| `stage1`                         | u16                    | 2     |
| `stage2`                         | u16                    | 2     |
| `properties`                     | 12-byte record         | 2     |
| `combinations_second`            | u32                    | 4     |
| `combinations_combined`          | u32                    | 4     |
| `nfd_entries`                    | 16-byte record         | 4     |
| `nfd_sequences`                  | u32                    | 4     |
| `uppercase_entries` / `_sequences` | 16-byte record / u32 | 4     |
| `lowercase_entries` / `_sequences` | 16-byte record / u32 | 4     |
| `casefold_entries` / `_sequences`  | 16-byte record / u32 | 4     |
| `gencat_ranges` / `script_ranges` | 8-byte range record | 4 |
| `gencat_names` / `script_names` | fixed-stride String record | 8 |

[[src/codegen/error/constants/error_constants.rs:UNICODE_STAGE1_SYMBOL]]

The emitter reads each table's `.len()` from `unicode::runtime_tables::tables()`
to compute object sizes; the runtime helpers likewise pass `tables().*.len()` as
the binary-search entry count, so embedded data and emitted code share one source
of truth. [[src/codegen/string/unicode_props.rs:emit_unicode_u32_mapping_lookup]]

## Runtime algorithm: UTF-8 codec

The shared scalar codec underpins every algorithm. `emit_utf8_decode_next` reads
the leading byte, branches on the 0x80/0xE0/0xF0 thresholds for 1–4 byte widths,
masks/shifts the continuation bytes, and returns both the scalar and its byte
width. The decoder is self-defending: even though every `String` is valid UTF-8
by construction, it validates each continuation byte (`0x80..=0xBF`) before
reading the next and rejects surrogates, overlong encodings, and leads beyond
U+10FFFF, decoding any malformed sequence as U+FFFD with width 1. The scalar it
produces is therefore always `<= 0x10FFFF`, which keeps the two-stage property
lookup in-bounds for decoded scalars. This helper takes a pointer, not a byte
length: it requires readable string storage and relies on the trailing NUL to
stop continuation-byte validation at a truncated tail. On a valid `String` the
substitution never fires. `emit_utf8_encoded_width` computes a scalar's encoded
width (thresholds 0x80 / 0x800 / 0x10000), and `emit_utf8_encode_next` writes
1–4 bytes and advances the cursor.
[[src/codegen/string/unicode_props.rs:emit_utf8_decode_next]]
[[src/codegen/string/unicode_props.rs:emit_utf8_encode_next]]

## Runtime algorithm: grapheme segmentation

`strings::graphemes` (and `graphemesCount`, `graphemeAt`) use an emitted
extended-grapheme boundary state machine based on UAX #29, with the Indic
context limitation described below. The emitted code walks scalars,
maintaining a two-field state machine — `state_bc` (boundclass) and `state_icb`
(Indic conjunct break) — and for each new scalar looks up `boundclass` and
`indic_conjunct_break` from the property record, then asks two helpers: whether
to break before this scalar, and how to fold it into the running state.
[[src/codegen/builtins/strings/gen_graphemes.rs:lower_strings_graphemes]]

Only `strings::graphemes` materializes the clusters. It runs the machine twice: a
count pass that sizes the result `List OF String`, then a write pass that fills
one entry per cluster.
`strings::graphemesCount` runs the count pass alone and allocates nothing, so it
cannot fail.
[[src/codegen/builtins/strings/gen_graphemes.rs:emit_grapheme_count_pass]]
`strings::graphemeAt` walks to the wanted cluster inside the string's own bytes,
stops there, and copies that one span out as its result; its only allocation is
that result. An index that is negative, or at or past the cluster count, raises
`ErrIndexOutOfRange` from the walk.
[[src/codegen/builtins/strings/gen_graphemes.rs:grapheme_at_window]]

`emit_grapheme_break_branch` is the boundary decision, encoding the UAX #29 rules
in order: GB3 (CR×LF no-break), GB4/GB5 (control boundaries), GB6/GB7/GB8 (Hangul
L/V/T jamo sequences), GB9/GB9a/GB9b (Extend, ZWJ, SpacingMark, Prepend),
GB11 (emoji ZWJ sequence: E_ZWG × Extended_Pictographic), GB12/GB13 (regional
indicator pairing), and GB9c (Indic linker conjunct: a linker state followed by
a consonant does not break). [[src/codegen/string/unicode_props.rs:emit_grapheme_break_branch]]

`emit_grapheme_state_update` advances the state machine after each scalar: it
updates the Indic property state and promotes the boundclass for emoji ZWJ sequences (Extended_Pictographic + Extend
stays pictographic; + ZWJ becomes E_ZWG) and resets regional-indicator pairing.
[[src/codegen/string/unicode_props.rs:emit_grapheme_state_update]]

The Indic state is the GB9c context, not the last scalar's raw property: "none"
(0), "consonant prefix" (CONSONANT, kept through Extend as EXTEND), or "linker
seen after a consonant" (LINKER, kept through Extend and further linkers). Only a
consonant opens the context. A walk seeds it from the first scalar through
`emit_seed_indic_conjunct_state`, which maps anything but a consonant to
"none". A leading linker (`U+094D U+0915`) therefore breaks before the consonant,
two clusters, as on the folded Rust path. Every walk seeds the same way: the
grapheme list, count and index, terminal measurement, and terminal placement's
standalone helpers.
[[src/codegen/string/unicode_props.rs:emit_seed_indic_conjunct_state]]
[[src/codegen/builtins/strings/gen_graphemes.rs:lower_strings_graphemes]]
[[src/codegen/builtins/term/func_measure_text.rs:measure_text_of]]
[[src/codegen/string/unicode_props.rs:emit_grapheme_break_branch_free]]
[[src/codegen/string/unicode_props.rs:emit_grapheme_state_update_free]]
[[src/unicode/backend.rs:graphemes]]

The compiled cluster boundary classes are integer constants matching utf8proc's
`UTF8PROC_BOUNDCLASS_*` numbering (e.g. CR=2, LF=3, CONTROL=4, EXTEND=5, L=6 …
ZWJ=14, EXTENDED_PICTOGRAPHIC=19, E_ZWG=20) and conjunct-break values
(LINKER=1, CONSONANT=2, EXTEND=3). [[src/codegen/string/unicode_props.rs:GRAPHEME_BOUNDCLASS_CR]]
[[src/unicode/runtime_tables.rs:boundclass_value]]

## Runtime algorithm: NFD and NFC normalization

`strings::normalizeNfc` copies all-ASCII input directly. Non-ASCII input uses
decomposition, canonical ordering and recomposition in the stages below. NFD is an intermediate representation; there is no public
`strings::normalizeNfd` member.
[[src/codegen/builtins/strings/func_normalize_nfc.rs:lower]]

1. **Count + allocate.** Decode each scalar, look it up in the NFD mapping table,
   and sum the decomposed lengths (or 1 for scalars with no entry) to size a
   temporary u64-per-scalar buffer. The size arithmetic is checked: an overflow,
   or a failed arena allocation, raises `ErrOutOfMemory`.
   [[src/codegen/builtins/strings/func_normalize_nfc.rs:lower]]
2. **Decompose (fill).** Decode again, expanding each scalar into the temp
   buffer: its NFD sequence if present, else the scalar itself.
3. **Canonical ordering.** A bubble-sort over the temp buffer that swaps adjacent
   scalars when the later one has a non-zero `combining_class` strictly lower
   than the earlier one (the Unicode canonical reordering algorithm), decrementing
   the index after a swap to re-check the prior pair.
4. **Recompose.** A starter-based scan with last-combining-class tracking
   (a virtual register). For each eligible scalar after a starter it first tries
   Hangul composition
   (`emit_hangul_composition_attempt`, the algorithmic LV/LVT formulas, no table),
   then falls back to the combinations tables — gated by the starter's
   `comb_index`/`comb_length` and the current scalar's `COMB_IS_SECOND` flag —
   and on success overwrites the starter in place and drops the combined scalar.
   Composition follows UAX #15 blocking. The remembered blocking class is the
   highest class kept since the starter, and everything kept there is a
   non-starter. A marked candidate is tried only when its class exceeds that
   class. A class-zero candidate is tried only when the remembered class is zero,
   that is when it directly follows the starter: any intervening mark blocks it.
   [[src/codegen/string/unicode_props.rs:emit_hangul_composition_attempt]]
5. **Encode.** Compute the composed byte length, arena-allocate the length-
   prefixed result string, and UTF-8-encode the composed scalars.

NFC composition therefore reads `combining_class`, `comb_index`, `comb_length`,
and the `COMB_IS_SECOND` flag from property records, plus the two combinations
tables and the NFD mapping tables. [[src/codegen/builtins/strings/func_normalize_nfc.rs:lower]]

Runtime and folded normalization agree: `U+1100 U+0301 U+1161` stays
`U+1100 U+0301 U+1161` both ways, because the accent blocks the L+V pair.
[[src/codegen/builtins/strings/func_normalize_nfc.rs:lower_out]]
[[src/unicode/backend.rs:normalize_nfc]]

The ordinary call uses arena blocks. An eligible self-assignment uses the same
`lower_out` algorithm with `StringOut::Scratch`, reserving a combined scalar and
output workspace; the assignment publishes the result into the destination.
The all-ASCII self-assignment needs no normalization rewrite.
[[src/codegen/collection/assign/string_self_update.rs:try_inplace_string_rewrite_assign]]
[[src/codegen/builtins/strings/func_normalize_nfc.rs:lower_out]]

## Runtime algorithm: case folding and upper/lower casing

`strings::upper`, `strings::lower`, and `strings::caseFold` share one routine
parameterized by a `UnicodeCaseMap` enum that selects the entries/sequences
symbols and entry count. The routine makes two passes: a counting pass to size
the output (summing the encoded widths of each scalar's mapped sequence, or the
scalar's own width when there is no mapping entry), then arena-allocates the
result and a writing pass that re-decodes, re-looks-up, and UTF-8-encodes either
the mapped sequence or the original scalar. A zero-length lookup result means
"identity" (no mapping). [[src/codegen/builtins/strings/gen_case_map.rs:lower_strings_case_map]]
[[src/codegen/builtins/strings/gen_strings_support.rs:UnicodeCaseMap]]

The routine first checks for an all-ASCII input and uses a direct byte loop for
that case. In the Unicode path, ASCII scalars also bypass mapping-table lookup.
An eligible self-assignment can use the scratch-output variant of the same
mapping routine.
[[src/codegen/builtins/strings/gen_case_map.rs:lower_strings_case_map_out]]

All three maps are full (sequence-valued), so multi-scalar expansions like
`ß → SS` (uppercase) and Turkish-dotted-I lowering are handled by the flattened
sequences, exactly as the compile-time crate path would produce; the
case-mapping behavior is not locale-tailored. [[src/unicode/runtime_tables.rs:parse_tables]]

The Unicode case-mapping and NFC writing passes check the encoded byte length
against the count pass. The grapheme writer checks both payload bytes and list
entry count. A mismatch triggers a null load. This is an end-of-pass
invariant check, not a bounds check before each write; it cannot undo an
out-of-bounds write that already occurred.
[[src/codegen/error/emission/builder_error_emission.rs:emit_write_cursor_assert]]

## Runtime algorithm: line breaking (UAX #14)

`strings::lineBreaks(text)` implements the Unicode Line Breaking Algorithm, UAX #14
for Unicode 16.0.0, **untailored**. [[src/codegen/builtins/strings/func_line_breaks.rs:LINE_BREAKS]]

Each scalar's class comes from the internal `strings::lineBreakClass`. That is a third
pinned range table, `unicode_linebreak_ranges.txt`, generated from the vendored
`LineBreak`, `EastAsianWidth` and `emoji-data` files at 16.0.0 and the committed
general-category runs. [[src/unicode/range_tables.rs:linebreak]]
[[src/codegen/builtins/strings/func_line_break_class.rs:lower]] It is the same
bisection as the category table's, but each run's second word is the value itself, so
there is no names table: `(u32 lastCodepoint, u32 value)`, emitted only when a function
reads it. [[src/codegen/string/unicode_props.rs:emit_unicode_range_value]]

The value is the class after LB1's default resolutions — AI, SG and XX are AL, SA is
CM for Mn or Mc and AL otherwise, CJ is NS — numbered in the generator's order, plus
64 for East_Asian_Width F, W or H (LB19a, LB21a, LB30), 128 for Extended_Pictographic
on an unassigned scalar (LB30b), and 256 or 512 for a QU that is Pi or Pf (LB15a,
LB15b, LB19). [[tools/unicode-tables/gen_linebreak_table.py:CLASSES]] CB is kept and
broken around (LB20). Class SA therefore breaks only at spaces and punctuation: no
dictionary segmentation is done for Thai, Lao, Khmer or Myanmar.

LB9 and LB10 are applied first, by cutting the text into units: a base and the CM and
ZWJ scalars it absorbs (any base but BK, CR, LF, NL, SP or ZW), or a lone CM or ZWJ
treated as U+0041. Every later rule reads units, so its look-behinds — `SP*`,
`(SY | IS)*`, regional-indicator parity — skip combining marks. Positions inside a unit
never break. A unit that ends in ZWJ forbids a break after it (LB8a). Each position
between units then runs LB4 to LB31 in order, and the first rule that matches decides
it.

The result lists each break as `(index, mandatory)`, in Unicode scalars. Position 0 is
never reported (LB2). The end of the text is always reported, as mandatory (LB3).
`mandatory` is set after BK, CR (not before its LF), LF and NL. An empty string has no
breaks. `tests/runtime/rt_strings_line_breaks.rs` runs all 16,672 cases of the vendored
`LineBreakTest-16.0.0.txt` in one program and requires every one to agree.

## See Also

- `./mfb spec unicode strings-model`
- `./mfb spec language types`
- `./mfb spec memory heap-values`
- `./mfb spec linker static-and-dynamic-output`
- `mfb man strings` (per-function `strings::` API)
