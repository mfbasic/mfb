# plan-156-C: `os::userHomePath` and `os::userDocumentsPath`

Last updated: 2026-09-24
Effort: large (3h–1d)
Depends on: plan-156-B

This sub-plan adds two calls:

- `os::userHomePath(relative AS String = "") AS String`
- `os::userDocumentsPath(relative AS String = "") AS String`

They return the user's home folder and Documents folder. These are not
app-scoped: no project name is appended. Both follow the plan-156 family
contract (plan-156-A intro): an empty `relative` returns the base with no
trailing `/`, a `.`/`..` component raises `ErrInvalidPath`, a failed lookup
raises `ErrUnsupported`, and the call never creates anything or checks that the
path exists.

**Correct behavior:**

| Target | `userHomePath()` | `userDocumentsPath()` |
| --- | --- | --- |
| macOS | `<home>` | `<home>/Documents` |
| Linux | `<home>` | `XDG_DOCUMENTS_DIR` from `<config>/user-dirs.dirs`, parsed as GLib does (§4.2); else `<home>/Documents` |
| Windows | `FOLDERID_Profile` | `FOLDERID_Documents` (follows OneDrive and folder redirection) |

- `<home>` is plan-156-B's `emit_posix_home_base`: `$HOME` if set and
  non-empty, else `getpwuid(getuid())->pw_dir`, else `ErrUnsupported`, with
  trailing `/` trimmed.
- `<config>` is `$XDG_CONFIG_HOME` if set, non-empty and absolute, else
  `<home>/.config`.

This sub-plan also adds `NSDocumentsFolderUsageDescription` to the macOS app
bundle's `Info.plist`, so that the macOS privacy prompt on first Documents
access carries an explanation.

References:

- plan-156-A (the family contract, and the prerequisites for all of plan-156).
- plan-156-B (the shared emitters in `gen_host_paths.rs`, the `HostPath`
  self-update arm, and the Windows known-folder helper).
- xdg-user-dirs `user-dirs.dirs` format. The file on box 2226 says:
  `XDG_xxx_DIR="$HOME/yyy"` or `XDG_xxx_DIR="/yyy"`, "No other format is
  supported."
- GLib `glib/gutils.c:load_user_special_dirs` is the parser GTK applications
  use. Mirroring it means an MFB program and a GTK app on the same desktop
  agree on the folder.
- Apple: the `NSDocumentsFolderUsageDescription` Info.plist key.
- `.ai/compiler.md`, `.ai/arch-abi.md`, `.ai/man-content.md`, `.ai/testing-gates.md`.

## Prerequisites

See plan-156-A. On top of that, plan-156-B must be complete: every phase has a
filled `Commit:` line, and
`rg -n 'fn emit_posix_home_base' src/codegen/builtins/os/gen_host_paths.rs`
→ 1 match.

## 1. Goal

- Both calls resolve on macOS, Linux and Windows to the table above, and
  fixtures prove it on each OS family.
- On Linux, `userDocumentsPath` gives the same answer as GLib's
  `g_get_user_special_dir(G_USER_DIRECTORY_DOCUMENTS)` whenever GLib's answer
  is non-NULL, and `<home>/Documents` otherwise.
- The macOS `.app` `Info.plist` carries `NSDocumentsFolderUsageDescription`.

### Non-goals (explicit constraints)

- **No real-Documents access from inside the macOS App Sandbox.** Sandboxed,
  `$HOME` is the container, so both calls return container paths. There is no
  Documents entitlement, and the real folder needs a user-chosen file panel,
  which MFB does not have. This is documented, not coded around (user-accepted).
- **No other XDG user dirs** (Desktop, Downloads, …).
- **No existence check.** On a server with no Documents folder,
  `userDocumentsPath()` still returns `<home>/Documents` (user decision).
- **No shell unescaping** of `user-dirs.dirs` values. GLib does none (§4.2), and
  matching GLib is the goal.

## 2. Current State

- `gen_host_paths.rs` (plan-156-B) provides `emit_validate_relative`,
  `emit_join_result`, `emit_trim_trailing_slashes`, `emit_posix_home_base` and
  `emit_posix_env_abs_base`.
- `emit_os_wide_string` (`src/target/win_x86_64/code.rs`) has the `"appData"`/
  `"appCache"` known-folder arms and `emit_known_folder(guid)` (plan-156-B §4.2).
- **The file-read precedent.** `src/codegen/builtins/fs/gen_atomic_write.rs:lower_fs_read_text_path_helper`
  uses host `open`/`read`/`close` through `platform.emit_external_call`, with
  flags from `open_flag_set(platform.family(), false)`, and carries the fd in a
  spilled vreg across calls. `platform.emit_errno` exists
  (`builtins/process/func_receive.rs`).
- **The plist.** `src/os/macos/link/mod.rs:app_info_plist` has 8 keys today:
  `CFBundleName`, `CFBundleExecutable`, `CFBundleIdentifier`,
  `CFBundlePackageType`, `CFBundleShortVersionString`, `CFBundleVersion`,
  `CFBundleIconFile`, `NSPrincipalClass`. Its tests are in
  `src/os/macos/link/tests.rs` (`app_info_plist_has_required_bundle_keys`, …).

### Measured populations

| What | Count | Command |
|---|---|---|
| `Info.plist` keys today | 8 | `sed -n '/fn app_info_plist/,/^}/p' src/os/macos/link/mod.rs \| grep -c '<key>'` → 8 |
| Existing `app_info_plist` tests | 4 | `rg -c 'fn app_info_plist_' src/os/macos/link/tests.rs` → 4 |


### Verified properties

- **GLib parser semantics** (§4.2). This is UNVERIFIED against this host's GLib
  source, because GLib is not vendored here. Task C3 proves equivalence at
  runtime on 2226, a Debian GTK box with GLib installed: for each fixture case,
  run the same file through a `python3 -c 'from gi.repository import GLib; …'`
  probe (or a C probe if `gi` is absent) and compare the outputs.
- **Known-folder GUIDs.** These are UNVERIFIED until C4 compares the results
  against PowerShell `[Environment]::GetFolderPath('UserProfile'|'MyDocuments')`.
  - `FOLDERID_Profile` = `{5E6C858F-0E22-4760-9AFE-EA3317B67173}`
  - `FOLDERID_Documents` = `{FDD39AD0-238F-46AF-ADB4-6C85480369C7}`

## 3. Design Overview

The pieces:

- **`userHomePath`** is the thinnest member. POSIX uses `emit_posix_home_base`
  with an empty suffix. Windows adds the `"userHome"` known-folder arm.
- **`userDocumentsPath`:**
  - macOS: home plus `/Documents`.
  - Windows: the `"userDocuments"` known-folder arm.
  - Linux: new code, a streaming `user-dirs.dirs` parser (§4.2). **The
    correctness risk concentrates here.** It is a byte-at-a-time state machine
    written in `abi::` instructions, with a chunked `read` refill. That is the
    largest new native routine in plan-156. It lands behind a dense fixture,
    including a match that straddles a chunk boundary.
- **The self-update arms** reuse plan-156-B's `GrowKind::HostPath`, with a row
  each.
- **The Info.plist key** is a one-line, isolated addition.

**Rejected alternatives:**
- **Read the whole file into an arena buffer, then scan it.** A fixed buffer
  caps the file size silently. Growing it adds an allocation plus a
  reallocation loop. Streaming needs neither, and the only bounded buffer (the
  value, capped at `PATH_MAX` = 4096) has a principled bound: a longer path is
  unusable anyway.
- **Parse `user-dirs.dirs` in an MFBASIC body.** Rejected in plan-156-B §3,
  because `os` injects no source.
- **Shell-style unescaping** (`\"`, `\\`, `\$`). GLib does not do it, and
  agreeing with GTK apps matters more than matching a comment in the file.

**Byte-identity:** this is not the gate, since the plan changes behavior. The
`os` byte-identity fixture gains both calls (C5). All five `.ncodesum` files
are expected to change and get regenerated in plan-156-D.

## 4. Detailed Design

### 4.1 `userHomePath`

`lower_user_home_path` follows the same steps as plan-156-B's `lower_app_dir`,
with `suffix = ""`:

1. Capture the argument.
2. Validate `relative`.
3. Take the env lock (POSIX).
4. Get the base: `emit_posix_home_base` on POSIX, or the Windows
   `emit_os_wide_string("userHome")` arm (the `FOLDERID_Profile` GUID).
5. Join.
6. Unlock and return.

### 4.2 Linux `userDocumentsPath`: the `user-dirs.dirs` parser

**Inputs.** All of this runs under the env lock, because the `getenv` results
are pointers into `environ`:

- `home` from `emit_posix_home_base`, a `BaseBytes`;
- `config` from `emit_posix_env_abs_base("XDG_CONFIG_HOME")`, or else `home` +
  `/.config`.

**Path assembly.** `config ++ "/user-dirs.dirs" ++ NUL` goes into a 4096-byte
frame buffer. If it would not fit, skip the file and fall back.

**Reading.** Call `open(path, open_flag_set(Linux, read) | O_CLOEXEC)`. `open`
failure → fall back. Then `read(fd, chunk, 4096)` into a 4096-byte frame chunk:

- `n < 0` and `errno == EINTR` → retry.
- `n < 0` otherwise → stop, and treat it as EOF.
- `n == 0` → EOF.

Close the fd on every exit path. Whether `open_flag_set` already sets
`O_CLOEXEC` is recorded in task C2. If it does not, OR in `0x80000`, which is
the same on x86_64, aarch64 and riscv64 (`O_CLOEXEC` = `02000000`).

**State machine.** This mirrors GLib `load_user_special_dirs`. It runs per
byte, and the state persists across chunk refills. Each line (`\n`-terminated;
the last line may lack one):

1. Skip leading spaces and tabs.
2. Match the literal `XDG_DOCUMENTS_DIR`. On a mismatch, skip to the end of the
   line.
3. Skip spaces and tabs, expect `=`, then skip spaces and tabs again.
4. Expect `"`.
5. If the next 5 bytes are `$HOME`, consume them and set `relative = true`.
   Otherwise, if the next byte is not `/`, the line is invalid.
6. Copy every remaining byte of the line into the 4096-byte value buffer,
   recording the index of the **last** `"` seen. That is GLib's
   `strrchr(p, '"')`: the closing quote is the last one on the line. If the
   line would overflow the buffer, the line is invalid.
7. At the end of the line:
   - If no `"` was seen, the line is invalid.
   - Otherwise `value = buf[..last_quote]`. If `value` is non-empty and ends in
     `/`, drop that one byte (GLib removes exactly one).
   - The line is valid, and the latest valid line **replaces** any earlier
     match (GLib overwrites on each hit).

**Result.** The last valid match gives the result:

- `relative`: `home ++ ("/" if value non-empty and value[0] != '/') ++ value`.
  This is GLib's `g_build_filename(home, value)` joining. An empty `value`
  gives `home`.
- absolute: `value`.

With no valid match, the result is `home ++ "/Documents"`. The chosen base then
goes through `emit_join_result` with `relative`.

**Frame budget.** The frame holds three buffers: the path buffer (4096), the
chunk (4096) and the value buffer (4096), 12 KiB of frame locals. That is in
line with `EXE_PATH_FRAME_LOCALS`-scale frames; record the actual value in C2.
Declare it through the body's `stack_size`, as `lower_app_resource_path` does
with `EXE_PATH_FRAME_LOCALS`.

**Code organization.** The parser is its own emitter,
`emit_linux_user_dirs_documents(ctx, home, vregs) -> BaseBytes`, in a new file
`src/codegen/builtins/os/gen_user_dirs.rs`. It is Linux-only, and
`lower_user_documents_path` calls it only in the `PlatformFamily::Linux` arm.

### 4.3 macOS and Windows `userDocumentsPath`

- **macOS:** `emit_posix_home_base`, with suffix `"/Documents"`.
- **Windows:** `emit_os_wide_string("userDocuments")`, using the
  `FOLDERID_Documents` GUID.

### 4.4 `Info.plist`

Add this to `app_info_plist`, after `NSPrincipalClass`:

```
  <key>NSDocumentsFolderUsageDescription</key>
  <string>{name} reads and writes files in your Documents folder.</string>
```

It is unconditional. The text shows only if the app actually touches Documents,
and the link stage does not know which calls the program makes. See Open
Decisions.

### 4.5 Tables, descriptors, docs

- **Tables:** the same set as plan-156-B B2:
  - `OS_ENV_LOCK_CALLS`;
  - `data_objects.rs`, for both errors;
  - the supported-call lists for all three backends, and the `plan.rs` import
    rows. Linux `userDocumentsPath` adds `open`, `read`, `close`,
    `__errno_location`; Windows adds `SHGetKnownFolderPath`, `CoTaskMemFree`;
  - `registry/mod.rs`'s `String`-result lists;
  - `audit/collect/source.rs` (`"environment"`, plus the fallible list; Linux
    Documents also reads a file, so check whether the audit has a
    `"filesystem"` capability and list it there too);
  - the `STRING_GROW_FNS`, `STRING_SELF_UPDATE_SPELLINGS` and
    `SELF_UPDATE_TABLE` rows (`GrowKind::HostPath`).
- **Man pages** (descriptor prose, `.ai/man-content.md`):
  - `userHomePath`: the home folder per OS; that it is not app-specific; the
    `HOME` then passwd fallback in developer terms ("if `HOME` is unset, the
    account's home directory is used").
  - `userDocumentsPath`: the per-OS table; that Linux honors the desktop's
    localized Documents folder (`~/Dokumente`); that Windows follows OneDrive;
    that macOS asks the user for permission the first time a program uses the
    folder; the sandbox sentence; and "may not exist — check with
    `fs::directoryExists`".

## Compatibility / Format Impact

- There are two new `os` members.
- **The macOS `.app` `Info.plist` gains one key.** It changes for every
  macOS app build. `scripts/test-macapp.sh` and the plist unit tests are
  expected to see it. Any golden containing a rendered `Info.plist` must be
  found by `rg -l NSPrincipalClass tests` and updated in C5 as an expected
  diff.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick in the same commit as
> the work. Use `- [~]` for partial, mark moot as `- [x] ~~text~~ — moot:
> <evidence>`, and fill `Commit:` on landing. **An unticked box means NOT DONE.**

### Phase C1: `userHomePath` on all three platforms

- [ ] Add a `"userHome"` arm (`FOLDERID_Profile`) to `emit_os_wide_string`.
- [ ] Add `src/codegen/builtins/os/func_user_home_path.rs`: the descriptor,
      man prose, and `lower_user_home_path` (§4.1).
- [ ] Add the table entries (§4.5) for `os.userHomePath`.
- [ ] Add `tests/rt-behavior/os/func_os_userHomePath_valid`, which prints only
      booleans and codes, branching on `os::name()`:
  - `HOME=/tmp/mfb156h` → `/tmp/mfb156h`;
  - `HOME=/tmp/mfb156h/` → the same (trimmed);
  - `userHomePath("a/b")` = `userHomePath() & "/a/b"`;
  - no trailing `/`;
  - the four dot-component codes `77030002`, plus `..\x` on Windows;
  - `HOME` unset → starts with `/`;
  - in place, including after `setEnv("HOME", …)`;
  - Windows: `userHomePath()` = `os::getEnv("USERPROFILE")`.
- [ ] Add `tests/syntax/os/func_os_userHomePath_invalid`, covering a wrong type
      and a wrong arity.
- [ ] Extend the every-backend-lowers test and the Win64 window structural test
      with `os.userHomePath`.

Acceptance: the fixture passes locally, on 2226 and on 2230.
  Check:
  - `FILTER=func_os_userHome scripts/test-accept.sh target/debug/mfb target/accept-actual-156c`
    (est. 2 min).
  - `FILTER=func_os_userHome scripts/linux-runtime-proof.sh target/debug/mfb 2226 linux-aarch64 glibc`
    (est. 3 min).
  - The 2230 ship-and-run (the plan-156-B B4 recipe), plus a PowerShell
    `GetFolderPath('UserProfile')` comparison (est. 5 min).
Commit: —

### Phase C2: `userDocumentsPath` on macOS and Windows, and the Linux parser

- [ ] Add a `"userDocuments"` arm (`FOLDERID_Documents`) to
      `emit_os_wide_string`.
- [ ] Add `src/codegen/builtins/os/gen_user_dirs.rs` with
      `emit_linux_user_dirs_documents` (§4.2). Record in this task line:
  - whether `open_flag_set` includes `O_CLOEXEC`;
  - the body's final `stack_size`.
- [ ] Add `src/codegen/builtins/os/func_user_documents_path.rs`: the
      descriptor, man prose, and `lower_user_documents_path` (§4.2, §4.3).
- [ ] Add the table entries (§4.5) for `os.userDocumentsPath`.

Acceptance: the function lowers on all five targets, and on macOS it returns
`<home>/Documents`.
  Check: `cargo test --bin mfb os` → pass (est. 6 min), and a `/tmp` scratch
  program on macOS with `HOME=/tmp/d` prints `/tmp/d/Documents` (est. 1 min).
Commit: —

### Phase C3: the Linux Documents fixture, and GLib equivalence

- [ ] Add `tests/rt-behavior/os/func_os_userDocumentsPath_valid`. The program
      builds each case itself: it sets `HOME` and `XDG_CONFIG_HOME` to
      directories under `fs::tempDirectory()`, writes `user-dirs.dirs` with
      `fs::writeText`, calls the function, and prints a boolean against the
      per-OS expectation (on macOS and Windows the file is ignored, and the
      expectation says so). Cases:
  1. No file → `<home>/Documents`.
  2. `XDG_DOCUMENTS_DIR="$HOME/Dokumente"` → `<home>/Dokumente`.
  3. `XDG_DOCUMENTS_DIR="/srv/docs/"` → `/srv/docs`.
  4. `   XDG_DOCUMENTS_DIR =  "$HOME/x"` (whitespace) → `<home>/x`.
  5. `XDG_DOCUMENTS_DIR=$HOME/x` (no quotes) → ignored → `<home>/Documents`.
  6. `XDG_DOCUMENTS_DIR="relative/x"` → ignored → `<home>/Documents`.
  7. Two valid lines → the last one wins.
  8. `# XDG_DOCUMENTS_DIR="/c"` (a comment) → ignored.
  9. `XDG_DOCUMENTS_DIR="$HOME"` → `<home>`.
  10. 5000 bytes of comment lines, then a valid line → it is found (proves the
      chunk refill).
  11. A value longer than 4096 bytes → ignored.
  12. `XDG_CONFIG_HOME` unset → reads `<home>/.config/user-dirs.dirs`.
  13. `XDG_CONFIG_HOME=rel` → ignored → `<home>/.config`.
  14. Relative join: `userDocumentsPath("r.txt")` = base `& "/r.txt"`.
  15. The dot-component codes.
  16. In place.
- [ ] Add `tests/syntax/os/func_os_userDocumentsPath_invalid`.
- [ ] GLib equivalence on 2226 (a one-off in `/tmp`): for cases 1–13, run
      `g_get_user_special_dir(G_USER_DIRECTORY_DOCUMENTS)` with the same
      `HOME`, `XDG_CONFIG_HOME` and file, using `python3` `gi` or a 10-line C
      program against the installed GLib. Compare it with our output, where
      GLib's NULL means our `<home>/Documents`. Record any disagreement in
      Corrections, and fix our parser toward GLib.

Acceptance: the fixture passes on macOS and on 2226, and every GLib-comparable
case agrees.
  Check:
  - `FILTER=func_os_userDocuments scripts/test-accept.sh target/debug/mfb target/accept-actual-156c`
    (est. 2 min).
  - `FILTER=func_os_userDocuments scripts/linux-runtime-proof.sh target/debug/mfb 2226 linux-aarch64 glibc`
    (est. 3 min).
  - The GLib probe (est. 5 min). This is the only check that catches a wrong
    parse rule, because our fixture encodes our reading of GLib, not GLib's.
Commit: —

### Phase C4: the Windows Documents proof

- [ ] Run the C3 fixture on 2230 (the B4 recipe).
- [ ] Compare `os::userDocumentsPath()` against PowerShell
      `[Environment]::GetFolderPath('MyDocuments')`.

Acceptance: the fixture output matches its golden, and the PowerShell
comparison is `TRUE`.
  Check: the 2230 run (est. 5 min).
Commit: —

### Phase C5: `Info.plist`, self-update rows, and byte-identity source

- [ ] Add the §4.4 key to `app_info_plist`. Update
      `app_info_plist_has_required_bundle_keys` to assert it, and fix any
      rendered-plist golden (`rg -l NSPrincipalClass tests`) as an expected
      diff.
- [ ] Add the `SELF_UPDATE_TABLE`, `STRING_GROW_FNS` and spelling rows for both
      members (if C1/C2 did not already add them to keep the census green; the
      census must pass at every commit).
- [ ] Add `io::print(os::userHomePath("h"))` and
      `io::print(os::userDocumentsPath("d"))` to
      `tests/byte-identity/os/src/main.mfb`.
- [ ] Update `src/docs/spec/stdlib/14_os.md`. Add a "User directories" section
      with the table, the GLib-mirroring parser rules from §4.2 (with a
      `[[src/codegen/builtins/os/gen_user_dirs.rs:emit_linux_user_dirs_documents]]`
      citation), the known-folder mapping, the sandbox note and the plist key.
      Add both calls to the error table.

Acceptance: the plist test passes, the self-update census passes, and the spec
builds.
  Check: `cargo test --bin mfb app_info_plist self_update_census spec` → pass
  (est. 6 min). Then `target/debug/mfb build --app` on a scratch project, and
  `plutil -p build/*.app/Contents/Info.plist | grep NSDocumentsFolderUsageDescription`
  → 1 line (est. 2 min).
Commit: —

## Validation Plan

- **Tests:** two valid fixtures (Documents with 16 cases), two invalid
  fixtures, the every-backend-lowers and Win64 window tests, the self-update
  census, and the plist unit test.
- **Coverage check:** open `--nir` for the Documents fixture and confirm the
  Linux build references `open`/`read` from the `_mfb_rt_os_userDocumentsPath`
  helper. That shows the parser is compiled in and not dead-stripped.
- **Runtime proof:** macOS (local), Linux aarch64 glibc (2226) plus the GLib
  equivalence probe, and Windows (2230). musl, x86_64 and riscv64 Linux are not
  executed unless plan-156-D's re-probe finds a box.
- **Doc sync:** two man pages, `14_os.md`.
- **Final gate:** plan-156-D.

## Open Decisions

- **The plist key is unconditional, not gated on `os::userDocumentsPath` use.**
  Recommended: unconditional. The link stage does not see the program's calls,
  and an unused purpose string is inert. The alternative, threading
  "program uses Documents" from codegen into the macOS link stage, adds a
  cross-stage channel for one string. Revisit only if App Review objects.
- **When Linux `user-dirs.dirs` sets a folder that does not exist,** we return
  it anyway, which matches GLib and the no-existence-check contract.

## Corrections

## Summary

The risk is the Linux streaming parser: new byte-level native code whose rules
come from GLib. It lands behind a 16-case fixture that includes a chunk-boundary
case, plus a direct GLib comparison on a real GTK box. Everything else composes
plan-156-B's pieces.
