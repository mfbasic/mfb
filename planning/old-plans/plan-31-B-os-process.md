# plan-31-B: `os` package — process & platform introspection

Last updated: 2026-07-08
Effort: medium (1h–2h)

Depends on: [[plan-31-A-os-environment]] (creates the `os` package skeleton —
`src/builtins/os.rs`, `os_specs.rs`, package registration, man overview page).
This sub-plan extends that package with **read-only process and host
introspection**: command-line arguments, process id, the running executable's
path, the platform name and CPU architecture, the host and user names, and the
CPU count. A correct implementation lets a program print its own argv, learn it
is running on e.g. `"linux"`/`"riscv64"`, and read the host name — all values a
host program routinely needs and none of which the stdlib exposes today.

It complements:

- `./mfb spec diagnostics error-codes` (reuses shared `7-705-xxxx` codes —
  `ErrUnsupported`, `ErrOutOfMemory`; no new block)
- `./mfb man fs` (structure/wiring precedent, shared with sub-plan A)
- [[plan-99-rv64-impl]] (the in-flight riscv64 backend that must gain the same
  emission)

## 1. Goal

- Extend `os` with:
  - `os::args() AS List OF String` — the program's command-line arguments
    (argv). Decision D1 fixes whether element 0 is the program name.
  - `os::pid() AS Int` — the current process id.
  - `os::executablePath() AS String` — absolute path to the running binary.
  - `os::name() AS String` — the OS family: `"macos"` or `"linux"`.
  - `os::arch() AS String` — the CPU architecture: `"aarch64"`, `"x86_64"`, or
    `"riscv64"`.
  - `os::hostName() AS String` — the host's network name.
  - `os::userName() AS String` — the effective user's login name.
  - `os::cpuCount() AS Int` — number of online logical CPUs.

### Non-goals (explicit constraints)

- Same guardrails as sub-plan A: no new language surface, no value/copy/move
  semantic change, no layout/ABI change, no new error-code block, no golden
  output change for existing programs.
- **All calls are read-only.** Nothing here mutates process state.
- No subprocess spawn/exec (future `process::`).

## 2. Current State

- The `os` package skeleton exists after sub-plan A (frontend metadata module,
  runtime-spec sibling, package registration, man overview).
- **`name`/`arch` are compile-time constants.** The binary is built for exactly
  one target, so these need no runtime helper — emit a rodata `String` constant
  chosen by the active target, the way any target-conditioned constant is
  selected in `src/target/<target>/`. This is the cheapest pair.
- **argv is available at entry.** The runtime already handles an arg-accepting
  `main` and had a startup-clobber bug fixed around argc/argv (see the
  arg-accepting-entry RNG-clobber fix in memory). `os::args()` needs argc/argv
  captured into a global at startup so a later call can materialise the
  `List OF String`.
- **libc primitives** for the rest: `getpid`, `gethostname`, `getpwuid_r`
  (or `getlogin_r`), `sysconf(_SC_NPROCESSORS_ONLN)`, and executable path —
  `_NSGetExecutablePath` on macOS vs. `readlink("/proc/self/exe")` on Linux
  (all three Linux arches).
- Precedent for emitting a libc call + registering its import per backend is
  `fs.currentDirectory`'s `getcwd` in each target's `code.rs`/`plan.rs`
  (`src/target/macos_aarch64/code.rs:361`, `linux_aarch64/code.rs:310`,
  `linux_x86_64/code.rs:444`).

## 3. Design Overview

Group the eight calls by implementation cost so the phases land low-risk first:

1. **Compile-time constants** — `name`, `arch`. No helper, no import; a rodata
   string per target. Trivial.
2. **Single-libc-call scalars** — `pid` (`getpid`), `cpuCount`
   (`sysconf`). One call, integer result, no buffer marshalling.
3. **Single-libc-call strings** — `hostName` (`gethostname` into a stack
   buffer), `userName` (`getpwuid_r`/`getlogin_r`). Buffer → `String`.
4. **Divergent-path string** — `executablePath` (`_NSGetExecutablePath` vs.
   `readlink /proc/self/exe`). The one genuinely per-OS-divergent helper.
5. **Startup-captured collection** — `args()` (capture argc/argv at entry, build
   `List OF String` on call).

Risk concentrates in (4) `executablePath` (two unrelated OS mechanisms, buffer
sizing/retry on macOS) and (5) `args()` (startup capture plumbing that must not
reintroduce the argc/argv clobber the arg-accepting-main fix cured).

## 4. Detailed Design

### 4.1 Frontend metadata (extend `src/builtins/os.rs`)

Add the eight names to `is_os_call`, `arity` (all fixed: `args`/`pid`/…
`(0,0)`), `call_return_type_name` (`args`→`List OF String`,
`pid`/`cpuCount`→`Int`, the rest→`String`), and `expected_arguments` (`()`).
No overloads here, so `resolve_call` stays simple.

### 4.2 Compile-time `name`/`arch`

Lower `os.name`/`os.arch` to a rodata `String` constant selected by the active
target in each `src/target/<target>/code.rs` (or a single shared switch keyed on
the target triple). No runtime-helper spec, no libc import.

Values: `name` ∈ {`"macos"`, `"linux"`}; `arch` ∈ {`"aarch64"`, `"x86_64"`,
`"riscv64"`}.

### 4.3 Runtime helpers (extend `os_specs.rs`)

Declare helpers for `pid`, `cpuCount`, `hostName`, `userName`,
`executablePath`, and the `args` builder. Each backend registers imports and
emits the call, reusing the shared buffer→`String` and `List` construction
helpers in `src/target/shared/code/`.

- `hostName`: `gethostname(buf, len)`; NUL-terminate defensively; buffer →
  `String`.
- `userName`: `getpwuid_r(getuid(), …)` into a stack `struct passwd` +
  scratch buffer (reuse the on-stack-struct pattern noted at
  `src/target/shared/code/codegen_utils.rs:490`), take `pw_name`; on lookup
  failure fall back to `getlogin_r`, else raise `ErrUnsupported`.
- `executablePath`: macOS `_NSGetExecutablePath(buf, &size)` with the size-retry
  protocol; Linux `readlink("/proc/self/exe", buf, len)` across all three
  Linux arches.
- `cpuCount`: `sysconf(_SC_NPROCESSORS_ONLN)` → `Int` (clamp ≥ 1).

### 4.4 `os::args()` argv capture

At program entry, stash `argc`/`argv` into a runtime global (the same values the
entry already receives; do not re-derive). `os::args()` reads that global and
builds a `List OF String` by copying each `char *` into a `String`. Guard the
capture so it does not clobber argc/argv before the RNG seed / arg-accepting main
consume them (the failure mode the arg-accepting-entry fix addressed).

## Layout / ABI Impact

None beyond a small runtime global holding captured `argc`/`argv` (module-level
runtime state, not a change to any user-visible record/struct layout or existing
helper ABI). Existing golden output is unchanged.

## Phases

### Phase 1 — Compile-time `name`/`arch` (lowest risk, no helper)

- [ ] Extend `src/builtins/os.rs` metadata for `name`/`arch`.
- [ ] Emit the per-target rodata `String` constant in each
  `src/target/<target>/code.rs` (or a shared target-keyed switch).
- [ ] Tests: `tests/func_os_name_valid/**`, `tests/func_os_arch_valid/**`
  (+ `_invalid` for arity/arg misuse).

Acceptance: a compiled program prints the correct `name`/`arch` string on each
backend (`"linux"`/`"riscv64"` on the riscv64 remote, etc.).
Commit: —

### Phase 2 — Scalar libc calls: `pid`, `cpuCount`

- [ ] Metadata + `os_specs` helpers for `pid` (`getpid`) and `cpuCount`
  (`sysconf`).
- [ ] Emit + register imports in all four backends' `code.rs`/`plan.rs`.
- [ ] Tests: `tests/func_os_{pid,cpuCount}_valid/**` + `_invalid/**`.

Acceptance: `os::pid()` prints a plausible pid matching the process; `cpuCount()`
≥ 1 on each backend.
Commit: —

### Phase 3 — String libc calls: `hostName`, `userName`

- [ ] Metadata + helpers; emit `gethostname` and `getpwuid_r`/`getlogin_r`
  across all four backends (reuse the on-stack-struct buffer pattern).
- [ ] Tests: `tests/func_os_{hostName,userName}_valid/**` + `_invalid/**`.

Acceptance: `os::hostName()`/`os::userName()` match `hostname`/`whoami` on each
buildable backend and the riscv64 remote.
Commit: —

### Phase 4 — `args()` + `executablePath` (highest-risk last)

- [ ] Startup argc/argv capture into a runtime global, guarded against the
  arg-accepting-entry clobber; `os::args()` builds `List OF String`.
- [ ] `executablePath`: `_NSGetExecutablePath` (macOS) and
  `readlink /proc/self/exe` (all Linux arches), emitted + imports registered.
- [ ] Tests: `tests/func_os_{args,executablePath}_valid/**` + `_invalid/**`.

Acceptance (runtime proof): a program run as `prog a b c` prints its argv via
`os::args()` matching D1's decision; `os::executablePath()` prints the absolute
path the binary was launched from, verified on each backend and the riscv64
remote. Acceptance suite (`scripts/test-accept.sh`) passes.
Commit: —

## Validation Plan

- Function tests: `tests/func_os_<func>_valid/**` and `_invalid/**` for all eight
  functions.
- Runtime proof: per-phase programs above, run natively per backend (macOS +
  Linux x86_64/aarch64 locally, riscv64 via `ssh -p 2229`, [[plan-99-rv64-impl]]).
- Doc sync: `src/docs/man/` pages for each function (extend the `os` overview
  from sub-plan A); Errors sections cite reused `7-705-xxxx` codes.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **D1 — `os::args()` element 0**: exclude the program name, returning only the
  arguments after it (recommended — matches the "arguments" mental model and the
  arg-accepting-`main` convention; program name is available via
  `executablePath`) vs. include argv[0] as element 0 (C convention). Recommend
  exclude, and document it.
- **D2 — `userName` source**: `getpwuid_r(getuid())` (recommended — works
  without a controlling terminal, e.g. under a service manager) with a
  `getlogin_r` fallback vs. `getlogin_r` first. Recommend `getpwuid_r` primary.
- **D3 — defer the costly pair?** `args`/`executablePath` (Phase 4) are the only
  non-trivial items; if timeboxing, Phases 1–3 are independently valuable and
  landable, with Phase 4 as a fast-follow. Recommend keeping all four here but
  landing in order.

## Non-Goals

- Environment variables — sub-plan A ([[plan-31-A-os-environment]]).
- Well-known directories, `os::exit`, subprocess spawn/exec — dropped/deferred
  as in sub-plan A.

## Summary

Seven of eight calls are thin libc wrappers over the `fs`-established
per-backend emission machinery, plus two compile-time constants that need no
helper at all. The real risk is `executablePath`'s two divergent OS mechanisms
and the `args()` startup-capture plumbing (which must respect the existing
argc/argv-clobber fix). No layout, ABI, semantics, or existing golden output
changes.
