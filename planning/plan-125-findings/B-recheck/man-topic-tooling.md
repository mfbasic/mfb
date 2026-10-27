### 1. fmt page's own example fails: a directory must be a project root, not a source folder
UNIT:      man-topic:tooling
PAGE:      fmt
CATEGORY:  overview-mismatch
CLAIM:     "A directory argument is walked recursively for MFBASIC source files." and the example "mfb fmt --check --indent 4 src"
VERDICT:   wrong
EVIDENCE:  In a fresh `mfb init demo` project, `mfb fmt --check --indent 4 src` printed `src/project.json:1 error[2-200-0001 PROJECT_JSON_MISSING]: project.json is required` / `error: project validation failed`, exit 1. `mfb fmt --check` (project root) and `mfb fmt --check src/main.mfb` (single file) both succeeded. Source: `src/cli/fmt.rs:format_path` requires `project.json` in a directory argument and formats `ast::selected_source_paths(path, &manifest)`, i.e. the manifest's selected sources, not a recursive walk. The overview's "operates on files or directories given on the command line" repeats the same over-promise.
SUGGESTED: "A directory argument must be a project root (it must contain project.json); the files the project selects are formatted. A single .mfb file may be given instead." Change the example to `mfb fmt --check --indent 4` run from the project root, or `mfb fmt --check --indent 4 src/main.mfb`. In the overview, say "operates on a project directory or a single source file".

### 2. Sibling pages disagree on whether `--opt=value` is documented
UNIT:      man-topic:tooling
PAGE:      fmt (vs audit)
CATEGORY:  consistency
CLAIM:     audit's Options table: "--format <type> … --format=json also works." fmt's Options table documents only "--indent <N>".
VERDICT:   inconsistent
EVIDENCE:  `src/cli/fmt.rs:run_fmt_command` accepts `arg.strip_prefix("--indent=")`; probe `mfb fmt --indent=4 --check` in the demo project ran and reported `Not formatted: …/src/main.mfb`, exit 1. One page advertises the `=` form, the sibling hides it.
SUGGESTED: Add "--indent=N also works." to fmt's `--indent` row so both commands describe the option syntax the same way.

### 3. fmt documents no exit codes while audit documents a four-row table
UNIT:      man-topic:tooling
PAGE:      fmt (vs audit)
CATEGORY:  consistency
CLAIM:     fmt: "exit non-zero if any file would change" and "reports problems on standard error and exits non-zero"; audit has an explicit Exit/Meaning table.
VERDICT:   inconsistent
EVIDENCE:  Probes in the demo project: `--check` with unformatted file → exit 1 with `FMT_CHECK_FAILED`; `mfb fmt nonexist` → exit 1; `mfb fmt --bogus` → exit 2; `mfb fmt --indent 300` → exit 2 (`src/cli/fmt.rs:run_fmt_command` returns 2 for usage errors, `format_path` Ok(false)/Err → 1). The audit table was verified: `--locked` with no lockfile → 1, `--bogus` / bare `--format` / two paths → 2, `mfb audit src` (no manifest) → 3. A CI author reading the unit gets a precise contract for one command and "non-zero" for the other.
SUGGESTED: Give fmt the same shape of table: 0 = all files formatted (or rewritten); 1 = `--check` found unformatted files, or a file/project could not be read or formatted; 2 = bad command line (unknown flag, bad or missing `--indent` value, a second path).

### 4. `mfb doc` meets the overview's own scope criterion but has no page anywhere in `mfb man`
UNIT:      man-topic:tooling
PAGE:      package-wide
CATEGORY:  coverage
CLAIM:     "This topic documents the developer-facing commands that read or rewrite source rather than produce a build artifact: mfb fmt and mfb audit."
VERDICT:   missing
EVIDENCE:  `mfb --help` lists `doc [options] [path]  Render HTML docs from package or file source` and `mfb doc --help` says "Render HTML documentation from source files or a project directory." It reads source and produces no build artifact, so it fits the stated criterion exactly. `grep -rn 'mfb doc' src/docs/man` finds only passing mentions (tour, optimizations, debug-external); no topic or page documents its options. A developer asking "how do I generate docs for my package" cannot find an answer from `mfb man`.
SUGGESTED: Either add a `doc` subtopic to tooling (synopsis, options, what a DOC block becomes) or narrow the overview sentence so it no longer claims to be the home of that category: "This topic covers the two source-checking commands, mfb fmt and mfb audit; mfb dbg is covered by mfb man debug."

I checked and found no problem with: the audit example output (byte-identical to a fresh `mfb init demo` audit), the audit exit table, `--format=json` producing `"schema": "mfb.audit.v1"`, the fmt nesting cap (`src/fmt.rs:MAX_NESTING_DEPTH = 1024`, `src/ast/stmt.rs:MAX_STMT_DEPTH = 256`), the `--indent` range 0..256 (`MAX_INDENT`), every cross-reference (`mfb man errors`, `mfb man types list`, `mfb man debug`, `mfb spec tooling`, `mfb spec tooling fmt`, `mfb spec tooling audit-format` all render), and the finding categories named on the audit page, which are a fair summary of the 24 `AUDIT-*` codes in `src/audit` with the full catalog reachable through the spec link the page already gives.
