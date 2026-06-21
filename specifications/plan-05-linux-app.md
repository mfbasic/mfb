# MFBASIC Linux App Mode Plan

Last updated: 2026-06-19

This document proposes a real Linux GUI application mode for MFBASIC native
targets. The goal is to make:

```text
mfb build -target linux-* -app <project>
```

produce a native Linux application whose `io::*` built-ins interact with a
window instead of the terminal.

This is a planning document. It describes the intended behavior, required
compiler and runtime changes, open design points, validation strategy, and a
recommended implementation sequence. It is the Linux counterpart to
`specifications/plan-macos-app.md` and shares its structure deliberately.

It complements:

- `specifications/plan-macos-app.md`
- `specifications/plan-linker.md`
- `specifications/architecture.md`
- `specifications/linker.md`
- `specifications/threading.md`
- `specifications/project.md`

## 1. Summary

Current Linux native output is a console-style executable:

- The CLI has no `-app` mode today.
- `src/main.rs` only understands output flags and `-target`.
- `src/target/linux_aarch64/mod.rs` exposes a console-oriented runtime call set.
- `src/target/linux_aarch64/plan.rs` and `src/target/linux_aarch64/code.rs` wire
  `io::*` to libc functions such as `write`, `read`, and `poll`.
- `src/target/shared/code/mod.rs` generates runtime helpers assuming terminal or
  file-descriptor semantics.

That design is correct for normal native executables, but it cannot satisfy the
desired app behavior:

- `io::print` and `io::write` must append to a window-backed output surface.
- `io::input`, `io::readLine`, `io::readChar`, `io::readByte`, and
  `io::pollInput` must read from window-backed input.
- `io::isInputTerminal`, `io::isOutputTerminal`, `io::isErrorTerminal`, and
  `io::terminalSize` need defined app-mode behavior.
- Program startup can no longer be a trivial `_start -> language entry -> exit`
  flow. The GTK main loop owns the process event loop.

The correct implementation is a dedicated Linux app runtime mode, not a terminal
helper shim.

### 1.1 Toolkit Choice: GTK4

App mode binds **GTK4** through its **GObject C ABI**. This is the Linux analog
of the macOS plan binding AppKit through `libobjc`/`objc_msgSend`:

- GTK is pure C with a stable C ABI. `mfb` emits ordinary C calls to
  `gtk_*`/`g_*` symbols exactly as it emits calls to `write`/`open`/`pow` today.
- This fits the existing "compiler emits and links everything, no host
  toolchain" architecture. There is no C++ ABI, no `moc` codegen step, and no
  compiled shim — which is why Qt/KDE was rejected (see
  `[[native-linking-decisions]]` and `plan-linker.md`).
- GTK provides the exact widgets the transcript+input model needs: `GtkTextView`
  + `GtkTextBuffer` for the read-only transcript (the `NSTextView` analog),
  `GtkText`/`GtkEntry` for single-line input (`NSTextField` analog), and
  `GtkScrolledWindow` for scrolling. Unicode text editing, IME, font metrics,
  and scrolling come for free.

App mode is **glibc-only for v1**. GTK is a glibc-world dependency, and a
single process cannot mix a musl libc with a glibc-built GTK. The console build
keeps emitting both `-glibc.out` and `-musl.out`; `-app` emits only the glibc
executable. A musl GTK flavor (running only on musl desktops such as Alpine with
musl-built GTK) is deferred, not impossible.

## 2. User-Facing Goal

When all of the following are true:

- build target is `linux-*`
- project kind is `executable`
- the CLI includes `-app`

the compiler should produce a Linux GUI application whose standard MFBASIC
`io::*` built-ins target an app window.

Expected user experience:

1. Launch the output.
2. A native window opens (GTK picks the X11 or Wayland GDK backend).
3. Calls to `io::print` / `io::write` append text into the window.
4. Calls to `io::input` / `io::readLine` / `io::readChar` / `io::readByte`
   consume user-entered text from the same application.
5. The program remains responsive while the GTK event loop is running.

## 3. Non-Goals

This plan does not require:

- replacing all `fs::*`, `thread::*`, or non-IO runtime helpers with GTK
  equivalents
- adding graphics, menus, buttons, multi-window UI, or widget APIs to MFBASIC
- introducing a general GObject/C FFI for user code
- changing non-Linux backends
- changing normal `mfb build -target linux-*` console behavior
- bundling GTK or its dependencies (system GTK is assumed present; see §4)

Future enhancements may add richer app metadata, `.desktop` integration, or UI
APIs, but they are not required for the first app-mode release.

## 4. Distribution Considerations

This is the Linux analog of the macOS plan's App Store section, but far lighter.
Linux has no mandatory review gate, so the considerations are practical rather
than policy.

### 4.1 Output Is A Bare ELF Executable

App mode emits a single glibc ELF executable that dynamically links system GTK4.
There is no bundle: GTK is a system dependency, so there is nothing to package
into the executable.

Implications:

- The target machine must have GTK4 and its runtime dependencies installed
  (`libgtk-4.so.1`, `libgobject-2.0.so.0`, `libglib-2.0.so.0`,
  `libgio-2.0.so.0`, plus their transitive deps). This is true of essentially
  all mainstream Linux desktops (GNOME and most environments pull GTK4 in).
- This is a softer "self-contained" story than macOS, where AppKit ships with
  every Mac. On Linux the program depends on the user having GTK4 installed. The
  plan accepts that as a stated assumption rather than bundling.

### 4.2 Optional Desktop Integration

A future enhancement may emit a `.desktop` launcher and icon for menu/dock
integration. v1 does not require it; the executable runs when launched directly.

### 4.3 Self-Contained Portable Bundles Are Future Work

AppImage/Flatpak/Snap-style portable bundles that ship a private GTK tree are
out of scope. They require rpath/vendoring, which `plan-linker.md` deliberately
keeps out of the built-in linker (vendoring lives on the user `LINK`/`dlopen`
path). A future packaging profile can layer this on without changing the runtime
design.

## 5. Proposed Semantics

### 5.1 CLI Contract

Add a new build flag:

```text
mfb build -app
```

Rules:

- `-app` is valid only with `build`.
- `-app` is valid only for executable projects.
- `-app` is valid only when `-target` resolves to a native target whose OS
  supports app mode (currently macOS or Linux).
- `-app` affects the default executable build path and native intermediate
  outputs (`-nir`, `-nplan`, `-nobj`, `-ncode`) because all of them need to
  represent the alternate runtime mode.
- `-app` is rejected for package projects.
- For a Linux target, `-app` produces only the glibc executable (the musl flavor
  is not emitted in app mode).

This flag is shared with `plan-macos-app.md`; the target's OS selects the
toolkit. Recommended validation:

```rust
if options.app_mode && !target_supports_app_mode(&options.target) {
    return Err("mfb build -app is not supported for this target".to_string());
}
if options.app_mode && project_kind != "executable" {
    return Err("mfb build -app requires an executable project".to_string());
}
```

### 5.2 Output Shape

Phase 1 emits a single glibc ELF executable:

```text
<project>/<name>.out
```

Notes:

- App mode does **not** emit the `-musl.out` flavor. Console mode is unchanged
  and continues to emit both flavors.
- The CLI should print:

  ```text
  Wrote executable to <project>/<name>.out
  ```

- Do not emit both a console and an app executable for the same build. `-app`
  selects the app executable.

### 5.3 Window Model

Initial app mode uses one primary window containing:

- a non-editable scrollable transcript/output region
- a single-line editable input field

Recommended composition (top to bottom in a vertical `GtkBox`):

```text
GtkApplicationWindow
  GtkBox (vertical)
    GtkScrolledWindow (expand)
      GtkTextView (read-only transcript, GtkTextBuffer)
    GtkText / GtkEntry (single-line input)
```

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
than trying to model raw key-event streams, and it sidesteps IME/composition
issues.

### 5.4 `io::*` Behavior In App Mode

App mode redefines runtime behavior, not type signatures. The semantics are
identical to `plan-macos-app.md` §5.4 — only the UI backend differs. They are
restated here for completeness.

#### `io::print(text AS String)`

- append `text`, append newline, render in transcript

#### `io::write(text AS String)`

- append `text`, no newline, render in transcript

#### `io::printError(text AS String)` / `io::writeError(text AS String)`

- append `text` (with/without newline) to transcript
- visually distinguish error output (a distinct text tag/color, or a `[stderr] `
  prefix); a `GtkTextTag` with a red foreground is the natural GTK mechanism

#### `io::flush()`, `io::flushError()`

- succeed immediately
- force any pending UI transcript updates to become visible before returning;
  they cannot be no-ops when output is buffered on the worker thread, so they
  must synchronize with the UI dispatch path (see §6.4)

#### `io::input()` / `io::input(prompt AS String)`

1. write `prompt` to the transcript (no forced newline unless the prompt
   contains one)
2. focus the input field
3. block the MFBASIC worker thread until the user submits a line
4. return the submitted line without its trailing newline

`io::input()` is equivalent to `io::input("")`.

#### `io::readLine()`

- block until one submitted line is available; return it without its terminator

#### `io::readChar()`

- return the next UTF-8 scalar from the committed-input queue
- if empty, block until the user submits a line, then consume from that line plus
  a synthetic `\n` boundary

#### `io::readByte()`

- return the next byte from the same committed-input queue used by `readChar`
- if empty, block until another committed line is submitted

#### `io::pollInput()` / `io::pollInput(timeoutMs)`

- `pollInput()` returns `TRUE` iff at least one committed input item is
  immediately available
- `pollInput(timeoutMs)` waits up to `timeoutMs`; `timeoutMs < 0` waits
  indefinitely, matching the standard contract

This implies a shared input state with three queue views (line, UTF-8 character,
byte), identical to the macOS design.

#### `io::isInputTerminal()`, `io::isOutputTerminal()`, `io::isErrorTerminal()`

Recommended app-mode result: `TRUE`, `TRUE`, `TRUE`. The app window is the
interactive console for the program; returning `FALSE` would cause many
console-oriented programs to disable interactive behavior. This is a semantic
"interactive terminal equivalent", not a literal POSIX TTY claim.

#### `io::terminalSize()`

Recommended app-mode result: the transcript viewport size expressed as text
columns and rows, computed from:

- the `GtkTextView` content width/height
- a chosen monospaced font's Pango metrics
  (`pango_font_metrics_get_approximate_char_width`, line height from the font
  description)

A fixed `80x24` fallback would violate the completion standard for real
behavior; app mode must compute actual visible dimensions before it is described
as complete.

## 6. Runtime Architecture

### 6.1 Core Model

App mode needs two threads with clearly separated responsibilities:

- UI thread: owns the `GtkApplication`, window, text view, input field, and the
  GLib/GTK main loop
- language runtime thread: runs MFBASIC `main` and any existing runtime helper
  calls

GTK is **not thread-safe**: all GTK/GDK calls must happen on the thread that runs
the main loop. Running the MFBASIC entry point directly on that thread is a bad
fit because:

- `io::readLine()` and friends are synchronous/blocking APIs
- user code may perform long-running work
- blocking the main thread would freeze the UI and stop event processing

This is the same constraint as AppKit on the main thread, and the same
worker-thread resolution.

Recommended process startup:

```text
_start (or libc-provided entry)
  -> initialize app-mode runtime globals
  -> gtk_init / create GtkApplication and signal handlers
  -> on "activate": create window and UI controls, spawn language thread
  -> g_application_run on the main thread
  -> exit when the app terminates
```

Language thread:

```text
thread start
  -> call MFBASIC entry symbol
  -> capture exit status / runtime trap
  -> notify UI thread that program completed (g_idle_add)
```

### 6.2 Shared App Runtime State

Introduce a dedicated runtime state object for app mode, identical in shape to
the macOS design except the UI-owned handles are GObject pointers:

```c
typedef struct {
    // lifecycle
    uint64_t program_done;
    int64_t  program_exit_code;
    const char *program_error_message;

    // output
    Mutex      output_lock;
    ByteBuffer pending_stdout;
    ByteBuffer pending_stderr;
    uint64_t   output_generation;

    // input
    Mutex     input_lock;
    CondVar   input_cv;
    LineQueue line_queue;
    ByteQueue byte_queue;
    CharQueue char_queue;
    uint64_t  input_generation;

    // UI-owned opaque handles (GObject*)
    void *application;     // GtkApplication*
    void *window;          // GtkApplicationWindow*
    void *scrolled;        // GtkScrolledWindow*
    void *text_view;       // GtkTextView*
    void *text_buffer;     // GtkTextBuffer*
    void *input_field;     // GtkText* / GtkEntry*
    void *stderr_tag;      // GtkTextTag* for styled error runs

    // sizing
    uint64_t visible_columns;
    uint64_t visible_rows;
} MfbGtkAppRuntime;
```

Rust compiler code does not need to understand GTK object layouts. It only emits
code and imports for helper functions that manipulate the state.

### 6.3 Runtime Helper Boundary

As on macOS, separate app-mode behavior behind new app-runtime helper symbols
rather than rewriting every shared helper into a platform/mode conditional.
Example conceptual helper surface:

```text
_mfb_gtkapp_bootstrap
_mfb_gtkapp_program_start
_mfb_gtkapp_program_finish
_mfb_gtkapp_write_stdout
_mfb_gtkapp_write_stderr
_mfb_gtkapp_flush_stdout
_mfb_gtkapp_flush_stderr
_mfb_gtkapp_input_line
_mfb_gtkapp_read_line
_mfb_gtkapp_read_char
_mfb_gtkapp_read_byte
_mfb_gtkapp_poll_input
_mfb_gtkapp_terminal_size
_mfb_gtkapp_is_interactive
```

App-mode `io::*` runtime helpers call those symbols instead of `write`, `read`,
`poll`, or `isatty`-style primitives. The layering stays explicit:

```text
shared builtin signature/typecheck
  -> native lowering chooses helper symbol by target + mode
  -> app helper invokes GTK runtime directly via GObject C ABI
```

### 6.4 GObject / GTK Integration

GTK is invoked through **direct C calls** to `gtk_*` and `g_*` symbols. Unlike
the macOS plan, there is no `objc_msgSend` indirection — every call is an
ordinary C function call the codegen already knows how to emit. This makes the
Linux runtime bridge *simpler* than the macOS one, at the cost of GObject
verbosity.

Practical notes:

- **Callbacks** use `g_signal_connect` (a varargs macro over
  `g_signal_connect_data`). Prefer the non-variadic `g_signal_connect_data` form
  from emitted code to avoid varargs ABI handling. Signal handler function
  pointers point at emitted code.
- **Object construction**: prefer the typed constructors
  (`gtk_application_window_new`, `gtk_text_view_new`, `gtk_scrolled_window_new`,
  `gtk_entry_new`) over the variadic `g_object_new` to avoid varargs.
- **Main-thread marshaling**: `g_idle_add` (thread-safe to call from any thread)
  schedules a `GSourceFunc` to run on the main loop — the analog of macOS
  `dispatch_async_main`. For synchronous flush, schedule the flush and wait on a
  condvar signalled by the scheduled function (the analog of `dispatch_sync`),
  or use `g_main_context_invoke_full` with a completion signal.
- Required libraries: `libgtk-4.so.1`, `libgobject-2.0.so.0`,
  `libglib-2.0.so.0`, `libgio-2.0.so.0`.

### 6.5 UI Design Details

Recommended properties:

- transcript uses a monospaced font (set via a `GtkCssProvider` applying
  `font-family: monospace;` to the text view, or a `PangoFontDescription`)
- transcript is non-editable (`gtk_text_view_set_editable(view, FALSE)`) and
  selectable
- transcript auto-scrolls to the bottom on appended output
  (`gtk_text_view_scroll_to_mark` on an end mark)
- input field keeps keyboard focus while the program is running
- the input field's `activate` signal (Return) commits the current buffer

Transcript model:

- maintain text in the `GtkTextBuffer`
- append stdout and stderr runs; apply the `stderr_tag` to error runs
- update in batches to avoid one main-loop dispatch per byte

Suggested batch rule (identical strategy to macOS):

- write helpers append bytes to shared buffers
- schedule a single pending UI flush if one is not already enqueued
- the UI thread drains accumulated output and applies one buffer update

### 6.6 Input Design Details

Do not treat raw key events as the canonical input source for `io::*`. Composed
text/IME, Unicode correctness, and line-oriented APIs all argue for committed
text instead.

Instead:

- accept text entry through the normal `GtkText`/`GtkEntry` control
- on `activate`, read the committed string with `gtk_editable_get_text`
- push one logical line to the line queue
- also encode it as UTF-8 and populate char/byte queues
- append a newline character and newline byte so `readChar`/`readByte` observe
  line boundaries

Queue conversion (identical to macOS):

```text
submitted text
  -> UTF-8 bytes
  -> line_queue.push(submitted_without_newline)
  -> char_queue.push(each Unicode scalar)
  -> char_queue.push("\n")
  -> byte_queue.push(each UTF-8 byte)
  -> byte_queue.push(0x0A)
```

Recommended first release: `readChar()` returns Unicode scalar values encoded
back into single-character MFB strings (not grapheme clusters), matching the
macOS decision and the "next character in the UTF-8 stream" model.

### 6.7 Shutdown And Exit

Program completion should not hard-exit the process from the worker thread.
Recommended behavior:

1. worker thread stores the final result in app runtime state
2. worker thread schedules a completion notification onto the main loop
   (`g_idle_add`)
3. the UI thread flushes remaining output, optionally disables the input field,
   and either keeps the window open or terminates the app

Recommended first release:

- keep the window open after normal completion
- show a status line such as `Program exited with code 0`
- allow window close (the `close-request` signal) to terminate the app process

## 7. Compiler And Backend Changes

### 7.1 CLI Layer (`src/main.rs`)

- parse `-app`
- store `app_mode` in `BuildOptions`
- validate target/project-kind compatibility (shared with macOS)
- thread `app_mode` into backend selection and output writing
- for Linux app mode, suppress the musl flavor

### 7.2 Target Abstraction / Build Mode

Reuse the native build-mode concept from `plan-macos-app.md`. The macOS plan
proposed `enum NativeBuildMode { Console, MacApp }`. Linux adds a third variant,
or — recommended — the enum collapses to a target-agnostic discriminant:

```rust
enum NativeBuildMode {
    Console,
    App, // toolkit chosen by target OS: AppKit on macOS, GTK4 on Linux
}
```

A single `App` variant avoids a combinatorial mode set as more app targets are
added; the target already determines the toolkit. Thread the mode through
lowering, planning, codegen, and linking, exactly as the macOS plan describes.

### 7.3 Shared Native Lowering

Carry build mode into NIR, native plan, and native code plan metadata
(`build_mode` fields), as in `plan-macos-app.md` §6.3. This matters for goldens,
validation, linker behavior, and artifact debugging.

### 7.4 Linux Backend Capability Table

`src/target/linux_aarch64/mod.rs` currently advertises a console-oriented subset
of runtime calls. App mode requires full `io::*` coverage:

```text
io.print  io.write  io.printError  io.writeError
io.flush  io.flushError
io.input  io.readLine  io.readChar  io.readByte  io.pollInput
io.isInputTerminal  io.isOutputTerminal  io.isErrorTerminal  io.terminalSize
```

Console mode may keep a smaller set if that reflects current status, but app mode
must not ship partially wired IO semantics.

### 7.5 Native Plan Changes

`src/target/linux_aarch64/plan.rs` currently imports libc helpers such as
`write`, `read`, and `poll` for `io::*`. App-mode native planning must instead
import the GTK runtime symbols and declare the GTK/GObject/GLib/GIO libraries:

```text
libc.so.6            process/thread/memory primitives, exit
libgtk-4.so.1        GtkApplication, GtkApplicationWindow, GtkTextView, ...
libgobject-2.0.so.0  g_signal_connect_data, g_object_*, GObject type system
libglib-2.0.so.0     g_idle_add, GMainLoop/GMainContext, memory, GString
libgio-2.0.so.0      GApplication plumbing used by GtkApplication
```

This depends on `plan-linker.md`: the Linux linker is already multi-library for
**functions** (one `DT_NEEDED` per distinct library), which covers the GTK call
surface. If any imported GTK/GObject **data global** is referenced, it requires
`GLOB_DAT` (the imported-data-global gap in `plan-linker.md` §6.1, Phase 4).
Audit the actual symbol references early; the GTK call surface is largely
function-based (`gtk_*_get_type()` accessors are functions), so `GLOB_DAT` may
not be required, but this must be confirmed against real symbol tables.

### 7.6 Native Codegen Changes

App mode needs a different entry path. Current conceptual entry:

```text
entry:
  call language entry
  map return value
  exit(status)
```

App-mode entry:

```text
entry:
  initialize app runtime state
  call _mfb_gtkapp_bootstrap(language_entry_symbol, argc, argv)
  exit when the app terminates
```

`_mfb_gtkapp_bootstrap` creates the `GtkApplication`, wires the `activate`
handler (which builds the window and spawns the worker thread), and runs
`g_application_run`. This keeps the generated entry small and keeps GTK logic
outside raw assembly.

### 7.7 Runtime Helper Codegen

Shared helper lowering in `src/target/shared/code/mod.rs` selects helper
implementations by `(target, build_mode, call)`:

```rust
match (platform.target(), build_mode, spec.call) {
    ("linux-aarch64", NativeBuildMode::App, "io.print")    => ...,
    ("linux-aarch64", NativeBuildMode::App, "io.readLine") => ...,
    _ => existing behavior,
}
```

Keep existing console helpers for console mode; add app-mode helper lowerers that
wrap the `_mfb_gtkapp_*` symbols. Do not branch inside the console helper bodies.

### 7.8 Linker / Object Writer Changes

The Linux linker already emits one `DT_NEEDED` per distinct library and binds
imported functions via `R_AARCH64_JUMP_SLOT`. App mode primarily needs the larger
import surface to remain layout-correct (see §9). Anything beyond multi-library
functions is governed by `plan-linker.md`:

- imported data globals (`GLOB_DAT`) — only if GTK/GObject data symbols are
  referenced (Phase 4)
- load-time initializers (`DT_INIT_ARRAY`) — only if emitted runtime support
  needs explicit startup init; GLib/GTK library constructors run automatically
  when their `.so` loads (Phase 4)
- symbol versioning is **not** required for GTK at the level `mfb` calls
  (validate, but GTK exports are generally usable unversioned); OpenSSL is the
  forcing case for versioning, not GTK

No bundle, `Info.plist`, or framework-path machinery is needed (those are macOS
concerns). The Linux output is a single ELF.

## 8. Risks

### 8.1 Highest-Risk Areas

1. **GTK on the main thread.** Mandatory. All GTK/GDK calls run on the main-loop
   thread. Any design that calls GTK from the worker thread is invalid; the
   worker marshals via `g_idle_add`.
2. **Larger import surface stressing the linker.** Going from a dozen libc
   symbols to dozens of GTK/GObject symbols across four-plus `DT_NEEDED`
   libraries stresses the import-stub/GOT layout code — exactly the
   layout-sensitive `SIGBUS` hazard in `[[macos-codegen-latent-bugs]]`. Treat
   import-table robustness as a first-class phase.
3. **Unicode semantics for `readChar`.** Specify scalars vs graphemes clearly;
   the plan chooses scalars.
4. **Blocking semantics with a responsive UI.** This is why the worker-thread
   model is required. Anything else risks UI hangs.
5. **GTK runtime dependency.** The target must have GTK4 installed; a missing
   `libgtk-4.so.1` fails at load with a loader error. v1 accepts this; a future
   profile can surface a friendlier diagnostic.

### 8.2 Common Failure Modes To Avoid

- treating the app window as a pseudo-file descriptor
- leaving `io::flush` a no-op while UI updates are buffered
- implementing only `print/write` and leaving `read*` or `terminalSize`
  unsupported
- returning a fake `80x24` terminal size in the final implementation
- capturing raw key events and breaking Unicode/IME input
- calling GTK from the worker thread
- exiting the process directly from the worker thread
- claiming completion based only on artifact generation

## 9. Testing And Validation

This feature must not be treated as complete based only on NIR/NPlan/NCode
goldens. Runtime validation is mandatory.

### 9.1 Required Function Test Coverage

Any modified `io::*` function must have matching valid and invalid coverage under
the repository's existing conventions, mirroring `plan-macos-app.md` §7.1:

```text
tests/func_io_print_valid/**          tests/func_io_print_invalid/**
tests/func_io_write_valid/**          tests/func_io_write_invalid/**
tests/func_io_printError_valid/**     tests/func_io_printError_invalid/**
tests/func_io_writeError_valid/**     tests/func_io_writeError_invalid/**
tests/func_io_flush_valid/**          tests/func_io_flush_invalid/**
tests/func_io_flushError_valid/**     tests/func_io_flushError_invalid/**
tests/func_io_input_valid/**          tests/func_io_input_invalid/**
tests/func_io_readLine_valid/**       tests/func_io_readLine_invalid/**
tests/func_io_readChar_valid/**       tests/func_io_readChar_invalid/**
tests/func_io_readByte_valid/**       tests/func_io_readByte_invalid/**
tests/func_io_pollInput_valid/**      tests/func_io_pollInput_invalid/**
tests/func_io_isInputTerminal_*/**    tests/func_io_isOutputTerminal_*/**
tests/func_io_isErrorTerminal_*/**    tests/func_io_terminalSize_*/**
```

Add any missing directories where the function exists but lacks dedicated
coverage.

### 9.2 App-Mode Runtime Tests

App mode needs end-to-end tests beyond normal acceptance:

- build a small app-mode project
- launch the produced executable in a controlled harness
- programmatically feed input
- observe transcript output or another runtime-observable artifact

#### Strategy A: headless app-runtime self-test mode (recommended for CI)

A test-only app runtime mode that does not create a visible window, uses the same
app-mode helper surface, captures transcript output to a buffer, and injects
queued input lines. GTK supports headless operation via offscreen/`GDK_BACKEND`
options, but a cleaner approach is to swap only the UI sink/source while reusing
the same helper logic. This is deterministic and CI-friendly and must exercise
the same helper logic, only swapping the UI sink/source.

#### Strategy B: UI automation

Drive the real window with AT-SPI accessibility tooling (e.g. `dogtail`). Closest
to real behavior, but brittle and slower. Use as a manual/occasional smoke test.

Recommended: Strategy A for automated acceptance, plus at least one manual
real-window smoke recipe.

### 9.3 Acceptance Suite

After implementation, run:

```text
scripts/test-accept.sh target/debug/mfb target/accept-actual
```

App-mode work also requires acceptance updates for CLI parsing diagnostics,
native intermediate outputs containing build-mode metadata, the Linux native
object/code plans importing GTK libraries, and the executable output path/flavor
selection under `-app`.

### 9.4 Manual Verification Checklist

1. Build with `mfb build -target linux-aarch64 -app`.
2. Confirm a single glibc executable is written (no musl flavor).
3. Launch and verify a window appears.
4. Verify `io::print` appends transcript text.
5. Verify `io::printError` is visibly distinguished.
6. Verify `io::input` blocks and returns submitted text.
7. Verify `io::readLine`, `io::readChar`, and `io::readByte` behave correctly.
8. Verify `io::pollInput(0)` reflects queued committed input.
9. Verify `io::terminalSize()` changes as the window is resized.
10. Verify program completion behavior and window-close shutdown.

## 10. Recommended Implementation Sequence

### Phase 1: Mode Plumbing
- add `-app` CLI parsing and diagnostics (shared with macOS)
- add/extend `NativeBuildMode`
- thread mode through IR lowering, NIR, native plan, native code plan
- update text/json artifact formats and validations
- suppress the musl flavor under `-app`

Deliverable: `-app` parses and propagates; intermediate outputs record app mode;
invalid target/project combinations are rejected.

### Phase 2: ELF / Linker Support For GTK Libraries
- confirm the Linux linker emits all required `DT_NEEDED` libraries and that the
  larger import surface stays layout-correct (GOT/stub validation)
- handle any GTK data-global / initializer needs per `plan-linker.md` (Phase 4
  there), if the symbol audit shows they are needed

Deliverable: a simple app-mode executable linking GTK can be emitted and launched.

### Phase 3: App Runtime Bridge Bootstrap
- add the GTK app runtime support emitted by `mfb`
- implement `_mfb_gtkapp_bootstrap`, window creation, transcript view, input field
- run the language entry on a worker thread

Deliverable: the app launches a window; the worker thread runs a trivial program.

### Phase 4: Output Path
- implement app-mode `print/write/printError/writeError`
- implement UI batching and flush semantics

Deliverable: the transcript correctly shows stdout/stderr output.

### Phase 5: Input Path
- implement committed-line input submission
- implement line/char/byte queues
- implement `input`, `readLine`, `readChar`, `readByte`, `pollInput`

Deliverable: blocking input APIs work against the window.

### Phase 6: Interactive Metadata
- implement `isInputTerminal`/`isOutputTerminal`/`isErrorTerminal`
- implement computed `terminalSize` and resize updates

Deliverable: all `io::*` semantics needed for app mode are live.

### Phase 7: Validation
- add/update mandatory function tests
- add app-mode end-to-end runtime tests
- run the full acceptance suite
- perform manual smoke verification

## 11. Recommended Initial Example Program

Used as a manual smoke test and later an automated runtime fixture (identical to
the macOS plan, so the same program validates both backends):

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

Expected observations: all output appears in the transcript; each prompt is
visible before blocking; line submission unblocks reads; terminal size reflects
the current window.

## 12. Open Design Decisions

1. Should app mode later emit a `.desktop` launcher and icon, and if so with what
   install layout?
2. Should stdout/stderr be styled (a `GtkTextTag`), prefixed, or both?
3. Should `readChar()` return Unicode scalars (planned) or grapheme clusters?
4. On normal completion, keep the window open until manually closed, or add a
   configurable policy?
5. Should app mode be represented only by the CLI flag, or also by manifest
   metadata for persistent project configuration (shared with macOS)?
6. GDK backend: let GTK auto-select X11/Wayland (recommended), or expose a
   selection mechanism?
7. GTK version: GTK4 (recommended). GTK3 is not a target.

## 13. Recommendation

Proceed with a dedicated Linux app runtime mode with these choices:

- `-app` is a shared executable build flag; the target OS selects the toolkit
- Linux app mode binds **GTK4** through the GObject C ABI
- app mode emits a single **glibc ELF executable** (no bundle, no musl flavor)
- the main thread runs the GTK main loop; MFBASIC entry runs on a worker thread
- generated code binds directly to public GTK/GObject/GLib/GIO C APIs
- all `io::*` interactive behavior is implemented against a transcript + input
  field model
- `io::is*Terminal()` returns `TRUE`
- `io::terminalSize()` returns measured visible text dimensions
- the runtime bridge is *simpler* than macOS (direct C calls, no `objc_msgSend`)

This is the smallest design that is still technically coherent and
production-grade, and it reuses the macOS plan's architecture, semantics, tests,
and example program wholesale.

## 14. C pseudocode

Guide-level pseudocode only. It describes the runtime behavior `mfb` emits
internally for Linux app mode. It is not an external helper source file or a
build dependency; all calls are emitted by `mfb` to public GTK/GObject/GLib/GIO
symbols. Unlike the macOS pseudocode, calls are ordinary C calls — there is no
`objc_msgSend` wrapper layer.

```c
// Pseudocode only. The generated binary calls libgtk-4 / libgobject-2.0 /
// libglib-2.0 / libgio-2.0 directly via their public C ABI. All such calls are
// emitted by mfb.

typedef struct {
    Mutex   lock;
    CondVar input_ready;
    CondVar worker_done;

    bool app_running;
    bool worker_finished;
    int  worker_exit_code;

    ByteBuffer stdout_pending;
    ByteBuffer stderr_pending;
    Size       terminal_cells;

    Queue<String>     committed_lines;
    Queue<Utf8Scalar> scalar_queue;
    Queue<uint8_t>    byte_queue;

    void *application;    // GtkApplication*
    void *window;         // GtkApplicationWindow*
    void *text_view;      // GtkTextView*
    void *text_buffer;    // GtkTextBuffer*
    void *input_field;    // GtkText* / GtkEntry*
    void *stderr_tag;     // GtkTextTag*
} AppRuntimeState;

static AppRuntimeState *STATE;

void app_mode_bootstrap(void (*language_entry)(void), int argc, char **argv) {
    STATE = state_create();

    // GtkApplication is created on the main thread; the main loop owns all GTK.
    STATE->application =
        gtk_application_new("dev.mfbasic.app", G_APPLICATION_DEFAULT_FLAGS);
    g_signal_connect_data(STATE->application, "activate",
                          (GCallback)on_activate, STATE, NULL, 0);

    // language program runs on a worker thread (spawned from on_activate).
    g_application_run((GApplication *)STATE->application, argc, argv);
}

void on_activate(void *app, void *user_data) {
    AppRuntimeState *s = user_data;

    s->window = gtk_application_window_new((GtkApplication *)app);
    gtk_window_set_title((GtkWindow *)s->window, "MFBASIC App");
    gtk_window_set_default_size((GtkWindow *)s->window, 900, 640);

    void *box   = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    void *scroll = gtk_scrolled_window_new();
    gtk_widget_set_vexpand(scroll, TRUE);

    s->text_view = gtk_text_view_new();
    gtk_text_view_set_editable((GtkTextView *)s->text_view, FALSE);
    gtk_text_view_set_monospace((GtkTextView *)s->text_view, TRUE);
    s->text_buffer = gtk_text_view_get_buffer((GtkTextView *)s->text_view);
    s->stderr_tag = gtk_text_buffer_create_tag((GtkTextBuffer *)s->text_buffer,
                                               "stderr", "foreground", "red", NULL);
    gtk_scrolled_window_set_child((GtkScrolledWindow *)scroll, s->text_view);

    s->input_field = gtk_entry_new();
    g_signal_connect_data(s->input_field, "activate",
                          (GCallback)on_input_committed, s, NULL, 0);

    gtk_box_append((GtkBox *)box, scroll);
    gtk_box_append((GtkBox *)box, s->input_field);
    gtk_window_set_child((GtkWindow *)s->window, box);

    g_signal_connect_data(s->window, "close-request",
                          (GCallback)on_window_closed, s, NULL, 0);

    state_lock(s);
    s->app_running = true;
    state_unlock(s);

    gtk_window_present((GtkWindow *)s->window);
    thread_start(worker_main, language_entry);
}

void worker_main(void *entry_ptr) {
    void (*language_entry)(void) = entry_ptr;
    int exit_code = run_language_entry_and_capture_exit(language_entry);

    state_lock(STATE);
    STATE->worker_finished = true;
    STATE->worker_exit_code = exit_code;
    state_signal_all(STATE->worker_done);
    state_unlock(STATE);

    // Final UI actions are scheduled back onto the main loop thread.
    g_idle_add((GSourceFunc)on_worker_finished_main_thread, NULL);
}

bool on_input_committed(void *entry_widget, void *user_data) {
    AppRuntimeState *s = user_data;
    const char *text = gtk_editable_get_text((GtkEditable *)entry_widget);
    String line = string_from_utf8(text);

    state_lock(s);
    queue_push(s->committed_lines, line);
    foreach (Utf8Scalar scalar in utf8_decode_scalars(line))
        queue_push(s->scalar_queue, scalar);
    queue_push(s->scalar_queue, '\n');
    foreach (uint8_t b in utf8_encode(line).bytes)
        queue_push(s->byte_queue, b);
    queue_push(s->byte_queue, '\n');
    state_signal_all(s->input_ready);
    state_unlock(s);

    transcript_append_stdout(line);
    transcript_append_stdout("\n");
    gtk_editable_set_text((GtkEditable *)entry_widget, "");
    gtk_widget_grab_focus((GtkWidget *)s->input_field);
    return true;
}

bool on_window_closed(void *window, void *user_data) {
    AppRuntimeState *s = user_data;
    state_lock(s);
    s->app_running = false;
    state_signal_all(s->input_ready);
    state_signal_all(s->worker_done);
    state_unlock(s);
    g_application_quit((GApplication *)s->application);
    return false; // allow default close
}

bool on_worker_finished_main_thread(void *unused) {
    flush_pending_output_main_thread(NULL);
    // policy: keep window open, show status; or quit. Decided elsewhere.
    return false; // remove the idle source
}

ResultNothing io_print(String text) {
    state_lock(STATE);
    buffer_append_utf8(&STATE->stdout_pending, text);
    buffer_append_utf8(&STATE->stdout_pending, "\n");
    state_unlock(STATE);
    schedule_flush_if_needed();
    return make_ok_nothing();
}

ResultNothing io_write(String text) {
    state_lock(STATE);
    buffer_append_utf8(&STATE->stdout_pending, text);
    state_unlock(STATE);
    schedule_flush_if_needed();
    return make_ok_nothing();
}

ResultNothing io_print_error(String text) {
    state_lock(STATE);
    buffer_append_utf8(&STATE->stderr_pending, text);
    buffer_append_utf8(&STATE->stderr_pending, "\n");
    state_unlock(STATE);
    schedule_flush_if_needed();
    return make_ok_nothing();
}

ResultNothing io_flush(void) {
    // schedule the flush on the main loop and wait for it to complete.
    flush_sync_on_main_thread();
    return make_ok_nothing();
}

ResultString io_read_line(void) {
    state_lock(STATE);
    while (queue_empty(STATE->committed_lines) && STATE->app_running)
        state_wait(STATE->input_ready, STATE->lock);
    if (!STATE->app_running) {
        state_unlock(STATE);
        return make_err(ERR_INPUT_FAILURE, "app window closed while reading input");
    }
    String line = queue_pop(STATE->committed_lines);
    state_unlock(STATE);
    return make_ok_string(line);
}

ResultString io_read_char(void) {
    state_lock(STATE);
    while (queue_empty(STATE->scalar_queue) && STATE->app_running)
        state_wait(STATE->input_ready, STATE->lock);
    if (!STATE->app_running) {
        state_unlock(STATE);
        return make_err(ERR_INPUT_FAILURE, "app window closed while reading input");
    }
    Utf8Scalar scalar = queue_pop(STATE->scalar_queue);
    state_unlock(STATE);
    return make_ok_string(string_from_scalar(scalar));
}

ResultByte io_read_byte(void) {
    state_lock(STATE);
    while (queue_empty(STATE->byte_queue) && STATE->app_running)
        state_wait(STATE->input_ready, STATE->lock);
    if (!STATE->app_running) {
        state_unlock(STATE);
        return make_err(ERR_INPUT_FAILURE, "app window closed while reading input");
    }
    uint8_t byte = queue_pop(STATE->byte_queue);
    state_unlock(STATE);
    return make_ok_byte(byte);
}

ResultBoolean io_poll_input(int timeout_ms) {
    state_lock(STATE);
    if (input_queue_has_data(STATE)) { state_unlock(STATE); return make_ok_boolean(true); }
    if (timeout_ms < 0) {
        while (!input_queue_has_data(STATE) && STATE->app_running)
            state_wait(STATE->input_ready, STATE->lock);
    } else if (timeout_ms > 0) {
        state_timed_wait(STATE->input_ready, STATE->lock, timeout_ms);
    }
    if (!STATE->app_running) {
        state_unlock(STATE);
        return make_err(ERR_INPUT_FAILURE, "app window closed while polling input");
    }
    bool ready = input_queue_has_data(STATE);
    state_unlock(STATE);
    return make_ok_boolean(ready);
}

ResultBoolean io_is_input_terminal(void)  { return make_ok_boolean(true); }
ResultBoolean io_is_output_terminal(void) { return make_ok_boolean(true); }
ResultBoolean io_is_error_terminal(void)  { return make_ok_boolean(true); }

ResultTerminalSize io_terminal_size(void) {
    state_lock(STATE);
    Size size = STATE->terminal_cells; // updated on size-allocate from Pango metrics
    state_unlock(STATE);
    if (size.columns <= 0 || size.rows <= 0)
        return make_err(ERR_UNSUPPORTED_OPERATION, "app transcript size is unavailable");
    return make_ok_terminal_size(size);
}

void flush_pending_output_main_thread(void *unused) {
    ByteString out_chunk, err_chunk;
    state_lock(STATE);
    out_chunk = buffer_take(&STATE->stdout_pending);
    err_chunk = buffer_take(&STATE->stderr_pending);
    state_unlock(STATE);

    if (!out_chunk.empty) transcript_append_stdout(out_chunk);
    if (!err_chunk.empty) transcript_append_stderr(err_chunk);
    transcript_scroll_to_end(STATE);
}

void transcript_append_stdout(ByteString text) {
    GtkTextIter end;
    gtk_text_buffer_get_end_iter((GtkTextBuffer *)STATE->text_buffer, &end);
    gtk_text_buffer_insert((GtkTextBuffer *)STATE->text_buffer, &end,
                           text.bytes, text.len);
}

void transcript_append_stderr(ByteString text) {
    GtkTextIter end;
    gtk_text_buffer_get_end_iter((GtkTextBuffer *)STATE->text_buffer, &end);
    gtk_text_buffer_insert_with_tags((GtkTextBuffer *)STATE->text_buffer, &end,
                                     text.bytes, text.len, STATE->stderr_tag, NULL);
}
```
