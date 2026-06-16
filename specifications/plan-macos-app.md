# MFBASIC macOS App Mode Plan

Last updated: 2026-06-14

This document proposes a real macOS GUI application mode for MFBASIC native
targets. The goal is to make:

```text
mfb build -target macos-* -app <project>
```

produce a native macOS application whose `io::*` built-ins interact with a
window instead of the terminal.

This is a planning document. It describes the intended behavior, required
compiler and runtime changes, open design points, validation strategy, and a
recommended implementation sequence.

It complements:

- `specifications/architecture.md`
- `specifications/linker.md`
- `specifications/threading.md`
- `specifications/project.md`

## 1. Summary

Current macOS native output is a console-style executable:

- The CLI has no `-app` mode today.
- `src/main.rs` only understands output flags and `-target`.
- `src/target/macos_aarch64/mod.rs` exposes a console-oriented runtime call
  set.
- `src/target/macos_aarch64/plan.rs` and `src/target/macos_aarch64/code.rs`
  wire `io::*` to `libSystem` functions such as `_read`, `_write`, and `_poll`.
- `src/target/shared/code/mod.rs` generates runtime helpers assuming terminal
  or file-descriptor semantics.

That design is correct for normal native executables, but it cannot satisfy the
desired app behavior:

- `io::print` and `io::write` must append to a window-backed output surface.
- `io::input`, `io::readLine`, `io::readChar`, `io::readByte`, and
  `io::pollInput` must read from window-backed input.
- `io::isInputTerminal`, `io::isOutputTerminal`, `io::isErrorTerminal`, and
  `io::terminalSize` need defined app-mode behavior.
- Program startup can no longer be a trivial `_main -> language entry -> _exit`
  flow. AppKit owns the process event loop.

The correct implementation is a dedicated macOS app runtime mode, not a
terminal helper shim.

## 2. User-Facing Goal

When all of the following are true:

- build target is `macos-*`
- project kind is `executable`
- the CLI includes `-app`

the compiler should produce a macOS GUI application whose standard MFBASIC
`io::*` built-ins target an app window.

Expected user experience:

1. Launch the output.
2. A native macOS window opens.
3. Calls to `io::print` / `io::write` append text into the window.
4. Calls to `io::input` / `io::readLine` / `io::readChar` / `io::readByte`
   consume user-entered text from the same application.
5. The program remains responsive while the AppKit event loop is running.

## 3. Non-Goals

This plan does not require:

- replacing all `fs::*`, `thread::*`, or non-IO runtime helpers with Cocoa
  equivalents
- adding graphics, menus, buttons, multi-window UI, or widget APIs to MFBASIC
- introducing a general Objective-C FFI for user code
- changing non-macOS backends
- changing normal `mfb build -target macos-*` console behavior

Future enhancements may add richer app metadata or UI APIs, but they are not
required for the first app-mode release.

## 4. Mac App Store Compatibility

This section is a compatibility check, not a change in product direction.

The current MFBASIC language/runtime model does not, as designed, depend on:

- downloading new executable code at runtime
- installing helper applications after launch
- loading user plugin code to add reviewed functionality later
- embedding a browser/runtime shell that fetches remote application logic

That is favorable for Mac App Store review. App mode should preserve those
properties.

## 4.1 Current Outlook

If implemented as described in this document, app mode should be broadly
compatible with Mac App Store review because it is intended to be:

- a native AppKit application bundle
- based on public Apple APIs
- ahead-of-time generated at build time
- self-contained at runtime

This is not a guarantee of approval. App Review depends on the full submitted
product, packaging, entitlements, metadata, and behavior at the time of review.

## 4.2 App Sandbox Requirement

Mac App Store apps must enable App Sandbox.

Implications for this plan:

- app-mode bundle output must support sandbox entitlements
- file access behavior must remain compatible with sandbox rules
- future app-mode features that operate on user files must use sandbox-friendly
  access patterns

The window-backed `io::*` behavior itself is compatible with sandboxing. The
main watch item is future filesystem UX, not transcript/input UI.

Recommended framing:

- `mfb build -app` remains the general macOS app-mode feature
- a future App Store distribution profile can layer on exact entitlements,
  signing, and packaging requirements

## 4.3 No Runtime Code Download Or Post-Review Feature Injection

App-mode output intended for the Mac App Store must remain self-contained after
build.

It must not:

- download new executable code after review
- install additional helper applications to add app behavior
- fetch runtime modules or packages that materially change functionality
- rely on plugin directories that extend application behavior post-review

This is mostly a confirmation of existing direction rather than a new
restriction. The current language/runtime model already points toward
build-time-produced native output.

Recommended invariant:

```text
App mode executes only the program compiled into the app bundle at build time,
together with bundled runtime support reviewed as part of that app.
```

## 4.4 Public API Only

The app-mode runtime should use only documented public APIs such as:

- AppKit
- Foundation
- libobjc / Objective-C runtime APIs

It should not depend on:

- private AppKit or WindowServer interfaces
- undocumented Objective-C runtime behavior as a core feature dependency
- process injection
- unsupported automation tricks for core functionality

This matters especially because app mode will bind directly to AppKit/libobjc
from code emitted by `mfb`.

## 4.5 Generated App Versus Compiler Product

There are two different App Store questions:

1. Can an app produced by MFBASIC app mode be compatible with the Mac App
   Store?
2. Can the MFBASIC compiler itself be sold in the Mac App Store if it builds
   and runs arbitrary user programs?

This document addresses question 1.

For question 1, the answer is broadly “yes, potentially”, assuming sandboxing,
public APIs, self-contained distribution, and correct Apple packaging/signing.

Question 2 is more sensitive and out of scope here. A compiler/IDE product that
builds or runs arbitrary user programs can trigger a different review analysis
than a generated native app.

## 4.6 Template/App-Generation Risk

App Store review can be stricter for commercial template/app-generation
services than for one-off native applications.

That does not invalidate this runtime plan, but the distinction should remain
clear:

- this feature is a native application output mode
- it is not, by itself, a claim that any app-factory business model built on
  top of MFBASIC will be approved by Apple

If MFBASIC is later used to mass-produce near-identical apps for many clients,
that creates a separate review risk unrelated to the technical correctness of
this app-mode design.

## 4.7 Release-Pipeline Requirements Outside Core Runtime

This plan focuses on compiler/runtime behavior. Mac App Store distribution also
needs release-pipeline support outside the core app-mode runtime:

- code signing
- entitlements
- bundle identifiers
- icons and app metadata
- a submission/archive workflow compatible with Apple tooling

The runtime design should keep these steps possible. The first implementation
phase does not need to automate all of them.

## 4.8 Recommendation

The plan should explicitly preserve this invariant:

```text
MFBASIC macOS app mode produces a self-contained native app bundle whose
runtime behavior is fully determined by code generated at build time and by the
bundled reviewed runtime support.
```

That is the key property that keeps the design aligned with likely Mac App
Store acceptance.

## 5. Proposed Semantics

## 5.1 CLI Contract

Add a new build flag:

```text
mfb build -app
```

Rules:

- `-app` is valid only with `build`.
- `-app` is valid only for executable projects.
- `-app` is valid only when `-target` resolves to a macOS native target.
- `-app` affects the default executable build path and native intermediate
  outputs (`-nir`, `-nplan`, `-nobj`, `-ncode`) because all of them need to
  represent the alternate runtime mode.
- `-app` is rejected for package projects.
- `-app` is rejected for non-macOS targets.

Recommended CLI parse shape:

```rust
struct BuildOptions {
    location: PathBuf,
    output: BuildOutput,
    target: target::BuildTarget,
    app_mode: bool,
}
```

Recommended validation:

```rust
if options.app_mode && options.target.os != "macos" {
    return Err("mfb build -app requires a macOS target".to_string());
}
if options.app_mode && project_kind != "executable" {
    return Err("mfb build -app requires an executable project".to_string());
}
```

## 5.2 Output Shape

Phase 1 should emit a real `.app` bundle, not only a raw Mach-O binary renamed
 as an app. The bundle should contain:

```text
<name>.app/
  Contents/
    Info.plist
    MacOS/
      <name>
```

Rationale:

- AppKit behavior is more predictable for a bundled application.
- Launch Services integration is cleaner.
- Dock/app activation behavior is easier to control.
- It leaves room for future icons, nib-less metadata, and resources.

The CLI should print:

```text
Wrote executable to <project>/<name>.app
```

Optional compatibility artifact:

- The compiler may also emit the raw Mach-O executable into
  `Contents/MacOS/<name>` only, without a sibling `.out`.
- Do not emit both `<name>.out` and `<name>.app` unless there is a strong
  developer workflow reason. Dual outputs complicate tests and messaging.

## 5.3 Window Model

Initial app mode should use one primary window containing:

- a non-editable scrollable transcript/output region
- a single-line editable input field

Recommended UX:

- program output appears in the transcript
- pressing Return in the input field commits one logical input line
- committed input lines are echoed into the transcript
- `io::input(prompt)` writes the prompt into the transcript, then blocks until a
  line is submitted
- `io::readLine()` blocks until a line is submitted
- `io::readChar()` and `io::readByte()` consume from a byte queue derived from
  committed input text

This is intentionally line-oriented. It matches the current API surface better
than trying to model raw terminal keypress streams.

## 5.4 `io::*` Behavior In App Mode

App mode should redefine runtime behavior, not type signatures.

### `io::print(text AS String)`

- append `text`
- append newline
- render in transcript

### `io::write(text AS String)`

- append `text`
- no newline
- render in transcript

### `io::printError(text AS String)`

- append `text` plus newline to transcript
- visually distinguish error output

Initial styling options:

- red foreground for error lines
- or a prefixed marker such as `[stderr] `

Using styled transcript output is preferable because it preserves stdout/stderr
distinction without forcing separate panes.

### `io::writeError(text AS String)`

- same as `io::printError`, but no newline

### `io::flush()`, `io::flushError()`

- succeed immediately
- force any pending UI transcript updates to become visible before returning

This means they cannot remain trivial no-ops if output is buffered on a worker
thread. They must synchronize with the UI dispatch path.

### `io::input()`

Equivalent to:

```basic
io::input("")
```

### `io::input(prompt AS String)`

Behavior:

1. write `prompt` to transcript without forcing a newline unless the prompt
   already contains one
2. focus the input field
3. block the MFBASIC worker thread until the user submits a line
4. return the submitted line without its trailing newline

### `io::readLine()`

- block until one submitted line is available
- return that line without its trailing newline

### `io::readChar()`

- return the next UTF-8 character from the committed-input queue
- if no queued character is available, block until the user submits a line, then
  consume from that line plus a synthetic `\n` boundary

Rationale:

- AppKit text input is naturally line/submission-oriented.
- Blocking on arbitrary uncommitted per-keystroke composition is much harder and
  interacts badly with IME/composed text.
- A committed-line queue preserves correct Unicode text semantics.

### `io::readByte()`

- return the next byte from the same committed-input queue used by
  `io::readChar()`
- if no queued byte exists, block until another committed line is submitted

### `io::pollInput()`

- `io::pollInput()` returns `TRUE` iff at least one committed input item is
  immediately available
- `io::pollInput(timeoutMs)` waits up to `timeoutMs` for committed input to
  arrive

Recommended definition of “available input”:

- at least one queued byte for `readByte`
- or at least one queued UTF-8 character for `readChar`
- or at least one queued submitted line for `readLine` / `input`

This implies a shared input state with multiple queue views:

- line queue
- UTF-8 character queue
- byte queue

### `io::isInputTerminal()`, `io::isOutputTerminal()`, `io::isErrorTerminal()`

Recommended app-mode result:

```text
TRUE
TRUE
TRUE
```

Rationale:

- The app window is the interactive console for the program.
- Returning `FALSE` would cause many console-oriented programs to disable
  interactive behavior.

This is a semantic “interactive terminal equivalent”, not a literal POSIX TTY
claim.

### `io::terminalSize()`

Recommended app-mode result:

- return the transcript viewport size expressed as text columns and rows

This can be computed from:

- transcript view content width
- chosen monospaced font metrics
- visible content height

If exact values are difficult early on, a fallback fixed size such as `80x24`
may be tempting, but that would violate the completion standard for real
behavior. App mode should compute actual visible dimensions before it is
described as complete.

## 5. Runtime Architecture

## 5.1 Core Model

App mode needs two threads with clearly separated responsibilities:

- UI thread: owns `NSApplication`, window, text view, input field, and the
  AppKit event loop
- language runtime thread: runs MFBASIC `main` and any existing runtime helper
  calls

This is the central design choice. Running the MFBASIC entry point directly on
the AppKit main thread is a bad fit because:

- `io::readLine()` and friends are synchronous/blocking APIs
- user code may perform long-running work
- blocking the main thread would freeze the UI and stop event processing

Recommended process startup:

```text
_main
  -> initialize app-mode runtime globals
  -> create NSApplication / app delegate
  -> create window and UI controls
  -> spawn language thread
  -> run AppKit event loop on main thread
```

Language thread:

```text
thread start
  -> call MFBASIC entry symbol
  -> capture exit status / runtime trap
  -> notify UI thread that program completed
```

## 5.2 Shared App Runtime State

Introduce a dedicated runtime state object for app mode.

Suggested conceptual structure:

```c
typedef struct {
    // lifecycle
    uint64_t program_done;
    int64_t program_exit_code;
    const char *program_error_message;

    // output
    Mutex output_lock;
    ByteBuffer pending_stdout;
    ByteBuffer pending_stderr;
    uint64_t output_generation;

    // input
    Mutex input_lock;
    CondVar input_cv;
    LineQueue line_queue;
    ByteQueue byte_queue;
    CharQueue char_queue;
    uint64_t input_generation;

    // UI-owned opaque handles
    void *ns_app;
    void *window;
    void *scroll_view;
    void *text_view;
    void *input_field;

    // sizing
    uint64_t visible_columns;
    uint64_t visible_rows;
} MfbMacAppRuntime;
```

Rust compiler code does not need to understand AppKit object layouts. It only
needs to emit code and imports for helper functions that manipulate the state.

## 5.3 Runtime Helper Boundary

The cleanest design is to separate app-mode behavior behind new app-runtime
helper symbols rather than rewriting every shared helper into a giant
platform/mode conditional.

Example conceptual helper surface:

```text
_mfb_macapp_bootstrap
_mfb_macapp_program_start
_mfb_macapp_program_finish
_mfb_macapp_write_stdout
_mfb_macapp_write_stderr
_mfb_macapp_flush_stdout
_mfb_macapp_flush_stderr
_mfb_macapp_input_line
_mfb_macapp_read_line
_mfb_macapp_read_char
_mfb_macapp_read_byte
_mfb_macapp_poll_input
_mfb_macapp_terminal_size
_mfb_macapp_is_interactive
```

Then app-mode `io::*` runtime helpers call those symbols instead of `_read`,
`_write`, `_poll`, or `isatty`-style primitives.

This keeps the layering explicit:

```text
shared builtin signature/typecheck
  -> native lowering chooses helper symbol by target + mode
  -> app helper invokes AppKit runtime directly
```

This is preferable to:

- teaching every shared helper to branch on global app state
- pretending a window is a fake file descriptor
- mixing AppKit concerns into the normal console backend path

## 5.4 Objective-C / AppKit Integration

**Pure C/ObjC runtime emitted or linked directly**

- use `objc_msgSend`, `objc_getClass`, selector registration, and Objective-C
  runtime APIs directly
- link `libobjc`, `Foundation`, and `AppKit`

Pros:

- no separate Swift/ObjC build toolchain step
- fits the current “compiler emits everything” architecture

Cons:

- typed `objc_msgSend` call signatures are error-prone on arm64
- manual class/selector wiring is verbose
- text system, delegate methods, and lifecycle code are awkward

## 5.5 UI Design Details

Recommended window composition:

```text
NSWindow
  contentView
    NSScrollView
      NSTextView (read-only transcript)
    NSTextField or NSTextView (single-line input)
```

Recommended properties:

- transcript uses a monospaced font
- transcript is non-editable and selectable
- transcript auto-scrolls to bottom on appended output
- input field keeps keyboard focus while program is running
- hitting Return submits current input buffer

Transcript model:

- maintain an attributed string or mutable string on the UI side
- append stdout and stderr runs with separate attributes
- update in batches to avoid one-main-thread-dispatch-per-byte

Suggested batch rule:

- write helpers append bytes to shared buffers
- schedule a single pending UI flush if one is not already enqueued
- UI thread drains accumulated output and applies one transcript update

## 5.6 Input Design Details

Do not treat raw keyDown events as the canonical input source for `io::*`.

Reasons:

- composed text and IME input become difficult
- Unicode correctness is poor
- line-oriented APIs become awkward
- per-keystroke behavior is harder to validate

Instead:

- accept text entry through a normal Cocoa text control
- on Return, capture the committed string
- push one logical line to the line queue
- also encode it as UTF-8 and populate char/byte queues
- optionally append a newline byte and newline character so `readChar` and
  `readByte` can observe line boundaries

Suggested queue conversion logic:

```text
submitted NSString
  -> UTF-8 bytes
  -> line_queue.push(submitted_without_newline)
  -> char_queue.push(each Unicode scalar/grapheme policy)
  -> char_queue.push("\n")
  -> byte_queue.push(each UTF-8 byte)
  -> byte_queue.push(0x0A)
```

Open policy choice:

- `readChar()` can be defined in terms of Unicode scalars
- or in terms of grapheme clusters

Recommended first release: Unicode scalar values encoded back into single-char
MFB strings.

Rationale:

- simpler and better aligned with “next character in UTF-8 stream”
- avoids full grapheme segmentation in the app runtime path

## 5.7 Shutdown And Exit

Program completion should not immediately hard-exit the process from the worker
thread.

Recommended behavior:

1. worker thread stores final result in app runtime state
2. worker thread dispatches completion notification to UI thread
3. UI thread:
   - flushes remaining output
   - optionally disables input field
   - either keeps the window open or terminates the app

Policy choice:

- default behavior should keep the window open briefly enough for the user to
  read output, or until closed manually

Recommended first release:

- keep the window open after normal completion
- show a status line such as `Program exited with code 0`
- allow window close to terminate the app process

This is more user-friendly than auto-closing on exit.

## 6. Compiler And Backend Changes

## 6.1 CLI Layer (`src/main.rs`)

Required changes:

- parse `-app`
- store `app_mode` in `BuildOptions`
- validate target/project-kind compatibility
- thread `app_mode` into backend selection and output writing

Potential interface:

```rust
let executable_paths = target::write_executable(
    &options.location,
    &ir,
    &target,
    &packages,
    options.app_mode,
)?;
```

The same is needed for `write_nir`, `write_native_plan`, `write_native_object_plan`,
and `write_native_code_plan`, because app mode changes native lowering.

## 6.2 Target Abstraction

Current `NativeBackend` APIs assume one executable mode per target. App mode
introduces a second variant of macOS executable output.

**add a native build profile/mode enum**

Recommended:

```rust
enum NativeBuildMode {
    Console,
    MacApp,
}
```

Thread this enum through lowering, planning, codegen, and linking.

This produces clearer code than a growing set of booleans.

## 6.3 Shared Native Lowering

The shared lowering path must carry target mode into:

- NIR metadata
- native plan metadata
- native code plan metadata

Suggested additions:

```rust
pub(crate) struct NirModule {
    pub target: String,
    pub build_mode: String,
    ...
}
```

```rust
pub(crate) struct NativePlan {
    pub target: String,
    pub build_mode: String,
    ...
}
```

```rust
pub(crate) struct NativeCodePlan {
    pub target: String,
    pub build_mode: String,
    ...
}
```

This is important for:

- golden outputs
- validation
- linker behavior
- debugging generated artifacts

## 6.4 macOS Backend Capability Table

`src/target/macos_aarch64/mod.rs` currently advertises a console-oriented subset
of runtime calls. App mode requires full `io::*` coverage relevant to GUI IO.

At minimum, app mode must support:

- `io.print`
- `io.write`
- `io.printError`
- `io.writeError`
- `io.flush`
- `io.flushError`
- `io.input`
- `io.readLine`
- `io.readChar`
- `io.readByte`
- `io.pollInput`
- `io.isInputTerminal`
- `io.isOutputTerminal`
- `io.isErrorTerminal`
- `io.terminalSize`

Console mode may continue to support a smaller or different set if that is the
current target status, but app mode should not ship partially wired IO
semantics.

## 6.5 Native Plan Changes

`src/target/macos_aarch64/plan.rs` currently imports `libSystem` helpers such as
`_write`, `_read`, and `_poll` for `io::*`.

App mode native planning must instead import:

- Objective-C runtime symbols, if using direct ObjC runtime calls
- or direct framework/runtime symbols required for app mode
- AppKit/Foundation dynamic libraries or frameworks
- `libobjc`
- any synchronization/thread primitives needed by the app runtime

Suggested logical imports for direct app-mode runtime calls:

```text
libSystem   pthread / locks / condvars / memory / exit
libobjc     Objective-C runtime
AppKit      NSApplication, NSWindow, NSTextView, NSScrollView, NSTextField
Foundation  NSString, attributed strings, autorelease pools
```

The exact Mach-O import model depends on how framework linkage is represented in
the current custom linker. That likely requires extending
`src/os/macos/object.rs` and `src/os/macos/link.rs` beyond the current
`/usr/lib/libSystem.B.dylib` assumption documented in `specifications/linker.md`.

## 6.6 Native Codegen Changes

App mode needs a different `_main`.

Current conceptual `_main`:

```text
_main:
  call language entry
  map return value
  _exit(status)
```

App-mode `_main`:

```text
_main:
  initialize app runtime state
  bootstrap autorelease pool / app runtime
  start worker thread with language entry
  run AppKit event loop
  exit when app terminates
```

Pseudo-structure:

```text
_main
  -> _mfb_macapp_bootstrap
  -> _mfb_macapp_run(language_entry_symbol, accepts_args, argv)
```

That bootstrap function can:

- create the app delegate
- build the window
- translate `argv` if needed
- start the worker thread
- call `-[NSApplication run]`

This keeps generated `_main` small and keeps AppKit logic outside raw assembly.

## 6.7 Runtime Helper Codegen

Shared helper lowering in `src/target/shared/code/mod.rs` should select helper
implementations by `(target, build_mode, call)`.

Pseudo-dispatch:

```rust
match (platform.target(), build_mode, spec.call) {
    ("macos-aarch64", NativeBuildMode::MacApp, "io.print") => ...,
    ("macos-aarch64", NativeBuildMode::MacApp, "io.readLine") => ...,
    _ => existing behavior,
}
```

Avoid sprinkling ad hoc conditionals into the existing console helper bodies.
Instead:

- keep existing console helpers for console mode
- add app-mode helper lowerers or direct framework/runtime call wrappers

Example wrapper idea:

```text
runtime.io.print (app mode)
  -> marshal MFB string pointer/length
  -> call _mfb_macapp_write_stdout
  -> return ok
```

## 6.8 Linker / Object Writer Changes

The macOS linker currently assumes a plain executable with `LC_MAIN` and
`libSystem` imports.

App mode requires:

1. dynamic dependency support for AppKit/Foundation/libobjc or their resolved
   dylib paths
2. `.app` bundle emission
3. `Info.plist` generation

Minimum `Info.plist` fields:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
 "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>${PROJECT_NAME}</string>
  <key>CFBundleExecutable</key>
  <string>${PROJECT_NAME}</string>
  <key>CFBundleIdentifier</key>
  <string>dev.mfbasic.${PROJECT_NAME}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
</dict>
</plist>
```

Future fields can be added for icons and version metadata.

Open linker question:

- whether the current custom Mach-O writer already supports enough dynamic
  library record flexibility for frameworks, or whether framework-specific path
  handling needs to be added

Likely answer: it needs extension.

## 7. Testing And Validation

This feature must not be treated as complete based only on NIR/NPlan/NCode
goldens. Runtime validation is mandatory.

## 7.1 Required Function Test Coverage

Any modified `io::*` function must have matching valid and invalid coverage
under the repository’s existing test conventions.

For this feature, that implies reviewing and likely updating/adding:

- `tests/func_io_print_valid/**`
- `tests/func_io_print_invalid/**`
- `tests/func_io_write_valid/**`
- `tests/func_io_write_invalid/**`
- `tests/func_io_printError_valid/**`
- `tests/func_io_printError_invalid/**`
- `tests/func_io_writeError_valid/**`
- `tests/func_io_writeError_invalid/**`
- `tests/func_io_flush_valid/**`
- `tests/func_io_flush_invalid/**`
- `tests/func_io_flushError_valid/**`
- `tests/func_io_flushError_invalid/**`
- `tests/func_io_input_valid/**`
- `tests/func_io_input_invalid/**`
- `tests/func_io_readLine_valid/**`
- `tests/func_io_readLine_invalid/**`
- `tests/func_io_readChar_valid/**`
- `tests/func_io_readChar_invalid/**`
- `tests/func_io_readByte_valid/**`
- `tests/func_io_readByte_invalid/**`
- `tests/func_io_pollInput_valid/**`
- `tests/func_io_pollInput_invalid/**`
- `tests/func_io_isInputTerminal_valid/**`
- `tests/func_io_isInputTerminal_invalid/**`
- `tests/func_io_isOutputTerminal_valid/**`
- `tests/func_io_isOutputTerminal_invalid/**`
- `tests/func_io_isErrorTerminal_valid/**`
- `tests/func_io_isErrorTerminal_invalid/**`
- `tests/func_io_terminalSize_valid/**`
- `tests/func_io_terminalSize_invalid/**`

Some of these directories do not exist today. The implementation must add them
where the function exists but dedicated test coverage is missing.

## 7.2 App-Mode Runtime Tests

App mode needs additional end-to-end tests beyond normal acceptance:

- build a small app-mode project
- launch the produced `.app` executable binary in a controlled test harness
- programmatically feed input
- observe transcript output or another runtime-observable artifact

There are multiple possible strategies.

### Strategy A: headless app-runtime self-test mode

Expose a test-only app runtime mode that:

- does not create visible windows
- still uses the same app-mode helper surface
- captures transcript output to a buffer
- injects queued input lines

Pros:

- deterministic
- CI-friendly
- tests runtime semantics without screen automation

Cons:

- introduces a parallel runtime path if done carelessly

This is acceptable only if the test mode exercises the same helper logic and
only swaps the UI sink/source.

### Strategy B: UI automation

Launch the real app and drive it with AppleScript or Accessibility APIs.

Pros:

- closest to real behavior

Cons:

- brittle
- harder in CI
- slower

Recommended validation approach:

- use Strategy A for automated runtime acceptance
- add at least one manual smoke test recipe for real-window verification

## 7.3 Acceptance Suite

After implementation, run:

```text
scripts/test-accept.sh target/debug/mfb target/accept-actual
```

App-mode work will also require acceptance updates for:

- CLI parsing diagnostics
- native intermediate outputs containing build mode metadata
- macOS native object/code plans importing new libraries
- executable output path changes when `-app` is selected

## 7.4 Manual Verification Checklist

Before declaring the feature complete:

1. Build an example with `mfb build -target macos-aarch64 -app`.
2. Confirm a `.app` bundle is written.
3. Launch the app and verify a window appears.
4. Verify `io::print` appends transcript text.
5. Verify `io::printError` is visibly distinguished.
6. Verify `io::input` blocks and returns submitted text.
7. Verify `io::readLine`, `io::readChar`, and `io::readByte` behave correctly.
8. Verify `io::pollInput(0)` reflects queued committed input.
9. Verify `io::terminalSize()` changes as the window is resized.
10. Verify program completion behavior and window-close shutdown.

## 8. Risks

## 8.1 Highest-Risk Areas

### 1. AppKit on the main thread

This is mandatory. Any design that runs `NSApplication` off the main thread is
not viable.

### 2. Raw Objective-C runtime messaging in generated helpers

Possible, but high-risk and hard to maintain. Keep the runtime surface small and
centralized inside the macOS OS/backend implementation.

### 3. Unicode semantics for `readChar`

Must be specified clearly. “Character” is ambiguous between bytes, scalars, and
graphemes.

### 4. Linker support for non-libSystem imports

The current Mach-O path appears tailored to `libSystem`. AppKit/Foundation/libobjc
will likely require explicit linker work.

### 5. Blocking semantics with responsive UI

This is why the worker-thread model is required. Anything else risks UI hangs.

## 8.2 Common Failure Modes To Avoid

- treating the app window as a pseudo-file descriptor
- leaving `io::flush` as a no-op while UI updates are buffered
- implementing only `print/write` and leaving `read*` or `terminalSize`
  unsupported
- auto-returning fake `80x24` terminal size in the final implementation
- capturing raw keyDown ASCII bytes and breaking Unicode input
- exiting the process directly from the worker thread
- claiming completion based only on artifact generation

## 9. Recommended Implementation Sequence

## Phase 1: Mode Plumbing

- add `-app` CLI parsing and diagnostics
- add `NativeBuildMode`
- thread mode through IR lowering, NIR, native plan, native code plan
- update text/json artifact formats and validations

Deliverable:

- `-app` parses and propagates
- intermediate outputs record app mode
- build rejects invalid target/project combinations

## Phase 2: macOS Bundle + Linker Support

- extend macOS linker/object pipeline for additional dylibs/frameworks
- emit `.app` bundle structure and `Info.plist`
- keep runtime behavior stub-free for supported features

Deliverable:

- simple app-mode executable bundle can be emitted and launched

## Phase 3: App Runtime Bridge Bootstrap

- add macOS app runtime support emitted by `mfb`
- implement app bootstrap, window creation, transcript view, input field
- run language entry on a worker thread
- written with libSystem objc send message calls

Deliverable:

- app launches a window
- worker thread can run a trivial program

## Phase 4: Output Path

- implement app-mode `print/write/printError/writeError`
- implement UI batching and flush semantics

Deliverable:

- transcript correctly shows stdout/stderr output

## Phase 5: Input Path

- implement committed-line input submission
- implement line/char/byte queues
- implement `input`, `readLine`, `readChar`, `readByte`, `pollInput`

Deliverable:

- blocking input APIs work against the window

## Phase 6: Interactive Metadata

- implement `isInputTerminal`, `isOutputTerminal`, `isErrorTerminal`
- implement computed `terminalSize`
- handle resize updates

Deliverable:

- all `io::*` semantics needed for app mode are live

## Phase 7: Validation

- add or update mandatory function tests
- add app-mode end-to-end runtime tests
- run full acceptance suite
- perform manual smoke verification

## 10. Recommended Initial Example Program

This should be used as a manual smoke test and later as an automated runtime
fixture:

```basic
IMPORT io

SUB main()
  io::print("App mode started")
  io::print("Enter your name:")
  LET name AS String = io::readLine()
  io::print("Hello, " & name)

  io::print("Type one character line:")
  LET ch AS String = io::readChar()
  io::print("First char: [" & ch & "]")

  io::print("Type one byte line:")
  LET b AS Byte = io::readByte()
  io::print("First byte: " & toString(b))

  LET size AS TerminalSize = io::terminalSize()
  io::print("Size: " & toString(size.columns) & "x" & toString(size.rows))
END SUB
```

Expected observations:

- all output appears in the transcript
- each prompt is visible before blocking
- line submission unblocks reads
- terminal size reflects the current window

## 11. Open Design Decisions

These should be resolved before implementation begins in earnest:

1. Should app mode always emit a `.app`, or also a raw sibling executable?
2. Should stdout/stderr be visually styled, prefixed, or both?
3. Should `readChar()` return Unicode scalars or grapheme clusters?
4. On normal program completion, should the window remain open until manually
   closed, or should there be a configurable policy?
5. Should app mode be represented only by CLI flag, or later also by manifest
   metadata for persistent project configuration?

## 12. Recommendation

Proceed with a dedicated macOS app runtime mode with these choices:

- `-app` is a macOS-only executable build flag
- app mode emits a real `.app` bundle
- the main thread runs AppKit
- MFBASIC entry runs on a worker thread
- generated code binds directly to public macOS runtime/framework APIs
- all `io::*` interactive behavior is implemented against a transcript + input
  field model
- `io::is*Terminal()` returns `TRUE`
- `io::terminalSize()` returns measured visible text dimensions

This is the smallest design that is still technically coherent and
production-grade.

## 13. C pseudocode

This is guide-level pseudocode only. It describes the runtime behavior that
`mfb` should emit internally for macOS app mode. It is not intended to imply
an external helper source file, Xcode dependency, or any compiler other than
`mfb`.

It intentionally keeps concrete examples of the kind of Objective-C runtime
calls the generated code will need to perform so the section remains useful as
an implementation guide, not just a structural sketch.

```c
// Pseudocode only.
//
// The generated binary may call libobjc/AppKit/Foundation directly via public
// system APIs already present on macOS. All such calls are emitted by mfb.

typedef struct {
    Mutex lock;
    CondVar input_ready;
    CondVar worker_done;

    bool app_running;
    bool worker_finished;
    int worker_exit_code;

    ByteBuffer stdout_pending;
    ByteBuffer stderr_pending;
    Size terminal_cells;

    Queue<String> committed_lines;
    Queue<Utf8Scalar> scalar_queue;
    Queue<uint8_t> byte_queue;

    id app;
    id window;
    id transcript_view;
    id input_field;
} AppRuntimeState;

static AppRuntimeState *STATE;

// Example helper shape only. Actual generated code must use the exact ABI for
// each objc_msgSend call site.
id objc_call_id(id receiver, const char *selector_name, ...);
void objc_call_void(id receiver, const char *selector_name, ...);
bool objc_call_bool(id receiver, const char *selector_name, ...);

id nsstring_from_utf8(const char *text) {
    Class NSString = objc_getClass("NSString");
    return objc_call_id((id)NSString, "stringWithUTF8String:", text);
}

void app_mode_bootstrap(void (*language_entry)(void)) {
    STATE = state_create();

    // AppKit always lives on the main thread.
    {
        Class NSApplication = objc_getClass("NSApplication");
        Class NSWindow = objc_getClass("NSWindow");
        Class NSScrollView = objc_getClass("NSScrollView");
        Class NSTextView = objc_getClass("NSTextView");
        Class NSTextField = objc_getClass("NSTextField");

        id app = objc_call_id((id)NSApplication, "sharedApplication");
        objc_call_void(app, "setActivationPolicy:", 0);
        STATE->app = app;

        CGRect window_frame = make_rect(100, 100, 900, 640);
        id window = objc_call_id((id)NSWindow, "alloc");
        window = objc_call_id(
            window,
            "initWithContentRect:styleMask:backing:defer:",
            window_frame,
            WINDOW_STYLE_TITLED
                | WINDOW_STYLE_CLOSABLE
                | WINDOW_STYLE_MINIATURIZABLE
                | WINDOW_STYLE_RESIZABLE,
            BACKING_BUFFERED,
            false
        );
        objc_call_void(window, "setTitle:", nsstring_from_utf8("MFBASIC App"));
        STATE->window = window;

        id content = objc_call_id(window, "contentView");

        CGRect transcript_scroll_frame = make_rect(20, 70, 860, 550);
        id transcript_scroll = objc_call_id((id)NSScrollView, "alloc");
        transcript_scroll = objc_call_id(
            transcript_scroll,
            "initWithFrame:",
            transcript_scroll_frame
        );

        id transcript_view = objc_call_id((id)NSTextView, "alloc");
        transcript_view = objc_call_id(
            transcript_view,
            "initWithFrame:",
            transcript_scroll_frame
        );
        objc_call_void(transcript_view, "setEditable:", false);
        objc_call_void(transcript_view, "setRichText:", false);
        objc_call_void(transcript_view, "setSelectable:", true);
        objc_call_void(transcript_scroll, "setDocumentView:", transcript_view);
        objc_call_void(transcript_scroll, "setHasVerticalScroller:", true);
        objc_call_void(content, "addSubview:", transcript_scroll);
        STATE->transcript_view = transcript_view;

        CGRect input_frame = make_rect(20, 20, 860, 32);
        id input_field = objc_call_id((id)NSTextField, "alloc");
        input_field = objc_call_id(input_field, "initWithFrame:", input_frame);
        objc_call_void(input_field, "setStringValue:", nsstring_from_utf8(""));
        objc_call_void(content, "addSubview:", input_field);
        STATE->input_field = input_field;
    }

    appkit_install_submit_handler(STATE, on_input_committed);
    appkit_install_resize_handler(STATE, on_window_resized);
    appkit_install_close_handler(STATE, on_window_closed);

    state_lock(STATE);
    STATE->app_running = true;
    state_unlock(STATE);

    // MFBASIC program logic runs on a worker thread.
    thread_start(worker_main, language_entry);

    appkit_show_window(STATE);
    appkit_activate_application(STATE);
    appkit_run_event_loop(STATE);
}

void worker_main(void *entry_ptr) {
    void (*language_entry)(void) = entry_ptr;
    int exit_code = run_language_entry_and_capture_exit(language_entry);

    state_lock(STATE);
    STATE->worker_finished = true;
    STATE->worker_exit_code = exit_code;
    state_signal_all(STATE->worker_done);
    state_unlock(STATE);

    // Final UI actions are scheduled back onto the AppKit thread.
    appkit_dispatch_async_main(on_worker_finished_main_thread);
}

void on_input_committed(String line) {
    state_lock(STATE);

    queue_push(STATE->committed_lines, line);

    foreach (Utf8Scalar scalar in utf8_decode_scalars(line)) {
        queue_push(STATE->scalar_queue, scalar);
    }
    queue_push(STATE->scalar_queue, '\n');

    ByteString utf8 = utf8_encode(line);
    foreach (uint8_t byte in utf8.bytes) {
        queue_push(STATE->byte_queue, byte);
    }
    queue_push(STATE->byte_queue, '\n');

    state_signal_all(STATE->input_ready);
    state_unlock(STATE);

    transcript_append_stdout(line);
    transcript_append_stdout("\n");
    input_field_clear_and_refocus(STATE);
}

void on_window_resized(Size pixel_size) {
    state_lock(STATE);
    STATE->terminal_cells = transcript_measure_visible_cells(pixel_size);
    state_unlock(STATE);
}

void on_window_closed(void) {
    state_lock(STATE);
    STATE->app_running = false;
    state_signal_all(STATE->input_ready);
    state_signal_all(STATE->worker_done);
    state_unlock(STATE);

    request_process_shutdown();
}

void on_worker_finished_main_thread(void) {
    bool keep_window_open = false;  // final policy decided elsewhere
    if (!keep_window_open) {
        appkit_stop_event_loop(STATE);
    }
}

void io_print(String text) {
    state_lock(STATE);
    buffer_append_utf8(&STATE->stdout_pending, text);
    buffer_append_utf8(&STATE->stdout_pending, "\n");
    state_unlock(STATE);
    appkit_dispatch_async_main(flush_pending_output_main_thread);
}

void io_write(String text) {
    state_lock(STATE);
    buffer_append_utf8(&STATE->stdout_pending, text);
    state_unlock(STATE);
    appkit_dispatch_async_main(flush_pending_output_main_thread);
}

void io_print_error(String text) {
    state_lock(STATE);
    buffer_append_utf8(&STATE->stderr_pending, text);
    buffer_append_utf8(&STATE->stderr_pending, "\n");
    state_unlock(STATE);
    appkit_dispatch_async_main(flush_pending_output_main_thread);
}

void io_write_error(String text) {
    state_lock(STATE);
    buffer_append_utf8(&STATE->stderr_pending, text);
    state_unlock(STATE);
    appkit_dispatch_async_main(flush_pending_output_main_thread);
}

void io_flush(void) {
    appkit_dispatch_sync_main(flush_pending_output_main_thread);
}

void io_flush_error(void) {
    io_flush();
}

String io_input_with_prompt(String prompt) {
    if (prompt.length > 0) {
        io_write(prompt);
    }
    input_field_focus(STATE);
    return io_read_line();
}

String io_read_line(void) {
    state_lock(STATE);
    while (queue_empty(STATE->committed_lines) && STATE->app_running) {
        state_wait(STATE->input_ready, STATE->lock);
    }
    if (!STATE->app_running) {
        state_unlock(STATE);
        return runtime_abort_due_to_closed_window();
    }
    String line = queue_pop(STATE->committed_lines);
    state_unlock(STATE);
    return line;
}

Utf8Scalar io_read_char(void) {
    state_lock(STATE);
    while (queue_empty(STATE->scalar_queue) && STATE->app_running) {
        state_wait(STATE->input_ready, STATE->lock);
    }
    if (!STATE->app_running) {
        state_unlock(STATE);
        return runtime_abort_due_to_closed_window();
    }
    Utf8Scalar scalar = queue_pop(STATE->scalar_queue);
    state_unlock(STATE);
    return scalar;
}

uint8_t io_read_byte(void) {
    state_lock(STATE);
    while (queue_empty(STATE->byte_queue) && STATE->app_running) {
        state_wait(STATE->input_ready, STATE->lock);
    }
    if (!STATE->app_running) {
        state_unlock(STATE);
        return runtime_abort_due_to_closed_window();
    }
    uint8_t byte = queue_pop(STATE->byte_queue);
    state_unlock(STATE);
    return byte;
}

bool io_poll_input(int timeout_ms) {
    state_lock(STATE);
    if (input_queue_has_data(STATE)) {
        state_unlock(STATE);
        return true;
    }
    if (timeout_ms > 0) {
        state_timed_wait(STATE->input_ready, STATE->lock, timeout_ms);
    }
    bool ready = input_queue_has_data(STATE);
    state_unlock(STATE);
    return ready;
}

bool io_is_input_terminal(void)  { return true; }
bool io_is_output_terminal(void) { return true; }
bool io_is_error_terminal(void)  { return true; }

Size io_terminal_size(void) {
    state_lock(STATE);
    Size size = STATE->terminal_cells;
    state_unlock(STATE);
    return size;
}

void transcript_append_stdout(ByteString text) {
    id storage = objc_call_id(STATE->transcript_view, "textStorage");
    id value = nsstring_from_utf8(text.bytes);
    objc_call_void(storage, "appendAttributedString:", plain_stdout_string(value));
}

void transcript_append_stderr(ByteString text) {
    id storage = objc_call_id(STATE->transcript_view, "textStorage");
    id value = nsstring_from_utf8(text.bytes);
    objc_call_void(storage, "appendAttributedString:", styled_stderr_string(value));
}

void input_field_clear_and_refocus(AppRuntimeState *state) {
    objc_call_void(state->input_field, "setStringValue:", nsstring_from_utf8(""));
    objc_call_void(state->window, "makeFirstResponder:", state->input_field);
}

void appkit_show_window(AppRuntimeState *state) {
    objc_call_void(state->window, "makeKeyAndOrderFront:", NULL);
}

void appkit_activate_application(AppRuntimeState *state) {
    objc_call_void(state->app, "activateIgnoringOtherApps:", true);
}

void appkit_run_event_loop(AppRuntimeState *state) {
    objc_call_void(state->app, "run");
}

void flush_pending_output_main_thread(void) {
    ByteString stdout_chunk;
    ByteString stderr_chunk;

    state_lock(STATE);
    stdout_chunk = buffer_take(&STATE->stdout_pending);
    stderr_chunk = buffer_take(&STATE->stderr_pending);
    state_unlock(STATE);

    if (!stdout_chunk.empty) {
        transcript_append_stdout(stdout_chunk);
    }
    if (!stderr_chunk.empty) {
        transcript_append_stderr(stderr_chunk);
    }
    transcript_scroll_to_end(STATE);
}
```
