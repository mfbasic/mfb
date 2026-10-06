# bug-927: tooling spec 01–06 drift: stale `Sub` block, missing `network` rank, unstated behaviors, small validator gaps

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Regression Test: none (doc sync); items 5 and 6 can get unit tests if fixed in code

**Owner decision (2026-10-05):** for the open items, all CODE changes.
- Unknown string `kind` (e.g. `"excutable"`) is a manifest **error**: `mfb build` and
  `mfb test` exit nonzero instead of warning and exiting 0.
- A `packages` entry whose name fails `validate_package_name` is a located manifest error
  naming the entry. It is never silently skipped (same shape as bug-353).
- `icon` is confined to the project with the same `path_stays_in_project` rule as
  `resources[].src`. An absolute or `..` path is a manifest error.
- The audit `network` category gets a rank alongside the other security categories, in
  `src/audit/report.rs:category_rank` and the spec's rank table.
- An `mfb.lock` that exists but is not a regular file (e.g. a directory) is
  `AUDIT-LOCK-MALFORMED`, not "absent".
All other items are spec fixes.

A collection of measured mismatches between `src/docs/spec/tooling/{01..06}` and
the code at HEAD (28a15a153). None of them is large enough for its own report.
Each item gives its class: SPEC (doc wrong, code right), CODE (small code gap),
UNCLEAR (owner decision).

References: `src/docs/spec/tooling/01_project-manifest.md`, `03_lockfile.md`,
`04_audit-format.md`, `05_fmt.md`, `06_doc-html.md`.

## Items

1. **SPEC — `05_fmt.md` lists a `Sub` block that does not exist.** *Block-structure
   operation model*: "`Block` variants: `Func`, `Sub`, `Type`, …". `src/fmt.rs:Block`
   has `Func, Type, Union, Enum, If, For, While, Do, Match, Trap, Case, Testing,
   Tgroup, Tcase`. There is no `Sub`, and `SUB` is not a keyword
   (`SUB dot()` → `MFB_PARSE_UNEXPECTED_STATEMENT`, measured).

2. **SPEC — `04_audit-format.md` category-rank table omits `network`.**
   `AUDIT-TLS-RELAXED-TRUST` has category `network`
   (`src/audit/collect/findings.rs:relaxed_trust_findings`), but
   `src/audit/report.rs:category_rank` has no `network` arm, so it ranks `9`
   ("any other"). Measured: the warning is listed **after** every
   `AUDIT-PERM-*` info finding. The rank table should list it (or the code should
   rank it). UNCLEAR which.

3. **SPEC — citation imprecision.** `01_project-manifest.md` cites
   `[[src/os/linux/appdir.rs:HICOLOR_SIZES]]`. The constant is defined in
   `src/os/icon.rs`; appdir.rs only imports and uses it.

4. **SPEC — unknown `kind` builds nothing and exits 0, unstated.** ¹ says an
   unknown string `kind` "only warns … and validation continues". It does not say
   what `mfb build` then does: it prints `Validated MFBASIC project at .`, writes
   no artifact, and exits **0** (measured with `"kind":"excutable"`). A typo in
   `kind` therefore passes a CI build step. `mfb test` still runs the tests. This
   is documented only in code (`src/cli/build/mod.rs`, the bug-300 E8 comment).
   The spec should state it (UNCLEAR: should a typo'd kind fail the build?).

5. **CODE — `maxBuffer` is not validated when `sources` is bad.**
   `validate_max_buffer` is called from inside `validate_sources`, after the
   missing/non-array/empty early returns (`src/manifest/mod.rs:validate_sources`).
   Measured: `{"sources":5,"maxBuffer":0,…}` reports only `Field `sources` must be
   an array.` That contradicts *Loading and Parse Errors*: "all field errors in a
   single run are reported". Fix: call it from `validate_project_manifest`.

6. **SPEC/CODE — `packages[]` entries are silently dropped in more cases than
   documented.** The spec says "each element must be an object with a string
   `name`. An entry whose `name` is absent, non-string, or blank-after-trim is
   silently skipped". Measured: a non-object entry (`"packages":[5]`) is silently
   skipped and builds. An entry whose name fails `validate_package_name`
   (`"my lib"`, `"../mylib"`, `"mylib/"`) is also silently skipped
   (`src/manifest/package.rs:common_dependency_fields`). The project then fails
   later with `IMPORT_PACKAGE_NOT_DECLARED … is not declared in project.json
   packages`, which contradicts the file. UNCLEAR: diagnose (preferred, since
   bug-353 had the same "silently vanished dependency" shape) or document.

7. **SPEC — unattributable-drift cause list is incomplete.** `03_lockfile.md` *Why a
   mismatch can be unattributable* names only `pin`/`source`. A local `file://`
   package is hashed into `projectHash` but `classify_drift` diffs only registry
   dependencies (`src/cli/resolve.rs:classify_drift`, `is_registry_dependency`),
   so adding or removing a local package beside a registry dependency also gives
   that error. Measured: `error: mfb.lock does not match project.json, but the
   difference is not in a field the lock records (most likely `pin` or `source`
   changed)`. The spec, and ideally the message, should list it.

8. **UNCLEAR — `mfb.lock` that is a directory reads as absent.**
   `src/audit/collect/lockfile.rs:collect_lockfile` gates on `lock_path.is_file()`.
   A directory named `mfb.lock` gives `present: false` and no finding (measured).
   The spec says an *unreadable* lock is `AUDIT-LOCK-MALFORMED`.

9. **UNCLEAR — `icon` is not confined to the project.** `resources[].src` rejects
   absolute and `..` paths (bug-298), but `icon` is joined and read unchecked.
   Measured: `"icon":"../m1/project.json"` in app mode is opened
   (`error: icon './../m1/project.json' is not a decodable image`). A 1024×1024 PNG
   outside the project would be embedded in the bundle. Either apply
   `path_stays_in_project` or document that `icon` may point anywhere.

10. **SPEC — the project-local parse error headline names project.json.** A
    malformed `project-local.json` reports `./project-local.json:1
    error[2-200-0003 PROJECT_JSON_PARSE_FAILED]: project.json is not valid JSON`.
    The path is right (as *Local overrides* promises), but the rule headline says
    `project.json`. Cosmetic.

11. **SPEC — `mfb doc <file>` accepts non-`.mfb` files.** `06_doc-html.md`: "if it
    is a single file, that file is parsed into a one-file project". Measured:
    `mfb doc project.json` parses the JSON as MFBASIC and prints a dozen
    `MFB_PARSE_UNEXPECTED_STATEMENT` errors. `mfb fmt project.json` gives
    `` error: `project.json` is not a .mfb source file ``. Consider matching fmt,
    or stating it.

12. **SPEC — `05_fmt.md` "line breaks preserved" vs CRLF.** `format_source` strips
    `\r`, so a CRLF file comes out LF (measured with `od -c`). This is stated later
    in *Top-level line model*, but the intro's "line breaks … are preserved" should
    say "line structure", not bytes. (The comma-only-line deletion is a separate
    report: B13-fmt-drops-comma-only-line-shifting-source-lines.)

## Validation Plan

- Doc sync for 1, 2, 3, 4, 7, 10, 11, 12. Code fixes with unit tests for 5 (and for 6/8/9 if the owner chooses the code direction).
