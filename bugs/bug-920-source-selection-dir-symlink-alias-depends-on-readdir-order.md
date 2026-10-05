# bug-920: Source selection with a directory-symlink alias depends on `read_dir` order (files silently dropped or renamed)

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Coordinator note: repro measured by the review agent only (HEAD 28a15a153 build); the coordinator did not re-run it.
Regression Test: tests/cli/source-selection-symlink-alias-order (new; name suggested)

A source root can contain a symlink to one of its own subdirectories, for example
`src/f -> c` beside the real `src/c/`. The source walker visits whichever of `c`
and `f` the OS lists first, records the canonical directory as visited, and skips
the other. The file's root-relative path, which include/exclude patterns are
matched against and which is used as its display path, then depends on
filesystem iteration order:

- With `include: ["main.mfb", "c/*.mfb"]`, if the alias is listed first the file is tested as `f/d.mfb`, fails the pattern, and is **silently not selected**. The build then fails with an unrelated `SYMBOL_UNKNOWN_IDENTIFIER`. If `c` is listed first, the same tree builds.
- With the default `**/*.mfb`, the file is selected either way, but its display path (in every diagnostic, and the sort key that decides `files[0]`) is `src/f/d.mfb` or `src/c/d.mfb` depending on order.

The spec promises the opposite: "the same manifest over the same tree must select
the same files in the same order", and "determinism comes only from the per-entry
and final display-path sorts, never from walk order". APFS orders entries by
hash, so renaming the symlink alone flips the result. ext4/NTFS use different
orders, so one checkout can build on one OS and not on another.

**Correct behavior:** the selected set and the display paths do not depend on
`read_dir` order. A fix might sort directory entries before recursing (so the
lexicographically first alias always wins), or skip directory symlinks that
resolve to a directory already inside the walked root, or test patterns against
every alias. Which one is chosen is an owner decision.

References:

- `src/docs/spec/tooling/02_source-selection.md` intro and *Recursive Walk* ("`read_dir` iteration order is OS-dependent; determinism comes only from the per-entry and final display-path sorts, never from walk order").
- Found by hardening batch B13 (tooling source-selection).

## Failing Reproduction

```
for L in b f; do
  rm -rf /tmp/s4 && mkdir -p /tmp/s4/src/c && cd /tmp/s4
  printf '{"name":"s4","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","include":["main.mfb","c/*.mfb"]}]}' > project.json
  printf 'IMPORT io\n\nFUNC main() AS Integer\n  io::print("dot=" & toString(dot()))\n  RETURN 0\nEND FUNC\n' > src/main.mfb
  printf 'FUNC dot() AS Integer\n  RETURN 4\nEND FUNC\n' > src/c/d.mfb
  ln -s c src/$L
  echo "link=$L readdir: $(ls -f src | tr '\n' ' ')"; mfb build 2>&1 | tail -1
done
```

- Observed (macOS APFS, HEAD build):
  ```
  link=b readdir: . .. main.mfb c b
  Wrote executable to ./build/s4.out
  link=f readdir: . .. f main.mfb c
                 Callable `dot` is not a top-level function.
  ```
- With the default include and a type error in `d.mfb`, the error is reported as
  `./src/c/d.mfb:2` for `link=b` and as `./src/f/d.mfb:2` for `link=f`.
- Expected: both trees select `src/c/d.mfb` (or both reject it), with identical diagnostics.

Contrast: without the symlink both trees build. A symlink to a directory outside
the project is rejected deterministically (`MFB_SOURCE_OUTSIDE_PROJECT`). A
symlink cycle (`src/a/cycle -> ..`) terminates correctly.

| link name | read_dir position | Result |
| --- | --- | --- |
| b, d, e, q | after `c` | selected ✓ |
| f, g, m, x1 | before `c` | silently dropped ✗ |

## Root Cause

`src/ast/manifest.rs:collect_mfb_files` dedupes **directories** through
`visited_dirs` (a set of canonical paths) and returns as soon as a canonical
directory is seen again. It then computes `relative_path` from the *logical*
path it walked (`normalized_relative_path(logical_root, &path)`). Which logical
spelling reaches a canonical directory first is decided by `fs::read_dir`
iteration order, which is never sorted.

## Goal

- Same tree gives the same selection and display paths on every filesystem, and the test above passes for every link name.

### Non-goals (must NOT change)

- Cycle termination, outside-project rejection, and file-alias `MFB_SOURCE_OVERLAP` detection.

## Blast Radius

- `src/ast/manifest.rs:collect_mfb_files`: fixed by this bug.
- `src/cli/build/resources.rs:copy_resources` (resources glob walk): latent, same pattern worth checking (not exercised).

## Fix Design

The smallest deterministic fix is to collect `read_dir` entries into a `Vec` and
sort by file name before iterating. The lexicographically first alias then always
claims the directory. The spec's "never from walk order" sentence must then say
the walk *is* sorted. Rejected alternative: leave the order unsorted and document
it. That breaks the cross-platform reproducibility contract.

## Validation Plan

- Regression test: the two-link-name loop above as a CLI test, which must agree for both names.
- Doc sync: `02_source-selection.md` *Recursive Walk* (state the sort, or the alias rule).

## Open Decisions

- Which alias wins: lexicographic-first (sort), real-directory-first (skip symlinked dirs that resolve inside the root), or reject in-root directory aliases with a diagnostic.
