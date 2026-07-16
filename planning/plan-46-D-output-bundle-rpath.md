# plan-46-D: output directory + RPATH + vendor bundle

Last updated: 2026-07-16
Effort: large (4h–1d)
Depends on: **Phase 1 depends on nothing** and lands first, ahead of plan-46-A.
Phases 2-3 depend on plan-46-C (which depends on plan-46-B → plan-46-A).

Landing order for the whole feature: **D-Phase-1 → A → B → C → D-Phase-2 →
D-Phase-3.** Phase 1 is the output-directory change; it is independent of the
vendor feature entirely, and its ~439-golden churn is kept out of the vendor
commits by landing it alone and early (Open Decisions).

Makes a `vendor` library actually **loadable**. Three changes that only make
sense together:

1. every executable build emits into a **directory** — `<name>/<name>.out`
   instead of a bare `<name>.out`;
2. the ELF and Mach-O writers emit an **RPATH** pointing at `vendor/` beside the
   executable — `$ORIGIN/vendor` / `@loader_path/vendor`, or
   `@executable_path/../Frameworks` for a macOS `.app` bundle, which puts the
   dylibs in the platform-standard `Contents/Frameworks/` (§4.4) — so `dlopen` of
   a bare filename finds it;
3. the build **copies** each resolved `vendor` locator's file from
   `<consumer root>/vendor/<source>` into `<outdir>/<name>/vendor/<source>`.

The single behavioral outcome: an executable importing a binding with a `vendor`
locator builds into a self-contained, relocatable directory and `dlopen`s the
vendored library successfully from any working directory — the thing plan-46-C
resolves and verifies but cannot yet load (plan-46-C §1.1).

References (read first):

- `src/os/linux/link/mod.rs:106-112` — the **only** Linux site that builds the
  `.out` path; `put_dynamic` (line 524).
- `src/os/macos/link/mod.rs:26` — the **only** macOS console site;
  `write_app_bundle` (lines 44-76) — the existing directory-output precedent,
  including its `fs::create_dir_all` calls (lines 55, 64).
- `src/os/linux/link/elf.rs` — `dynstr` construction (line 440), the `DYNAMIC`
  tag block (lines 700-728).
- `src/os/macos/link/macho.rs` — `load_dylib` emission (line 142),
  `load_commands_size` (line 289), `load_command_count` (line 319).
- `scripts/test-accept.sh:253` (stdout-parsing binary locator — survives) and
  lines 161, 192, 265, 345, 360 (the `rm -f` cleanups that do **not**).
- `src/cli/build.rs:504`, `:509` — the two "Wrote … executable to {}" print
  sites.

## 1. Goal

- `./mfb build` on any console project writes `<outdir>/<name>/<name>.out`
  (Linux: `<outdir>/<name>/<name>-glibc.out` and `<name>-musl.out`, both in the
  one directory).
- An executable whose build resolved any `vendor` locator carries an RPATH
  pointing at the directory holding exactly the resolved vendor files, per the
  §4.4 table: `$ORIGIN/vendor` → `<name>/vendor/` (ELF), `@loader_path/vendor` →
  `<name>/vendor/` (macOS console), `@executable_path/../Frameworks` →
  `<name>.app/Contents/Frameworks/` (macOS `-app`, the platform-standard
  location).
- That executable runs and `dlopen`s its vendored library **from any working
  directory**, and continues to work after the whole `<name>/` directory (or
  `<name>.app` bundle) is moved elsewhere.
- A build with no vendor locators emits no vendor directory and no RPATH —
  byte-identical code to plan-46-C output, just relocated on disk.

### Non-goals (explicit constraints)

- **No automated acquisition.** Nothing fetches a vendored `.so` from a registry
  or embeds it in the `.mfp`. The consumer still places the file in
  `<consumer root>/vendor/` by hand (plan-46-C §4.4); this plan only *copies* a
  file that is already there and already hash-verified. Registry distribution
  remains a separate future plan.
- No change to the resolver, the match rule, the hash verify, or any `.mfp`
  format — all settled in plan-46-A/B/C.
- No change to `mfb test`'s in-process run path (it uses a temp dir and the
  returned `PathBuf`; §2.3).
- Do not change the macOS `.app` bundle's existing layout — `Contents/MacOS`,
  `Contents/Resources`, and `Info.plist` are untouched; the bundle only *gains*
  `Contents/Frameworks/` when the build vendors something (§4.4).
- **No Apple code-signing work.** The build does not sign, re-sign, or verify the
  signature of a vendored dylib, and adds no diagnostic for it (§4.4).

## 2. Current State

### 2.1 The `.out` path is built in exactly two places

Neither `src/cli/build.rs` nor `src/target/**` spells the extension — the path
is constructed at the OS link layer and flows back as a `PathBuf`:

- **`src/os/linux/link/mod.rs:106-112`**:
  ```rust
  let path = if app_mode {
      project_dir.join(format!("{project_name}.out"))
  } else {
      project_dir.join(format!("{project_name}-{}.out", flavor.suffix()))
  };
  ```
  The **caller loops flavors** (`src/target/linux_x86_64/mod.rs:302-324`,
  `linux_aarch64/mod.rs:286-308`, `linux_riscv64/mod.rs:317`) and pushes one path
  per iteration, so a console build returns a `Vec<PathBuf>` of **two** paths.
- **`src/os/macos/link/mod.rs:26`**: `project_dir.join(format!("{project_name}.out"))`.

`src/target/**` only picks the writer and passes `project_dir` + `ir.name`. The
CLI only *prints* the result (`src/cli/build.rs:509` / `:504`).

**Neither `write_executable` creates a directory** — both assume `project_dir`
exists. `write_app_bundle` (`src/os/macos/link/mod.rs:44-76`) already does
(`create_dir_all` at lines 55, 64), and is the precedent to follow.

### 2.2 No RPATH support exists anywhere

**Verified: zero hits** for `DT_RUNPATH`/`DT_RPATH`/`LC_RPATH`/`@loader_path`/
`$ORIGIN` across `src/**`. Both writers must gain it:

- **ELF** is straightforward. `dynstr` is built at `elf.rs:440` (a `Vec<u8>`
  starting `[0]`, with offsets recorded as it grows) and the `DYNAMIC` block is a
  flat run of `put_dynamic(&mut bytes, tag, value)` calls at `elf.rs:700-728`
  (`put_dynamic` itself is at `src/os/linux/link/mod.rs:524`). Adding
  `DT_RUNPATH` = one string appended to `dynstr` + one `put_dynamic(.., 29,
  offset)`. `DT_STRSZ` (tag 10) is already computed as `dynstr.len()`
  (`elf.rs:705`) and `dynsym_offset` is already derived from
  `dynstr_offset + dynstr.len()` (`elf.rs:492`), so growing `dynstr` propagates
  through the layout on its own.
- **Mach-O is the risky half.** The load-command bytes, their total size, and
  their count are computed in **three independent places** that must agree:
  emission (`load_dylib` at `macho.rs:142` is the template), `load_commands_size`
  (`macho.rs:289`), and `load_command_count` (`macho.rs:319`) — feeding
  `sizeofcmds` (`macho.rs:82`) and `ncmds` (`macho.rs:80`). An `LC_RPATH` added
  to one and not the others produces a header whose declared size disagrees with
  its contents, which `dyld` rejects at launch. This triple-maintenance hazard is
  the main correctness risk in this plan (§4.3).

### 2.3 What survives the layout change, and what does not

**Survives — the binary is located by parsing stdout, not by constructing a path:**
- `scripts/test-accept.sh:253`:
  `run_path=$(... sed -n 's/^Wrote executable to //p' | tail -n 1)`, then run at
  `:255-256`. Unchanged.
- **10 integration tests** parse the same line: `tests/build_verbosity_output.rs:75`,
  `tests/entry_args.rs:227`, `tests/native_io_runtime.rs:40`,
  `tests/native_loop_runtime.rs:46`, `tests/native_numeric_pow_div_runtime.rs:57`,
  `tests/native_float_pow_operator_runtime.rs:48`,
  `tests/native_size_arith_overflow.rs:78`, `tests/macos_tls_write_capacity.rs:124`,
  `tests/fs_error_path_hygiene.rs:282`, `tests/linux_app_mode.rs:103,134`.
- **`mfb run` does not exist** — `src/main.rs:249-493` dispatches only `help`,
  `init`, `init-pkg`, `build`, `test`, `pkg`, `repo`, `machine`, `key`, `org`,
  `token`, `audit`, `man`, `spec`, `doc`, `fmt`. `mfb test` runs the binary
  in-process from the returned path (`run_test_binary`, `src/cli/build.rs:490`,
  `833-842`) against a temp dir (`:455-459`, `make_temp_output_dir` at `:807-829`)
  — path-agnostic, no change.

**Breaks:**
- **`scripts/test-accept.sh` cleanup, 5 sites** (161, 192, 265, 345, 360), all
  `rm -f "$test_dir/$package_name.out"` — they would leave a directory behind.
  `.gitignore` has **no `*.out` entry** (verified: 0 matches), so every stale
  output would pollute the worktree and show up in `git status`.
- **6 test files hardcode the filename**: `tests/fs_create_mode_0600.rs:82`,
  `tests/linux_pie_headers.rs:55-56`, `tests/linux_rodata_readonly.rs:62-63`,
  `tests/macos_rodata_readonly.rs:65`, `tests/linux_app_mode.rs:112-113,144-148`,
  `tests/repo_acceptance.rs:360`; plus unit assertions at `src/os/linux/mod.rs:129`,
  `src/os/macos/mod.rs:135`, `src/os/linux/link/tests.rs:302,320`.
- **12 spec docs** state `<name>.out` as the contract:
  `architecture/08_artifacts.md:46`, `tooling/07_cli-reference.md:65,70`,
  `linker/{06_macos-aarch64.md:150, 07_linux-aarch64.md:14, 08_linux-x86_64.md:16,
  09_linux-riscv64.md}`, `architecture/{06_native.md:406-407, 10_boundaries.md:24,
  01_commands.md, 07_flows.md, spec.md}`, `threading/09_os-integration.md:31`.
- **~439 golden `build.log` files** — 47% of the 935 build-log goldens — churn on
  **two lines each** (~878 lines): the `Wrote executable to …` line and the
  harness's echoed `$ tests/…/<name>.out` run line. Mechanical and bulk-regenerable
  via `scripts/sync-goldens.sh`.

## 3. Design Overview

Three phases, each independently landable and independently valuable:

1. **Layout** — `<name>/<name>.out`. Touches 2 real lines of logic (+2
   `create_dir_all`) and a long tail of tests/docs/goldens. No behavior change.
2. **RPATH** — `DT_RUNPATH` / `LC_RPATH` pointing at `vendor/`. Inert on its own
   (a runpath to a non-existent directory is silently ignored by every loader),
   which makes it safe to land before the copy exists.
3. **Vendor copy** — copy resolved vendor files into `<outdir>/<name>/vendor/`.
   Completes the feature.

Risk is concentrated in the Mach-O header triple-maintenance (§2.2) and in the
harness cleanup change (§4.1), not in the layout itself.

RPATH also turns out to be what lets the macOS `.app` bundle use the *standard*
`Contents/Frameworks/` location without any special-casing downstream: the vendor
directory's position relative to the executable differs per output shape, but
that difference is expressed entirely as one string handed to the encoder (§4.3),
and `dlopen` stays a bare-filename call in every case.

### 3.1 Why the layout is unconditional

Rejected alternative: keep `<name>.out` and only switch to `<name>/<name>.out`
when the build has vendor libraries. Rejected — it makes the output *shape* a
function of the dependency graph. A transitive binding three levels down that
vendors a library would silently relocate the user's output, and every wrapper
(CI, scripts, the harness) would need to handle both shapes forever. The
conditional buys the churn of the directory layout *and* the unpredictability.
One shape, always.

Rejected alternative: keep `<name>.out` and put vendor files in a sibling
`<name>.vendor/`. This preserves the executable path exactly (zero golden churn),
but leaves two artifacts that must be moved together to keep working — the same
"move them as a unit" constraint as a directory, with none of the tidiness. If
the Phase-1 churn ever proves intolerable, this is the fallback; otherwise prefer
the directory.

### 3.2 Why RPATH rather than a runtime exe-dir lookup

Rejected alternative: have `_mfb_linker_init` read its own executable path
(`readlink("/proc/self/exe")`, `_NSGetExecutablePath`) and build an absolute
`<exedir>/vendor/<source>` to `dlopen`. Rejected — it needs a new per-OS runtime
helper, more codegen on the link path, and error handling for the lookup itself,
to reimplement what both loaders already do natively. RPATH costs no runtime code
at all: `dlopen("libfoo.so")` stays exactly the bare-filename call plan-46-C
already emits (plan-46-C §3.1), and the loader resolves it.

Note the semantics that make this work: for a `dlopen` issued **from the
executable itself** — which is where `_mfb_linker_init` lives — both glibc and
musl consult the calling object's `DT_RUNPATH`, and dyld consults its
`LC_RPATH`. (`DT_RUNPATH` deliberately does not propagate to transitive
dependencies; that limitation does not apply here.)

## 4. Detailed Design

### 4.1 Output layout

Both writers gain `create_dir_all(project_dir.join(project_name))` and join one
more component:

| build | today | after |
| --- | --- | --- |
| macos console | `<name>.out` | `<name>/<name>.out` |
| linux console | `<name>-glibc.out`, `<name>-musl.out` | `<name>/<name>-glibc.out`, `<name>/<name>-musl.out` |
| linux `-app` | `<name>.out` | `<name>/<name>.out` |
| macos `-app` | `<name>.app/…` | unchanged, plus `Contents/Frameworks/` when vendoring (§4.4) |

Both Linux flavor executables land in **one** directory and share **one**
`vendor/` — which is sound only because plan-46-A §4.3 forces vendor `source`
filenames to be unique project-wide, so a glibc blob and a musl blob never
collide (plan-46-C §4.3).

**Harness cleanup (`scripts/test-accept.sh`).** All 5 sites that remove
`"$test_dir/$package_name.out"` must instead remove the directory
`"$test_dir/$package_name"`. Two traps:

1. **Lines 192 and 360 are compound `rm -f`s** — the `.out` element is the tail
   of a long list of dump/sidecar paths (`$ast_path`, `$ir_path`, …
   `coverage.html`). Do **not** flip those whole commands to `rm -rf`. Split the
   `.out` element out into its own separate, guarded directory removal and leave
   the file list on `rm -f`. (Line 360's `rm -f` begins on line 359 — a plain
   `grep 'rm -f.*\.out'` misses it. The five sites are 161, 192, 265, 345, 360.)
2. **Guard the `rm -rf`.** `$package_name` comes from `project_name()`
   (`:79-81`), a `sed` over `project.json`. Today a bad parse makes `rm -f` a
   harmless no-op; after the change the same bad parse makes `rm -rf` delete a
   **source directory**, and an empty `$package_name` expands to
   `rm -rf "$test_dir/"` — which would delete the fixture itself. Assert
   `$package_name` is non-empty and that the target is a directory containing the
   built executable before removing anything.

Add `*.out` (or the output dirs) to `.gitignore` while here — it has no such
entry today, which is why a missed cleanup shows up as worktree pollution.

### 4.2 ELF: `DT_RUNPATH`

In `elf.rs`, when the build has vendor libraries:
- append `"$ORIGIN/vendor\0"` to `dynstr` (built at line 440), recording its
  offset;
- emit `put_dynamic(&mut bytes, 29, runpath_offset)` in the tag block
  (lines 700-728). Tag **29** is `DT_RUNPATH`; use it, not `DT_RPATH` (15), which
  is deprecated and — unlike `RUNPATH` — cannot be overridden by
  `LD_LIBRARY_PATH`.

`DT_STRSZ` and `dynsym_offset` derive from `dynstr.len()` already (§2.2), so no
manual offset fixups. Emit the tag **only** when vendor libraries exist, so
non-vendor binaries stay byte-identical (§Compatibility).

The literal `$ORIGIN` goes in the string verbatim — it is expanded by the loader,
not the build. Take care that no Rust format string interpolates it.

### 4.3 Mach-O: `LC_RPATH`

`LC_RPATH` (cmd `0x8000001C` — `0x1c | LC_REQ_DYLD`) is
`{ u32 cmd, u32 cmdsize, u32 path_offset }` followed by the NUL-terminated path,
the whole command padded to an 8-byte multiple — the same shape `load_dylib`
(`macho.rs:142`) already builds for `LC_LOAD_DYLIB`, so mirror it.

**The RPATH string is not fixed — it depends on the output shape** (§4.4). Rather
than branch inside the encoder, pass the rpath list **in** from the caller:
`encode_mach_o(…, rpaths: &[&str], …)`. This mirrors how `libraries` is already
threaded, and it makes the size math fall out naturally:

- emission: one `load_rpath(&mut bytes, path)` per entry, mirroring `load_dylib`;
- `load_commands_size` (`macho.rs:289`): add
  `rpaths.iter().map(rpath_command_size).sum()`, exactly as it already does
  `dylib_command_size(path)` over libraries at `macho.rs:310-313`;
- `load_command_count` (`macho.rs:319`): add `rpaths.len()`.

Callers pass `[]` when the build vendors nothing, one entry when it does.

**The hazard (§2.2):** emission, `load_commands_size`, and `load_command_count`
are three independent computations feeding `sizeofcmds` (`macho.rs:82`) /
`ncmds` (`macho.rs:80`). All three must agree or `dyld` rejects the binary at
launch. Mitigate by deriving the size from **one** shared
`rpath_command_size(path)` helper called by both the emitter and
`load_commands_size` — never open-code the arithmetic twice.

Adding a load command also shifts every subsequent offset, including the
`LC_CODE_SIGNATURE` that `linkedit_data(&mut bytes, 0x1d, …)` emits at
`macho.rs:150` and the `.mfb_sign` payload whose offsets `macho.rs:250-258`
compute. `load_commands_size` already accounts for `signing_metadata.is_some()`
(`macho.rs:297`, a fixed 152 bytes). Verify a signed build still launches on real
hardware — an offset error here is invisible to a round-trip unit test and fatal
at exec.

### 4.4 macOS `.app` bundles — `Contents/Frameworks/`

Vendor dylibs go in the **standard** location: `<name>.app/Contents/Frameworks/`,
which is where Apple specifies private shared libraries and frameworks live, and
where every tool that inspects a bundle expects them. The RPATH is
`@executable_path/../Frameworks` — the string Xcode emits for app targets.
(`@loader_path/../Frameworks` is equivalent here, since the loader *is* the
executable; prefer `@executable_path` to match the platform convention that
tooling and readers recognize.)

So the rpath passed to `encode_mach_o` (§4.3) is:

| build | vendor rpath | vendor files |
| --- | --- | --- |
| macos console | `@loader_path/vendor` | `<name>/vendor/` |
| macos `-app` | `@executable_path/../Frameworks` | `<name>.app/Contents/Frameworks/` |
| linux console / `-app` | `$ORIGIN/vendor` | `<name>/vendor/` |

`write_app_bundle` (`src/os/macos/link/mod.rs:44-76`) already creates
`Contents/MacOS` and `Contents/Resources` with `create_dir_all` (lines 55, 64);
`Contents/Frameworks` follows the same pattern, created only when the build has
vendor files (an empty `Frameworks/` in every bundle would be noise).

#### The byte-identity invariant

`write_executable` and `write_app_bundle` both call
`encode_executable_bytes(project_name, image)` today, and the bundled Mach-O is
**byte-identical to the console `<name>.out`**. This is not an incidental
property — it is asserted in **three** places, all of which must be updated
together:

1. the doc comment at `src/os/macos/link/mod.rs:41-43`;
2. an **active test** at `src/os/macos/link/tests.rs:811` ("The bundled binary
   must be byte-identical to the console `.out`");
3. the **published spec** — `src/docs/spec/linker/06_macos-aarch64.md:109`
   (inside the bundle-layout tree: "the Mach-O executable, byte-identical to
   `<project>.out`") and again at `:151`.

Miss (3) and the spec silently contradicts the compiler, which `.ai/specifications.md`
forbids.

Because the two shapes now need *different* RPATH strings, that invariant cannot
hold for a vendor-bearing build — and it **should not**: the two binaries load
from genuinely different places, so identical bytes would mean one of them is
wrong.

The saving grace is that it breaks **only** where it must:

- **No vendor libraries → rpath list is empty → zero `LC_RPATH` commands → the
  binaries stay byte-identical.** Every existing project, every existing fixture,
  and the `tests.rs:811` test itself (which vendors nothing) are unaffected.
- **Vendor libraries → the two differ by exactly one `LC_RPATH` string**, which
  is the correct and necessary difference.

Amend all three sites to state the qualified invariant ("byte-identical unless
the build vendors native libraries, which add one `LC_RPATH`"), and leave
`tests.rs:811` asserting it for the non-vendor case. Add a *new* test asserting
the vendor case differs in exactly the expected way, so the narrowed invariant is
pinned rather than quietly abandoned.

`src/docs/spec/linker/06_macos-aarch64.md:106-110` also carries the bundle-layout
tree, which gains the `Contents/Frameworks/` entry:

```text
<project>.app/
  Contents/
    Info.plist
    MacOS/<project>            (the Mach-O executable)
    Resources/AppIcon.icns     (multi-resolution app icon)
    Frameworks/<source>...     (vendored native libraries; only when the build vendors any)
```

#### Signing (not checked)

Per the decision on this feature, **the build performs no signature check on a
vendored dylib.** Worth recording as a known constraint rather than a surprise:
Apple Silicon requires loadable code to carry at least an ad-hoc signature, so an
unsigned vendored `.dylib` may be refused by `dyld` at `dlopen` regardless of
where it sits. That is the vendoring author's responsibility to satisfy (most
distributed dylibs already are signed); the build neither verifies nor re-signs
it, and no diagnostic covers it.

### 4.5 Vendor copy

After plan-46-C's resolve + hash verify succeeds, for each resolved `vendor`
locator: copy `<consumer root>/vendor/<source>` → `<outdir>/<name>/vendor/<source>`,
preserving the executable bit. Copy **only resolved** locators — not the whole
`vendor/` directory — so a project vendoring blobs for six targets ships one per
build.

The copy is the last step, after the hash verify that plan-46-C §4.4 already
performs on the same file, so the bytes landing in the output are the bytes that
were verified. Do not re-hash.

## Compatibility / Format Impact

- **Breaking, deliberately:** the build output path changes for every project.
  This is a tooling-contract change (12 spec docs, ~439 goldens), not a format
  change. No `.mfp`, ABI, or wire change.
- **Codegen:** unchanged for any build with no vendor libraries — no RPATH tag,
  no new load command, byte-identical to plan-46-C output. Only the file's
  location on disk moves. The artifact/byte-diff gate must confirm this.
- **Vendor builds** gain one `DT_RUNPATH` / `LC_RPATH` entry.

## Phases

### Phase 1 — output directory

**Lands first, ahead of plan-46-A** — this phase depends on nothing in plan-46
and nothing in plan-46 depends on it. No behavior change; pure relocation.

- [ ] Add `create_dir_all` + the extra path component to the two writer sites
      (`src/os/linux/link/mod.rs:106-112`, `src/os/macos/link/mod.rs:26`),
      following `write_app_bundle`'s precedent.
- [ ] Fix the 5 cleanup sites in `scripts/test-accept.sh` (161, 192, 265, 345,
      360) per §4.1 — guarded directory removal, with the `.out` element split
      out of the compound `rm -f`s at 192 and 360. Assert non-empty
      `$package_name` before any `rm -rf`.
- [ ] Add **both** `*.out` and the new output directories to `.gitignore` — it
      has no `out` entry of any kind today (verified), and `*.out` still matters
      for stale pre-change artifacts already sitting in working trees.
- [ ] Update the 6 test files + 4 unit assertions that hardcode the filename
      (§2.3). Enumerate them fresh with a grep — do not work from this list.
- [ ] Update the 12 spec docs that state `<name>.out` (§2.3).
- [ ] Regenerate the ~439 goldens (`scripts/sync-goldens.sh`) **after** the
      cleanup fix, not before — otherwise every synced fixture leaves an
      un-ignored output directory in `tests/`.

Acceptance: `./mfb build` writes `<name>/<name>.out` (Linux: both flavors in one
directory); `scripts/test-accept.sh` green with no leftover directories and a
clean `git status`; `scripts/artifact-gate.sh` clean (the binary's *bytes* must
not change — only its path).
Commit: —

### Phase 2 — RPATH emission

Inert until Phase 3 (a runpath to a missing directory is ignored by every
loader), so it is safe to land alone.

- [ ] ELF: append `$ORIGIN/vendor` to `dynstr` and emit `DT_RUNPATH` (tag 29)
      per §4.2, gated on the build having vendor libraries.
- [ ] Mach-O: take an `rpaths: &[&str]` parameter through `encode_mach_o` per
      §4.3, emit one `LC_RPATH` per entry, and feed the **same**
      `rpath_command_size` helper into `load_commands_size` and
      `load_command_count`. Callers pass `@loader_path/vendor` (console),
      `@executable_path/../Frameworks` (`-app`), or `[]` (no vendor libs).
- [ ] Amend the byte-identity claim to its qualified form in **all three** places
      it is asserted (§4.4): the doc comment at `src/os/macos/link/mod.rs:41-43`,
      and the published spec at `src/docs/spec/linker/06_macos-aarch64.md:109`
      and `:151`. Add a test pinning the narrowed invariant: console vs bundled
      binaries are byte-identical with no vendor libs, and differ by exactly the
      one `LC_RPATH` with them.
- [ ] Tests: a vendor-bearing build's ELF has exactly one `DT_RUNPATH` whose
      string is `$ORIGIN/vendor` (assert by decoding the dynamic section, in the
      style of `tests/linux_pie_headers.rs`); a non-vendor build has **none**.
      Same for `LC_RPATH` on Mach-O, with the console and `-app` strings each
      asserted. Verify a real Linux binary with `readelf -d` and a real macOS
      binary with `otool -l`.

Acceptance: `readelf -d` shows `RUNPATH [$ORIGIN/vendor]`, `otool -l` shows
`LC_RPATH @loader_path/vendor` (console) and `@executable_path/../Frameworks`
(`-app`) on vendor builds, and none appear on non-vendor builds; the existing
`tests.rs:811` byte-identity test still passes unchanged; all binaries still
launch on real hardware (`.ai/compiler.md` runtime completion gate);
`scripts/artifact-gate.sh` clean for non-vendor builds.
Commit: —

### Phase 3 — vendor copy

- [ ] Copy each resolved vendor file into `<outdir>/<name>/vendor/<source>` per
      §4.5, preserving the executable bit; only resolved locators.
- [ ] macOS `-app`: `create_dir_all` and copy into
      `<name>.app/Contents/Frameworks/` (§4.4), only when the build vendors
      something — no empty `Frameworks/` in ordinary bundles.
- [ ] Tests: golden + runtime — a project importing a `vendor`-locator binding
      builds into `<name>/` with `<name>/vendor/<source>` present, and the
      executable **runs and loads the library from a different working
      directory** (`cd /tmp && /path/to/<name>/<name>.out`), and **still runs
      after the whole `<name>/` directory is moved**. Both are the actual proof
      that RPATH resolution works; a build-time assertion is not.
- [ ] Tests: the same runtime proof for a macOS `-app` build — the dylib sits in
      `<name>.app/Contents/Frameworks/`, the bundle launches from a foreign CWD,
      and it survives being moved (e.g. to `/Applications`, the case the bundle
      layout exists for). Assert `Contents/MacOS/` holds no vendor files and an
      ordinary non-vendor bundle has no `Contents/Frameworks/` at all.
- [ ] Doc: update `src/docs/spec/architecture/08_artifacts.md` (the output
      layout is now a directory; vendor bundle + RPATH; the `.app` bundle's
      `Contents/Frameworks/`), the 4 linker spec pages,
      `tooling/07_cli-reference.md`, and
      `src/docs/spec/language/17_native-libraries.md` (loading model: vendor
      libraries load via RPATH from the output bundle; automated distribution
      still deferred). Add `Contents/Frameworks/` to the bundle-layout tree in
      `src/docs/spec/linker/06_macos-aarch64.md:106-110` (§4.4).

Acceptance: a vendored binding loads at runtime from any CWD and survives moving
the output directory — verified by actually running the binary on real hardware
for at least one Linux flavor, macOS console, **and a macOS `.app` bundle**, per
`.ai/compiler.md`. This is the acceptance for the whole plan-46 feature.
Commit: —

## Validation Plan

- Tests: header assertions for `DT_RUNPATH`/`LC_RPATH` presence *and absence*,
  including the per-shape macOS strings; the narrowed byte-identity invariant
  (§4.4); golden builds for the new layout; runtime vendor-load tests that run
  from a foreign CWD and from a moved directory.
- Runtime proof: **required, not optional** — a codegen + loader change is not
  done until a real binary `dlopen`s a real vendored library on real hardware
  (`.ai/compiler.md`). Cover at least one Linux flavor, macOS console, and a
  macOS `.app` bundle (the three distinct RPATH forms); the
  `.ai/remote_systems.md` boxes cover the rest.
- Doc sync: `architecture/08_artifacts.md`, the 4 linker pages,
  `tooling/07_cli-reference.md`, `language/17_native-libraries.md`, and the rest
  of the 12 docs listing `<name>.out`; `.ai/specifications.md` obligation.
- Acceptance: `scripts/test-accept.sh` green; `scripts/artifact-gate.sh` clean —
  Phase 1 must not change a single output byte, and Phase 2 must not change
  non-vendor builds.

## Open Decisions

None outstanding. Settled:

- **Phase 1 lands first, on its own, ahead of plan-46-A/B/C.** It is independent
  of the whole vendor feature and its ~439-golden churn is unrelated to it, so
  landing it alone keeps every later vendor commit readable. This makes plan-46-D
  **not** strictly last in the sequence: **D-Phase-1 → A → B → C → D-Phase-2 →
  D-Phase-3**. Nothing in D-Phase-1 depends on A/B/C, and the `Depends on` header
  above applies only to Phases 2-3.
- **`.gitignore` gets both** `*.out` and the output directories — `*.out` covers
  stale pre-change artifacts already sitting in working trees, the directory
  entries cover what the new layout produces.

## Summary

Turns plan-46-C's verified-but-unloadable vendor locator into a working library
load. The layout change is trivial in code (2 sites) and long in tail (~439
goldens, 12 docs, 6 test files) — mechanical, but it must land with the harness
`rm -rf` guard or it pollutes the worktree. The RPATH is the real engineering:
ELF is a one-string, one-tag addition, while Mach-O's `LC_RPATH` must be added to
emission, `load_commands_size`, and `load_command_count` in lockstep or `dyld`
rejects the binary. RPATH was chosen over a runtime exe-dir lookup precisely
because it keeps `dlopen` a bare-filename call and adds zero runtime code — and
it is also what lets the `.app` bundle put its dylibs in the platform-standard
`Contents/Frameworks/` for free, since the whole per-shape difference reduces to
one string passed into the encoder.

The one invariant this plan knowingly narrows: the macOS bundled binary is no
longer *unconditionally* byte-identical to the console `.out` (`mod.rs:41-43`,
tested at `tests.rs:811`). It still is for every build that vendors nothing —
which is every existing project and fixture — and differs by exactly one
`LC_RPATH` when it must, because the two binaries genuinely load from different
places. That narrowing is pinned by a new test rather than left implicit.
