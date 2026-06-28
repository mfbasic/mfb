# Source Selection

How the manifest's `sources[]` array turns into the ordered set of `.mfb` files
the compiler parses. This is the contract a reimplementation, an IDE indexer, or
a build cache keys on: the same manifest over the same tree must select the same
files in the same order, or reject the tree with a fixed diagnostic.

The `project.json` schema for `sources[]` itself is `./mfb spec tooling
project-manifest`; this topic owns only the *selection algorithm* over it.

## Source Entries

Each element of the manifest `sources` array contributes one **source entry**:
[[src/ast/manifest.rs:source_entries]]

| Field     | JSON type          | Required | Default              |
| --------- | ------------------ | -------- | -------------------- |
| `root`    | string             | yes      | — (entry is skipped if absent) |
| `include` | array of string    | no       | `["**/*.mfb"]`       |
| `exclude` | array of string    | no       | `[]` (empty)         |

`root` is resolved relative to the project directory. An entry whose `root` is
missing or non-string is silently dropped. `include`/`exclude` arrays keep only
their string elements; non-string elements are dropped. When `include` is absent
the default `["**/*.mfb"]` is substituted; when `exclude` is absent the empty
list is substituted (nothing excluded). [[src/ast/manifest.rs:source_entries]]

Selection iterates the entries in manifest order, building one combined,
de-duplicated set. The two public entry points share the same collector:
`parse_project` (the build path) and `selected_source_paths` (raw-source tools
such as `mfb fmt`) both call `collect_selected_source_files`. [[src/ast/manifest.rs:collect_selected_source_files]]

## Per-Entry Resolution

For each entry, in order: [[src/ast/manifest.rs:collect_selected_source_files]]

1. **Join** `root` onto the project directory. If the joined path does not exist,
   emit `MFB_SOURCE_ROOT_MISSING` and fail.
2. **Canonicalize** the root (resolving symlinks). A canonicalize failure emits
   `MFB_SOURCE_READ_FAILED` and fails.
3. **Containment check.** If the canonical root is not within the canonical
   project directory, emit `MFB_SOURCE_OUTSIDE_PROJECT` and fail (see
   *Containment*).
4. **Collect** the entry's `.mfb` files:
   - If the root is a **file**: it is selected directly **iff** its extension is
     `mfb`. `include`/`exclude` are *ignored* for a file root — a file root names
     exactly one file. A file root whose extension is not `mfb` selects nothing
     (and then trips the empty check below). [[src/ast/manifest.rs:collect_selected_source_files]]
   - If the root is a **directory**: walk it recursively (see *Recursive Walk*),
     keeping each `.mfb` file whose root-relative path matches the entry's
     `include`/`exclude` patterns.
5. **Sort** the entry's collected files by display path (the un-canonicalized,
   on-disk path), ascending.
6. **Empty check.** If the entry selected zero files, emit `MFB_SOURCE_EMPTY` and
   fail. Every source entry must contribute at least one file.
7. **Overlap check.** For each selected file, key on its **canonical** path. If a
   prior entry already selected that canonical path, emit `MFB_SOURCE_OVERLAP`
   (naming both roots) and fail (see *Duplicate / Overlap Detection*).

After all entries are processed, the combined list is sorted once more by display
path so the final order is deterministic regardless of entry order.
[[src/ast/manifest.rs:collect_selected_source_files]]

Only files with the `mfb` extension are ever collected; the extension test
`extension() == Some("mfb")` is applied both to file roots and to every walked
entry. [[src/ast/manifest.rs:collect_mfb_files]]

## Recursive Walk

A directory root is walked depth-first via `read_dir`. [[src/ast/manifest.rs:collect_mfb_files]] At each directory:

- The directory is canonicalized and re-checked for containment (a symlinked
  subdirectory pointing outside the project fails the walk — see *Containment*).
- A **visited-set** of canonical directory paths guards against symlink cycles:
  re-entering an already-visited canonical directory returns immediately, so a
  loop terminates instead of recursing forever. [[src/ast/manifest.rs:collect_mfb_files]]
- Each entry is canonicalized and containment-checked before use.
- Subdirectories recurse; non-`.mfb` files are skipped.
- A surviving `.mfb` file's path is made **relative to the logical root** (the
  directory `root`, not the project), backslashes normalized to `/`, and tested
  against the entry's patterns with `matches_source_patterns`. A file matches
  when it matches **any** `include` pattern and **no** `exclude` pattern.
  [[src/ast/manifest.rs:matches_source_patterns]]

Glob patterns are matched against the **root-relative** path. With root `src` and
include `pkg/**/*.mfb`, the file `src/pkg/keep.mfb` is tested as `pkg/keep.mfb`.

`read_dir` iteration order is OS-dependent; determinism comes only from the
per-entry and final display-path sorts, never from walk order.

## Glob Algorithm

Patterns are matched **segment-wise** on `/`. Both the pattern and the path have
backslashes normalized to `/`, then each is split on `/` into segments; matching
is recursive over the segment lists. [[src/ast/manifest.rs:glob_matches]]

Segment-level rules: [[src/ast/manifest.rs:glob_match_segments]]

| Pattern segment | Meaning                                                         |
| --------------- | -------------------------------------------------------------- |
| `**`            | Matches **zero or more** whole path segments (cross-segment).  |
| anything else   | Matches **exactly one** path segment, compared component-wise. |

`**` is the only cross-segment construct: at a `**` the matcher tries consuming
the rest of the pattern against the current position, or (if the path is
non-empty) keeps `**` and drops one path segment — so `**/*.mfb` matches both
`main.mfb` (zero segments consumed) and `pkg/main.mfb` (one). An ordinary segment
requires a path segment to be present and to match component-wise before
recursing on the tails. [[src/ast/manifest.rs:glob_match_segments]]

Within a single segment, `glob_match_component` does a classic backtracking
wildcard match (the bytewise two-pointer algorithm with a remembered star
position): [[src/ast/manifest.rs:glob_match_component]]

| Within-segment token | Meaning                                              |
| -------------------- | --------------------------------------------------- |
| `*`                  | Matches zero or more characters **within** the segment (never crosses `/`). |
| `?`                  | Matches exactly one character within the segment.   |
| any other byte       | Matches itself literally (byte-exact, case-sensitive). |

There are **no character classes** (`[...]`), no brace expansion `{a,b}`, and no
escape mechanism — every byte other than `*` and `?` is a literal, including `.`.
Matching is byte-exact and case-sensitive. A `*` within a segment binds inside
that segment only: `pkg/*.mfb` matches `pkg/main.mfb` but **not**
`pkg/nested/main.mfb` (the single `*` segment cannot span the `/`). Use `**` to
cross directories. [[src/ast/manifest.rs:glob_match_segments]]

Worked results (from the in-tree tests): [[src/ast/manifest.rs:glob_matches]]

```text
**/*.mfb        vs  main.mfb           -> match
**/*.mfb        vs  pkg/main.mfb       -> match
pkg/*.mfb       vs  pkg/main.mfb       -> match
pkg/*.mfb       vs  pkg/nested/main.mfb-> no match
**/*_test.mfb   vs  pkg/math_test.mfb  -> match
**/*_test.mfb   vs  pkg/math.mfb       -> no match
```

## Containment

A path is *within the project* when it equals the canonical project directory or
is a prefix-descendant of it (`Path::starts_with` on the canonical paths).
[[src/ast/manifest.rs:path_within_project]] The check runs on **canonical** paths, so it
sees through symlinks: a `src` symlink pointing at a sibling directory outside the
project resolves outside and is rejected. The check is applied at three points —
the entry root, every directory entered during the walk, and every directory
entry encountered — so an escape anywhere in the tree is caught.

Inside the walk, a containment failure is surfaced as `MFB_SOURCE_OUTSIDE_PROJECT`
and then propagated as a `PermissionDenied` I/O error to abort the walk. The
caller suppresses a duplicate `MFB_SOURCE_READ_FAILED` for `PermissionDenied`
specifically (the real diagnostic was already shown); any other walk I/O error
*does* surface as `MFB_SOURCE_READ_FAILED`. [[src/ast/manifest.rs:collect_selected_source_files]]

## Duplicate / Overlap Detection

De-duplication is by **canonical path**, so two entries that reach the same file
through different relative roots (or through a symlink) are detected even when
their display paths differ. The first entry to select a canonical path records
which root claimed it; a later entry selecting the same canonical path triggers
`MFB_SOURCE_OVERLAP`, whose message names the selected file (as a
project-relative display path) and both the previous and current `root` strings.
The selection then fails. [[src/ast/manifest.rs:collect_selected_source_files]] A single
entry never overlaps itself (the recursive walk visits each file once and the
visited-set prevents cycle re-entry).

## Output

`collect_selected_source_files` yields, per file, a `SelectedSource` carrying both
the **canonical** path (`actual_path`, used to read bytes and to key
de-duplication) and the **display** path (`display_path`, the on-disk path used
for diagnostics and sort order). [[src/ast/manifest.rs:SelectedSource]] `parse_project`
reads each canonical path and parses it; `selected_source_paths` returns the
canonical paths alone for raw-text tooling. [[src/ast/manifest.rs:selected_source_paths]]
The compiler then appends its own built-in prelude and any imported built-in
package source *after* the user files, so the user's first selected file remains
`files[0]` (the monomorphizer's emission target). [[src/ast/manifest.rs:parse_project]]

## Diagnostics

All five are errors that abort selection; codes are the `rules` table entries.
[[src/rules/table.rs:RULES]]

| Name                         | Code        | Raised when                                                          |
| ---------------------------- | ----------- | ------------------------------------------------------------------- |
| `MFB_SOURCE_READ_FAILED`     | `1-100-0001`| Project dir, source root, or a walked path cannot be canonicalized/read (non-`PermissionDenied`). |
| `MFB_SOURCE_ROOT_MISSING`    | `1-100-0002`| A `root` joined onto the project directory does not exist.           |
| `MFB_SOURCE_EMPTY`           | `1-100-0003`| A source entry selected zero `.mfb` files.                          |
| `MFB_SOURCE_OUTSIDE_PROJECT` | `1-100-0004`| A root or walked path canonicalizes outside the project directory.   |
| `MFB_SOURCE_OVERLAP`         | `1-100-0005`| The same canonical file is selected by more than one source entry.   |

The full rule-code scheme and the `rules` registry that owns these names are
`./mfb spec diagnostics rule-codes`.

## See Also

* ./mfb spec tooling project-manifest — the `project.json` `sources[]` schema this consumes
* ./mfb spec architecture frontend — where parsed source feeds the pipeline
* ./mfb spec architecture flows — the end-to-end build pipeline
* ./mfb spec diagnostics rule-codes — the rule-code scheme behind these MFB_SOURCE_* diagnostics
* ./mfb spec language modules-and-packages — how selected files form a package's namespace
