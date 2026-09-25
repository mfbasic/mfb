# plan-157-A: rename `os::resourcePath` → `os::appResourcePath`, optional `relative`

Last updated: 2026-09-24
Overall Effort: x-large (1d–3d)
Effort: medium (1h–2h)
Depends on: nothing

plan-157 adds one family of five host-path calls to `os`. Each one returns an
absolute path and does nothing else:

| Call | Base |
| --- | --- |
| `os::appResourcePath(relative = "")` | the build's resource directory (today's `os::resourcePath`) |
| `os::appDataPath(relative = "")` | the per-user, per-app data directory (plan-157-B) |
| `os::appCachePath(relative = "")` | the per-user, per-app cache directory (plan-157-B) |
| `os::userHomePath(relative = "")` | the user's home directory (plan-157-C) |
| `os::userDocumentsPath(relative = "")` | the user's Documents directory (plan-157-C) |

All five follow the same contract. It was settled with the user in the design
discussion and is not up for re-litigation:

- The signature is `(relative AS String = "") AS String`.
- If `relative` contains a whole `.` or `..` path component, the call raises
  `ErrInvalidPath`. A component ends at `/`, and on Windows also at `\`. This is
  today's `resourcePath` rule, `lower_resource_path`'s validate loop.
- An empty `relative` returns the base with **no trailing `/`**. A non-empty
  `relative` returns `<base>/<relative>`, joined with `/` on every target.
- The call never creates a directory and never checks that the path exists.
- If the host lookup fails, the call raises `ErrUnsupported`. Together with
  `ErrInvalidPath`, those are the only two errors.
- The app-scoped directory name is the project (module) name, the same on every
  platform and in both console and app builds.

**This sub-plan (A)** renames the existing member and brings it onto the family
contract. It is a **hard rename with no alias**. `relative` becomes optional, and
an empty `relative` now yields the bare base. Today it yields `<base>/`, because
`lower_resource_path` always stores the `/`.

**Correct behavior after A.** On every target:

- `os::appResourcePath("music/x.ogg")` returns exactly what
  `os::resourcePath("music/x.ogg")` returned before.
- `os::appResourcePath()` and `os::appResourcePath("")` return the base with no
  trailing `/`.
- `os::resourcePath` no longer resolves. Calling it is an unknown-member
  diagnostic.

References:

- `.ai/compiler.md` — the Hard Completion Gate and the fixture rules (a valid
  rt-behavior fixture plus an invalid syntax fixture per function).
- `.ai/codegen-invariants.md`, `.ai/arch-abi.md` — the Win64 acquisition-window
  hazard the `appResourcePath` body already respects (`gen_paths.rs`, the
  `emit_executable_path_into` Windows arm comment).
- `.ai/testing-gates.md` — byte-identity, `test-accept.sh`, and the concurrent
  actual-dir hazard.
- `.ai/man-content.md` — the man page for the renamed member.
- `.ai/specifications.md`, `.ai/spec-content.md` — `mfb spec stdlib os`
  (`src/docs/spec/stdlib/14_os.md`, "Build resources (resourcePath)").

## Prerequisites

This is the one hard gate for all of plan-157; B, C and D point here.

| Must be true | Command | Status |
|---|---|---|
| Worktree `paths` exists on branch `worktree-paths` | `git -C .claude/worktrees/paths rev-parse --abbrev-ref HEAD` → `worktree-paths` | MET (2026-09-24) |
| The debug compiler builds at the worktree's HEAD | `cargo build` → exit 0 | MET (2026-09-24, `cargo build` → Finished) |
| A glibc Linux box answers ssh (runtime proof, plan-157-D) | `ssh -o ConnectTimeout=5 -p 2226 test@127.0.0.1 true` → exit 0 | MET (2026-09-24; 2222/2223/2227/2228/2229/2232 refused) |
| The Windows box answers ssh (runtime proof, B/C/D) | `ssh -o ConnectTimeout=5 -p 2230 test@127.0.0.1 echo ok` → `ok` (Windows has no `true`) | MET (2026-09-24, re-run at A start) |

Everything below assumes these conditions hold.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites**, not only
> the one that blocked you.

## 1. Goal

- `os::appResourcePath` exists with signature `(relative AS String = "") AS String`,
  and `os::resourcePath` does not.
- For a non-empty `relative`, the result bytes are identical to the old call's.
- For an empty `relative`, the result is the base with no trailing `/`. This
  holds on both lowerings: the copying helper and the in-place self-update arm
  (`s = os::appResourcePath(s)` with `s = ""`).

### Non-goals (explicit constraints)

- **No alias.** `os::resourcePath` is removed outright. Nothing forwards to it,
  and no deprecation diagnostic is added.
- **No change to the base rules.** `resource_base_offset`
  (`src/codegen/builtins/os/gen_paths.rs`) and the four-row base table stay
  exactly as they are.
- **The error codes stay the same.** That means `ErrInvalidPath` (`77030002`)
  and `ErrUnsupported` (`77050007`).
- **History stays untouched.** `planning/completed/**`, `bugs/completed/**` and
  `planning/old_man/**` keep the old name, because they record what was true.
  The same goes for `planning/plan-143-findings/**`, `planning/plan-144-findings/**`
  and `planning/coverage-baseline.txt`.
- The rust-side symbol names (`func_resource_path.rs`, `lower_resource_path`,
  `GrowKind::ResourcePath`, `string_resource_base`) are renamed **only** where
  the old name would now mislead. Each rename is listed in a task below.
  Otherwise the churn buys nothing.

## 2. Current State

- **The descriptor.** `src/codegen/builtins/os/func_resource_path.rs:register`
  registers `name: "resourcePath"`. It has one `Parameter` (`relative`,
  `DefaultValue::None`), declares `errors: ["ErrUnsupported", "ErrInvalidPath"]`,
  and its body is `Body::abi_function(lower_resource_path)`.
- **The lowering.** `lower_resource_path`, in the same file, runs these steps:
  1. It captures the argument.
  2. It rejects any `.`/`..` component, with `\` also counted on Windows.
  3. It acquires the executable path (`gen_paths.rs:emit_executable_path_into`).
  4. It strips `strip` components.
  5. It allocates `prefix + 1 (+ suffix + 1) + arg_len + 9`.
  6. It copies the prefix, stores `/` unconditionally
     (`emit_store_byte_advance(b'/', …)`), then copies the suffix and the
     argument.

  So `relative = ""` yields `<base>/` today.
- **The in-place arm.** `src/codegen/collection/assign/string_self_update.rs`
  has `GrowKind::ResourcePath` (`STRING_GROW_FNS` row
  `("resourcePath", 1..=1, …)`). `emit_resource_prefix` builds
  `<exe dir>[/<suffix>]/` (trailing `/` included) into a frame buffer, caching
  `os::executablePath()`'s block in `string_resource_base`. The arm then grows
  `s` by that prefix. For `s = ""` it also produces `<base>/`.
- **The self-update census.** `src/codegen/collection/assign/self_update.rs`
  `SELF_UPDATE_TABLE` has a row `function: "os::resourcePath"`
  (`ArmId::StrGrow`), and `STRING_SELF_UPDATE_SPELLINGS` maps
  `("os.resourcePath", "resourcePath")`. The census test
  `self_update_census_covers_every_registry_overload` fails if a
  self-update-shaped registry member has no row.
- **The arity bound.** `STRING_GROW_FNS`'s `1..=1` is the arity the arm
  accepts. With an optional `relative` a caller can also write
  `os::appResourcePath()`, but that form has no `s` argument, so it is not a
  self-update and the bound stays `1..=1`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Live files naming `resourcePath` (excluding `planning/`, `bugs/`, `.claude/`) | 61 files, 228 hits | `rg --hidden -c resourcePath --glob '!.git/**' --glob '!planning/**' --glob '!bugs/**' --glob '!.claude/**'` → `61 files, 228 hits` |
| …of those, under `src/` | 31 files | same command saved to a file, then `grep -c "^src/"` |
| …under `tests/` | 24 files | same, `grep -c "^tests/"` |
| …examples | 3 files (`audio`, `dungeon`, `wind`) | same, `grep -c "^examples/"` |
| …`.ai/` notes | 3 files (`arch-abi.md` 5, `collections.md` 1, `testing-gates.md` 1) | same command, `\| grep '^\.ai/'` |
| Live planning files naming it (NOT history) | 1 (`planning/todo.md`) | `rg -c resourcePath planning/todo.md` → 1 |
| rt-behavior fixture directories named after it | 2 (`func_os_resourcePath_valid`, `func_os_resourcePath_reads_resource`) | `ls tests/rt-behavior/os \| grep resourcePath` |
| Invalid (syntax) fixture for it | **0**, a gap `.ai/compiler.md` forbids | `ls tests/syntax/os \| grep -i resource` → nothing |
| Hand-maintained call-name tables listing `os.resourcePath` | 28 quoted-name lines in 16 files (tables: `data_objects.rs` ×2, `linux_common/{mod,plan}.rs`, `macos_aarch64/{mod,plan}.rs`, `win_x86_64/{mod,plan}.rs`, `registry/mod.rs` ×2, `audit/collect/source.rs` ×2, `plan/symbols.rs`, `string_self_update.rs` ×5, `self_update.rs` ×3; the rest are the descriptor, `os/mod.rs` tests, `builtins/tests/os.rs`, `validate/mod.rs` tests) | `rg -n '"os\.resourcePath"\|"resourcePath"' src --glob '!src/docs/**' \| cut -d: -f1 \| sort \| uniq -c` → 28 lines, 16 files |

### Verified properties

- **Byte-identity will diff, and that is expected.** The `os` byte-identity
  fixture calls `os::resourcePath` (`tests/byte-identity/os/src/main.mfb:25`),
  and all 5 of its `.ncodesum` goldens exist
  (`ls tests/byte-identity/os/golden/*.ncodesum | wc -l` → 5). Renaming the
  helper's symbols and changing the join changes those bytes. A diff there
  means the plan is working.
- **`.ai/testing-gates.md` is stale about Windows.** It says Windows carries 21
  of 24 byte-identity fixtures and that `os` has none "because
  `os.resourcePath`" is unsupported. The tree says otherwise:
  `find tests/byte-identity -name '*.windows-x86_64.ncodesum' | wc -l` → 27,
  `… '*.macos-aarch64.ncodesum' | wc -l` → 28, and `os` has a Windows golden.
  Task A5 fixes this (a doc bug found here gets fixed now).

## 3. Design Overview

This is a mechanical rename plus one semantic change: an empty `relative` means
the base.

- **Byte-identity is not the gate.** Every `os` target's `.ncodesum` is
  expected to change, and all five `os` goldens get regenerated. Regenerate
  only after the full suite runs, per AGENTS.md. The gate is the rt-behavior
  fixtures (the valid fixture's output plus a new empty-`relative` line) and the
  new invalid fixture.
- **Where the risk sits:** the in-place arm's empty-value case. The copying
  helper has a simple fix: branch around the `/` store when `arg_len == 0`. The
  arm builds a prefix that already ends in `/`, so for an empty `s` it must
  write the prefix *minus* its trailing byte. That path is new code on a
  register-lifetime-sensitive lowering (`.ai/compiler.md`, "Native Codegen
  Register Lifetimes"), and a fixture has to exercise it.
- **Rejected: keeping `resourcePath` as an alias.** The user decided this.
  Every caller is in this repo, and an alias would be the fallback code
  AGENTS.md forbids.

## 4. Detailed Design

### 4.1 Descriptor

In `func_resource_path.rs`, the file is renamed to `func_app_resource_path.rs`
(its module doc names the member) and changes as follows:

- `name: "appResourcePath"`.
- The `relative` parameter gets
  `default: DefaultValue::Fill { type_name: ParameterType::String, expr: "\"\"" }`.
  The precedent is csv's `delimiter` (`csv/func_parse_stream.rs`).
- The parameter `desc` gains "Omit it (or pass `\"\"`) for the resource
  directory itself."
- `lower_resource_path` → `lower_app_resource_path`. `void_result("os.appResourcePath")`.
- `INTRO`/`DESC`/`EX` are rewritten for the new name, per `.ai/man-content.md`.
  `DESC` gains one paragraph: an empty `relative` returns the base directory
  with no trailing `/`. The man page cross-links the family by name only; the
  pages for the other four do not exist until B and C land.

### 4.2 Empty `relative` → the bare base (copying helper)

In `lower_app_resource_path` step 4 (total length), `extra` counts the joining
`/`. Change the layout to `prefix [+ "/" + suffix] [+ "/" + relative]`:

- The `/` before the suffix is unconditional. In an app build it is part of the
  base (`…/Contents` + `/Resources`).
- The `/` before `relative` is emitted only when `arg_len != 0`. The total
  length is `prefix_len + suffix_extra + arg_len + (arg_len != 0 ? 1 : 0)`,
  where `suffix_extra` is `0` or `suffix.len() + 1`. Compute it with a
  compare-and-branch; there is no conditional-select op in `abi::`.

### 4.3 Empty value → the bare base (in-place arm)

`emit_resource_prefix` keeps producing `<base>/`. In the `GrowKind::ResourcePath`
arm (renamed `GrowKind::AppResourcePath`), after `emit_reject_dot_components`:

- Load `value_len`. When it is 0, use `prefix_len - 1` as the effective prefix
  length. The frame buffer is unchanged, so the trailing `/` is simply not
  copied.
- `newlen_slot` then gets `prefix_len - 1`.

`GrowWrite::Prefix { ptr_slot, len_slot }` takes the length from a slot, so the
arm stores the adjusted length into a fresh slot and hands that over. Every
value stays in a frame slot across the helper calls, as the file's convention
requires.

### 4.4 Tables

Each site in the census table above is updated as follows:

- `"os.resourcePath"` → `"os.appResourcePath"`
- `"resourcePath"` → `"appResourcePath"` (bare-name tables:
  `STRING_GROW_FNS`, `STRING_SELF_UPDATE_SPELLINGS`, `plan/symbols.rs`)

The site comments keep their `plan-55-B`/`bug-454`/`plan-146-D` citations,
because those record why the entry exists.

## Compatibility / Format Impact

- **Source-breaking:** a program calling `os::resourcePath` stops compiling. The
  user accepted this; the three in-tree examples are migrated in A4.
- **Behavior change:** `os::appResourcePath("")` returns `<base>`, not `<base>/`.
- **No change:** the base per build mode, the error codes, the resource
  manifest format, or the bundle layout.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial and say what remains. Mark a moot
> task `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` the moment a phase
> lands. **An unticked box means NOT DONE.**

### Phase A1: rename and the optional parameter

This phase makes the compiler accept only the new name, and it lands alone
because every in-tree caller moves in the same commit.

- [x] Rename `src/codegen/builtins/os/func_resource_path.rs` →
      `func_app_resource_path.rs`. Update the `mod` line and the `register`
      call in `os/mod.rs`, and apply §4.1. — done: `git mv` + `mod func_app_resource_path`; `relative` is `DefaultValue::Fill { String, "" }` (raw value, as `strings::padRightToWidth`'s `" "`: `ir/lower.rs` pushes `expr` verbatim as the `Const`).
- [x] Update `os/mod.rs`'s `MODULE_DESC`/module doc sentences (`:13`, `:16`,
      `:74`) and its unit tests (`:150`, `:222`) to the new name. — done (identifier rename across the file).
- [x] Update the 28 quoted-name lines (§4.4). Command:
      `rg -n '"os\.resourcePath"|"resourcePath"' src --glob '!src/docs/**'`.
      It must return 0 lines afterwards. — done: `rg -n '"os\.resourcePath"|"resourcePath"' src --glob '!src/docs/**'` → 0 lines.
- [x] Rename, in `string_self_update.rs`/`self_update.rs`/`builder/mod.rs`/
      `builder_values.rs`: — done.
  - `GrowKind::ResourcePath` → `AppResourcePath`
  - `emit_resource_prefix` → `emit_app_resource_prefix`
  - `string_resource_base[_env]` → `string_app_resource_base[_env]`
  - `prescan_string_resource_base` → `prescan_string_app_resource_base`
  - the `SELF_UPDATE_TABLE` row's `function`/`probes`.
- [x] Update every remaining `src/` comment that names the call. Scope: the
      `rg -c resourcePath src --glob '!src/docs/**'` rows above. Afterwards,
      `rg resourcePath src --glob '!src/docs/**'` must return only
      `lower_app_resource_path`-style identifiers that contain the substring,
      and no bare `resourcePath` call spelling. — done: `rg resourcePath src tests examples .ai Cargo.toml --glob '!**/golden/**' | rg -v appResourcePath` → 0 lines.
- [x] Update the unit tests: `src/codegen/builtins/tests/{app_surface,os,corpus}.rs`,
      plus `tests/codegen/codegen_win64_resource_path.rs`,
      `tests/codegen/codegen_helper_scratch_release.rs`,
      `tests/runtime/rt_trapped_call_capability_gate.rs`, and
      `tests/runtime/inplace_self_update/{cases,field_expect}.tsv`.
      Rename `codegen_win64_resource_path.rs` →
      `codegen_win64_app_resource_path.rs`, and grep `Cargo.toml` for its
      `[[test]]` entry, which `rg -n resource_path Cargo.toml` finds. — done, incl. `Cargo.toml` `[[test]] codegen_win64_app_resource_path` and `_mfb_rt_os_os_appResourcePath` in `codegen_helper_scratch_release.rs`.

Acceptance: the compiler accepts `os::appResourcePath(...)`, and
`os::resourcePath` is an unknown member.
  Check: `cargo build && cargo test --bin mfb os` → pass (est. 6 min, since the
  registry tests live in the bin target, per `.ai/testing-gates.md`, "Compiler
  tests live in the bin target"). Then run
  `printf 'IMPORT os\nIMPORT io\nSUB main()\n  io::print(os::resourcePath("x"))\nEND SUB\n'`
  as a scratch project in `/tmp` → the build fails naming `resourcePath`
  (est. 1 min).
  Result: `cargo test --bin mfb os` → `test result: ok. 586 passed; 0 failed; 1 ignored`. The scratch `os::resourcePath` build → `SYMBOL_UNKNOWN_IDENTIFIER … Built-in package `os` does not export `os.resourcePath``.
Commit: 10c43e3bc

### Phase A2: empty `relative` means the base

The copying helper and the in-place arm now both return `<base>` for an empty
`relative`.

- [x] Apply §4.2 in `lower_app_resource_path`. — done: the `/` before `relative` is branched on `arg_len == 0`; the suffix `/` is unconditional.
- [x] Apply §4.3 in the `AppResourcePath` grow arm. — done: the effective length goes to a fresh `str_respath_used_len` slot, one byte shorter for an empty value.
- [x] Rename `tests/rt-behavior/os/func_os_resourcePath_valid` →
      `func_os_appResourcePath_valid`, and update its `project.json` `name` and
      source. Add lines printing: — done; run prints `FALSE TRUE TRUE TRUE TRUE` after the old 7 lines. `-ncode` shows 7 `bl _mfb_rt_os_os_appResourcePath` = the 7 copying sites, so both in-place sites took the arm.
  - `strings::endsWith(os::appResourcePath(), "/")` → `FALSE`
  - `os::appResourcePath() = os::appResourcePath("")` → `TRUE`
  - `os::appResourcePath("x") = os::appResourcePath() & "/x"` → `TRUE`
  - an in-place `MUT s AS String = ""` / `s = os::appResourcePath(s)` equal to
    `os::appResourcePath()` → `TRUE`
  - a non-empty in-place `s = "a/b"` equal to `os::appResourcePath("a/b")` →
    `TRUE`
- [x] Rename `tests/rt-behavior/os/func_os_resourcePath_reads_resource` →
      `func_os_appResourcePath_reads_resource` (name, source). — done.
- [x] Update `tests/rt-behavior/strings/self-update-grow-valid/src/main.mfb`
      and `tests/rt-behavior/native/libsnd-playback-rt/src/main.mfb` to the new
      name. — done. The rename regex also hit the helper `SUB resourcePaths` and two printed tags; reverted so only the call spelling changes.
- [x] Add the missing invalid fixture
      `tests/syntax/os/func_os_appResourcePath_invalid`. It must reject a wrong
      argument type (`os::appResourcePath(1)`) and a wrong arity
      (`os::appResourcePath("a", "b")`), with golden `build.log` diagnostics. — done: `TYPE_CALL_ARGUMENT_MISMATCH` (Integer vs [String]) and `TYPE_CALL_ARITY_MISMATCH` (2 vs 0 to 1); golden `build.log` written from the run.
- [x] Update `tests/acceptance/src/os.mfb` (4 call sites) and
      `tests/byte-identity/os/src/main.mfb:25`. — done.

Acceptance: the renamed valid fixture prints `FALSE` then four `TRUE`s, the old
lines are unchanged, and the invalid fixture reports both diagnostics.
  Check: `FILTER=os scripts/test-accept.sh target/debug/mfb target/accept-actual-156a`
  → only the renamed/new fixtures report new goldens, and every other `os`
  fixture passes (est. 4 min). Use a private actual dir, per `.ai/testing-gates.md`
  ("Concurrent test-accept clobbers actuals").
  Result: `scripts/test-accept.sh target/debug/mfb target/accept-actual-156a 'func_os_*' 'os-*' 'self-update-grow-valid' 'libsnd-playback-rt'` → 50 ran, 10 mismatches, all on the expected list (the renamed fixtures' `.ast`/`.ir`/`build.log`, `self-update-grow-valid`'s, and the new invalid fixture's missing golden, since written). No other `os` fixture changed.
Commit: 10c43e3bc

### Phase A3: docs, spec, and `.ai` notes

- [x] Update `src/docs/spec/stdlib/14_os.md`: — done: signature `= ""`, empty-`relative` paragraph citing `lower_app_resource_path` and `emit_app_resource_prefix`; error rows renamed.
  - "Build resources (resourcePath)" → "Build resources (appResourcePath)",
    with the new signature, the empty-`relative` rule, and `[[path:Symbol]]`
    citations re-pointed at `func_app_resource_path.rs:lower_app_resource_path`.
  - The error table rows at `:280`/`:281`.
  - The line at `:267`.
- [x] Update the other spec topics: `src/docs/spec/architecture/06_native.md`,
      `memory/05_collections.md`, `tooling/01_project-manifest.md` (one hit
      each). — done.
- [x] Update the `.ai/arch-abi.md` (5), `.ai/collections.md` (1) and
      `.ai/testing-gates.md` (1) references. In `testing-gates.md` also correct
      the stale Windows fixture count with the measured `27`/`28` and the fact
      that `os` has a Windows golden (§2 Verified properties). — done; `testing-gates.md` now states the measured 29 Unix / 27 Windows goldens (only `link-const-pins` and `crypto-ec-valid` lack Windows).
- [x] Update `planning/todo.md` (1 hit). — done: #12 now points at plan-157 and strikes `os::homePath`.

Acceptance: no live doc names the old call, and the spec citations resolve.
  Check: `rg --hidden -c resourcePath --glob '!.git/**' --glob '!planning/completed/**' --glob '!bugs/**' --glob '!planning/old_man/**' --glob '!planning/plan-14[34]-findings/**' --glob '!planning/coverage-baseline.txt' --glob '!.claude/**' --glob '!planning/plan-157-*'`
  → only identifiers that contain `app_resource_path`/`AppResourcePath`, and no
  bare `resourcePath`. Then `scripts/spec-census.sh --citations` → 0 dangling
  in `14_os.md`, and `cargo test --bin mfb spec` → pass (est. 5 min).
  Result: `rg` leftover sweep → 0 bare `resourcePath` outside history. `scripts/spec-census.sh --citations` → `MISS-PATH 0 MISS-LINE 0 MISS-SYMBOL 0`. `cargo test --bin mfb spec` → `43 passed; 0 failed`.
Commit: 10c43e3bc

### Phase A4: examples, man page render, and golden refresh

- [x] Migrate `examples/audio/src/main.mfb` (2), `examples/dungeon/src/render.mfb`
      (1) and `examples/wind/src/land.mfb` (1). Build each with
      `target/debug/mfb build examples/<x>`, which must exit 0. — done: `dungeon` and `wind` build (exit 0). `audio` needs the gitignored installed `libsnd.mfp`; with it copied from the main tree it fails on `SoundFile`/`SoundInfo` unknown types, which is package type resolution, not this rename. It fails identically at `main` HEAD, so the bug predates this plan; it is fixed here (Correction A-1). After the fix `audio` builds too.
- [x] Render the page with `target/debug/mfb man os appResourcePath`. The
      output must show the `relative` parameter as optional, the Errors section
      must list both codes, and `scripts/man-census.sh --memory-scope` must
      report 0 unclassified hits. — done: `(opt)` parameter, Errors lists `77030002` and `77050007`; `MFB=./target/debug/mfb scripts/man-census.sh --memory-scope os` → `unclassified memory-vocabulary hits: 0`.
- [x] Run `scripts/man-run-examples.sh os --run`, and the renamed page's
      examples must pass. — done: `examples: 23 built: 23 ran: 23 not run: 0 failed: 0`.

Acceptance: the examples build and the page renders and runs.
  Check: the three commands above (est. 4 min).
  Result: `examples/{audio,dungeon,wind}` → `Wrote executable` for each (audio after Correction A-1). Man and example results are on the task lines.
Commit: 10c43e3bc, 93be6f452

(The byte-identity `.ncodesum` and AST/IR golden regeneration for the `os`
fixture waits for plan-157-D's full-suite run. Until then those goldens are
**expected red** on exactly `tests/byte-identity/os/**`, the renamed fixtures,
and `self-update-grow-valid`. Record the red set in Corrections if anything
else goes red.)

## Validation Plan

- **Tests:** the renamed valid fixture (with the new empty-`relative` and
  in-place lines), the reads-resource fixture, and the new invalid fixture. The
  unit tests in `builtins/tests/os.rs` must pass, including the
  every-backend-lowers test at `:438`.
- **Coverage check:** the in-place empty-value path is exercised by the
  `s = os::appResourcePath(s)` with `s = ""` line. Build that fixture with
  `--nir` and confirm the call site took the grow arm, i.e. no
  `_mfb_rt_os_appResourcePath` call at that site.
- **Runtime proof:** the renamed valid fixture's run on macOS (A2). Linux and
  Windows execution happens in plan-157-D.
- **Doc sync:** the A3 list.
- **Final gate:** deferred to plan-157-D (one full-suite run for the whole feature).

## Open Decisions

- None. The name, the lack of an alias, and the default were decided by the user.

## Corrections

- **A-0: renumbered from plan-156 to plan-157 at the end.** While this work ran
  on `worktree-paths`, `main` landed an unrelated plan-156 ("Thread and
  ThreadWorker become RES resources", `9f49cf2a8`). Every file, citation and
  code comment of this plan was renamed to plan-157 before merging. The commit
  messages on the branch still say plan-156; they cannot be changed without
  rewriting history.

- **A-1: `examples/audio` did not build at `main` HEAD.** A `/tmp/mfb156head`
  detached worktree at `main`, with the installed `libsnd.mfp` copied in, gave
  the same 3 `SYMBOL_UNKNOWN_TYPE` errors (`SoundFile`, `SoundInfo`) as this
  branch. The cause: commit `b97744633` added `RES music AS SoundFile STATE
  SoundInfo` unqualified, but imported package types must be qualified (the
  bug-480 rule, commit `61aa98f83`). With `libsnd::SoundFile` /
  `libsnd::SoundInfo` the example builds (`Wrote executable to
  examples/audio/build/audio.out`). This was fixed in A4 as its own commit. The
  plan did not predict it, because A4 assumed all three examples built before
  the rename.
- **A-2: the rename regex over-matched.** `(?<![A-Za-z_])resourcePath` also
  renamed the user helper `SUB resourcePaths` and the printed tags
  `resourcePathEmpty`/`resourcePathLoop`/`"resourcePath"`/`"g.resourcePath"`
  in `self-update-grow-valid`. These were reverted, so that fixture's run output
  is unchanged. Its golden diff is only the call spelling in the `.ast`/`.ir`.

## Summary

This sub-plan is the mechanical rename with one real semantic change: an empty
`relative` means the base. The risk is in the in-place arm's empty-value path,
and a fixture proves it.
