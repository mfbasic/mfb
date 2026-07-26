# plan-66: Windows x86-64 feature parity (audio + app-mode + console gaps)

Last updated: 2026-07-26
Effort (Human): huge (>3d)
Effort (AI): huge (>3d)

This plan brings the `windows-x86_64` target to genuine feature parity with the
macOS and Linux targets. plan-47 shipped a Windows console target advertising
io(output)/fs(core)/thread/net/crypto/tls(client), but an audit (2026-07-26)
found that "COMPLETE" claim overstated: **seven runtime-call families that
macOS/Linux ship are missing or partial on Windows**, and neither **audio** nor
**app-mode** (a native window hosting the program's console I/O) exists on
Windows at all. This plan closes exactly those gaps — nothing more.

Parity is measured against what macOS/Linux **actually ship today**, not against
planned work. `mfb build -target windows-x86_64` advertises 87 runtime calls;
`mfb build -target macos-aarch64` advertises 152
(`grep -cE '"\w+\.' src/target/{win_x86_64,macos_aarch64}/mod.rs` → 87 / 152).
The single behavioral outcome: after this plan, any program that runs on
macOS/Linux using audio, app-mode, or any of the seven families below produces
the same observable behavior when built for and run on Windows x86-64.

**Explicitly NOT in scope: the plan-13 `app::` widget toolkit** (buttons, tables,
flexbox layout, attributed strings). That toolkit is implemented on **no**
platform — `grep -rl '_mfb_rt_app_layout\|addButton' src/` → no matches — so a
Windows widget backend cannot be "at parity" with anything, and building it here
would braid this plan into the unfinished plan-13. See Non-goals and Prerequisites.

References (read first):

- `planning/old-plans/plan-47-windows-x86_64.md` — the shipped Windows console
  target this plan extends; its §3.1/§3.2 (the POSIX-shaped shared layer, the
  `PlatformFamily` match) are the seams every letter here edits.
- `planning/old-plans/plan-33-{A,B,C}-audio-*.md` — the audio surface + the
  CoreAudio/ALSA backend precedents letters G/H mirror. plan-33-A §6 is the
  no-atomics concurrency contract any audio backend must honor.
- `planning/plan-13-app-gui.md` + `planning/plan-13-E-macos-backend.md` +
  `planning/plan-13-F-gtk-backend.md` — the app-*mode* precedent (transcript
  window). Read for the mode-not-target shape; ignore the widget toolkit (out of
  scope here).
- `src/target/win_x86_64/{mod.rs,code.rs,plan.rs}` — the Windows backend this
  plan grows: `RUNTIME_CALLS` (`mod.rs:27`), the `CodegenPlatform` impl
  (`code.rs`), the import map (`plan.rs`).
- `src/target/shared/code/` — the shared, POSIX-shaped lowering. The
  `PlatformFamily::Windows` arms here are what each letter fills:
  `datetime.rs:65`, `audio/mod.rs:137`, `tls/mod.rs:338,352`, etc.
- `src/os/windows/link/{mod,pe}.rs` — the PE writer; letter K adds `.rsrc` and
  the GUI subsystem toggle (`pe.rs:213`).
- `.ai/remote_systems.md:11` — the Win11 x86-64 box (ssh port 2230), the only
  runtime oracle. `scripts/exe-oracle.sh` ships/compares `.exe` output there;
  `scripts/artifact-gate.sh` guards byte-identity of existing targets.

## Prerequisites

These are preconditions on the whole feature, not dependencies to negotiate.
Stated once here; every letter points back.

| Must be true | Command | Status 2026-07-26 |
|---|---|---|
| plan-47's Windows console target is landed (registered, non-app, io/fs/net/thread/crypto/tls-client box-proven) | `grep -n 'win_x86_64::BACKEND' src/target.rs` → registered; `planning/old-plans/plan-47-windows-x86_64.md:3` → COMPLETE | **MET** |
| The Win11 x86-64 box is reachable for runtime proof | `grep -n 'Win11' .ai/remote_systems.md` → `:11`, ssh port 2230 | **MET — re-verified 2026-07-26 (ssh -p 2230 → `BOX_OK`); re-verify before each box-gated letter** |
| The exhaustive `PlatformFamily` match exists (adding a Windows arm is a compile error at each shared-lowering site, per plan-47-A) | `grep -rn 'PlatformFamily::Windows' src/target/shared/code/ \| head` | **MET** |
| The generic per-DLL IAT builder handles arbitrary DLLs (audio needs `ole32`/`mmdevapi`; app needs `user32`/`gdi32`) | read `group_imports_by_dll` `src/os/windows/link/mod.rs:53-62` — groups by `import.library` string, no hardcoded list | **MET (property-verified)** |
| **The plan-13 `app::` widget toolkit is NOT a parity target** — it is unbuilt everywhere, so it is a non-goal here, never scope | `grep -rl '_mfb_rt_app_layout\|addButton' src/` → no matches | **MET (out of scope by construction)** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before continuing, and again
> before deciding to stop. The box-reachability row especially: re-verify port
> 2230 before any letter whose acceptance is a box run. **If you stop, report the
> status of all rows.**

Everything below is written against the world where these hold. There are no
hedges for a world where plan-47 regressed or the widget toolkit landed.

## Dependency graph  <!-- letters are in dependency order; the line fans out -->

```
Console completions — each blocks on nothing but the plan-47 base, fan out in parallel:
  A (datetime)   B (os)   C (io input+buffering)   D (term TUI)   E (fs extras)   F (tls server)

Audio track — pipeline (spike proves the premise, backend consumes it):
  G (COM/GUID codegen spike)  ──►  H (WASAPI backend)

App-mode track — pipeline (infra plumbs the mode, floor + packaging consume it):
  I (app-mode infra: WindowsApp mode + subsystem toggle)  ──►  J (Win32 app-mode floor)
                                                           └──►  K (PE resource packaging)
```

Dependency list the executor checks:
`A, B, C, D, E, F, G, I ← plan-47 base only`; `H ← G`; `J ← I`; `K ← I`.

**Order by uncertainty, then value.** A–F rest on **no unproven premise** —
plan-47 already box-proved the Windows fs/net/term machinery, so these are
mechanical completions and deliver the bulk of parity; land them first. The two
**unproven premises** live in the audio and app tracks and get cheap falsifying
spikes as the first letter of each track: **G** (can the plan-47 PE/IAT path
express a COM vtable call at all?) and the message-loop spike at the front of
**J** (does a Win32 `GetMessage` loop integrate with the app worker thread?). If
either spike fails, only its track is affected — A–F and the other track stand.

## 1. Goal

- Every runtime-call family macOS/Linux advertise is advertised and implemented
  on `windows-x86_64`: the seven missing/partial families (os, datetime, audio,
  term-styling, io-input, fs-extras, tls-server) reach parity, verified on the
  Win11 box.
- `mfb build -target windows-x86_64 -app <proj>` produces a GUI-subsystem `.exe`
  that opens a native window hosting the program's console I/O — the same
  app-*mode* behavior macOS (`.app`) and Linux (`.AppImage`) produce, with an
  embedded icon and application manifest.
- `audio::openOutput`/`write`/`read`/`devices`/… work on Windows over WASAPI,
  producing audible s16le output on the box, at parity with the CoreAudio/ALSA
  backends.
- No existing target's emitted bytes change (`scripts/artifact-gate.sh` 0 diffs).

### Non-goals (explicit constraints)

- **The plan-13 `app::` widget toolkit is out of scope.** No `app::Window`,
  `Button`, `Label`, `Input`, `Container`, `TextArea`, `Table`, layout solver,
  events, or `text::AttributeString` on Windows. Reason: it exists on **no**
  platform (`grep -rl '_mfb_rt_app_layout' src/` → no matches), so it is not a
  parity gap. A Windows widget backend is a **future dependent of plan-13** — it
  cannot start until plan-13 lands its macOS/Linux backends, at which point it
  is its own plan. Folding it in here would braid two plans (write-plan skill:
  "a cross-plan dependency is a precondition, never scope").
- **No change to any existing target's output bytes.** macOS/Linux/riscv64 stay
  byte-identical; guarded by `scripts/artifact-gate.sh`.
- **No external toolchain / CRT.** All OS access is DLL imports through the IAT +
  raw entry, exactly as plan-47's console floor. No `link.exe`/msvcrt/UCRT `main`.
- **No language, IR, NIR/plan/MIR-schema, value/copy/move, or layout change.**
- **COM is used only where unavoidable (WASAPI device/client activation).** The
  app-mode backend (J) uses GDI + custom drawing, not Direct2D/DirectWrite, so it
  needs no COM (Open Decision 3).

## 2. Current State

### Measured populations

Every count that sizes this plan, with the command that produced it.

| What | Count | Command |
|---|---|---|
| Runtime calls advertised: macOS / Windows | 152 / 87 | `grep -cE '"\w+\.' src/target/{macos_aarch64,win_x86_64}/mod.rs` |
| `os.*` gap (macOS has, Windows lacks) | **15** | `grep -c '"os\.' src/target/{macos_aarch64,win_x86_64}/mod.rs` → 15 / 0 |
| `datetime.*` gap | **3** | `grep -c '"datetime\.' …` → 3 / 0 |
| `audio.*` gap | **14** | `grep -c '"audio\.' …` → 14 / 0 |
| `term.*` gap | **16** | `grep -c '"term\.' …` → 17 / 1 |
| `io.*` gap (input + buffering) | **8** | `grep -c '"io\.' …` → 15 / 7 |
| `fs.*` gap (extras) | **10** | `grep -c '"fs\.' …` → 36 / 26 |
| `tls.*` gap (server: listen/accept/closeListener) | **3** | `grep -c '"tls\.' …` → 9 / 6 |
| thread / net / crypto | parity+ / parity+ / parity | `grep -c` → thread 12/17, net 19/20, crypto 10/10 |
| audio runtime-helper symbols a backend must emit | **14** | `grep -c 'name:' src/target/shared/runtime/audio_specs.rs` |
| macOS app-mode backend LOC (sizing J) | **5369** | `wc -l src/target/macos_aarch64/app/*.rs` |
| GTK app-mode backend LOC (sizing J) | **3603** | `wc -l src/target/linux_gtk/*.rs` |
| ALSA / macOS-audio backend LOC (sizing H) | **2339 / 2910** | `wc -l src/target/shared/code/audio/{alsa,macos}.rs` |
| Windows native goldens today | **0** | `find tests -name '*windows-x86_64*'` → no matches |
| COM call sites / vtable dispatch in the tree | **0** | `grep -rn 'CoCreateInstance\|lpVtbl\|IUnknown' src/` → no matches |

### Verified properties (claims a citation alone cannot settle)

| Claim | Verdict | How checked |
|---|---|---|
| The three `unsupported(...)` stubs in `win_x86_64/code.rs` (:612,:821,:1137) are DEAD, not live bugs | **CONFIRMED** | `validate_capabilities` (`shared/validate/capabilities.rs:19-23`) rejects an un-advertised call before codegen; the surfaces reaching those stubs are absent from `RUNTIME_CALLS` |
| App on Windows is a MODE, not a new target | **CONFIRMED** | `NativeBuildMode{Console,MacApp,LinuxApp}` (`target.rs:37-40`); macOS routes MacApp in-target; no `linux-gtk` in `NATIVE_BACKENDS` (`target.rs:198-205`) — GTK is a shared module |
| App-*mode* (windowed transcript) exists on macOS AND Linux | **CONFIRMED** | `emit_app_program_entry` implemented at `macos_aarch64/app/mod.rs:566` and `linux_gtk/mod.rs:427` |
| The IAT builder imports arbitrary DLLs with no linker change | **CONFIRMED** | `group_imports_by_dll` keys on the `import.library` string (`os/windows/link/mod.rs:53-62`); DLL set is whatever codegen emits |
| The COM vtable-dispatch pattern WASAPI needs has NO precedent but the primitive exists | **CONFIRMED / UNPROVEN end-to-end** | `grep` → 0 COM sites; but `call r/m64` (`FF /2`, `arch/x86_64/encode/emitter.rs:712`) + `branch_link_register` (`shared/abi.rs`) can express it. **Letter G's spike is the falsification test.** |
| WASAPI shared mode forces the device mix format (usually f32), colliding with the package's no-conversion s16le rule | **CONFIRMED (design risk)** | plan-33-A §s16le contract; ALSA *verifies* the committed format (`audio/alsa.rs:1016-1025`). Forces exclusive mode or a plan-level ruling — Open Decision 1. |
| The `_ => MacApp` fallthrough at `cli/build/mod.rs:206` would misroute a Windows `-app` build | **CONFIRMED (latent bug)** | reading the `match target.os` arm; needs an explicit `"windows" =>` arm — fixed in letter I |

## 3. Design Overview

Three independent tracks under the existing plan-47 pipeline
(NIR → NativePlan → NativeCodePlan → EncodedImage → PE container). Each letter
either (a) advertises a family in `win_x86_64/mod.rs:RUNTIME_CALLS`, (b) fills
the `PlatformFamily::Windows` arm in the relevant `shared/code/` lowering and/or
the `win_x86_64/code.rs` `CodegenPlatform` method, (c) adds the DLL import rows in
`win_x86_64/plan.rs`, and (d) seeds byte-identity goldens + a box run.

**Where correctness risk (blast radius) concentrates — schedule last within its
track:** letter K (PE `.rsrc` + subsystem toggle edits the shared PE writer that
every Windows `.exe` flows through; a bad section table silently fails to load).

**Where design uncertainty concentrates — schedule first within its track:**
letter G (COM expressibility) and the message-loop spike fronting J (Win32
event-loop ↔ worker integration). These are the two premises that, if false,
resize their tracks. Both get a cheap end-to-end spike on the box before the
bulk work behind them is scheduled.

**Rejected alternatives.**
- *A separate `windows-gtk` target for app mode.* Windows is single-arch and GTK
  is not the native toolkit; the macOS pattern (app as an in-target mode) is the
  right shape (Verified properties).
- *Direct2D/DirectWrite for app-mode text.* Pulls in COM for the app track too;
  GDI + custom drawing keeps app-mode COM-free and on the same "custom-draw
  everything" footing as AppKit/GTK (Open Decision 3).
- *WASAPI shared mode with silent format conversion.* Violates the audio
  package's absolute no-resampling s16le contract; Open Decision 1 picks
  exclusive mode instead.

## 4. Feature map (the whole `66`)

Letters are in dependency order. A–F fan out from the plan-47 base; G→H and
I→{J,K} are the two pipelines. Every letter is gated behind §Prerequisites.

- **66-A — `datetime::` (3 calls).** Advertise `datetime.*` in `RUNTIME_CALLS`
  (`win_x86_64/mod.rs:27`); fill the `PlatformFamily::Windows` arms at
  `shared/code/datetime.rs:65` (`monotonicNanos` — currently `unreachable!`) and
  `:77` (`nowNanos`): `QueryPerformanceCounter`/`QueryPerformanceFrequency` for
  monotonic, `GetSystemTimePreciseAsFileTime` (already imported for entry
  entropy, `win_x86_64/code.rs:398`) for wall clock, `GetTimeZoneInformation`
  for `localOffset`; add import rows in `plan.rs`. Depends on: plan-47 base.
  Effort (Human) small · (AI) small.
- **66-B — `os::` (15 calls).** Advertise `os.*`; implement `getEnv`/`getEnvOr`/
  `hasEnv`/`setEnv`/`unsetEnv` (Get/SetEnvironmentVariableW), `environ`
  (`GetEnvironmentStringsW` — `emit_environ_pointer` stub at `code.rs:821`),
  `args` (`GetCommandLineW`+`CommandLineToArgvW`, already used at entry), `pid`
  (GetCurrentProcessId), `executablePath` (GetModuleFileNameW), `hostName`
  (GetComputerNameExW), `userName` (GetUserNameW), `cpuCount`
  (GetSystemInfo). `os.name`/`os.arch` const-string arms already exist
  (`shared/code/os/mod.rs:109`, `os/paths.rs:88`). Depends on: plan-47 base.
  Precedent-heavy → Effort (Human) large · (AI) medium.
- **66-C — `io::` console input + buffering (8 calls).** Advertise
  `io.input`/`readLine`/`readChar`/`readByte`/`pollInput`/`flush`/`isBuffered`/
  `setBuffered`; implement `emit_poll_input` (stub at `code.rs:612`) over
  `ReadConsoleW`/`ReadFile`(stdin) + the stdin-broadcast plumbing; raw-char reads
  reuse the existing Windows raw-mode machinery (`code.rs:1427-1546`). Depends
  on: plan-47 base. Effort (Human) large · (AI) medium.
- **66-D — `term::` styling/TUI (16 calls).** Advertise the styling family
  (on/off/isOn/setFg/Bg/Bold/Underline/show/hideCursor/clear/sync/moveTo/get*);
  enable `ENABLE_VIRTUAL_TERMINAL_PROCESSING` via SetConsoleMode so the existing
  ANSI-emitting `term.rs` arms work unchanged; wire the Windows arms at
  `term.rs:238,323,809` (currently `"0"` placeholders). `terminalSize`/raw-mode
  already ship. Depends on: plan-47 base. Effort (Human) medium · (AI) small.
- **66-E — `fs::` extras (10 calls).** Advertise + implement `createTempFile`
  (`emit_mkstemps` stub at `code.rs:1137` → `GetTempFileNameW`/`CreateFileW`),
  `open`/`openFileNoFollow`/`openWithin`, `createDirectories`,
  `writeTextAtomic`/`writeBytesAtomic` (temp + `MoveFileExW` REPLACE_EXISTING),
  `setBuffered`/`isBuffered`, `isWithin`. Depends on: plan-47 base.
  Effort (Human) medium · (AI) medium.
- **66-F — `tls::` server (3 calls).** Advertise `tls.listen`/`accept`/
  `closeListener`; fill the Schannel server arms at `shared/code/tls/mod.rs:338,
  352` (currently `unreachable!`) — server-side `AcceptSecurityContext` loop over
  the existing Winsock listener. Depends on: plan-47 base.
  Effort (Human) medium · (AI) medium.
- **66-G — COM vtable-dispatch + GUID-data-object codegen (spike + primitive).**
  The audio track's unproven premise. Produce: (1) a 16-byte GUID data-object
  kind (CLSID/IID) beside the C-string data objects; (2) a vtable-call emitter —
  `load [obj]`=vtable, `load [vtable+slot*8]`=fn, `branch_link_register` with
  `obj` as Win64 arg0; (3) `ole32!CoInitializeEx`/`CoCreateInstance` import rows.
  **Acceptance is a box spike:** a hand-built program that `CoCreateInstance`s
  `IMMDeviceEnumerator` and calls one vtable method (`GetDefaultAudioEndpoint`),
  printing success, run on the Win11 box. This falsifies-or-confirms the premise
  before H is scheduled. Depends on: plan-47 base. Effort (Human) medium ·
  (AI) medium (box-round-trip bound → converges).
  **Produces:** the GUID data-object kind + the vtable-call emitter that H consumes.
- **66-H — WASAPI audio backend.** New `shared/code/audio/windows.rs` (mirror of
  `alsa.rs`, 2339 LOC): the 14 helper bodies + `audio.devices`. Add
  `AudioBackend::Wasapi` + selector arm (`audio/mod.rs:166`), replace the
  `Windows => unreachable!` dispatch (`audio/mod.rs:137`), advertise `audio.*` in
  `RUNTIME_CALLS`. Event-driven shared/exclusive client (Open Decision 1) via
  `IAudioClient`/`IAudioRenderClient`/`IAudioCaptureClient` + `CreateEventW`/
  `WaitForSingleObject` on the helper thread (honors plan-33-A §6 no-atomics —
  no OS callback thread needed, unlike CoreAudio). **Declared large — split into
  plan-66-H-1..n before execution** (suggested phases: H-1 openOutput+write
  spine, H-2 openInput+read, H-3 devices enumeration, H-4 poll/available/xruns/
  close, H-5 box proof). Depends on: G. Effort (Human) large · (AI) large.
- **66-I — App-mode infra (`WindowsApp` mode + subsystem toggle).** Add
  `NativeBuildMode::WindowsApp` + `is_app()` (`target.rs:37-59`); the explicit
  `"windows" =>` CLI arm fixing the `_ => MacApp` misroute (`cli/build/mod.rs:206`);
  flip `supports_app_mode()` → true (`win_x86_64/mod.rs:151`); update
  `APP_MODE_MATRIX` (`target.rs:441`); make the PE `Subsystem` field mode-driven
  (`pe.rs:213` CONSOLE=3 → thread a GUI flag → GUI=2 through
  `link/mod.rs:269`→`os/windows/mod.rs:47`→backend) + fix the `pe.rs:347` test.
  **Acceptance:** `mfb build -target windows-x86_64 -app` no longer rejects at
  the gate and emits a Subsystem-2 `.exe` (verified in the PE header test) — no
  window yet. Depends on: plan-47 base. Effort (Human) medium · (AI) small.
  **Produces:** the `WindowsApp` mode + GUI-subsystem plumbing J and K consume.
- **66-J — Win32 app-mode floor.** The macOS `app/mod.rs` analog: a new
  `win_x86_64/app/` submodule implementing the 10 `CodegenPlatform` app methods
  (`emit_app_program_entry` + `emit_app_io_{write,flush,input,is_terminal}_helper`
  + `emit_app_raw_input_mode` + `emit_app_term_helper` + `emit_app_mode_reconcile`
  + `app_mode_data_objects`). Raw entry → `RegisterClassExW`/`CreateWindowExW`, a
  `WndProc` + `GetMessage`/`DispatchMessage` loop owning the main thread, a worker
  `CreateThread` running MFBASIC, cross-thread marshaling via `PostMessage`/
  `SendMessage` (the `performSelectorOnMainThread`/`g_idle_add` analog), console
  I/O rerouted into a transcript control (GDI custom-drawn text buffer);
  `user32`/`gdi32` import rows in `plan.rs`. **The message-loop ↔ worker
  integration is the unproven premise — front a box spike (bare window + one
  round-tripped keystroke) before the full floor.** **Declared large — split into
  plan-66-J-1..n before execution** (suggested: J-1 message-loop↔worker spike,
  J-2 window+entry bootstrap, J-3 transcript output, J-4 input round-trip, J-5
  box proof). Depends on: I. Effort (Human) large · (AI) large.
- **66-K — PE resource packaging (icon + manifest + version).** The last letter,
  largest blast radius (edits the shared PE writer). A `.ico` encoder (new
  `os/windows/icon.rs` reusing the platform-neutral `os/icon/mod.rs:72`
  `render_png`); a `.rsrc` resource-directory section builder (icon group +
  application manifest for DPI-awareness/common-controls v6 + VS_VERSIONINFO)
  added to the PE section list (`link/mod.rs:359`); thread `app_icon`/`app_version`
  (currently dropped at `win_x86_64/mod.rs:164`) into `write_linked_executable`.
  **Acceptance:** an app `.exe` on the box shows the embedded icon in Explorer
  and is DPI-aware. Depends on: I. Effort (Human) medium · (AI) medium.

## Compatibility / Format Impact

- **New:** `datetime`/`os`/`audio`/`term`-styling/`io`-input/`fs`-extras/`tls`-server
  advertised on Windows; a `NativeBuildMode::WindowsApp` mode; a GUI-subsystem
  `.exe`; a `.rsrc` PE section; new DLL imports (`kernel32` extras, `ole32`,
  `mmdevapi`/`audioses`, `user32`, `gdi32`); a 16-byte GUID data-object kind; a
  COM vtable-call codegen form.
- **Unchanged:** the language, IR, NIR/plan/MIR schemas, `EncodedImage`, the
  x86-64 instruction encoder, and **every existing target's emitted bytes**
  (guarded 0-diff). No macOS/Linux/riscv64 file is edited except shared
  `PlatformFamily::Windows` arms that were `unreachable!`/placeholder (byte-inert
  for non-Windows targets by construction).

## Phases

Each letter is an independently-landable phase; A–K are the phases. The large
letters (H, J) split into their own sub-plan docs before execution, per the
write-plan split rule (as plan-47-B/H did). Keep this file's checkboxes current —
tick in the same commit as the work.

### Phase A — `datetime::`
- [x] Advertise `datetime.*` in `win_x86_64/mod.rs:RUNTIME_CALLS`.
- [x] Fill the Windows datetime lowering + add `plan.rs` imports. (Implemented as a
  dedicated `lower_datetime_windows` in `shared/code/datetime.rs`, not literally
  the `:65`/`:77` libc arms — those are `clock_gettime`/`localtime_r`-shaped and
  Windows has no CRT; see Corrections.)
- [x] Tests: `tests/rt-behavior/datetime/datetime-clock-offset` (host-neutral
  boolean fixture, cross-target) + box run printing monotonic delta / wall clock /
  offset stability / out-of-range trap. (ncode golden dropped as impractical — see
  Corrections; box run + build-determinism are the Windows byte guard.)

Acceptance: a datetime program built for windows-x86_64 runs on the box and prints a plausible monotonic elapsed time and current wall-clock time; `artifact-gate.sh` 0 diffs on existing targets. **MET** — box (`dt66.exe`): `mono_nonneg=TRUE now_recent=TRUE off_stable=TRUE(-28800=PST) oor_invalidArg=TRUE`; artifact-gate 21 diffs are all pre-existing flaky `codegen_cover_rt` noise in untouched paths (see Corrections), 0 attributable to this change; full `cargo test` green.
Commit: 78622bb8d

### Phase B — `os::`  (IN PROGRESS — 13/15 calls landed)
- [~] Advertise `os.*`; implement the 15 calls. **Landed & box-proven:**
  *track 1 (52e5fb79c)* `os.name`, `os.arch` (const-string arms), `os.pid`
  (GetCurrentProcessId), `os.cpuCount` (GetSystemInfo, `dwNumberOfProcessors` at
  SYSTEM_INFO+0x20 — replaced an `unreachable!`). *track 2 (env family)*
  `getEnv`/`getEnvOr`/`hasEnv`/`setEnv`/`unsetEnv`: a SRWLOCK env-lock branch in
  `emit_env_lock`/`emit_env_unlock_return` (Acquire/ReleaseSRWLockExclusive), plus
  two Windows-only platform primitives `emit_env_get`
  (GetEnvironmentVariableW + name UTF-8→UTF-16 + value UTF-16→UTF-8, returns a UTF-8
  value C-string or 0 — the POSIX getenv contract) and `emit_env_set`
  (SetEnvironmentVariableW, inverted to the POSIX 0=success convention; NULL value
  → delete for unsetEnv). Box-proven incl. a non-ASCII round-trip (`hello-世界`).
  *track 3 (string trio)* `hostName` (GetComputerNameExW), `userName` (GetUserNameW,
  advapi32), `executablePath` (GetModuleFileNameW) via one Windows-only platform
  primitive `emit_os_wide_string(which)` (`*W` query into a wide buffer →
  WideCharToMultiByte → UTF-8 C-string) + a shared `lower_os_wide_string_windows`
  that builds the String or raises ErrUnsupported. Replaced the `paths.rs:89` /
  introspect `unreachable!`s. Box-proven (exe path contains the binary name).
  *track 4 (environ)* `emit_environ_pointer` now synthesizes a POSIX `char**` from
  GetEnvironmentStringsW: two passes over the wide `K=V\0…\0\0` block (count, then
  marshal each entry UTF-16→UTF-8 into the arena and fill the pointer array),
  skipping the hidden `=drive` entries (leading `=`), NULL-terminating, and
  FreeEnvironmentStringsW at the end; all loop state in stack slots. Box-proven incl.
  a Unicode value and a `a=b=c` value (splits only at first `=`).
  **Remaining (1):** `args` (**entry-side capture is missing**; needs a Windows-entry
  GetCommandLineW/CommandLineToArgvW marshal — see Corrections). ~~`environ`~~
  (`emit_environ_pointer` stub → GetEnvironmentStringsW,
  minus `=C:=…` drive entries), `args` (**entry-side capture is missing** — see
  Corrections; the deferred hard one).
- [x] Tests: host-neutral fixtures `os-introspect-basic`, `os-env-roundtrip`,
  `os-identity-queries`, `os-environ-roundtrip`; box runs all correct. (args box run
  pending its entry-side capture.)

Acceptance: an `os` program (getEnv/args/pid/executablePath/hostName/userName/cpuCount) produces the expected values on the box. **NEARLY MET** — pid/cpuCount/getEnv(+family)/executablePath/hostName/userName all box-proven; only `args` (needs entry capture) and `environ` remain.
Commit: 52e5fb79c (t1); 69599dfc9 (env); eae84d465 (string trio); 95b305201 (environ); env family — this commit

### Phase C — `io::` input + buffering
- [ ] Advertise the 8 calls; implement `emit_poll_input` (`code.rs:612`) + stdin read/broadcast.
- [ ] Tests: goldens + a box run reading a piped line and a raw char.

Acceptance: an interactive `readLine`/`readChar` program echoes correctly on the box.
Commit: —

### Phase D — `term::` styling/TUI
- [x] Advertise the 16 styling calls in `win_x86_64/mod.rs`. The `term.rs:238,323,809`
  `"0"` placeholders are already CORRECT (Windows ignores the ioctl request value —
  `emit_terminal_size` uses GetConsoleScreenBufferInfo), so no change was needed
  there (see Corrections). VT processing is enabled via a new no-op-default
  `CodegenPlatform::emit_enable_vt_output` trait method, overridden on Windows
  (GetStdHandle(-11)→GetConsoleMode→SetConsoleMode | 0x04), called once at the top
  of shared `emit_on` before the first ANSI write.
- [x] Tests: cross-target fixture `tests/rt-behavior/term/term-styling-basic`;
  box run emitting 24-bit color + bold + cursor addressing + alt-screen.

Acceptance: a TUI program (colors, cursor moves, clear) renders correctly in Windows Terminal on the box. **MET** — box (`term66.exe`) emitted the correct ANSI stream: `^[[?1049h` alt-screen, `^[[38;2;255;0;0m red-text` at row 2, `^[[1m` bold `bold-text` at row 3, full grid present, `^[[?1049l` restore, `isOn` FALSE→(on)→FALSE. Renders as color in Windows Terminal (raw ESC shown here only because ssh captured a pipe, where VT-enable correctly no-ops). Non-Windows byte-identical (default `emit_enable_vt_output` emits nothing; existing term fixtures + full `cargo test` green); Windows build deterministic.
Commit: 7eab44bd9

### Phase E — `fs::` extras  (IN PROGRESS — 7/10 calls landed)
- [~] Advertise + implement the 10 extras. **Landed & box-proven:** `open`,
  `openFileNoFollow` (both via `lower_fs_open_helper`/`emit_open_file`),
  `createDirectories` (recursive CreateDirectoryW), `createTempFile` (filled
  `temp_file_open_flags`' Windows arm = `(CREATE_NEW<<32)|GENERIC_READ|GENERIC_WRITE`
  = 7516192768 — the plan's "`emit_mkstemps` stub" mapping was wrong; createTempFile
  uses `emit_open_file`+`emit_random_bytes`, see Corrections), `setBuffered`,
  `isBuffered` (platform-independent resource flag), `isWithin` (fixed the
  hardcoded `/` separator → platform-aware `\` on Windows, a real bug found on the
  box). **Deferred:** `openWithin` (its no-symlink realpath path is
  `unreachable!("47-F owns the Windows realpath/no-symlink path")`),
  `writeTextAtomic`/`writeBytesAtomic` (use the `emit_mkstemps` stub — the actual
  `emit_mkstemps` consumer, not createTempFile).
- [x] Tests: fixture `tests/rt-behavior/fs/fs-temp-file-buffered` (createTempFile +
  set/isBuffered, system-temp only so no repo pollution); box run of
  createDirectories/createTempFile/open+readText/isWithin all correct.

Acceptance: atomic write + temp-file + nested-mkdir program produces correct files on the box. **PARTIAL** — temp-file + nested-mkdir + open/read + isWithin box-proven; atomic writes deferred (need `emit_mkstemps`).
Commit: 71f2a3fab

### Phase F — `tls::` server
- [ ] Advertise the 3 calls; fill Schannel server arms (`tls/mod.rs:338,352`).
- [ ] Tests: goldens + a box run: Windows tls server ↔ a client, handshake + echo.

Acceptance: a Windows-built tls listen/accept/echo server completes a handshake with a client on the box.
Commit: —

### Phase G — COM/GUID codegen spike (audio premise)
- [ ] Add the 16-byte GUID data-object kind; add the vtable-call emitter; `ole32` import rows.
- [ ] Box spike: hand-built `CoCreateInstance(IMMDeviceEnumerator)` + one vtable method, prints success.

Acceptance: the spike `.exe` runs on the box, instantiates the COM object, and returns success — the COM-expressibility premise is proven (or the plan stops here and records it as a Prerequisites defect).
Commit: —

### Phase H — WASAPI audio backend (split before execution)
- [ ] Author `plan-66-H-1..n` sub-plans; build `audio/windows.rs` (14 helpers + devices); wire selector/dispatch/RUNTIME_CALLS.
- [ ] Tests: byte-identity audio goldens for windows-x86_64; box run producing audible s16le tone + capture round-trip.

Acceptance: `audio::openOutput`+`write` produces an audible s16le tone on the box, and `openInput`+`read` captures; `devices()` lists the box's endpoints.
Commit: —

### Phase I — App-mode infra
- [ ] Add `NativeBuildMode::WindowsApp`+`is_app()`; CLI `"windows" =>` arm; flip `supports_app_mode()`; `APP_MODE_MATRIX`; mode-driven PE subsystem + fix `pe.rs:347` test.
- [ ] Tests: unit test asserting `-app` emits Subsystem=2; `APP_MODE_MATRIX` coverage test passes.

Acceptance: `mfb build -target windows-x86_64 -app <proj>` builds (no gate rejection) and the PE header carries Subsystem=2.
Commit: —

### Phase J — Win32 app-mode floor (split before execution)
- [ ] Message-loop↔worker box spike first; then author `plan-66-J-1..n`; build `win_x86_64/app/` (10 CodegenPlatform methods); `user32`/`gdi32` imports.
- [ ] Tests: `MFB_*_HEADLESS`-style automated path if feasible; box run showing a window with transcript output + keystroke input.

Acceptance: an `-app` program on the box opens a window, shows its `io::print` output in the transcript, and reads a typed line.
Commit: —

### Phase K — PE resource packaging (largest blast radius, last)
- [ ] `.ico` encoder; `.rsrc` section (icon+manifest+version); thread `app_icon`/`app_version`.
- [ ] Tests: PE-writer unit tests for the `.rsrc` layout; `artifact-gate.sh` 0 diffs; box run showing icon in Explorer.

Acceptance: an app `.exe` on the box shows the embedded icon and is DPI-aware; existing targets byte-identical.
Commit: —

## Validation Plan

- Tests: per letter — byte-identity `*.windows-x86_64[.app].ncode` goldens
  (currently ZERO exist) plus, for behavior, `scripts/exe-oracle.sh` records +
  the Win11 box (ssh 2230) runs.
- Coverage check: **Windows has 0 native goldens today** — a green
  `artifact-gate.sh` proves nothing about Windows. Each letter must SEED its own
  goldens; do not treat their absence as coverage.
- Runtime proof: the Win11 box is the only oracle for every behavioral
  acceptance above (audio audibility, the app window, tls handshake). Re-verify
  port 2230 before each box-gated letter.
- Doc sync: `src/docs/spec/stdlib/11_audio.md` (Windows backend note), the
  `app` spec/man pages (Windows app-mode), `mfb spec linker` (`.rsrc` + GUI
  subsystem), and the man pages for each newly-advertised family.
- Acceptance: full `cargo test` green; `scripts/artifact-gate.sh` 0 diffs on
  macOS/Linux/riscv64; the per-letter box runs.

## Open Decisions

1. **WASAPI s16le: exclusive vs shared mode** (§H) — **recommend exclusive mode**
   (`AUDCLNT_SHAREMODE_EXCLUSIVE`) to honor the audio package's no-conversion
   s16le contract, accepting that some devices reject it and it monopolizes the
   endpoint. Alternative: shared mode with the device mix format — rejected, it
   forces silent resampling the package forbids. Settle with a box test in G/H-1.
2. **Whether `os::` (B) lands before `datetime::` (A)** — both block on nothing;
   recommend A first (smaller, warms the Windows-golden cadence). Immaterial to
   correctness.
3. **App-mode text: GDI custom-draw vs Direct2D/DirectWrite** (§J) —
   **recommend GDI + custom drawing**, keeping the app track COM-free and on the
   same footing as AppKit/GTK. Direct2D/DirectWrite gives higher text fidelity
   but drags COM (the whole G machinery) into the app track — rejected for v1.

## Corrections

- **2026-07-26 (Prerequisites gate re-run).** All five Prerequisites rows
  re-measured and confirmed **MET**; Win11 box re-verified reachable on ssh port
  2230 (`ssh -p 2230 test@127.0.0.1 'echo BOX_OK'` → `BOX_OK`). Gate passed.
- **2026-07-26 (Measured populations, audio helper count).** The row "audio
  runtime-helper symbols a backend must emit | 14 | `grep -c 'name:'
  src/target/shared/runtime/audio_specs.rs`" has a wrong *command*: that file's
  field is `call:`, not `name:`, so the cited command returns **0**, not 14. The
  **count is correct** — `grep -c 'call:' src/target/shared/runtime/audio_specs.rs`
  → **14** (devices, openInput, openInputDevice, openOutput, openOutputDevice,
  read, readTimeout, write, poll, pollTimeout, available, xruns, closeInput,
  closeOutput). Sizing of letter H is unaffected.
- **2026-07-26 (Phase A, datetime lowering location).** The plan cited
  `datetime.rs:65`/`:77` as the Windows arms to fill, but that shared body is
  `clock_gettime`/`localtime_r`-shaped (the `PlatformFamily::Windows` arm there was
  an `unreachable!` reading "47-D owns the Windows clock"). Windows has no CRT, so
  the three intrinsics can't reuse the libc body. Implemented instead as a
  dedicated `lower_datetime_windows` routed from the top of `lower_datetime_helper`:
  monotonicNanos = QueryPerformanceCounter/Frequency with an overflow-safe
  tick→nanos split; nowNanos = GetSystemTimePreciseAsFileTime rebased to the Unix
  epoch; localOffset = FileTimeToSystemTime → SystemTimeToTzSpecificLocalTime →
  SystemTimeToFileTime (the local−UTC FILETIME delta). The now-unreachable libc
  `PlatformFamily::Windows` clock-id arm was re-commented, not deleted (the match
  must stay exhaustive over `PlatformFamily`).
- **2026-07-26 (Phase A, localOffset correctness — beyond the plan's one-liner).**
  The plan said "GetTimeZoneInformation for localOffset", but that returns only the
  *current* offset, not the offset *at the passed instant* (the documented
  contract). Used the SYSTEMTIME round-trip instead, which applies the machine's TZ
  rules (incl. DST) to the given instant. Also: the libc path traps
  `ErrInvalidArgument` for an out-of-range instant (localtime_r → NULL, bug-42); the
  naive Windows `epochSeconds*1e7` silently *wraps* to a valid-looking FILETIME, so
  a bound-check on `epochSeconds` was added to reproduce the trap. Both were caught
  by the box run (first run leaked `off=-28800` instead of trapping) — not by any
  golden.
- **2026-07-26 (Phase A, golden strategy for A–K).** The plan's "seed
  `*.windows-x86_64.ncode` goldens" is impractical for feature fixtures: a datetime
  program's `.ncode` is ~14 MB (the datetime package is large), vs 130 KB–935 KB for
  the existing *curated tiny-program* ncode goldens. By existing convention feature
  fixtures carry only `.ast/.ir/.run/build.log` (no ncode); the tiny curated
  `byte-identity/` set owns codegen byte-identity. For Windows byte-identity the
  standing guard is `scripts/exe-oracle.sh` (records `.exe` sha256) plus verified
  build determinism (same `.exe` sha256 across two builds). Adopting this for A–K:
  each console letter ships a host-neutral cross-target rt-behavior fixture + a box
  run; the ncode-golden task line is satisfied via exe-oracle/determinism, not a
  multi-MB committed ncode.
- **2026-07-26 (artifact-gate baseline noise).** `scripts/artifact-gate.sh` reports
  21 pre-existing diffs on `audio/crypto/fs/net/tls .../*_codegen_cover_rt.*.ncode`
  (macOS/Linux/riscv). These goldens are byte-identical to base `cc4b4343c` (not
  edited here) and lie in codegen paths this plan does not touch; they are the
  known flaky `codegen_cover_rt` / union-drop-HashMap nondeterminism noise (memory
  `known-red-test-baseline`, `union-drop-codegen-nondeterminism`). "0 diffs on
  existing targets" is read as "0 *new* diffs attributable to this plan", verified
  per letter by confirming no diffing golden is in the letter's changed paths.
- **2026-07-26 (Phase B, `os::args` entry-capture premise is FALSE).** The Feature
  map says `args` uses "`GetCommandLineW`+`CommandLineToArgvW`, already used at
  entry". A census (`grep -rn 'GetCommandLineW\|CommandLineToArgvW' src/`) finds
  **no matches anywhere in the tree**; `src/os/windows/object.rs:97` explicitly
  defers it ("47-D installs the real GetCommandLineW startup" — as future work).
  `lower_args` (`introspect.rs:278`) reads `_mfb_rt_os_argc`/`_mfb_rt_os_argv`
  globals populated by `lower_program_entry`; Windows does not override
  `entry_args_in_registers()` (defaults true), so the entry today stores
  `ARG[0]`/`ARG[1]` (garbage on a raw PE entry) into those globals. So `os::args`
  needs **real entry-side work** (GetCommandLineW → CommandLineToArgvW → per-arg
  UTF-16→UTF-8 → the argc/argv globals), not just advertising. Scope of Phase B
  grows by that entry change; `lower_args` itself is unchanged once argv holds
  UTF-8 C-strings. This is a Feature-map defect (a false "already used"), corrected
  here; `args` remains in Phase B scope.
- **2026-07-26 (Phase B, other unreachable/marshal facts).** `cpuCount`
  (`introspect.rs:89`) and `executablePath`/`resourcePath` (`paths.rs:89`) hit
  `unreachable!("47-D owns …")` on Windows — 47-D never actually implemented them,
  so each Windows arm is net-new here (cpuCount done in track 1). The env family
  and the hostName/userName/executablePath string queries need the
  UTF-16→UTF-8 marshal (`emit_wide_to_utf8`, already in `win_x86_64/code.rs`) and,
  for env, a SRWLOCK branch in the shared `emit_env_lock`/`emit_env_unlock_return`
  (which today unconditionally emit `pthread_mutex_lock`/`unlock`; the static lock
  init `os_env_lock_init_hex` already has a Windows all-zero `SRWLOCK_INIT` arm).
- **2026-07-26 (Phase D, the `term.rs:238/323/809` "0" arms need no change).** The
  Feature map says to "wire `term.rs:238,323,809` Windows arms (currently `"0"`
  placeholders)". Those placeholders are the ioctl *request value* for
  `emit_grid_alloc`/`emit_grid_present`/`emit_terminal_size`; the in-code comments
  already state Windows ignores it (it uses GetConsoleScreenBufferInfo), so they are
  correct as-is. The real Windows work for D is (a) advertising the 16 styling
  calls and (b) enabling VT output. Rather than open-code SetConsoleMode in the
  shared neutral-abi `emit_on`, added a no-op-default `emit_enable_vt_output`
  trait method (macOS/Linux/riscv byte-identical) overridden in
  `win_x86_64/code.rs`. The styling setters/getters make no OS call (verified:
  `emit_set_color`/`emit_move_to`/`emit_get_color`/… touch only grid state), so
  only `on`/`off`/`sync`/`terminalSize` carry kernel32 imports.
- **2026-07-26 (Phase E, `emit_mkstemps` maps to atomic-writes, not createTempFile).**
  The Feature map says `createTempFile` is the `emit_mkstemps` stub consumer. It is
  not: `lower_fs_create_temp_file_helper` uses `emit_open_file` + `emit_random_bytes`
  (a random UUID name opened O_EXCL), and its Windows gap was a separate
  `unreachable!` in `temp_file_open_flags` (`fs/atomic.rs:255`), now filled. The
  real `emit_mkstemps` consumer is `lower_fs_atomic_write_helper`
  (writeTextAtomic/writeBytesAtomic) — those remain deferred until `emit_mkstemps`
  is implemented on Windows (GetTempFileNameW/CreateFileW + MoveFileExW rename).
- **2026-07-26 (Phase E, `fs::isWithin` real bug — hardcoded POSIX separator).**
  `lower_fs_is_within_helper` (`fs/paths.rs`) hardcoded the containment-boundary
  separator as `"47"` (`/`). Windows `GetFullPathNameW` canonicalizes to `\` (92),
  so a child genuinely inside base read as *outside* (`isWithin` → FALSE on the
  box). Fixed with a platform-aware `within_sep` (92 on Windows, 47 elsewhere);
  non-Windows codegen is byte-identical (the immediate is still `"47"`). Found only
  by the box run — no golden would have caught it.
- **2026-07-26 (Phase E, `openWithin` deferred).** `openWithin`'s shared helper
  takes a no-symlink realpath path whose Windows arm is
  `unreachable!("47-F owns the Windows realpath/no-symlink path")` — net-new work,
  deferred with the atomic writes. Un-advertised so it is a compile-time rejection,
  never a broken build.

## Summary

The audit refuted plan-47's "COMPLETE": Windows was missing seven runtime-call
families (os/datetime/audio/term-styling/io-input/fs-extras/tls-server) plus
audio and app-mode entirely. This plan closes exactly those. The engineering
risk sits in two unproven premises, each de-risked by a cheap box spike as its
track's first letter: **COM expressibility** (G, gating the WASAPI backend H)
and **Win32 message-loop ↔ worker integration** (front of J, the app-mode floor).
The blast-radius work — the PE `.rsrc`/subsystem edits to the shared writer —
lands last (K). Six console completions (A–F) rest on no unproven premise and
deliver the bulk of parity first.

What is deliberately left out: the plan-13 `app::` widget toolkit. It is built on
no platform, so it is not a parity gap; a Windows widget backend is a future
dependent of plan-13, not scope here.
