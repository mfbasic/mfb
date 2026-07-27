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

### Phase B — `os::`  (COMPLETE — 15/15 calls landed)
- [x] Advertise `os.*`; implement the 15 calls. **Landed & box-proven:**
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
  *track 5 (args)* the last call: a `defers_arg_capture()` predicate makes the
  Windows entry SKIP the pre-arena register store, and a new post-arena
  `emit_build_argv_utf8` (GetCommandLineW → CommandLineToArgvW → per-arg
  UTF-16→UTF-8 arena marshal → NULL-terminated `char**`, LocalFree) leaves argc/argv
  for the shared entry to store into the `os::args` globals. Gated on
  `capture_args` (== uses `os.args`), so non-args programs keep a byte-identical
  entry; non-Windows keeps the pre-arena path. Box-proven with real args incl. a
  quoted `gamma with spaces` and Unicode `世界`. ~~`environ`~~
  (`emit_environ_pointer` stub → GetEnvironmentStringsW,
  minus `=C:=…` drive entries), `args` (**entry-side capture is missing** — see
  Corrections; the deferred hard one).
- [x] Tests: host-neutral fixtures `os-introspect-basic`, `os-env-roundtrip`,
  `os-identity-queries`, `os-environ-roundtrip`, `os-args-basic`; box runs all
  correct incl. `os::args` with real quoted + Unicode arguments.

Acceptance: an `os` program (getEnv/args/pid/executablePath/hostName/userName/cpuCount) produces the expected values on the box. **MET** — all 15 calls box-proven on 2230, incl. `os::args alpha beta "gamma with spaces" 世界` → four args parsed and UTF-8-marshaled. Non-args + non-Windows entries byte-identical; full `cargo test` green.
Commit: 52e5fb79c (t1); 69599dfc9 (env); eae84d465 (string trio); 95b305201 (environ); aa138536a (args)

### Phase C — `io::` input + buffering  (COMPLETE — 8/8)
- [x] Advertise the 8 calls. `emit_read_file` now resolves fd 0 → GetStdHandle(
  STD_INPUT)+ReadFile (mirroring emit_write); `emit_poll_input` waits on the stdin
  handle (WaitForSingleObject, mapping WAIT_OBJECT_0/WAIT_TIMEOUT → 1/0). The
  stdin-broadcast log needed two CRT-less seams (see Corrections): a
  `emit_heap_alloc`/`emit_heap_free` platform pair (default = libc malloc/free,
  byte-identical; Windows = GetProcessHeap+HeapAlloc/HeapFree) and routing its
  pthread mutex/condvar names through the existing `emit_thread_external_call`
  pthread→Win32 (SRWLOCK/CONDITION_VARIABLE) seam on Windows only. isBuffered/
  setBuffered/flush are platform-independent. Removed the now-dead `unsupported()`
  helper (every Windows floor stub is implemented).
- [x] Tests: cross-target fixture `io-input-eof-buffering` (isBuffered/setBuffered/
  flush + readLine-on-EOF trap, identical on host and box); box runs with piped
  input.

Acceptance: an interactive `readLine`/`readChar` program echoes correctly on the box. **MET** — box (piped): `readChar`=X, `readByte`=121(y), `readLine`=done interleave correctly through the broadcast log; `io::input` prints its prompt and reads the line; `pollInput(0)`=TRUE with data waiting; two-line pipe reads both lines. Non-Windows byte-identical; full `cargo test` green.
Commit: 4cf083fc1

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

### Phase E — `fs::` extras  (10/10 — COMPLETE, box-proven)
- [x] Advertise + implement the 10 extras. **Landed & box-proven:** `open`,
  `openFileNoFollow` (now a real Windows whole-path no-symlink open — see the
  no-symlink Corrections; the earlier "via `lower_fs_open_helper`/`emit_open_file`"
  claim was overstated, it would panic at `io.rs:478`),
  `createDirectories` (recursive CreateDirectoryW), `createTempFile` (filled
  `temp_file_open_flags`' Windows arm = `(CREATE_NEW<<32)|GENERIC_READ|GENERIC_WRITE`
  = 7516192768 — the plan's "`emit_mkstemps` stub" mapping was wrong; createTempFile
  uses `emit_open_file`+`emit_random_bytes`, see Corrections), `setBuffered`,
  `isBuffered` (platform-independent resource flag), `isWithin` (fixed the
  hardcoded `/` separator → platform-aware `\` on Windows, a real bug found on the
  box). `writeTextAtomic`/`writeBytesAtomic` now land via a real `emit_mkstemps`
  (Windows): fill the template's `XXXXXX` markers with random lowercase letters
  (BCryptGenRandom + mod 26) and CreateFileW(CREATE_NEW) with a 100-try
  collision-retry loop, returning the handle-as-fd; the shared helper then
  writes/flushes/closes and MoveFileExW-renames. Box-proven (Unicode text + a byte
  list). **`openWithin` + `openFileNoFollow` (the whole-path no-symlink pair) now
  DONE** via the open-then-verify design the plan called for: two Windows
  `CodegenPlatform` hooks — `emit_verify_nofollow` (handle's
  `GetFinalPathNameByHandleW` path == `GetFullPathNameW` lexical canonical of the
  request, else refuse) and `emit_verify_within` (opened handle's resolved final
  path is under the root's own resolved final path + `\`, else refuse) — both
  leaving 0/1 in the return register; the shared helpers close the fd and raise
  `ErrAccessDenied` on a 1. The `io.rs:478`/`:815` `unreachable!`s are now
  `Windows => false` (Windows takes the plain-open path + the post-open verify).
- [x] Tests: fixtures `fs-temp-file-buffered` (createTempFile + set/isBuffered) and
  `fs-atomic-write` (writeTextAtomic/writeBytesAtomic under `target/`); box runs of
  createDirectories/createTempFile/open+readText/isWithin/atomic-writes all correct.
  no-symlink box proof (2230, real reparse points via `mklink`/`mklink /J`):
  `direct=ok`, `finallink=denied`, `interlink=denied` (junction), `contained=ok`,
  `absolute=invalid`, `dotdot=invalid`, `escape=denied` (junction escape), EXIT=0.

Acceptance: atomic write + temp-file + nested-mkdir program produces correct files on the box. **MET (10/10)** — all nine landed calls re-box-proven after the merge-drop repair (`mkdir=ok temp-buffered=TRUE roundtrip=hello-世界 bytes=ok within-yes/no`), and `openFileNoFollow`/`openWithin` box-proven refusing real Windows reparse-point escapes while opening non-symlink paths.
Commit: 71f2a3fab (7/10); 28078edae (atomic writes, 9/10); 87e25d6b0 (restore merge-dropped advertising); e958b9332 (Windows no-symlink openFileNoFollow + openWithin, 10/10)

### Phase F — `tls::` server  (COMPLETE — box-proven)
- [x] **Advertised + implemented** `tls.listen`/`accept`/`closeListener` over Schannel
  (new `tls/schannel_server.rs`, ~1000 LOC): listen binds a Winsock socket + builds
  the server credential (PEM cert/key → DER via CryptStringToBinaryA →
  CertCreateCertificateContext; PKCS#8→PKCS#1 via CryptDecodeObjectEx ×2 →
  CryptImportKey into a **named keyset container** → CERT_KEY_PROV_INFO(AT_KEYEXCHANGE)
  → AcquireCredentialsHandleW(SECPKG_CRED_INBOUND)); accept does WSAPoll+accept then
  the AcceptSecurityContext handshake loop reusing the client SecBuffer/STATE
  machinery; closeListener frees the credential + socket. **Two real fixes:**
  (1) `SEC_E_NO_CREDENTIALS` — an ephemeral/VERIFYCONTEXT key isn't reachable by
  Schannel's CryptGetUserKey, so a *named* container is required; (2) the legacy
  CAPI key cannot do TLS 1.3 RSA-PSS, so `SCHANNEL_CRED.grbitEnabledProtocols` is
  pinned to `SP_PROT_TLS1_2_SERVER` (0x400) — without it AcceptSecurityContext
  writes 0 bytes on the first ClientHello. Also fixed a **latent client bug**
  (`schannel_impl.rs`): `tls::connect` left `RECV_LEN` non-zero after the handshake,
  stranding every read from a TLS 1.2 server (invisible against google's TLS 1.3).
- [x] Tests: box handshake proofs — `openssl s_client -tls1_2` full handshake +
  encrypted echo (`hello-tls`→`echo:hello-tls`, server `server_done`); a PowerShell
  `SslStream` client (`proto=Tls12 cipher=Aes256`, echo received); and an MFB↔MFB
  run with the cert trusted. Client-vs-google (TLS 1.3) still passes (no regression).
  Full `cargo test` green.

Acceptance: a Windows-built tls listen/accept/echo server completes a handshake with a client on the box. **MET** — box-proven (openssl + PowerShell SslStream + MFB↔MFB).
Commit: 8138a3e2d (server TLS1.2 pin + client RECV_LEN fix; base impl in 0cbfe4463)

### Phase G — COM/GUID codegen spike (audio premise)  (COMPLETE — subsumed by H)
- [x] The COM-expressibility premise is PROVEN. The two primitives the plan called
  for already existed and needed no new kind/emitter: a 16-byte GUID/CLSID/IID is a
  `kind:"raw"` data object (arbitrary bytes, Windows GUID byte order); a COM vtable
  call is `load vtable=[obj]; load fn=[vtable+slot*8]; branch_link_register(fn)` →
  the x86 encoder's existing `call r/m64` (FF /2). `ole32` import rows landed with H.
- [x] Box proof folded into H: the WASAPI backend `CoCreateInstance`s
  `IMMDeviceEnumerator` and calls `GetDefaultAudioEndpoint`/`Activate`/… through
  vtables live on the box (`devices()`→2 endpoints). The premise held.

Acceptance: COM object instantiated + a vtable method returns success on the box. **MET** — proven end-to-end by H's live WASAPI COM calls (no separate spike needed).
Commit: 48e20c1e9 / merged 7b0d6ccf7

### Phase H — WASAPI audio backend  (COMPLETE — box-proven)
- [x] Built `audio/windows{,_open,_io,_devices}.rs` (all 14 calls + devices);
  `AudioBackend::Wasapi` + selector/dispatch/RUNTIME_CALLS + `plan.rs` `audio.*`
  imports (ole32). COM vtable dispatch throughout. **Also fixed a pre-existing Win64
  codegen defect** (xmm6–15 callee-saved saved with a GPR `str_u64` → encoder
  rejected `xmm10`; now `str q`/`ldr q`, byte-identical off Win64) and a `devices()`
  frame-slot collision. Open Decision 1: EXCLUSIVE s16le attempted first, SHARED
  mix-format fallback (integer s16↔f32 conversion) when the device rejects it.
- [x] Tests: box run — `devices=2`; agent proof `openOutput`+`write` (s16→f32
  SHARED) and `openInput`+`read` (512 bytes) both OK. Full `cargo test` green;
  `artifact-gate.sh` **0 diffs** (the golden regen fixed the stale codegen_cover_rt
  noise). Non-Windows byte-identical.

Acceptance: openOutput+write produces s16le on the box, openInput+read captures, devices() lists endpoints. **MET** (audibility is a by-ear check; the pipeline is box-proven).
Commit: 48e20c1e9 / merged 7b0d6ccf7

### Phase I — App-mode infra  (COMPLETE)
- [x] Added `NativeBuildMode::WindowsApp`+`is_app()`+`APP_MODE_MATRIX`; the CLI
  `"windows" => WindowsApp` arm (fixing the `_ => MacApp` misroute); flipped
  `supports_app_mode()`→true; `lower_validated_module` accepts WindowsApp; mode-driven
  PE Subsystem WINDOWS_GUI(2) via `pe::write_image` gui flag + fixed the subsystem
  test; threaded `app_icon`/`app_version` toward the writer for K.
- [x] Tests: `app_mode_links_gui_subsystem` asserts Subsystem=2; `APP_MODE_MATRIX`
  coverage green. Full `cargo test` green.

Acceptance: `mfb build -target windows-x86_64 -app` no longer rejects at the gate; PE header Subsystem=2. **MET** (Subsystem=2 unit-test-proven; note: a full `-app` build still errors at codegen until J-full lands — a clean error, not a wrong result).
Commit: d9025af8c / merged e5c500020

### Phase J — Win32 app-mode floor  (SPIKE DONE, box-proven; FULL FLOOR NOT BUILT)
- [~] **Message-loop↔worker spike: DONE and BOX-PROVEN** (`spike.rs`, test-only):
  a hand-assembled GUI-subsystem PE does RegisterClassExW→CreateWindowExW→
  CreateThread(worker)→GetMessage loop; the worker cross-thread `PostMessageW`s a
  `WM_APP` the WndProc handles on the UI thread, writes a proof-of-life, and
  PostQuitMessages. Box (2230): exit 0 + `SPIKE_OK`. **The unproven premise HOLDS.**
  **NOT built:** the 10 `CodegenPlatform` app methods in `win_x86_64/app/`
  (emit_app_program_entry + io write/flush/input/is_terminal + raw_input + term +
  mode_reconcile + app_mode_data_objects) — the GDI custom-drawn transcript buffer
  (WM_PAINT/TextOutW, scrollback, cross-thread print marshaling, line-input editing),
  modeled on macOS `app/*.rs` (5369 LOC) + `linux_gtk` (3603). Needs `user32`/`gdi32`
  imports and a plan-66-J-1..n split. A full `-app` build errors cleanly at codegen
  until this lands (no stub shipped — Hard Completion Gate).

**Sub-plan split (executed on resume 2026-07-26; J-1 already done):**
- **J-1 — message-loop↔worker spike.** DONE + box-proven (above), `spike.rs`.
- **J-2 — bootstrap floor. DONE + box-proven.** New `win_x86_64/app/mod.rs`:
  `emit_app_program_entry` → `_main`+worker+`WndProc` (ported from `spike.rs` via the
  neutral abi), `MFB_WINAPP_HEADLESS` gate, `emit_app_io_write_helper` (WriteFile to
  the inherited std handle), flush/is_terminal, `app_mode_data_objects` (UTF-16), +
  `plan.rs` `app_mode_imports` (user32/kernel32). Subsystem=2 verified; 4 unit tests.
  Box (2230, headless, stdout→file): a `-app` hello ran the worker/program and emitted
  `hello-from-winapp`/`second-line`. Commit `b6642fe62`.
  (original J-2 description:) New `win_x86_64/app/` module. `emit_app_program_entry`
  emits `_main` (GetModuleHandleW → RegisterClassExW(WndProc) → CreateWindowExW →
  CreateThread(worker,hwnd) → GetMessageW loop) modeled on `spike.rs`, plus the
  worker shim (calls `MACAPP_PROGRAM_SYMBOL`) and a `WndProc`. An
  `MFB_WINAPP_HEADLESS` env gate skips the window/loop (like macOS's
  `MFB_MACAPP_HEADLESS`) so CI/box can exercise the worker without a GUI. Plus the
  minimum io methods so a printing program builds: `emit_app_io_write_helper`,
  `emit_app_io_flush_helper`, `emit_app_io_is_terminal_helper`. `user32`/`gdi32`
  import rows. **Acceptance:** a `-app hello` build produces a Subsystem-2 `.exe`
  that, run headless on the box, executes the worker and emits its output.
- **J-3 — transcript. DONE + box-proven.** Uses a stock multiline Win32 **EDIT
  control** as the transcript (matching macOS, which uses stock NSTextView for the
  transcript and custom-draw only for the TUI grid — a documented deviation from
  Open Decision 3's "GDI custom-draw everything", which was an oversimplification).
  `_main` creates the EDIT child + stores its HWND in a writable global;
  `emit_app_io_write_helper` routes there when the window is attached
  (MultiByteToWideChar the UTF-8 print text → `EM_SETSEL`/`EM_REPLACESEL`
  SendMessageW append — the worker→UI cross-thread marshal Windows does
  synchronously) else the J-2 std-handle fallback. The worker's `finish` posts
  `WM_APP_QUIT` so the **UI thread** does teardown (a worker `ExitProcess` faults in
  GDI teardown). **Fixed a real SIGSEGV**: `MultiByteToWideChar`'s `int` return has
  garbage high `rax` bits; using it as a pointer offset for the NUL-terminate wild-
  pointered — now NUL-terminate at `wbuf + str[0]*2` (trusted byte length). An
  `MFB_WINAPP_DUMP` gate makes `_main` read the transcript back (`WM_GETTEXT`) to
  stdout for ssh verification. 8 app unit tests. **Acceptance MET** — box (2230):
  with `MFB_WINAPP_DUMP` a `-app` hello's transcript readback shows
  `hello-from-winapp`/`second-line` (io::print reached the EDIT control); without
  it the non-headless run exits cleanly (0) with no side effect; headless still
  prints. Commit `<pending>`.
- **J-4 — input round-trip.** A pipe (`CreatePipe`) whose read end is dup'd onto the
  worker's stdin path; WndProc `WM_CHAR`/`WM_KEYDOWN` does line editing + commit to
  the pipe; `emit_app_io_input_helper` + `emit_app_raw_input_mode` wired. **Acceptance:**
  box run reads a typed line.
- **J-5 — term:: TUI grid + mode reconcile + full box proof.** `emit_app_term_helper`
  (cell-grid custom draw: colors/cursor/clear, modeled on `term_view.rs`),
  `emit_app_mode_reconcile` + reconcile data objects. **Acceptance:** the full Phase-J
  acceptance below, box-proven.
- [ ] Tests: box run showing a window with transcript output + keystroke input.

Acceptance: an `-app` program opens a window, shows `io::print` output, reads a typed line. **NOT MET** — spike proves the mechanics; building J-2..J-5 (see split above).
Commit: 9718fd97d (spike) / merged e5c500020

### Phase K — PE resource packaging  (NOT STARTED)
- [ ] `.ico` encoder (`os/windows/icon.rs` reusing `os/icon/mod.rs::render_png`);
  `.rsrc` section (icon group + DPI/common-controls manifest + VS_VERSIONINFO) added
  to the PE section list; thread `app_icon`/`app_version` into the writer (the params
  are already threaded there by I, so K plugs in at section-assembly). Gate `.rsrc`
  on app-mode-with-icon so console builds stay byte-identical.
- [ ] Tests: PE-writer unit tests for the `.rsrc` layout; artifact-gate 0 diffs; box
  run showing the icon in Explorer.

Acceptance: an app `.exe` shows the embedded icon and is DPI-aware; existing targets byte-identical. **NOT MET** — not started; depends on J-full for a real `-app` `.exe` to carry the icon.
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

- **2026-07-26 (Phase E REGRESSION — merge dropped the advertising for 9 landed
  fs calls).** On resume, a census (`comm -23` of macOS vs Windows `"fs.` advertise
  sets) found that `win_x86_64/mod.rs` advertises only 26 fs calls where macOS has
  36. Bisection (`git show 71f2a3fab -- …/mod.rs` added them; `git show
  a3de67f94:…/mod.rs` lacks them; no commit after `a3de67f94` touched the file)
  proved a **stale main→P-66 merge silently dropped** the Phase-E advertise block
  (`fs.open`, `fs.createDirectories`, `fs.createTempFile`, `fs.writeTextAtomic`,
  `fs.writeBytesAtomic`, `fs.setBuffered`, `fs.isBuffered`, `fs.isWithin`, plus
  `fs.openFileNoFollow`) AND the plan.rs import arms for `createTempFile`/atomic/
  `isWithin` — while the code.rs + shared impls survived. So those box-proven calls
  were being **rejected at `validate_capabilities`** on this branch. Restored the 8
  genuinely-working calls' advertising + plan.rs arms; re-box-proved on 2230
  (`mkdir=ok temp-buffered=TRUE roundtrip=hello-世界 bytes=ok within-yes=TRUE
  within-no=FALSE`, EXIT=0). `openFileNoFollow`/`openWithin` restored separately —
  see next. This is the [[subagent-edits-can-silently-vanish]] / merge-drift class.
- **2026-07-26 (Phase E — `openFileNoFollow` claim was overstated; shares
  openWithin's missing primitive).** The Phase-E text claims `openFileNoFollow`
  landed "via `lower_fs_open_helper`/`emit_open_file`". It did NOT work: lowering it
  (`no_follow=true`) evaluates `fs/io.rs:478`'s `PlatformFamily::Windows =>
  unreachable!("47-F owns the Windows openFileNoFollow path")` and panics the
  compiler. It was advertised-but-unused at 71f2a3fab, so the panic never fired in
  that letter's box run. Both `openFileNoFollow` and `openWithin` need the same
  missing Windows whole-path no-symlink primitive — implemented together via a
  `GetFinalPathNameByHandleW` open-then-verify (see the openWithin entry below).
- **2026-07-26 (Prerequisites gate re-run — resume for J-full/K/E-openWithin).**
  Gate re-run in a fresh integration worktree `.claude/worktrees/P-66`
  (`worktree-P-66` off main `b2227871a`, which already carries A–I+F/G/H merged as
  ancestor `a3de67f94`). All five rows re-measured **MET**: `win_x86_64::BACKEND`
  registered (`target.rs:210`); box reachable (`ssh -p 2230 … 'echo BOX_OK'` →
  `BOX_OK`); `PlatformFamily::Windows` exhaustive across `shared/code/`;
  `group_imports_by_dll` string-keyed (`os/windows/link/mod.rs:57`); widget toolkit
  still absent (`grep -rl '_mfb_rt_app_layout\|addButton' src/` → no matches).
  Windows now advertises **104** runtime calls (was 87 at plan authoring;
  `grep -cE '"\w+\.' src/target/win_x86_64/mod.rs`), macOS 155. Remaining plan work:
  **E `openWithin`**, **J-full** (10 app methods), **K** (`.ico`/`.rsrc`).
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
- **2026-07-26 (Phase B, `os::args` needs a POST-arena entry hook).** Beyond the
  false "already used at entry" premise (above), the entry timing blocks the obvious
  fix: the shared `capture_args` block (`entry.rs:60`) runs *before* the arena is
  mapped (`ARENA_STATE_REGISTER` is set at `entry.rs:151`). A Windows argv capture
  must `arena_alloc` to marshal each UTF-16 arg (from CommandLineToArgvW) into a
  UTF-8 `char**`, so it cannot run at line 60. The clean design is a two-hook split:
  a `platform.defers_arg_capture()` predicate that makes Windows SKIP the line-60
  register/stack store (it would store garbage — the raw PE entry delivers no
  argc/argv), plus a new `emit_capture_args_post_arena` hook invoked after the arena
  setup doing GetCommandLineW → CommandLineToArgvW → per-arg UTF-16→UTF-8 arena
  marshal → store `_mfb_rt_os_argc`/`_mfb_rt_os_argv` (then LocalFree). This is the
  one Phase-B item that touches the shared program entry (floor-wide blast radius),
  deferred to its own focused change rather than risking the box-proven console
  entry. `lower_args` itself is unchanged. **Remaining Phase-B work = `os::args`.**
- **2026-07-26 (Phase C, the stdin-broadcast is pthread+malloc-based).** The Feature
  map said io-input "reuses the existing Windows raw-mode machinery"; it omits that
  the shared stdin-broadcast log (`stdin_broadcast.rs`, linked by every io read) is
  built on libc `malloc`/`free` and `pthread_mutex_*`/`pthread_cond_*` — none of
  which exist on the CRT-less Windows floor. Two seams close the gap without
  touching non-Windows codegen: (1) a `emit_heap_alloc`/`emit_heap_free`
  `CodegenPlatform` pair (default = the same libc `malloc`/`free` the broadcast
  already emitted; Windows = GetProcessHeap + HeapAlloc/HeapFree), and (2) routing
  the broadcast's pthread primitives through the *existing* pthread→Win32
  `emit_thread_external_call` seam (plan-47-H's SRWLOCK/CONDITION_VARIABLE map) on
  Windows only. `emit_read_file` also gained the fd 0 → GetStdHandle(STD_INPUT)
  resolution (it previously served only fs handles). With the io/fs/os stubs all
  implemented, the `unsupported()` helper in `win_x86_64/code.rs` became dead and
  was deleted (no-dead-code rule).
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
