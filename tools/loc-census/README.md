# loc-census

The independent oracle for `tests/guards/file_size_budget.rs` (plan-176). It counts the
non-comment, non-blank lines of every `.rs` and `.mfb` file under `src/`, `packages/`,
`examples/`, `repository/` and `tests/`, split into code lines and test lines. It is a second,
separate implementation of the guard's counting rules: the two must agree on every file, and
a disagreement is a bug in one of them.

It is not a gate. The guard is what runs in `cargo test`; this script is for cross-checking
the guard, seeding `tests/guards/file_size_ceilings.txt`, and measuring a part of a file
before and after a split. Python 3, stdlib only.

## Usage

```
python3 -I tools/loc-census/census.py [--root DIR] [--json OUT]   # per-file JSON
python3 -I tools/loc-census/census.py --seed                      # ceiling rows
python3 -I tools/loc-census/census.py --file PATH                 # "code test" for one file
python3 -I tools/loc-census/census.py --file PATH --lines A B     # "code test" for lines A..B
```

- `--root` defaults to the repository that holds the script. `--file PATH` is relative to the
  root (or absolute).
- The JSON has one object per file: `path`, `code`, `test`, `gen` (generated), `testfile`
  (test-only file).
- `--seed` prints `ceiling <code|inline|test> <n> <path>` for every non-generated (file,
  kind) over its limit, sorted by path then kind: the body of `file_size_ceilings.txt`.
- `--lines A B` (1-based, inclusive) counts the lines of a range, with the whole-file
  context applied (a range inside a block comment or a `#[cfg(test)]` item is counted as
  the full file counts it).

## Counting rules

| Language | Not LOC | Test LOC |
|---|---|---|
| Rust `.rs` | blank lines; `//`, `///`, `//!` to end of line; `/* … */` including nested, `/** */` and `/*! */`. Comment markers inside string, raw-string (`r"…"`, `r#"…"#`, `br…`, `cr…`), byte-string and char literals are not comments. | the whole file if a directory component is `tests`, it is a module declared under `#[cfg(test)]` (`mod x;` or `#[path = "…"] mod x;`, and that module's subtree), or a line begins with `#![cfg(test)]`. Otherwise, the lines of every item under a `#[cfg(test)]` / `#[cfg(all(test, …))]` attribute. |
| MFBASIC `.mfb` | blank lines; lines whose first token is `'`; lines whose first token is `REM` followed by whitespace or end of line; every line from a statement-leading `DOC` line (optionally followed by uppercase attribute words) through its `END DOC`. | the whole file if a directory component is `tests` or the file name starts with `test_`. Otherwise, the lines from `TESTING` through `END TESTING`. |

**Extent of a `#[cfg(test)]` item**, scanned over the code text (comments removed, literals
blanked), with `d` = the bracket depth at the attribute (`(`, `[`, `{` counted together). The
item ends at the first of: a `;` or `,` at depth `d`; a `}` that returns to depth `d` after
the item's `{` body opened; a closer that drops below `d` (a line starting with that closer
belongs to the enclosing construct and is not test code). Every line from the attribute
through the ending line is test code.

**Kinds and limits.** A test-only file has one kind, `test` (limit 1000). Any other file has
`code` (limit 1000) and `inline` (its test lines, limit 250).

**Generated files** are exempt: one of the first three lines is a comment (`//`, `//!`, `/*`,
`'`, `REM`) carrying the uppercase word `GENERATED`.

Directories named `target` or `.git` are skipped; symlinked directories are not entered.
