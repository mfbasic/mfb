# bug-340: `src/builtins/` + CLI/manifest cleanup cluster — a human diagnostic string used as load-bearing type information, five hand-maintained package chains, and a CLI entry point that is 97% not an entry point

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup) / Dead-code

Status: Open
Regression Test: existing acceptance goldens (no new expected output); new unit
tests only where an item collapses two implementations into one (A1, A3, A4, A6,
B2, B5, B9).

A cluster of duplication, placement, and layering residue across two adjacent
surfaces — the builtin-package front-end tables (`src/builtins/`, 14,091 lines
across 26 modules) and the CLI / manifest / audit layer (`src/main.rs`,
`src/cli/`, `src/manifest/`, 17,228 lines). Every item below was re-verified
against the current worktree and carries a citation the reader can open; where
the original review's numbers were wrong they are **corrected inline and
labelled**, and two leads were dropped outright (see *Dropped leads*).

None of these items changes what the compiler emits. The reason this is filed
rather than left alone is **A1**: `expected_arguments` returns a string written
for a human error message, and `src/ir/lower.rs` parses that string back into a
positional type signature. A diagnostic string is currently load-bearing type
information, and the parse silently declines for six of the twenty-two packages
that define one. The single correct outcome of a fix is that argument types are
carried by a machine-readable table for every package, that each duplicated
helper has exactly one implementation, that each file's contents match its name,
and that all generated artifacts (`-ast`, `-ir`, `-br`, `-nir`, `-nplan`,
`-nobj`, `-ncode`, `-mir`) stay **byte-identical** to today's committed goldens.

References:

- Found during the tree-wide cleanup review (Agent 17 — builtins Rust; Agent 15 —
  CLI/manifest/audit), base `25c38ba1`.
- `src/docs/spec/tooling/07_cli-reference.md` — the CLI surface B1/B2 dispatch.
- `src/docs/spec/tooling/01_project-manifest.md` — the manifest schema B7/B8 parse.
- Memory note: plan-42 CLI modernization (the `--flag` surface `main.rs` dispatches).

### Covered by sibling bugs — NOT in scope here

- The `src/cli/build.rs` (2,946 lines) and `src/builtins/general.rs`
  (1,532 lines) **file splits themselves** → **bug-327** (oversized file splits).
  This document owns only the *duplication* inside those files, and B3 below
  records the measured structure so bug-327 can act on it.
- Man-page drift for `math::` Money overloads, `net::toAddress`, `http::`
  coverage, and the `filters` pseudo-package → **bug-337**.
- The `.txt`-only man-corpus guard hole → **bug-336**.
- Repo-wide dead-code sweep (`resource.rs` stale `allow`s,
  `testutil::EMPTY_MAIN`, `strings_specs.rs`) → **bug-326**.
- Spec drift for the undocumented `resources` manifest section, `--unsigned`,
  and the audit text format → **bug-338**.
- `check_*_builtin_call` ×22 in `src/syntaxcheck/builtins.rs` → **bug-324**.

## Current State

Measured in worktree `cleanup-review`, base `25c38ba1`.

| File | Lines | Note |
| --- | --- | --- |
| `src/cli/build.rs` | 2,946 | `build_project` alone is 615 |
| `src/cli/pkg.rs` | 2,092 | |
| `src/manifest/mod.rs` | 1,689 | |
| `src/manifest/package.rs` | 1,562 | ~126 of them parse manifest JSON |
| `src/builtins/general.rs` | 1,532 | 20 `collections::` resolvers + their tests |
| `src/testing/desugar.rs` | 1,326 | |
| `src/doc.rs` | 1,098 | |
| `src/builtins/mod.rs` | 1,000 | four package dispatch chains |
| `src/main.rs` | 880 | ~30 lines are an entry point |
| `src/builtins/thread.rs` | 862 | ~290 of them a type-string grammar |
| `src/coverage.rs` | 442 | |
| `src/cli/mod.rs` | 339 | |
| `src/builtins/io.rs` | 126 | the only builtin module with no tests |

Two measurements that motivate the cluster:

```
$ grep -n 'contains(.\[.)' src/ir/lower.rs
2675:    if expected.contains('[') || expected.contains(" or ") { return None; }
2678:    let params = expected.split(", ").map(str::to_string).collect::<Vec<_>>();
```

22 builtin modules define `expected_arguments`; **4** additionally define a
machine-readable `argument_types()`; the `lower.rs` dispatch chain that consumes
the string covers **16** of the 22.

```
$ md5 of the 15 `fn exact` bodies in src/builtins/  ->  one hash, 15 files
```

---

## Group A — `src/builtins/`: the argument-type chain and its copy-paste

### A1 — `expected_arguments` is a human diagnostic string that IR lowering parses as a positional signature

*The most consequential item in this document.*

- Producer: `expected_arguments` in **22** builtin modules — e.g.
  `src/builtins/math.rs:191`, `src/builtins/strings.rs:202`,
  `src/builtins/collections.rs:189`, `src/builtins/net.rs:246`,
  `src/builtins/term.rs:118`. Its return value is prose intended for a
  diagnostic ("expected …").
- Consumer: `builtin_argument_types` at `src/ir/lower.rs:2655-2683`. It
  dispatches to the per-package `expected_arguments` (chain at
  `src/ir/lower.rs:2656-2671`), then at `:2675` **bails out** if the string
  contains `'['` or `" or "`, and at `:2678` **splits the remainder on `", "`**
  and treats each fragment as a positional parameter type.
- Four packages have since grown a second, genuinely machine-readable table —
  `argument_types()` at `src/builtins/crypto.rs:302`,
  `src/builtins/audio.rs:353`, `src/builtins/net.rs:276`,
  `src/builtins/tls.rs:154`. **Eighteen** of the 22 have not.
- The chain covers 16 packages: `general`, `strings`, `math`, `bits`, `fs`,
  `os`, `io`, `json`, `csv`, `regex`, `net`, `tls`, `audio`, `crypto`, `http`,
  `thread`. It **silently omits six packages that do define
  `expected_arguments`**: `collections`, `encoding`, `datetime`, `money`,
  `term`, `vector`. For those, `builtin_argument_types` returns `None` with no
  diagnostic — indistinguishable from "this builtin has no argument types".

Why this is the load-bearing item: the two bail conditions mean the type
information available to IR lowering is a function of how the *error message* is
phrased. Rewording a diagnostic to read "an Integer or a Float" turns the
signature off; adding a bracketed optional turns it off. There is no test and no
compiler check tying the two together.

Fix: promote `argument_types()` to the canonical per-package table for all 22
modules and derive `expected_arguments` from it (the shape `term.rs` already
uses — see A6). `builtin_argument_types` then reads the table directly and the
string parse at `:2675-2678` is deleted. Land the six omitted packages
explicitly and gate on `scripts/artifact-gate.sh` — the omission may currently
be masking a resolution difference, so this is the one item in this document
that must not be assumed output-neutral until proven.

### A2 — `source_file` / `uses_package` / `augmented_project` copied into 13 modules (~338 lines)

Thirteen modules define all three functions: `audio`, `collections`, `crypto`,
`csv`, `datetime`, `encoding`, `http`, `json`, `money`, `net`, `regex`,
`strings`, `vector`.

**Correction to the original lead** ("differing only in three literals"): that
holds for **10** of the 13 — `src/builtins/audio.rs:196-221`,
`crypto.rs:362-387`, `csv.rs:68-94`, `datetime.rs:346-371`,
`encoding.rs:255-280`, `http.rs:304-330`, `json.rs:105-131`,
`money.rs:85-110`, `net.rs:327-353`, `vector.rs:372-397`, each differing only in
the `include_str!` path, the doc path literal, and the `package_name()` compare.
Three are **structurally different and must not be folded blindly**:

- `src/builtins/regex.rs:104-141` — `source_file` concatenates
  `regex_package.mfb` with `regex_unicode.mfb` via `format!`, not a bare
  `include_str!`.
- `src/builtins/strings.rs:260-318,435-444` — `uses_package` (`:307-318`) gates
  on scalar-seam member usage (`imports_strings && … item_references_seam`), not
  a plain import check; `source_file` also rewrites `__regex_genCat` →
  `__strings_genCat`.
- `src/builtins/collections.rs:227-256` — `augmented_project` takes
  `AstProject` **by value**, not `&AstProject`.

Fix: a macro (or a `fn(pkg_name, source_text)` helper) for the 10 uniform cases;
leave the three special cases as explicit overrides with a comment saying why.

### A3 — `fn exact` duplicated byte-for-byte in 15 modules (105 lines)

All fifteen bodies are byte-identical (verified by extracting and hashing each):
`audio.rs:409`, `crypto.rs:389`, `csv.rs:96`, `datetime.rs:373`, `fs.rs:312`,
`general.rs:616`, `http.rs:332`, `io.rs:120`, `json.rs:133`, `money.rs:112`,
`net.rs:355`, `os.rs:114`, `regex.rs:143`, `strings.rs:446`, `tls.rs:205`.

```rust
fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types.iter().zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}
```

Fix: one `pub(super) fn exact` in `src/builtins/mod.rs`; delete 15 copies
(15 × 7 = 105 lines).

### A4 — 20 `collections::` member resolvers live in `general.rs`, consumed only by `collections.rs`

**Correction to the original lead**: the count is **20**, not 21.

- Implementations: `src/builtins/general.rs:384-614` — `resolve_find_list`
  (`:384`), `resolve_mid_list` (`:396`), `resolve_replace_list` (`:406`),
  `resolve_get` (`:418`), `resolve_get_or` (`:433`), `resolve_set` (`:448`),
  `resolve_append` (`:463`), `resolve_prepend` (`:473`), `resolve_insert`
  (`:483`), `resolve_remove_at` (`:493`), `resolve_remove_key` (`:500`),
  `resolve_keys` (`:510`), `resolve_values` (`:520`), `resolve_has_key`
  (`:530`), `resolve_contains` (`:540`), `resolve_sum` (`:550`),
  `resolve_for_each` (`:568`), `resolve_transform` (`:579`), `resolve_filter`
  (`:590`), `resolve_reduce` (`:601`) — 231 lines.
- Their tests: `src/builtins/general.rs:993-1434` (~440 lines). *(The original
  lead said `:993-1437`; `1437` falls inside the next, unrelated test.)*
- Sole consumer: the dispatch table at `src/builtins/collections.rs:117-145`
  (match arms `:122-142`), whose `NATIVE_MEMBERS` list at `:47-68` names exactly
  these 20. A repo-wide grep finds **no other caller** of any of the 20.

Together these ~670 lines are the single largest reason `general.rs` is 1,532
lines. Fix: move all 20 plus their tests into `src/builtins/collections.rs`
(pure lift-and-shift, no call-graph surgery). Note for bug-327: this alone takes
`general.rs` to ~860 lines.

### A5 — `thread.rs` is call registration plus a 290-line type-string grammar

`src/builtins/thread.rs` is 862 lines. Lines **`:211-501`** are a type-string
grammar (`matches_start` at `:211` through `split_top_level_types` at
`:480-500`) with nothing thread-specific about it; the remainder is ordinary
call registration. `split_top_level_types` (`:480-500`) is a near-duplicate of
`split_top_level_commas` at `src/builtins/mod.rs:430-447` — same
depth-tracked split at paren depth 0, differing only in return type
(`Vec<String>` vs `Vec<&str>`) and in `index + ch.len_utf8()` vs `index + 1`
(equivalent, the delimiter is always ASCII `,`).

Fix: move the grammar beside the other type-name helpers in
`src/builtins/mod.rs` and collapse the two splitters into one.

### A6 — Five hand-maintained package dispatch chains, three orders, one drifted

Four chains in `src/builtins/mod.rs`, plus the fifth in `src/ir/lower.rs`:

| # | Chain | Location | Entries | Omits |
| --- | --- | --- | --- | --- |
| 1 | `resolve_call_return_type` | `mod.rs:297-328` (entries `:305-326`) | 22 | — |
| 2 | `call_return_type_name` | `mod.rs:330-351` (entries `:331-350`) | 20 | **`thread`, `vector`** |
| 3 | `is_builtin_call` | `mod.rs:368-400` (entries `:377-398`) | 22 | — |
| 4 | `call_param_names` | `mod.rs:513-536` (entries `:514-535`) | 22 | — |
| 5 | `builtin_argument_types` | `ir/lower.rs:2655-2671` | 16 | the six in A1 |

**Correction to the original lead** ("two of them omitting packages"): of the
four `mod.rs` chains only **one** — chain 2, `call_return_type_name` — omits
anything, and it omits exactly `thread` and `vector`. Chains 1, 3, 4 all cover
all 22. The "three different orders" claim holds: chain 1 starts
`general, collections, strings, …`; chains 2 and 4 start `audio, general,
collections, …` but disagree on the tail (`… term, tls` vs `… term, tls,
thread, vector`); chain 3 orders `audio, collections, general, …` and places
`thread` before `tls`.

The consequence is the actionable part: adding a builtin package means editing
five lists in two crates' worth of files with **no compiler assistance**, and
the mechanism has already failed twice (chain 2 and chain 5). Fix: one
`const PACKAGES: &[&dyn BuiltinPackage]` (or a macro-generated table) that all
five sites iterate, so an omission is a compile error rather than a silent
`None`.

### A7 — `term.rs` diverges from the 21 sibling signatures — and is the only module doing it the right way

The divergence is real and costs two special cases:

- `src/builtins/term.rs:111`:
  `fn resolve_call<'a>(name: &str) -> Option<ResolvedCall<'a>>` — **no
  `arg_types` parameter**, unlike `src/builtins/math.rs:142`
  (`fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> …`).
- `src/builtins/term.rs:118`: `expected_arguments(name) -> Option<String>`
  (owned) vs `src/builtins/math.rs:191` `-> Option<&'static str>` (borrowed) —
  and every one of the other 21 modules matches `math.rs`.
- Special cases forced by this: `src/builtins/mod.rs:322`
  (`try_pkg!(term::resolve_call(callee)); // no arg_types param`) and the
  dedicated `check_term_builtin_call` at `src/syntaxcheck/builtins.rs:1016`,
  whose `:1072` calls
  `builtins::term::expected_arguments(callee).unwrap_or_else(|| "no arguments".to_string())`.

But `term.rs` is also **the only module with a single `param_types` table
(`src/builtins/term.rs:89`) from which both `expected_arguments` and `arity`
(`:127`, via `param_types(name)?.len()`) are derived**. A grep for
`fn param_types` across `src/builtins/*.rs` returns exactly one hit — `term.rs`.

So this item is not "make `term.rs` look like its siblings". It is: **`term.rs`
is the prototype for A1.** Adopt its `param_types`-derived shape in the other 21
modules, then bring `resolve_call`'s signature into line so the two special
cases can go. Doing it the other way round — normalizing `term.rs` first —
would delete the only working example of the target design.

### A8 — `io.rs` has no tests; `testing.rs` uses a naming convention no sibling follows

- `src/builtins/io.rs` (126 lines) is the only one of the **25** builtin modules
  (excluding `mod.rs`) with no `mod tests`. *(Correction: the other **24** have
  one, not "25 others".)*
- `src/builtins/testing.rs` names its predicates after the *builtins* rather
  than the *package*: `is_expect_call` (`:44`), `is_equality_assert` (`:52`),
  `is_inequality_assert` (`:61`), `expect_operand_type` (`:70`), `expect_arity`
  (`:81`). Every other package uses `is_<pkg>_call` matching its own module name
  — verified for all 22 (`is_bits_call` `bits.rs:31`, `is_audio_call`
  `audio.rs:80`, `is_term_call` `term.rs:42`, …).

Fix: add an `io.rs` test module mirroring a sibling's; rename
`testing::is_expect_call` → `is_testing_call` (private to the module, no
external surface).

---

## Group B — CLI / manifest / audit

### B1 — `src/main.rs` is 880 lines of which ~30 are an entry point

- **12** help-screen constants at `src/main.rs:45-250` — `USAGE` (`:45`),
  `INIT_HELP` (`:80`), `INIT_PKG_HELP` (`:88`), `PKG_HELP` (`:96`), `REPO_HELP`
  (`:120`), `BUILD_HELP` (`:153`), `TEST_HELP` (`:182`), `FMT_HELP` (`:196`),
  `AUDIT_HELP` (`:208`), `DOC_HELP` (`:217`), `MAN_HELP` (`:228`), `SPEC_HELP`
  (`:241`). *(Correction: twelve, not ten.)*
- The dispatcher `fn main` at `:257-524`.
- `mod tests` at `:532-880` (349 lines) testing functions defined in **other**
  modules: `crate::cli::build::{parse_build_options, BuildOutput}` (`:540`),
  `crate::cli::pkg::{package_dependency_status, package_verify_line,
  package_version_matches, verify_package_dependency}` (`:542-545`),
  `crate::manifest::{parse_project_json, validate_project_manifest}` (`:548`).

**Correction to the original lead**: `src/cli/mod.rs` is **339 lines**, not 10.
Its first 10 lines are the `pub mod` declarations; the rest is real code
(`stage_package_blob` `:22`, `commit_staged_package` `:51`,
`install_verified_package` `:61`, `install_vendor_file` `:91`,
`local_paths_for_repo` `:136`). The point survives — the help text and the
dispatcher belong under `src/cli/`, and each test belongs beside the function it
tests — but "`cli/mod.rs` is an empty shell" is not the argument for it.

Fix: `src/cli/help.rs` for the 12 constants, `src/cli/dispatch.rs` for the
dispatcher, and move the 349 test lines to the three modules they test.
`main.rs` ends at ~30 lines.

### B2 — Four identical 19-line dispatch arms, plus two more of the same shape, over two identical error enums

- Byte-identical modulo the substituted command identifier:
  `src/main.rs:390-408` (machine), `:409-427` (key), `:428-446` (org),
  `:447-465` (token) — 76 lines, diffed directly, no other difference.
- `src/main.rs:352-370` (pkg) and `:371-389` (repo) repeat the same 19-line
  shape and the same exit mapping (`2` → usage, `1` → failed).
- The two error types are structurally identical:
  `src/cli/pkg.rs:19-22` `enum PkgCommandError { Usage(String), Failed(String) }`
  and `src/cli/repo.rs:18-21`
  `enum RepoCommandError { Usage(String), Failed(String) }`.

Fix: one `enum CommandError { Usage(String), Failed(String) }` in `src/cli/`,
one `fn dispatch(result: Result<(), CommandError>, help: &str) -> i32`; the six
arms become six one-line calls.

### B3 — `src/cli/build.rs`: a 615-line `build_project` over five concerns

Measured: `src/cli/build.rs` is 2,946 lines; `build_project` is
`:240-854` — exactly 615 lines. The four other concerns are
signing (`:978-1372`), native libraries (`:1381-~1735`), resources
(`:1745-1845`), and test mode (`:859-976`).

**Correction to the original lead**, and it matters for how bug-327 splits the
file: the five concerns do **not** "never call each other". `build_project` is
their orchestrator and calls into every one of them —
`load_build_signing_info` (`:449`), `assemble_native_libraries_for_ir` (`:485`),
`verify_vendor_libraries` (`:528`), `copy_vendor_libraries` (`:559`),
`copy_resources` (`:572`), `run_test_binary` (`:607`),
`generate_coverage_report` (`:614`), `assemble_native_libraries` (`:653`). What
is true is that the four *concerns* do not call each other — they are
independent leaves under one long caller, which is exactly the shape that splits
cleanly into modules with `build_project` left as the wiring. Cross-reference
**bug-327** for the split; the duplication items below stay here.

### B4 — Five near-identical artifact-dump arms (~70 lines) with labels re-derived

`src/cli/build.rs:779-789` (NativeIr), `:790-805` (NativePlan), `:806-821`
(NativeObjectPlan), `:822-837` (NativeCodePlan), `:838-848` (Mir). Each is the
same `match target::write_X(...) { Ok(path) => …, Err(err) => { eprintln!; return
Err(()) } }; println!(…)` differing only in the writer function and the message
noun. The labels are re-derived separately at `:721-727`
(`let what = match output { … }`).

Fix: `BuildOutput::label()` + one generic dump arm.

### B5 — `parse_test_options` copies `parse_build_options`' flag arms verbatim

`src/cli/build.rs:165-174` vs `:869-878` (the `--target`/`-target` arm) and
`:198-207` vs `:879-888` (the `--regalloc` arm). Diffed: **the only difference
in either pair is one line**, `"mfb build -target …"` vs `"mfb test -target …"`.

*(Correction: the duplicated content is ~20 lines (2 × 10), not ~24. The wider
range `:165-207` also holds `--sign`/`--app`/`--app-debug`/`--unsigned`, which
have no counterpart in `parse_test_options`.)*

Fix: one `parse_common_option(arg, cmd: &str)` helper taking the command name.

### B6 — `install_vendor_file` re-implements the ritual its own doc says it shares

`src/cli/mod.rs:91-122` inlines `create_dir_all` → `.part` staging name →
`OpenOptions::create_new` → `write_all` + `sync_all` → `rename` — the exact
sequence already implemented by `stage_package_blob` (`:22-42`) and
`commit_staged_package` (`:51-56`), which `install_verified_package` (`:61-78`)
actually calls. The tell is its own doc comment at `src/cli/mod.rs:82`:
"*using the same stage-verify-rename discipline as [`install_verified_package`]*".

Fix: call the two existing helpers.

### B7 — `escape` and `anchor` duplicated between the two HTML generators

Two independent HTML generators with no shared code:
`src/doc.rs` (`anchor` `:126-145`, `escape` `:382-393`, `STYLE` `:1053`) and
`src/coverage.rs` (`anchor` `:253-266`, `escape` `:268-279`, `STYLE` `:282`).
*(The relevant file is `src/doc.rs`; `src/cli/doc.rs` is only the command
dispatcher.)*

- `escape` is **byte-for-byte identical** in both — a clean dedup.
- `anchor` is **not**: `src/doc.rs:126-145` lowercases
  (`c.to_ascii_lowercase()`), `src/coverage.rs:253-266` preserves case. Merging
  them without picking a behavior would change generated anchor ids in one of
  the two HTML outputs. Treat as a real decision, not a mechanical dedup.
- Each file carries its own distinct `STYLE` CSS constant.

Fix: hoist `escape` to a shared `html` module immediately; resolve `anchor`'s
case behavior first (see *Open Decisions*); consider one shared `STYLE`.

### B8 — `manifest/package.rs` is four unrelated subsystems, one of them a layering inversion

`src/manifest/package.rs` is 1,562 lines, of which roughly **126** actually
parse manifest JSON:

| Range | Subsystem |
| --- | --- |
| `:10-291` | `.mfp` container-header reader (`MFP_MAGIC` `:10`, `read_mfp_header` `:127-225`, `read_u64` ending `:290`) |
| `:47-112` | URL/path handling (`package_file_url_path` `:47` … `hex_value`) |
| `:420-545` | manifest parsing (`package_metadata` … `project_package_dependency`) |
| `:713-831` | a bespoke JSON scanner (`json_array_bounds` …) |

The layering inversion is real: `src/binary_repr/reader.rs:185`
`fn mfp_binary_repr_payload(bytes: &[u8]) -> Result<MfpContainer<'_>, String>`
decodes the **identical** container format — same magic, container version
check, name/ident/version/author/url/identKey/signingKey/proof/proofSig/
attestation/attestationSig, signature type and length, `binary_repr` length —
that `read_mfp_header` re-implements by hand. Two decoders for one wire format,
in the layer that owns it and in the layer above it.

**One correction that changes the fix's size**: `mfp_binary_repr_payload` is
`pub(super)` and its `MfpContainer`/`MfpIdentity` types are private
(`src/binary_repr/reader.rs:172-180`). `binary_repr` does **not** currently
expose this. The fix is therefore "promote the `binary_repr` decoder to a
`pub(crate)` API and delete the `manifest` copy", not "switch a call site" —
budget accordingly.

### B9 — `package_dependencies` and `project_package_dependency` are divergent copies; only one has the bug-195 guard

- `src/manifest/package.rs:457-492` `package_dependencies` — builds
  `BinaryReprDependency` from `name`/`ident`/`version`/`pin` with **no**
  blank-name check and **no** `validate_package_name` call.
- `src/manifest/package.rs:494-545` `project_package_dependency` — applies both:
  the blank-name guard at `:524-526` and `validate_package_name` at `:533-535`,
  with an explicit bug-195 comment at `:528-532`.

~30 lines duplicated between them, and the copy without the guard is the one
that reaches `.mfp` metadata. Note this is adjacent to a real defect (a package
name like `../x` reaching `.mfp` metadata through the unguarded path); this
document scopes only the *dedup*, which incidentally closes it. If the security
side is to be tracked separately, file it — do not let the dedup silently be the
only record.

### B10 — `src/coverage.rs` splits one feature across three files at two nesting levels

`CovSlot` is defined at `src/coverage.rs:16` (crate root) and consumed by
`src/testing.rs:19` and `src/testing/desugar.rs:17`. The four filename constants
live at `src/testing.rs:37-40`; instrumentation is `instrument_coverage` at
`src/testing/desugar.rs:404`; and the report is generated by
`generate_coverage_report` at `src/cli/build.rs:919-928`, which reaches into
`crate::coverage::{read_covmap, read_counts, read_failed, generate_html}` and
`crate::testing::{COVMAP_FILE, COVDATA_FILE, COVFAIL_FILE, COVERAGE_HTML}`.

Fix: move `src/coverage.rs` under `src/testing/coverage.rs` so the feature lives
in one subtree at one level.

### B11 — `hex` / `hex_bytes`: the same one-liner in two CLI files

`src/cli/build.rs:1567-1569` `fn hex(bytes: &[u8]) -> String` and
`src/cli/pkg.rs:1410-1412` `pub(crate) fn hex_bytes(bytes: &[u8]) -> String`
have identical bodies (`bytes.iter().map(|byte| format!("{byte:02x}")).collect()`).
`hex_bytes` is already `pub(crate)`.

Fix: delete `hex`, call `hex_bytes`.

### B12 — `mfb <cmd>`-shaped integration tests unfoldered at `tests/` root

Cargo requires integration tests at the `tests/` root, so the fix here is a
**naming prefix, not a move**.

The nine `mfb <cmd>`-shaped tests: `tests/build_verbosity_output.rs`,
`tests/linux_app_mode.rs`, `tests/repo_acceptance.rs`, `tests/entry_args.rs`,
`tests/tls_listen_accept_build.rs`, `tests/linux_pie_headers.rs`,
`tests/macos_rodata_readonly.rs`, `tests/linux_rodata_readonly.rs`,
`tests/macos_tls_write_capacity.rs`.

**Correction to the original lead**, which framed these nine as an exception:
`tests/` root holds **20** `.rs` files, and `find tests/{acceptance,syntax,rt-error,rt-behavior}
-name '*.rs'` returns **0**. The four folders hold fixtures and goldens
(`.mfb`, `.json`, `.log`, `.ast`, `.ir`), not harness code — every Rust test
file in the crate is flat at the root. The other eleven are
`fs_atomic_int_return`, `fs_create_mode_0600`, `fs_error_path_hygiene`,
`gtk_term_utf8_grid`, `macos_app_io_input_imports`,
`native_float_pow_operator_runtime`, `native_io_runtime`, `native_loop_runtime`,
`native_numeric_pow_div_runtime`, `native_size_arith_overflow`,
`syscall_return_robustness`.

Fix: a consistent prefix scheme across all 20 (`cli_*`, `rt_*`, `link_*`) so the
flat directory groups visually. Renaming a test file changes no behavior.

---

## Dropped leads

These were in the source material and are **not** carried into this document:

- *"The five `cli/build.rs` concerns never call each other."* — refuted;
  `build_project` calls all four (see B3).
- *"`src/cli/mod.rs` is 10 lines."* — refuted; it is 339 (see B1).

## Goal

- `expected_arguments` is a diagnostic string only; argument types for all 22
  packages come from a machine-readable table, and no package can be omitted
  from a dispatch chain without a compile error (A1, A6, A7).
- Each duplicated helper (`exact`, the three package-source functions, the two
  type splitters, `escape`, `hex`, the six dispatch arms, the two option
  parsers, the vendor-file writer, the two dependency builders) has exactly one
  implementation.
- Each file's contents match its name: no `collections::` resolvers in
  `general.rs`, no type grammar in `thread.rs`, no `.mfp` decoder in
  `manifest/`, no help screens or foreign tests in `main.rs`.
- Every generated artifact stays byte-identical to today's committed goldens.

### Non-goals (must NOT change)

- Any diagnostic **text** — error message wording, help-screen content, and
  usage strings must be byte-identical after A1/B1/B2. Rewording a diagnostic
  "while we are in there" would be exactly the change A1 exists to make safe.
- Any `.mfp` wire format, manifest schema, or CLI flag surface. B8 promotes an
  existing decoder to `pub(crate)`; it does not change the format it decodes.
- Exit codes and the usage/failure mapping in B2.
- Generated HTML content beyond the `anchor` decision in B7.
- The tempting wrong fix, named and forbidden: **do not "fix" A1 by tightening
  the string parse** (more bail conditions, a smarter split). That deepens the
  dependency on diagnostic prose. The parse must be deleted, not improved.
- Do not delete the six omitted packages' `expected_arguments` to "make the
  chain consistent" — the omission is the bug, not the tables.

## Blast Radius

- `src/ir/lower.rs:2655-2683` and its 16-entry chain — fixed by A1.
- `src/builtins/mod.rs:297-536` (four chains) — fixed by A6.
- `src/syntaxcheck/builtins.rs:1016,1072` — the `term` special case, removed by
  A7. `check_*_builtin_call` ×22 in the same file is **out of scope** (bug-324).
- The 4 modules with `argument_types()` (`crypto`, `audio`, `net`, `tls`) —
  already at the target shape; A1 extends it to the other 18.
- `src/builtins/{regex,strings,collections}.rs` — same *shape* as the 10
  uniform copies in A2, **latent, deliberately out of scope for the macro**;
  they carry real behavioral differences and stay explicit.
- `src/binary_repr/reader.rs:172-185` — must gain a `pub(crate)` surface for B8;
  its decoding behavior is unaffected.
- `src/cli/{build,pkg,repo,mod}.rs`, `src/doc.rs`, `src/coverage.rs`,
  `src/testing{,/desugar}.rs` — Group B call sites, all cited above.
- `src/cli/build.rs` and `src/builtins/general.rs` **as files** — unaffected
  here; their splits are bug-327's.

## Fix Design

The document is a cluster: each item lands independently and is independently
revertible. Two shape decisions:

**A1/A6/A7 are one arc, in this order.** A7 first (adopt `term.rs`'s
`param_types` shape in the other 21 modules) — this is mechanical and creates
the table A1 needs. Then A1 (delete the string parse; `builtin_argument_types`
reads the table). Then A6 (one package table drives all five chains, which by
then includes the argument-type chain). Doing A6 first would freeze the current
16-package chain into a new abstraction.

Rejected alternative for A1: keep the string and add a `#[test]` asserting every
`expected_arguments` string parses. This was rejected because it makes the
diagnostic wording *formally* load-bearing — the opposite of the goal — and
still cannot express an optional or alternative-typed parameter.

Rejected alternative for A4: leave the 20 resolvers in `general.rs` and re-export
them. Rejected — the point is that `general.rs`'s size is not about `general::`.

Expected output shift: **none**, for every item except A1, where collapsing the
six omitted packages into the table may change which calls resolve. That is why
A1 lands alone, with `scripts/artifact-gate.sh` run before and after.

## Phases

### Phase 1 — measurement + gate baseline (no behavior change)

- [ ] Record a clean `scripts/artifact-gate.sh <exe>` baseline at `diffs=0`.
- [ ] Add unit tests pinning today's behavior at the seams that will move:
      `builtin_argument_types` for all 22 packages (asserting today's six
      `None`s explicitly, so A1's change is visible in a diff), and the four
      `mod.rs` chains' package sets.
- [ ] Confirm the A2 three-way split (regex/strings/collections) against the
      current source before writing the macro.

Acceptance: baseline recorded; the new tests pass against unmodified code and
would fail if any chain's coverage changed.
Commit: —

### Phase 2 — mechanical items (output-neutral)

- [ ] A3 (`exact` ×15 → 1), A2 (10 uniform copies → macro; 3 stay explicit),
      A4 (20 resolvers + tests → `collections.rs`), A5 (grammar → `mod.rs`,
      two splitters → one), A8 (`io.rs` tests, `testing.rs` rename).
- [ ] B2 (one `CommandError` + one dispatch fn), B4 (`BuildOutput::label()`),
      B5 (`parse_common_option`), B6 (call the two existing helpers), B11
      (delete `hex`), B12 (rename the 20 test files).
- [ ] B1 (`cli/help.rs` + `cli/dispatch.rs`; 349 test lines to their modules),
      B10 (`coverage.rs` → `testing/coverage.rs`).
- [ ] B7 `escape` only (identical bodies); leave `anchor` pending the decision.
- [ ] B9 (single dependency builder, keeping the guarded implementation).
- [ ] B8 (`pub(crate)` decoder in `binary_repr`; delete `manifest`'s copy).

Acceptance: `scripts/artifact-gate.sh` at `diffs=0` after **each** commit;
`cargo test` green; zero modified files under any `tests/**/golden/`.
Commit: —

### Phase 3 — the argument-type arc (A7 → A1 → A6)

- [ ] A7: give the other 21 modules a `param_types` table; derive
      `expected_arguments` and `arity` from it. Assert the derived strings are
      byte-identical to today's.
- [ ] A1: `builtin_argument_types` reads the table; delete the `'['`/`" or "`
      bail and the `", "` split at `src/ir/lower.rs:2675-2678`. Land the six
      previously-omitted packages.
- [ ] A6: one package table; all five chains iterate it. `call_return_type_name`
      gains `thread` and `vector`.
- [ ] A7 tail: normalize `term::resolve_call`'s signature; delete the special
      cases at `src/builtins/mod.rs:322` and `src/syntaxcheck/builtins.rs:1072`.

Acceptance: after A7 and A6, `artifact-gate.sh` at `diffs=0`. After A1, any
non-zero diff must be inspected and explained per fixture — it is the one place
a real resolution change can appear, and an unexplained diff blocks the merge.
Commit: —

### Phase 4 — full validation

- [ ] `scripts/test-accept.sh` full run on macOS and Linux.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`.
- [ ] Diff every help screen and diagnostic message against pre-change output.

Acceptance: full suite green; no golden moved; help/diagnostic text identical.
Commit: —

## Validation Plan

- Regression tests: no new fixture. New unit tests for the merged helpers and,
  critically, for the package table in A6 — asserting the **full** 22-package
  set — so a future omission fails a test instead of drifting silently, which
  is precisely how chains 2 and 5 got into this state.
- Runtime proof: `scripts/artifact-gate.sh` at `diffs=0` on every commit — this
  *is* the proof for an output-preserving change — plus one full
  `scripts/test-accept.sh` before merge.
- Byte-identity guard: `git status` must show **zero** modified files under any
  `tests/**/golden/` directory. If a golden moves, the change is out of scope.
- Doc sync: none expected. Man/spec drift for these packages is bug-337/bug-338.
- Full suite: `cargo test`, `scripts/test-accept.sh`, `cargo clippy`.

## Open Decisions

- **A1 — does any of the six omitted packages change resolution?** Must be
  answered by running the gate, not by reasoning, before Phase 3 merges.
  Recommended: land A1 as its own commit so a revert is one `git revert`.
- **A7 — which direction does `term.rs` normalize?** Recommended: siblings adopt
  `term.rs`'s `param_types` design, then `term.rs` adopts the sibling
  `resolve_call` signature. Alternative (normalize `term.rs` to the siblings
  outright) deletes the only working example of the target shape — do not.
- **B7 — should `anchor` lowercase?** Recommended: adopt `src/doc.rs`'s
  lowercasing everywhere and regenerate the coverage HTML; alternative is to
  keep two `anchor`s and dedup only `escape`. This changes generated anchor ids
  in one output, so it needs a decision before the merge.
- **B9 — is the unguarded `package_dependencies` path a security finding worth
  its own bug?** Recommended: file it separately and let this dedup close it, so
  the record is not buried in a cleanup document.
- **B12 — prefix scheme.** Recommended `cli_*` / `rt_*` / `link_*`; alternative
  is to leave the names alone since cargo does not care.

## Summary

Twenty-one verified cleanup items across `src/builtins/` and the CLI/manifest
layer. The engineering risk is concentrated in exactly one place: **A1**, where
IR lowering currently recovers positional type information by parsing a string
written for a human, declining silently for six of twenty-two packages. That is
the only item that can change what the compiler resolves, so it lands alone and
gated. Everything else — 105 lines of `exact`, ~338 lines of package-source
boilerplate, 670 lines of `collections::` resolvers in the wrong file, a 290-line
grammar in `thread.rs`, six copy-pasted CLI dispatch arms, two duplicate option
parsers, a second `.mfp` decoder, and 20 flat test files — is mechanical and
provable by `scripts/artifact-gate.sh` staying at `diffs=0`. Left untouched: the
file splits (bug-327), the man-page and spec drift (bug-336/337/338), the
dead-code sweep (bug-326), the builtin checker table (bug-324), and every
diagnostic string in either layer.
