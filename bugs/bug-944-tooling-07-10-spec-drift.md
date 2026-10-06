# bug-944: tooling spec 07–10 drift: app-mode targets/texts stale, help screens disagree with the CLI, small CLI parser gaps

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Regression Test: none for the doc items. Items 8, 9 and 12 can get unit tests in `src/cli/build/options.rs`, `src/cli/pkg.rs` and `src/cli/init.rs`.

Measured mismatches between `src/docs/spec/tooling/{07_cli-reference,09_debug-report,10_debugger}.md`,
the `--help` screens, and the code at HEAD 28a15a153 (binary
`/tmp/hardening/mfb-head`). None is large enough for its own report. Each item gives
its class: SPEC (doc wrong, code right), CODE (small code gap), UNCLEAR (owner
decision). Every observation below comes from a command run in
`/tmp/hardening/B14/`.

References: `src/docs/spec/tooling/07_cli-reference.md`, `09_debug-report.md`,
`10_debugger.md`; `src/cli/help.rs` (`USAGE`, `BUILD_HELP`, `SPEC_HELP`/`MAN_HELP`).

## Items

1. **SPEC: app mode is documented as macOS/Linux only, but Windows has it, and both
   quoted error texts are stale.** 07 *build Flags*: "`--app` is
   executable-only and requires a native target that supports app mode
   (`macos-aarch64`, `linux-aarch64`, or `linux-x86_64`); otherwise it errors …
   (`mfb build -app requires an executable project` / `mfb build -app requires a
   macOS or Linux target`)". It also says "App mode selects
   `NativeBuildMode::LinuxApp`/`MacApp`". Measured:
   - `mfb build --target windows-x86_64 --app` → `Wrote executable to ./build/p1.exe`, exit 0.
   - `mfb build --target linux-riscv64 --app` → `error: app mode is not supported on linux-riscv64 (app-mode targets: macos-aarch64, linux-aarch64, linux-x86_64, windows-x86_64)`, exit 1.
   - `--app` on a package → `error: app mode requires an executable project`, exit 1.

   Source: `src/cli/build/mod.rs:build_project` (the two `eprintln!` sites) and
   `src/target.rs:target_supports_app_mode`. The *Output shape per target*
   paragraph also has no Windows `--app` row.

2. **SPEC: the top-level screen advertises `repo publish` too.** 07: "The top-level
   screen … advertises only the common `pkg` (`add`/`update`/`install`/`verify`)
   and `repo` (`register`/`auth`) members". `mfb --help` also lists `repo publish
   <owner>  Sign and publish a package project to a repository`
   (`src/cli/help.rs:USAGE`).

3. **CODE (help text): `build --help` misdescribes `--app`.** It says
   `--app  Build as a standalone application instead of a library`. `--app` is the
   GUI app-mode runtime, and it is refused for a library (package) project.

4. **CODE (help text): `spec --help` and `man --help` omit flags the commands
   take.** `mfb spec --help` lists only `--all`. `--width N`, `--color` and
   `--no-color` all work (`mfb spec --width 5 tooling` exit 0). The no-topic
   screen's first line is `Usage: mfb spec [topic] [subtopic] [--all]`.
   `mfb man --help` says `Usage: mfb man <package> [function]`. It omits `--all` and
   that the package is optional (`mfb man` prints the index). `mfb man a b c`'s own
   error says `Usage: mfb man <package> [function] [--all]`.

5. **SPEC: `-v` output and the `-O1` row list.** 07's `-v` row says only
   "additionally print a `phase <name> <N>ms` line per front-end stage". `mfb build
   -v` also prints one `<pass>: <count>` line per enabled optimizer row (for
   example `Register coalescing: 8`). `BUILD_HELP` and `mfb man optimizations`
   document this. 07's `-O` row lists the -O1 rows as "literal-`LET` folding,
   leaf-function inlining, constant folding, algebraic simplification, non-loop
   strength reduction, stack slot promotion, register coalescing and the
   post-regalloc machine peepholes". `mfb man optimizations` puts **Place copy
   forwarding** at level 1 too, and `-v` prints `Place copy forwarding: 0` and
   `Machine copy propagation / redundant-move elimination: 0` at the default level.

6. **UNCLEAR: `--sign` combined with an output flag exits 1, not 2.** `mfb build
   --sign x --ir` prints the `Building …` line, then `error: mfb build --sign is
   only supported for package and executable builds`, exit **1**. The command
   table says `2 bad flags; 1 build failed`, and this is a flag-combination
   error detected before any build work.
   **Owner decision (2026-10-05):** usage error: `--sign` with an output flag is rejected up front, exit 2. (CODE change)

7. **UNCLEAR: fmt/doc treat an unknown single-dash flag as a path; doc rejects
   `--out=`.** `mfb fmt -x` → `error: no such file or directory: `-x``, exit 1.
   `mfb doc -x` → `error: failed to read '-x' …`, exit 1. Compare `mfb build -x` /
   `mfb audit -x` → `unknown … option `-x``, exit 2. `mfb doc --out=x.html` and
   `mfb pkg doc k1 --out=…` → `unknown flag `--out=…``, exit 2, while `--indent=`,
   `--format=`, `--config=`, `--target=` and `--sign=` are all accepted. 07 does not
   promise `=` for `--out`, but the inconsistency surprises.
   **Owner decision (2026-10-05):** any unknown argument starting with `-` is a usage error (exit 2) for `fmt` and `doc`. A path that starts with `-` is written `./-x` or after `--`. (CODE change)

8. **CODE: `--sign=` (empty) is accepted.** `mfb build --sign=` gets past option
   parsing and fails later with `error: missing local ident private key
   '/Users/…/.mfb/43fcab…/keys/.ident.prv'`, exit 1. The empty owner is hashed into
   a key path. `--config=` is correctly `mfb build --config requires a config name`
   (exit 2). `parse_build_options`'s `--sign=` arm should reject an empty value the
   same way (`src/cli/build/options.rs:parse_build_options`).

9. **CODE: `pkg add file://… --no-pin` exits 1, but 07 calls it a usage error.** 07
   *pkg add Pin Inference*: "A `file://` add is always `pin: true` and `--no-pin` on
   one is a usage error". Measured: `error: mfb pkg add --no-pin cannot apply to a
   file:// package: there is no registry version stream to float along`, exit
   **1**. `--pin --no-pin` together is correctly exit 2.

10. **UNCLEAR: `pkg info <name>` does not resolve `packages/<name>.mfp`.** 07 says
    `pkg validate` "resolves `<package>` like `pkg doc` (a direct `.mfp` path, or
    `packages/<name>.mfp`)". Both do. `mfb pkg info k1` in a project that has
    `packages/k1.mfp` → `error: failed to read 'k1': No such file or directory (os
    error 2)`, exit 1. `mfb pkg validate k1` and `mfb pkg doc k1` in the same
    project both find `./packages/k1.mfp`. Either resolve the way validate/doc do,
    or document that `pkg info` takes only a path.
   **Owner decision (2026-10-05):** `mfb pkg info <name>` resolves `packages/<name>.mfp` in the current project when no path is given. (CODE change)

11. **SPEC: the `pkg info` layout sketch differs from the output.** 07 shows the
    *Binary Representation* counts three to a line (`types: <n>  constants: <n>
    resources: <n>`). The real output prints one per line. 07 marks only `Exports`
    with `(or "  <none>")`, but `Package State:`, `Resource Cleanups:` and
    `Imports:` also print `  <none>` when empty (`mfb pkg info k1/k1.mfp`).

12. **CODE: `mfb init` leaves a half-initialized project when a later file
    exists.** With `pi/src/main.mfb` already present, `mfb init pi` writes
    `pi/project.json` and then fails with `error: refusing to overwrite
    'pi/src/main.mfb'` (exit 1). It writes no `.gitignore`, and a rerun now fails on
    `project.json`. 07 says only "Both refuse to overwrite an existing file
    (`write_new_file`)". Checking every target before writing the first one would
    make the refusal all-or-nothing (`src/cli/init.rs`).

13. **CODE (message): the `repo` subcommand-missing error lists 4 of 9
    subcommands.** `mfb repo` → `error: mfb repo requires a subcommand (register,
    auth, link, trust)`. It omits publish, check-abi, release-state, transfer and
    transfer-accept. Also `repo release-state bogus` and `repo transfer-accept bad`
    are argument-shape errors that exit 1, not 2.

14. **SPEC: 09 says the report runs "after the main arena has been destroyed". That
    is false for any program that uses `thread`.** `src/codegen/engine/builder/mod.rs`
    sets `skip_entry_arena_destroy` whenever a `thread.*` runtime call is present,
    so `_mfb_shutdown` skips `_mfb_arena_destroy`. Measured with a two-worker
    program, 3 of 3 runs: `arena.0.maps 1 … arena.0.unmaps 0 arena.0.unmapped_bytes
    0`. A single-threaded program reports `arena.0.unmaps 1` (destroyed). The
    "never reads the arena register" claim may still hold, but the destroyed-arena
    premise and the reading of `arena.0.unmaps` need a sentence.

15. **CODE (message): `bt all 1` contradicts itself.** `error: 'all' is not a number
    of frames (1 or more), or 'all'`. It refuses `all` while offering `all`. The
    real problem is that `all` takes no further argument (10's refusal table has no
    row for this) (`src/debugger/prompt.rs`, the backtrace parse).

16. **UNCLEAR: `show mem` shows the debugger's own `brk` bytes.** With `bp touch`
    set, `show mem touch 8` prints `00 00 20 d4 …` (`brk #0`) where the program's
    instruction is. 10 promises that `dis` (`read_code`) "puts each breakpoint's
    saved instruction back, so a listing never shows the `brk`", but says nothing
    for `show mem`. lldb shows original bytes in memory reads. Either restore the
    saved bytes in `show_memory`, or state that `show mem` shows the patched bytes.
   **Owner decision (2026-10-05):** `show mem` shows the program's original bytes under breakpoints (substituting the saved bytes, as gdb/lldb do). (CODE change)

## Verified clean (for the coordinator)

Every `[[…]]` citation in 07, 08, 09 and 10 resolves to a defined symbol or file
(`/tmp/hardening/B14/cites.py`).
