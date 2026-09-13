### 1. Static-only calls do not embed tables
UNIT:      spec-file:unicode/01_tables-and-algorithms.md
LINE:      6
CATEGORY:  wrong-claim
CLAIM:     "read-only Unicode property and mapping tables into every program that calls a Unicode-aware `strings::` builtin"
EVIDENCE:  The static `strings::upper("ß")` probe built with `mfb build --nobj` has no `_mfb_unicode_*` object, because static folding produces a literal in `src/codegen/engine/types/type_utils.rs:strings_package_static_string_value`; `src/codegen/builtins/strings/func_graphemes.rs:lower` similarly lowers static graphemes to a list literal.
SUGGESTED: "For dynamic Unicode-aware calls, the compiler embeds the read-only property and mapping tables whose generated code relocates against them. [[src/codegen/memory/data/data_objects.rs:unicode_runtime_data_objects]]"

### 2. Runtime mapping tables are not derived from utf8proc
UNIT:      spec-file:unicode/01_tables-and-algorithms.md
LINE:      25
CATEGORY:  wrong-claim
CLAIM:     "The tables are derived from utf8proc, not from those Rust crates — except general category and Script, which come from separate pinned tables (next section)."
EVIDENCE:  `src/unicode/runtime_tables.rs:parse_tables` builds NFD, uppercase, lowercase, and casefold entries with `build_mapping_tables`; `build_mapping_tables` invokes Rust normalization/case closures, rather than parsing utf8proc.
SUGGESTED: "The property and composition tables are derived from utf8proc; NFD and case-mapping tables are generated through Rust normalization/case crates, while general category and Script come from separate pinned tables. [[src/unicode/runtime_tables.rs:parse_tables]]"

### 3. Unicode-version attribution is wrong for normalization and case mapping
UNIT:      spec-file:unicode/01_tables-and-algorithms.md
LINE:      59
CATEGORY:  wrong-claim
CLAIM:     "So grapheme segmentation, normalization, case mapping and display width follow utf8proc's Unicode version, while category and Script follow 16.0.0, and the two can disagree about a scalar assigned after 16.0.0."
EVIDENCE:  `src/unicode/runtime_tables.rs:parse_tables` creates NFD/upper/lower/casefold mappings through `unicode_normalization`, Rust `char` casing, and `unicode_casefold`; only the trie properties and composition tables are parsed from utf8proc.
SUGGESTED: "Grapheme segmentation and display width read utf8proc-derived properties; normalization and case mapping read tables generated through the Rust normalization/case crates, while category and Script follow Unicode 16.0.0. [[src/unicode/runtime_tables.rs:parse_tables]]"

### 4. Parser no longer maps category or decomposition constants
UNIT:      spec-file:unicode/01_tables-and-algorithms.md
LINE:      74
CATEGORY:  wrong-claim
CLAIM:     "Symbolic constants (`UTF8PROC_CATEGORY_*`, `UTF8PROC_BOUNDCLASS_*`, `UTF8PROC_DECOMP_TYPE_*`, `UTF8PROC_INDIC_CONJUNCT_BREAK_*`, `UINT16_MAX`, `true`/`false`) are mapped to integers by hand-written match tables; bidi-class references collapse to `0` (unused)."
EVIDENCE:  `src/unicode/runtime_tables.rs:parse_value` maps only `UINT16_MAX`, booleans, bidi classes, boundclasses, and Indic-conjunct values. Its adjacent comment states category and decomposition fields are never consumed and their former match tables were removed.
SUGGESTED: "The parser maps `UTF8PROC_BOUNDCLASS_*`, `UTF8PROC_INDIC_CONJUNCT_BREAK_*`, `UINT16_MAX`, and booleans to integers; bidi-class references collapse to `0`, while category and decomposition fields are not consumed. [[src/unicode/runtime_tables.rs:parse_value]]"

### 5. Emission count omits four pinned-table objects
UNIT:      spec-file:unicode/01_tables-and-algorithms.md
LINE:      218
CATEGORY:  wrong-claim
CLAIM:     "A compiler pass emits the thirteen tables as raw, read-only `CodeDataObject`s."
EVIDENCE:  `src/codegen/memory/data/data_objects.rs:unicode_runtime_data_objects` emits the thirteen utf8proc/mapping objects plus general-category ranges/names and Script ranges/names; its `unicode_runtime_data_objects_emit_only_referenced_tables` test asserts `all.len() == 17`.
SUGGESTED: "The compiler pass can emit seventeen raw, read-only `CodeDataObject`s: thirteen utf8proc/mapping tables plus four pinned general-category and Script range/name tables. [[src/codegen/memory/data/data_objects.rs:unicode_runtime_data_objects]]"

### 6. Record alignment summary contradicts the properties object
UNIT:      spec-file:unicode/01_tables-and-algorithms.md
LINE:      233
CATEGORY:  wrong-claim
CLAIM:     "sizes and alignments are fixed per table (u16 tables align 2, u32 / record tables align 4)."
EVIDENCE:  `src/codegen/memory/data/data_objects.rs:unicode_runtime_data_objects` emits `UNICODE_PROPERTIES_SYMBOL`—a 12-byte record table—with alignment `2`; the table immediately below correctly says `properties` aligns to 2.
SUGGESTED: "Sizes and alignments are fixed per table: u16 tables and the 12-byte `properties` records align 2; u32 tables and 16-byte mapping-entry records align 4. [[src/codegen/memory/data/data_objects.rs:unicode_runtime_data_objects]]"

### 7. Uppercase sharp-s example has the wrong result
UNIT:      spec-file:unicode/01_tables-and-algorithms.md
LINE:      346
CATEGORY:  wrong-claim
CLAIM:     "`ß → ss` (uppercase)"
EVIDENCE:  The probe at `/tmp/plan-125-scratch/A-spec-iter2/spec-file-unicode-01_tables-and-algorithms.md/upper_sharp_s.mfb`, built and run with the supplied release `mfb`, prints `SS`; `src/unicode/backend.rs:upper` uses `char::to_uppercase`.
SUGGESTED: "`ß → SS` (uppercase) and Turkish-dotted-I lowering are handled by the flattened sequences. [[src/unicode/runtime_tables.rs:parse_tables]]"

### 8. Allocation/overflow failure behavior is absent from the normalization algorithm
UNIT:      spec-file:unicode/01_tables-and-algorithms.md
LINE:      309
CATEGORY:  missing-failure-mode
CLAIM:     "**Count + allocate.** Decode each scalar, look it up in the NFD mapping table, and sum the decomposed lengths (or 1 for scalars with no entry) to size a temporary u64-per-scalar buffer."
EVIDENCE:  `src/codegen/builtins/strings/func_normalize_nfc.rs:lower` routes both checked `scalar_count * 8` overflow and failed arena allocation to `raise_error_bare("ErrOutOfMemory")`; this observable failure contract is not stated.
SUGGESTED: "The temporary-buffer size multiplication is checked; size overflow or arena allocation failure raises `ErrOutOfMemory`. [[src/codegen/builtins/strings/func_normalize_nfc.rs:lower]]"