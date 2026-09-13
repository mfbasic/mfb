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

    python3 tools/unicode-tables/<generator> > <artifact>
