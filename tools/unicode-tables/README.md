# tools/unicode-tables

Generators for the pinned Unicode tables compiled into emitted programs. Each is
re-run by `scripts/check-generated.sh` (CI), which fails if the committed artifact no
longer matches, so never hand-edit an artifact: change the generator and re-run it.

- **gen_unicode_gencat_table.py** → `src/codegen/string/unicode/unicode_gencat_ranges.txt`:
  general-category runs behind `regex::genCat` / `strings::genCat`. Reads the
  interpreter's `unicodedata`, so it must run under **Python 3.14** (Unicode 16.0.0).
- **gen_regex_scripts.py** → `src/codegen/string/unicode/unicode_script_names.mfb`:
  canonical script names, from the vendored `third_party/unicode/Scripts-16.0.0.txt`
  (any Python 3 reproduces it).
- **gen_unicode_script_table.py** → `src/codegen/string/unicode/unicode_script_ranges.txt`:
  Script-property runs behind `\p{Script=…}`. It imports `gen_regex_scripts.runs()`
  from this directory, so the two tables cannot disagree; keep the two files together.
- **gen_linebreak_table.py** → `src/codegen/string/unicode/unicode_linebreak_ranges.txt`:
  UAX #14 Line_Break classes behind `strings::lineBreakClass` (plan-162-E), with LB1's
  default resolutions applied and the East-Asian-width, Extended_Pictographic-unassigned
  and Pi/Pf flags the rules need. Reads only vendored files (`third_party/unicode/`
  LineBreak, EastAsianWidth and emoji-data at 16.0.0) and the committed general-category
  artifact, so any Python 3 reproduces it.

- **gen_casefold_table.py** → `src/unicode/casefold_cf.txt`: Unicode full case folding
  (the C and F mappings) behind `strings::caseFold` and regex `(?i)`, read by both the
  compile-time fold and the runtime casefold tables through `src/unicode/casefold.rs`.
  Reads only the vendored `third_party/unicode/CaseFolding-16.0.0.txt`, so any Python 3
  reproduces it.

    python3 tools/unicode-tables/<generator> > <artifact>
