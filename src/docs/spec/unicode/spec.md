# Unicode Runtime Model

The Unicode data and algorithms behind MFBASIC's text operations: what the
compiler folds at compile time, and the tables and routines it embeds into a
program whose generated code performs a Unicode-aware operation at run time
(only the tables that code actually references — see `tables-and-algorithms`). This is the model behind the `strings::`
package: how a `String` is indexed (scalars vs graphemes vs bytes), and the
grapheme-segmentation, normalization, and case-mapping algorithms that operate on
it, plus the embedded property tables they consult.

The per-function `strings::` API is documented by `./mfb man strings`; this
package specifies the underlying *model and algorithms* a faithful reimplementation
must reproduce — the part `man` does not capture.

## Reading order

- `tables-and-algorithms` — the embedded (utf8proc-derived) property tables, their
  two-stage lookup, and the runtime grapheme-segmentation, NFC/NFD normalization,
  case-fold, and upper/lower algorithms.
- `strings-model` — the scalar / grapheme / byte indexing model, scalar-boundary
  rules, and the `mid`/`find`/`trim`/`split` semantics shared by `strings::`.

## See Also

* ./mfb man strings — the per-function `strings::` API reference
* ./mfb spec language types — the `String` type and comparison/ordering rules
* ./mfb spec memory heap-values — the in-memory `String` byte layout
