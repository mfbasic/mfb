### 1. Terminal width disagrees for all-zero-width graphemes
UNIT:      spec-file:unicode/02_strings-model.md
LINE:      38
CATEGORY:  wrong-claim
CLAIM:     "The `term::` backends lay one grapheme per cell at this width; a wide grapheme reserves a trailing cell and wraps at the right edge."
EVIDENCE:  `src/codegen/term/grid/term_grid.rs:emit_grid_write` explicitly converts an all-zero-width cluster to width 1 before stamping; `src/codegen/term/core/term.rs:emit_draw_text` does the same. Thus a lone combining mark has `strings::displayWidth` 0 but occupies one terminal cell.
SUGGESTED: "The `term::` backends reserve a trailing cell for a wide grapheme and wrap it at the right edge. For terminal placement only, an all-zero-width grapheme is promoted to one cell. [[src/codegen/term/grid/term_grid.rs:emit_grid_write]]"

### 2. Whitespace can begin an extended grapheme cluster
UNIT:      spec-file:unicode/02_strings-model.md
LINE:      159
CATEGORY:  wrong-claim
CLAIM:     "Trimming operates scalar by scalar from the end(s); it is not grapheme-aware (it cannot strip a whitespace scalar buried inside a cluster, but no standard cluster begins with a `White_Space` scalar)."
EVIDENCE:  `src/codegen/builtins/strings/gen_trim.rs:lower_strings_trim` scans and removes scalars independently. The probe at `/tmp/plan-125-scratch/A-spec-iter2/spec-file-unicode-02_strings-model.md` printed `1` for `len(strings::trim(" \u{301}"))`: the leading U+0020 is removed while its following combining mark remains. That pair is an extended grapheme cluster.
SUGGESTED: "Trimming operates scalar by scalar from the end(s), not by grapheme cluster; removing a whitespace scalar can leave a following combining scalar behind. [[src/codegen/builtins/strings/gen_trim.rs:lower_strings_trim]]"

### 3. `join` can fail
UNIT:      spec-file:unicode/02_strings-model.md
LINE:      211
CATEGORY:  wrong-claim
CLAIM:     "The inverse `join(parts, delimiter)` concatenates with the delimiter between parts and never errors."
EVIDENCE:  `src/codegen/builtins/strings/func_join.rs:lower` checks each output-size addition and the allocation-header addition, then raises `ErrOutOfMemory` on either allocation failure or overflow.
SUGGESTED: "The inverse `join(parts, delimiter)` concatenates with the delimiter between parts; it accepts an empty delimiter, but can raise `ErrOutOfMemory` if constructing the result cannot be allocated. [[src/codegen/builtins/strings/func_join.rs:lower]]"

### 4. Global string immutability has no auditable provenance
UNIT:      spec-file:unicode/02_strings-model.md
LINE:      4
CATEGORY:  uncited-uncheckable
CLAIM:     "A `String` is an immutable, UTF-8-encoded byte sequence; the runtime never mutates it in place."
EVIDENCE:  This is a global runtime/value-model assertion, but the paragraph has no `[[path:Symbol]]` citation. The nearby `src/unicode/backend.rs:graphemes` citation concerns segmentation only and cannot establish immutability across all String-producing and String-consuming paths.
SUGGESTED: "A `String` is an immutable UTF-8 byte sequence." Add a citation to the authoritative String value-model implementation, or reduce this topic to its indexing contract and link the owning memory-model topic.

### 5. “Never constant-folded” is uncited
UNIT:      spec-file:unicode/02_strings-model.md
LINE:      104
CATEGORY:  uncited-uncheckable
CLAIM:     "`mid` (and `find`) are **never constant-folded**: a call with static, out-of-range arguments still compiles and raises the catchable runtime error, never a build error."
EVIDENCE:  No citation accompanies this compiler-pipeline claim. `src/codegen/memory/value/builder_value_semantics.rs:static_string_value` enumerates the String calls eligible for this fold and excludes `strings.mid` and `strings.find`, but the spec does not point readers there.
SUGGESTED: "`mid` and `find` are not among the String calls folded by `static_string_value`; static invalid arguments therefore reach their runtime error path. [[src/codegen/memory/value/builder_value_semantics.rs:static_string_value]]"

### 6. The asserted performance bottleneck is unmeasured
UNIT:      spec-file:unicode/02_strings-model.md
LINE:      59
CATEGORY:  uncited-uncheckable
CLAIM:     "Mapping is a scan that counts non-continuation bytes (`byte & 0xC0 != 0x80`), the dominant cost in `mid`/`find`, each direction being O(n) in the bytes scanned."
EVIDENCE:  `src/codegen/collection/search/builder_search.rs:lower_mid` and `lower_find` implement scans, but neither establishes “the dominant cost”; both also contain ASCII fast-forward paths. No benchmark or measurement citation supports that performance assertion.
SUGGESTED: "Scalar-to-byte mapping scans UTF-8 scalar boundaries and is linear in the bytes traversed. [[src/codegen/collection/search/builder_search.rs:lower_mid]] [[src/codegen/collection/search/builder_search.rs:lower_find]]"