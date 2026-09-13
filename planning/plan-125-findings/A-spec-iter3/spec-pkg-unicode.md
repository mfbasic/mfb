### 1. Grapheme “backing” contradicts the native-runtime contract
UNIT:      spec-pkg:unicode
TOPICS:    tables-and-algorithms; strings-model
CATEGORY:  divergence
QUOTE-A:   “MFBASIC's `strings::` package performs Unicode-correct grapheme segmentation, normalization, and case mapping at runtime, in the compiled native binary, with no external library dependency.” — `src/docs/spec/unicode/01_tables-and-algorithms.md:3-4`
QUOTE-B:   “| Grapheme | One user-perceived character (extended grapheme cluster) | `unicode-segmentation` | `graphemes`, `graphemesCount`, `graphemeAt` |” — `src/docs/spec/unicode/02_strings-model.md:18`
VERDICT:   `tables-and-algorithms` owns runtime implementation. Its cited `lower_strings_graphemes` emits the segmentation state machine; `unicode-segmentation` is used by the compile-time folding path, not by the generated program. The indexing table currently presents the host crate as the runtime backing.
SUGGESTED: Replace the Grapheme backing cell with: “Emitted UAX #29 state machine and embedded property tables; `unicode-segmentation` is compile-time folding only — see `mfb spec unicode tables-and-algorithms`.”

### 2. The API-contract ownership boundary is stated incompatibly
UNIT:      spec-pkg:unicode
TOPICS:    tables-and-algorithms; strings-model
CATEGORY:  lost-ownership
QUOTE-A:   “Per-function `strings::` API contracts are owned by `mfb man`.” — `src/docs/spec/unicode/01_tables-and-algorithms.md:32-34`
QUOTE-B:   “The per-function `strings::` API (arguments, return types, error codes) is owned by `./mfb man strings`; this topic specifies only the *indexing model and slice semantics* a faithful reimplementation must reproduce.” — `src/docs/spec/unicode/02_strings-model.md:9-11`
VERDICT:   `strings-model` gives the necessary boundary: man owns callable API presentation, while this package owns implementation-relevant semantics. The broader wording in `tables-and-algorithms` falsely implies that its own algorithm contracts are outside the spec’s ownership.
SUGGESTED: Replace the first wording with: “`mfb man strings` owns callable API presentation; this package owns the runtime model and semantic contracts a faithful reimplementation must reproduce.”