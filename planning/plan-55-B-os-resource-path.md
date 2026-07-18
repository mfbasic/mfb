# plan-55-B: `os::resourcePath()` builtin

Last updated: 2026-07-17
Effort: medium (1h–2h)
Depends on: plan-55-A (defines where resources are copied), plan-51-A (Linux AppDir
layout — only for the Linux `--app` base; console + macOS land without it)

`os::resourcePath(relative As String) As String` returns the **absolute** on-disk
path of a resource that plan-55-A copied into the build output. The same call resolves
correctly regardless of build shape, because the base directory is derived at runtime
from the running executable's own location and a build-mode-selected offset baked into
the binary:

- **console** — resources sit beside the executable in `build/`; base = the executable's
  directory. `resourcePath("music/x.ogg")` → `<exe-dir>/music/x.ogg`.
- **macOS `--app`** — executable at `…/Contents/MacOS/<name>`, resources at
  `…/Contents/Resources/`; base = strip `MacOS/<name>`, append `Resources`.
- **Linux `--app`** (AppDir / sealed AppImage) — executable at `…/usr/bin/<name>`,
  resources at `…/usr/share/<name>/`; base = strip `bin/<name>`, append `share/<name>`.

The behavioral outcome: a program built from `examples/audio` calls
`os::resourcePath("music/Mozart1.ogg")` and receives an absolute path that `fs::open`
opens, whether run as `./build/audio`, from a macOS `.app` double-click (arbitrary
cwd), or from inside a mounted `.AppImage`.

References (read first):

- `src/target/shared/code/os.rs:1380` — `lower_executable_path`: the exe-path
  acquisition (`_NSGetExecutablePath` on macOS, `readlink("/proc/self/exe")` on
  Linux) this reuses, and the `build_string_from_cstr`/`build_string_from_len`
  string builders it calls. `:175` is the call dispatch to extend.
- `src/builtins/os.rs` — frontend registration (`is_os_call`, `resolve_call`,
  `arity`, `call_param_names`, `call_return_type_name`, `expected_arguments`); every
  one gains a `RESOURCE_PATH` arm. This is the first `os::` call taking an argument
  that is not an env name.
- `src/target/shared/runtime/os_specs.rs:129` — `OS_EXECUTABLE_PATH_SPEC`; add
  `OS_RESOURCE_PATH_SPEC` beside it (params `["String"]`, returns `String`).
- `src/target/shared/code/stdin_broadcast.rs:256,371` — the precedent for baking a
  build-time value into a runtime helper (`stdin_log_cap` → `move_immediate`), and
  `src/target/shared/nir/mod.rs:24` — `stdin_log_cap` carried on the NIR module;
  `build_mode` is threaded the same way (it already reaches `nir::lower_module`).
- `plan-55-A` §4.3 — the resource-directory contract this must resolve against
  (the worked-example table is the shared source of truth).
- `src/docs/spec/stdlib/14_os.md:165`, `src/docs/man/builtins/os/` — the doc surfaces.

## 1. Goal

- `os::resourcePath("music/x.ogg")` compiles as an `os::` builtin returning `String`,
  rejects a non-`String` / non-1 argument at the frontend, and appears in the `os`
  man/spec.
- At runtime it returns an **absolute** path (leading `/`, no `..` segments) formed as
  `<base>/<relative>`, where `<base>` is derived from the executable's own path and the
  build-mode offset, per §4.2's table — so console, macOS `--app`, and Linux `--app`
  each resolve to the directory plan-55-A copied resources into.
- On a platform where the executable path cannot be obtained (the same failure
  `os::executablePath` already handles), it returns `ErrUnsupported` via the standard
  error-result path, catchable with `TRAP`.
- A `relative` argument containing a `.` or `..` **path component** is rejected at
  runtime with `ErrInvalidPath` (catchable with `TRAP`) — a resource path must not
  navigate the tree. A dot *inside* a filename (`x.ogg`) is fine; only a whole
  component that is exactly `.` or `..` is an error.

### Non-goals (explicit constraints)

- **No manifest or copy change.** Where files land is plan-55-A. This sub-plan only
  computes the path to them.
- **No dependence on `$APPDIR` or any env var.** plan-51-A's `AppRun` is a bare symlink
  (no wrapper exporting `APPDIR`), so a directly-run AppDir has no such var. Resolution
  is purely off the executable path, which `/proc/self/exe` gives correctly for both a
  symlinked AppDir run and a FUSE-mounted `.AppImage`.
- **No change to `os::executablePath`'s observable result.** Its acquisition code is
  factored out for reuse; its output string is byte-identical.
- **No new arch-specific code.** `os::` helpers are emitted once as arch-neutral vreg
  IR (via `abi::` helpers); `resourcePath` is one shared body, like
  `lower_executable_path`, not one per backend.
- **The relative argument is validated, not silently sanitized.** A `.`/`..` component
  is an `ErrInvalidPath` error (§4.4), never stripped or normalized away. A leading `/`
  is left as-is (it simply produces a `<base>//…` that still resolves under the base);
  only `.`/`..` components are rejected, since those are the ones that navigate out.

## 2. Current State

### 2.1 `os::executablePath` already gets the absolute exe path on both platforms

`lower_executable_path` (`src/target/shared/code/os.rs:1380`) branches on target:
macOS calls `_NSGetExecutablePath(buf, &size)`; Linux `readlink("/proc/self/exe", …)`.
Both yield an absolute path in a frame buffer, then hand it to `build_string_from_cstr`
(macOS, NUL-terminated) or `build_string_from_len` (Linux, byte count). The failure
path emits `ERR_UNSUPPORTED` and an error-result. `resourcePath` needs the *same
buffer*, so §4.1 factors the acquisition into a helper both call.

### 2.2 `os::` calls are registered in five parallel tables

`src/builtins/os.rs` has one `const` per call and five metadata functions
(`is_os_call`, `call_param_names`, `call_return_type_name`, `resolve_call`,
`expected_arguments`, `arity`) that each match on it. Every existing call is nullary or
takes env-name strings; `resource_path` is the first `String → String` unary call, so
`resolve_call` gets a new arm shaped like `GET_ENV`'s (one `String`) but returning
`String`.

### 2.3 Build-time values already ride the NIR module into helper codegen

`stdin_log_cap` (`src/target/shared/nir/mod.rs:24`) is computed at lowering from the
manifest and consumed by the stdin-broadcast helper via `move_immediate`
(`stdin_broadcast.rs:371`). `build_mode` (`NativeBuildMode`) is passed into
`nir::lower_module` already. So the resource-base offset — a function of `build_mode`
and the module name — is derivable at helper-emission time with no new plumbing
through `write_executable`; it is baked into the `resourcePath` helper as constants,
exactly as `stdin_log_cap` is baked into the broadcast helper.

## 3. Design Overview

Four pieces:

1. **Factor exe-path acquisition** (§4.1) — split `lower_executable_path` into
   `emit_executable_path_into(buf, len, fail_label)` (fills a frame buffer, branches to
   `fail` on error) + the existing string-build tail. `executablePath` keeps its exact
   behavior; `resourcePath` reuses the front half.
2. **Frontend registration** (§4.3) — add `RESOURCE_PATH` to the five tables in
   `src/builtins/os.rs` and `OS_RESOURCE_PATH_SPEC` to `os_specs.rs`.
3. **The base offset** (§4.2) — a `(strip_components, suffix)` pair chosen by
   `build_mode`, baked into the helper. `strip_components` is how many trailing
   path components (including the exe filename) to drop; `suffix` is appended after.
4. **The helper body** (§4.4) — validate the argument has no `.`/`..` component
   (else `ErrInvalidPath`), acquire exe path, scan backward to drop `strip_components`
   slash-delimited components, concatenate `prefix + "/" + suffix + "/" + argument`
   into an arena `String`.

The correctness risk concentrates in **§4.4's backward scan + concatenation** — raw
vreg string work, the same class as `build_string_from_*`. It is bounded (a single
backward pass counting slashes, then a length-summed alloc and three `memcpy`-style
copies) and fully unit-testable through runtime behavior on each mode. The base-offset
table (§4.2) must match plan-55-A §4.3 exactly; they are cross-referenced.

**Rejected — implement `resourcePath` in MFBASIC source over `os::executablePath` +
`strings`.** Far less assembly, but `os::` is intrinsic-only today (no source layer),
and adding one for a single function is a larger structural change than one more
hand-emitted helper beside `lower_executable_path`. The hand-emitted path also keeps
the failure semantics identical to `executablePath` for free. Reconsider if a second
path-manipulating `os::` call ever wants the same string work.

**Rejected — bake the base as a global data object (`_mfb_rt_resource_base`) written by
`write_executable`.** Works, but `build_mode` already reaches `nir::lower_module`, so a
new `write_executable` parameter and a new global symbol are redundant; baking
constants into the helper (the `stdin_log_cap` precedent) is less machinery.

**Rejected — allow `..` in the result (`<exe-dir>/../Resources/…`).** Such a path is
absolute and the kernel resolves it, so it would "work". But the goal says absolute
*and clean*; the strip-N-components approach costs one extra backward-scan step and
yields a canonical path, which is friendlier to log and to compare. Chosen.

## 4. Detailed Design

### 4.1 Factor the executable-path acquisition

Extract from `lower_executable_path` (`src/target/shared/code/os.rs:1380`) a helper
that emits the platform acquisition into a caller-provided frame region and yields the
buffer pointer + a length register (Linux) / NUL-terminated flag (macOS), branching to
a caller-named `fail` label on error. `lower_executable_path` becomes: call it, then
`build_string_from_{cstr,len}` as today — its output is unchanged and its existing
tests (`os::executablePath` behavior) still pass verbatim.

### 4.2 The base offset per build mode

```rust
/// (components-to-strip, suffix-to-append) for `os::resourcePath`, per build mode
/// (plan-55-B §4.2). Strip drops trailing '/'-delimited components of the absolute
/// executable path (the filename is component 1); suffix is appended after. Must
/// stay in lockstep with plan-55-A `resource_output_dir`.
///
/// | build         | exe path                       | strip | suffix          | base            |
/// | ---           | ---                            | ---   | ---             | ---             |
/// | console       | `…/build/<name>`               | 1     | ``              | `…/build`       |
/// | macos `--app` | `…/Contents/MacOS/<name>`      | 2     | `Resources`     | `…/Contents/Resources` |
/// | linux `--app` | `…/usr/bin/<name>`             | 2     | `share/<name>`  | `…/usr/share/<name>`   |
fn resource_base_offset(build_mode: NativeBuildMode, module_name: &str) -> (u32, String)
```

The Linux suffix embeds `module_name`, known at lowering. All three bases are absolute
(strip only ever removes suffix components of an already-absolute path) and contain no
`..`. This matches `resource_output_dir` in plan-55-A §4.3 row-for-row.

### 4.3 Frontend + spec registration

- `src/builtins/os.rs`: add `const RESOURCE_PATH: &str = "os.resourcePath";` and an arm
  in each of `is_os_call`, `call_param_names` (`&[&["relative"]]`),
  `call_return_type_name` (`"String"`), `expected_arguments` (`"String"`), `arity`
  (`(1, 1)`), and `resolve_call` (`RESOURCE_PATH if exact(arg_types, &["String"])`
  → `String`). Extend the test `ALL` list and the metadata-parity tests.
- `src/target/shared/runtime/os_specs.rs`: add `OS_RESOURCE_PATH_SPEC` (symbol
  `_mfb_rt_os_os_resourcePath`, params `&["String"]`, returns `"String"`,
  `IO_PRINT_CLOBBERS`) and register it in the catalog + the `OS_ENV_CALLS` parity test.
- Platform import wiring: `resourcePath` needs the same libc imports as
  `executablePath` (`_NSGetExecutablePath` on macOS, `readlink` on Linux). Add the
  `"os.resourcePath"` arm beside every `"os.executablePath"` arm in the per-target
  `plan.rs` files (`macos_aarch64`, `linux_aarch64`, `linux_x86_64`, `linux_riscv64`)
  and the `data_objects.rs:165` unsupported-guard list.

### 4.4 The helper body

`lower_resource_path(symbol, arg, build_mode, module_name, platform_imports, platform)`
emitted once when the module uses `os.resourcePath`:

1. **Validate the argument** (its pointer + length come from the incoming `String`).
   Scan it as `/`-delimited components: a component is the run of bytes between two
   slashes (or between a slash and a string end). Reject — branch to a `bad_arg` label
   emitting `ErrInvalidPath` — if any component is exactly `.` (one dot) or `..` (two
   dots). Empty components (from `//` or a leading/trailing `/`) are *not* an error;
   they collapse harmlessly under the base. This is a single forward pass tracking
   "component start index" and "component is all-dots so far": a component that ends
   with length 1 or 2 and was all dots is the rejection. A dot inside a longer
   component (`x.ogg`, `..foo`, `a..b`) never triggers it — only a whole component of
   just `.` or `..` does.
2. `emit_executable_path_into` (§4.1) → exe buffer pointer `buf`, plus its byte length
   `n` (Linux returns it; macOS computes it via a `strlen` scan of the NUL-terminated
   buffer). Branch to `fail` on acquisition error.
3. `(strip, suffix) = resource_base_offset(build_mode, module_name)` — baked constants.
   Scan `buf[0..n]` backward counting '/' bytes; stop after the `strip`-th slash from
   the end. `prefix_len` = index of that slash (prefix = `buf[0..prefix_len]`, no
   trailing slash). If fewer than `strip` slashes exist (malformed/short path), branch
   to `fail` — defensive; the real layouts always have enough.
4. Allocate an arena `String` of length
   `prefix_len + 1 + suffix.len() + (suffix nonempty ? 1 : 0) + arg_len` (the argument
   pointer + `arg_len` are the ones from step 1), then copy: prefix, `'/'`, suffix (if
   any) + `'/'`, argument. Reuse the arena-alloc + copy idiom the existing string
   builders use; on alloc failure branch to the shared `alloc_error` path.
5. Return the `String` (result-value/tag registers), matching `executablePath`'s
   return convention. The `fail` path emits `ERR_UNSUPPORTED` + error-result exactly as
   `lower_executable_path` does; the `bad_arg` path (step 1) emits `ERR_INVALID_PATH`
   + error-result the same way. Both are ordinary catchable errors.

Dispatch: add `"os.resourcePath" => lower_resource_path(...)` beside
`"os.executablePath"` (`src/target/shared/code/os.rs:175`), threading `build_mode` and
the module name from the NIR module (§2.3) into the call — the dispatch already has the
module in scope where `stdin_log_cap` is read.

## Compatibility / Format Impact

**Changes:**

- New builtin `os::resourcePath(String) -> String`. Purely additive to the `os`
  surface; no existing call changes.
- Executable bytes change **only for programs that call `os::resourcePath`** (they gain
  the new helper). Programs that do not call it are byte-identical — the helper is
  emitted on demand, like every other `os::` helper. New goldens only for new tests.

**Unchanged:**

- `os::executablePath` output and its goldens (its code is refactored but its emitted
  behavior is identical — verify via its existing tests).
- The manifest schema, `.mfp`, and every non-`resourcePath` program's bytes.

## Phases

### Phase 1 — Frontend + spec registration

No codegen; makes the call resolve and type-check, failing to *link* until Phase 3.
Lands with the type/arity tests.

- [ ] Add `RESOURCE_PATH` to the five tables in `src/builtins/os.rs` (§4.3) and extend
      its tests.
- [ ] Add `OS_RESOURCE_PATH_SPEC` to `src/target/shared/runtime/os_specs.rs` and its
      parity test.
- [ ] Add the `"os.resourcePath"` import arm to each `plan.rs` and the
      `data_objects.rs:165` guard list (§4.3).

Acceptance: `os::resourcePath("x")` type-checks to `String`; `os::resourcePath()` and
`os::resourcePath(3)` are frontend errors with the standard arity/type diagnostics;
`cargo test -p mfb os` green.
Commit: —

### Phase 2 — Factor exe-path acquisition

Pure refactor; keeps `os::executablePath` byte-identical.

- [ ] Extract `emit_executable_path_into` from `lower_executable_path`
      (`src/target/shared/code/os.rs:1380`) per §4.1; rewrite `lower_executable_path`
      to call it.
- [ ] Tests: existing `os::executablePath` behavior tests still pass unmodified.

Acceptance: a program calling `os::executablePath` produces byte-identical output
before and after (compare via `scripts/artifact-gate.sh`); `cargo test` green.
Commit: —

### Phase 3 — The `resourcePath` helper (highest-risk)

The string codegen. Lands last, behind Phases 1–2, verified on real hardware per
`.ai/compiler.md`.

- [ ] Add `resource_base_offset` (§4.2) and `lower_resource_path` (§4.4), including the
      step-1 `.`/`..`-component validation branching to `ErrInvalidPath`; dispatch it
      at `os.rs:175`, threading `build_mode` + module name from the NIR module.
- [ ] Tests (runtime behavior): a fixture program printing
      `os::resourcePath("music/x.ogg")` — assert the console result is
      `<exe-dir>/music/x.ogg` (absolute, no `..`); assert a macOS `--app` build's result
      is `…/Contents/Resources/music/x.ogg`; assert `TRAP` catches the unsupported case
      via a stubbed acquisition failure if feasible, else document the manual check.
- [ ] Tests (argument validation): `os::resourcePath("../secret")`,
      `os::resourcePath("a/../b")`, `os::resourcePath("./x")`, and
      `os::resourcePath("music/..")` each `TRAP` `ErrInvalidPath`; `os::resourcePath("x.ogg")`,
      `os::resourcePath("a/b.c/d")`, and `os::resourcePath("..foo/bar")` succeed (a dot
      inside a component is not a `.`/`..` component).
- [ ] Tests: `resource_base_offset` unit cases for all three modes incl. the
      `module_name` interpolation in the Linux suffix.

Acceptance: on macOS hardware, a fixture built console-mode prints an absolute path
under `build/` that `fs::open` opens; the same fixture built `--app` prints a path
under `Contents/Resources/` that opens after double-click launch from an arbitrary cwd.
On the Linux GTK box (once plan-51-A lands): the AppDir/`.AppImage` result resolves
under `usr/share/<name>/` and opens.
Commit: —

## Validation Plan

- Tests: unit — `os` frontend metadata/arity for `resourcePath`, `os_specs` parity,
  `resource_base_offset` per mode. Runtime — a `tests/` fixture asserting the resolved
  path per build mode (console + macOS app landing this sub-plan; Linux app when 51-A
  exists), a `TRAP` case for the unsupported path, and the `.`/`..`-component rejection
  cases (both rejected and dot-in-filename-accepted forms, §4.4).
- Runtime proof: build `examples/audio` (once its program calls `os::resourcePath`),
  run it, and confirm it opens `build/music/Mozart1.ogg`; repeat with `--app` on macOS
  from Finder (arbitrary cwd) to prove cwd-independence.
- Doc sync: `src/docs/spec/stdlib/14_os.md` (add `resourcePath` beside
  `executablePath`, document the per-mode base and the absolute-path guarantee);
  `src/docs/man/builtins/os/resourcePath.md` (new man page via
  `scripts/update_man.sh`, following `.ai/man_template.md`); update the `os` package
  overview page if it enumerates calls.
- Acceptance: `scripts/artifact-gate.sh` (Phase 2 refactor must be byte-neutral for
  `executablePath`; Phase 3 adds goldens only for new `resourcePath` fixtures),
  `scripts/test-accept.sh`, `cargo test`, `cargo fmt` (+ `repository/` second pass).
  `.ai/compiler.md`'s hardware-validation gate applies — Phase 3 is a codegen change.

## Open Decisions

- **Reject `.`/`..` components in the argument** — *decided (user, 2026-07-17)*: reject
  at runtime with `ErrInvalidPath` (§4.4 step 1), never sanitize silently. A dot inside
  a filename stays legal; only a whole `.`/`..` component errors. A leading `/` is left
  as-is (it collapses under the base and cannot escape). Not a fork anymore. (§4.4)
- **macOS length via `strlen` scan vs. threading the length out of acquisition** —
  recommend the `strlen` scan in `lower_resource_path` to keep `emit_executable_path_into`'s
  interface uniform across platforms; the buffer is ≤4096 and scanned once. (§4.4)

## Summary

`resourcePath` is `executablePath` plus a backward slash-scan and a length-summed string
concatenation, with a three-row base-offset table selected by `build_mode`. The risk is
the raw string codegen in §4.4, which is the same well-trodden class as the existing
`build_string_from_*` builders and is isolated in the final phase behind the frontend
and the byte-neutral refactor. The base-offset table is the one thing that must track
plan-55-A §4.3; both carry the same worked-example rows so a drift is visible in review.

Everything else is additive: five one-line table arms on the frontend, one runtime
spec, the per-target import wiring, and a man/spec page. No existing program's bytes
change, and `os::executablePath` stays byte-identical by construction.
