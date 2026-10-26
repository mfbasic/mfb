### 1. "one past" understates the upper-bound error condition
UNIT:      man-page:strings/find
CLAIM:     "A negative `start`, or one past the scalar length, raises `ErrIndexOutOfRange`."
VERDICT:   misleading
EVIDENCE:  Any `start` greater than the scalar length raises, not only length+1. Probe `far.mfb` built via `mfb build -q p` from `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-find`: `strings::find("abc", "", 10)` printed `ERR 77050001`. `edge.mfb` also printed `start==len+1 -> ERR 77050001` and `negative start -> ERR 77050001`. The lowering confirms it: `lower_find` in `src/codegen/collection/search/builder_search/mod.rs` branches to `find_invalid_start` whenever `remaining` hits 0 before `scalar_index == start`, for any larger `start`.
SUGGESTED: "A negative `start`, or one greater than the scalar length, raises `ErrIndexOutOfRange`."

### 2. Third example's caption promises a `contains` guard the code does not show
UNIT:      man-page:strings/find
CLAIM:     "Guard with `contains`, or catch the absence with `TRAP`:"
VERDICT:   incomplete
EVIDENCE:  The example under that caption contains only the `TRAP` form (`ex3.mfb`, built and run: prints `absent`, exit 0). No `strings::contains` call appears anywhere in the three examples, although the description recommends it twice as the primary absence-handling pattern and warns about its interaction with `start`.
SUGGESTED: Either retitle the block "Catch the absence with `TRAP`:" or add the guarded form before the `TRAP` form, e.g.
```
IF strings::contains("hello", "z") THEN
  io::print(toString(strings::find("hello", "z")))
ELSE
  io::print("absent")
END IF
```

### 3. `astrings::AttributedString` is not the name a developer can type
UNIT:      man-page:strings/find
CLAIM:     "`value` may also be an `astrings::AttributedString`: the query runs on its visible text and returns exactly what the `String` overload returns (same value, type, and errors)."
VERDICT:   misleading
EVIDENCE:  The behavioral half is true: `astr.mfb` with `LET a AS AttributedString = astrings::fromString("hello")` printed `3` for `strings::find(a, "l", 3)` and `ERR 77050004` for `strings::find(a, "z")`. But declaring the variable with the page's spelling, `LET a AS astrings::AttributedString`, fails to compile with `SYMBOL_UNKNOWN_IDENTIFIER: Built-in package astrings does not export astrings.AttributedString`. The derived See-also entry `astrings::AttributedString` likewise does not render: `mfb man astrings AttributedString` prints `unknown astrings function AttributedString`. The `mfb man astrings` page itself writes the type bare (`AttributedString`) in its code and qualified in prose, so this is a shared convention rather than a page-local slip, but a reader of this page alone will copy the qualified form into a declaration and hit a compile error.
SUGGESTED: "`value` may also be an `AttributedString` (see `mfb man astrings`): the query runs on its visible text and returns exactly what the `String` overload returns (same value, type, and errors)." The See-also row is derived, so its non-resolving target is a descriptor or renderer issue for the main thread, not a prose edit.

Everything else on the page was verified against the release binary and found true. Probe summary, all built from the scratch project under `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-find`:

- `ex1.mfb` printed `2` then `3`; `ex2.mfb` printed `1` then `2`; `ex3.mfb` printed `absent`. All three page examples compile and match the prose.
- `edge.mfb`: `start==len` with empty needle returned `3`; `start==len` with `"c"` raised 77050004; empty value with empty needle returned `0`; empty needle at `start` 1 returned `1`; `"abcabc"`, `"a"`, `5` raised 77050004 while `strings::contains("abcabc", "a")` printed `TRUE`; `"Hello"`, `"h"` raised 77050004 (no case folding); `"😀😀a"`, `"a"`, `2` returned `2`; needle longer than value raised 77050004.
- `nfd.mfb`: haystack with precomposed é and needle `e` + U+0301 raised 77050004, confirming no normalization.
- Cross-page claim that `collections::find`, `findIndex`, `findLastIndex` and `regex::find` all raise `ErrNotFound`: each page's rendered text mentions `ErrNotFound` (counts 4, 4, 3, 2 via `grep -c`), and `regex find` states it never returns -1.
- Package empty-needle rule quoted from `mfb man strings` ("A member that answers a question about an occurrence reports it") agrees with the page.
- No banned memory vocabulary and no compiler-internal terms appear on the rendered page; "exact byte comparison" is a developer-facing normalization fact, not an implementation model.
