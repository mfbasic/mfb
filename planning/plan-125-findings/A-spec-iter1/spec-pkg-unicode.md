### 1. Dead constant-fold provenance
UNIT:      spec-pkg:unicode
TOPIC:     01_tables-and-algorithms.md
CATEGORY:  stale-by-deletion
CLAIM:     “When a Unicode-aware call has a statically known string argument, the compiler evaluates it in-process using the Rust crates `unicode_segmentation`, `unicode_normalization`, and `unicode_casefold`, and the result is baked in as a literal.”
VERDICT:   misleading
EVIDENCE:  `./scripts/spec-census.sh --citations unicode` reports `MISS-SYMBOL ... gen_strings_support.rs:static_strings_package_string ELSEWHERE=comment-only STALE-BY-DELETION`; `src/codegen/engine/types/type_utils.rs:strings_package_static_string_value` and `src/target/shared/nir/constfold.rs:native_strings_package_static_string_value` now implement the upper/lower/caseFold/NFC folds, while `src/codegen/builtins/strings/func_graphemes.rs:lower` implements the grapheme fold.
SUGGESTED: Replace the deleted citation with `[[src/codegen/engine/types/type_utils.rs:strings_package_static_string_value]]`, `[[src/target/shared/nir/constfold.rs:native_strings_package_static_string_value]]`, and `[[src/codegen/builtins/strings/func_graphemes.rs:lower]]`.

### 2. Unicode-version split is undocumented
UNIT:      spec-pkg:unicode
TOPIC:     package-wide
CATEGORY:  missing-contract
CLAIM:     No Unicode topic states that general-category and Script answers are pinned to Unicode 16.0.0 in separate range tables, while grapheme, normalization, case, and width properties use the newer vendored utf8proc data.
VERDICT:   missing
EVIDENCE:  `src/unicode/range_tables.rs:gencat` and `script` define Unicode 16.0.0 tables; its module documentation states the utf8proc UCD is newer and differs on 4,804 scalars. `src/codegen/builtins/strings/func_gen_cat.rs:lower` and `src/codegen/builtins/regex/func_gen_cat.rs:lower` consume those tables. `mfb spec unicode --all` contains no range-table or Unicode-16 contract.
SUGGESTED: Add a canonical “Pinned category and Script tables” section to `tables-and-algorithms`: “`strings` scalar classifiers and `regex` general-category/Script properties use separate Unicode 16.0.0 range tables; they must not be derived from the utf8proc property trie, whose newer UCD intentionally differs.” Cite `[[src/unicode/range_tables.rs:gencat]]`, `[[src/unicode/range_tables.rs:script]]`, and `[[src/codegen/builtins/strings/func_gen_cat.rs:lower]]`; make `stdlib regex` link here rather than repeat the table implementation.

### 3. Lone zero-width grapheme has contradictory width
UNIT:      spec-pkg:unicode
TOPIC:     package-wide
CATEGORY:  contradiction
CLAIM:     `tables-and-algorithms` says “0/1 are one column (a lone zero-width scalar still occupies a cell)”; `strings-model` says “a zero-width scalar ... contributes 0” but then says “a grapheme is 1 or 2 columns.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/strings/func_display_width.rs:lower` initializes each cluster width to its first scalar, replaces zero only with a later scalar, and sums it; its descriptor explicitly specifies `0` for a cluster consisting only of zero-width scalars. Thus a lone combining mark, ZWSP, or ZWJ has width 0.
SUGGESTED: Make `strings-model` canonical: “A grapheme contributes 0, 1, or 2 columns: its width is its first non-zero-width scalar’s width, or 0 if none exists.” Replace the conflicting table-topic sentence with a short link to `./mfb spec unicode strings-model`. Cite `[[src/codegen/builtins/strings/func_display_width.rs:lower]]`.

### 4. Dependency order is backwards
UNIT:      spec-pkg:unicode
TOPIC:     spec.md
CATEGORY:  reading-order
CLAIM:     The overview orders `tables-and-algorithms` before `strings-model`.
VERDICT:   misleading
EVIDENCE:  `src/docs/spec/unicode/spec.md` lists tables first, but `01_tables-and-algorithms.md` immediately depends on scalars, graphemes, UTF-8 strings, and display-width terms defined by `02_strings-model.md`.
SUGGESTED: Order `strings-model` first, then `tables-and-algorithms`: “the scalar/grapheme/byte and display-column model” before “the tables and runtime algorithms that implement it.”

### 5. Contract status of emitted-table details is unstated
UNIT:      spec-pkg:unicode
TOPIC:     01_tables-and-algorithms.md
CATEGORY:  guarantee-unstated
CLAIM:     “Emission is **per-table**, driven by the relocations the generated code actually carries: a table is emitted iff some function relocates against its `_mfb_unicode_*` symbol.”
VERDICT:   misleading
EVIDENCE:  `src/codegen/memory/data/data_objects.rs:unicode_runtime_data_objects` implements this current relocation-based selection, including a fallback that emits every table; the topic never says whether this output-minimization behavior is a required generated-artifact contract or an implementation optimization.
SUGGESTED: Label it explicitly: “Implementation detail, not a language or binary-format guarantee: the current emitter selects tables by relocation and may change this selection without changing Unicode results.” Cite `[[src/codegen/memory/data/data_objects.rs:unicode_runtime_data_objects]]`.