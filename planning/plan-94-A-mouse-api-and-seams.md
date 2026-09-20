# plan-94-A: Mouse events — API, types, and registration seams

Last updated: 2026-09-20
Overall Effort: huge (>3d)
Effort: large (3h–1d)
Depends on: nothing

Add mouse event support to the `term::` surface (`planning/term.md` item 8 —
clicks/drag/scroll) **and to the `canvas::` surface**, across CLI and `--app`
modes, delivered as an opt-in, poll-based API over a **unified stdin input
decoder** that CLI and every app backend share. This sub-plan (A) lands the
language surface — four new functions and six new types — and every registration
seam, wired to **no-op stubs** so the feature typechecks, lowers, and runs
(returning "no event") on all five targets before any decoder or backend exists.
It is the hub for the whole `plan-94` feature: sub-plans **B–E** reference §2–§4
here for the shared design.

Behavioral outcome for A alone: a program can `IMPORT term`, call
`term::enableMouse(TRUE)` and `term::pollMouse()`, or `IMPORT canvas` and call
`canvas::enableMouse(TRUE)`/`canvas::pollMouse()`, and build+run on every target;
`pollMouse()` always returns a `MouseEvent` with `kind = None`, and `enableMouse`
is inert. Nothing decodes mouse input yet.

References:

- `planning/term.md` item 8 (mouse) and item 7 (decoded key input — the same
  decoder, added later).
- `.ai/resources-packages.md` (the package/import subsystem and builtin-package
  authoring seams) and `.ai/compiler.md` (runtime completion gate, validation and
  function tests).
- `.ai/canvas-threading.md` §2 — **arena state is PER-THREAD**, the fact that
  shapes where mouse state may live in canvas mode. Read before §4.4.
- The landed `term::didResize` work — the closest precedent for a new no-arg
  `term::` call with a per-arena state slot, and for "the same concept exposed
  once per package" (`src/codegen/builtins/term/func_did_resize.rs` and
  `src/codegen/builtins/canvas/func_did_resize.rs` are two independent members).
- `src/docs/spec/app/04_term-backend.md` (term backend spec),
  `src/docs/spec/language/18_builtin-functions.md`,
  `src/docs/spec/memory/08_program-startup.md` (the arena-state region chain).

## Prerequisites

Everything below is written against the world where these hold.

| Must be true | Command | Status |
|---|---|---|
| On a branch, not `main` | `git rev-parse --abbrev-ref HEAD` → not `main` | MET (measured 2026-09-20 in `.claude/worktrees/P-94` → `worktree-P-94`) |
| Tree builds clean before starting | `cargo build` → `Finished` | MET (measured 2026-09-20, `Finished \`dev\` profile … in 44.28s`, exit 0) |

> The Status column is a snapshot; the Command column is the truth. Re-run before
> starting and before stopping.

## 1. Goal

- `term::enableMouse(enabled AS Boolean)` and `term::pollMouse() AS MouseEvent`
  are registered members that build and run on all five targets
  (`linux-{aarch64,riscv64,x86_64}`, `macos-aarch64`, `windows-x86_64`), console
  and `--app`.
- `canvas::enableMouse(enabled AS Boolean)` and
  `canvas::pollMouse() AS MouseEvent` are registered members that build and run
  in `--app` builds on macOS, GTK and Windows.
- The types `term::MouseEvent`/`MouseKind`/`MouseButton` and
  `canvas::MouseEvent`/`MouseKind`/`MouseButton` resolve and construct.
- `pollMouse()` returns a `MouseEvent` with `kind = MouseKind.None`
  unconditionally; `enableMouse(...)` is a no-op. No decoder, no queue, no ANSI
  emission yet.
- The **mouse-state arena region** and the **process-global mouse-mode word**
  (§4.2/§4.4) are reserved and zero-initialized, so B–E have their storage.

### Non-goals (explicit constraints)

- **No behavior change to existing `term::`/`canvas::`/`io::` calls.** In
  particular the stdin read path (`io::readChar`/`readByte`/`readLine`/
  `pollInput`, the plan-15 broadcast log) is untouched in A — only new symbols
  are added.
- **No new ANSI bytes emitted** by `term::on`/`off` in A (mouse tracking is
  opt-in and unimplemented here).
- **API shape is frozen by this sub-plan.** B–E implement behavior behind exactly
  these signatures and record layout; they do not renegotiate the surface.
- The six new types are additive; no existing builtin type changes.
- **No existing arena-state offset moves.** The new region is appended past the
  presentation-mode word (§4.4), so every program that does not use mouse keeps
  its exact entry frame and goldens.

## 2. Current State

### 2.1 How a builtin package is authored today

`term` and `canvas` are **clean-room registry** packages. A package's `mod.rs`
builds a `RegistryPackage` and hands it to the `Registry`; each member lives in
its own `func_*.rs` that registers a `RegistryFunction` carrying its own prose
(`intro`/`desc`/`example`) and one or more `Implementation`s, each with a
`Body::abi_function(lower_*)` lowering. Measured: `term` registers 24 members
(`grep -c '^    func_.*::register(&mut pkg);' src/codegen/builtins/term/mod.rs`
→ 24).

Every native `term::` body is a three-line delegation to the one family-generic
`gen_shared::lower_term_helper`, which branches app-vs-console off the `AbiCtx`
and appends `TermBodyParts` the `abi_function` wrapper finalizes
(`src/codegen/builtins/term/func_did_resize.rs` is the shortest example;
`src/codegen/builtins/term/gen_shared.rs::lower_term_helper` is the seam).

Two type *kinds* are the precedents, and both are registry-modeled:

- **Records** — `pkg.add_record(RegistryRecord { … props: vec![RecordProp{…}] })`.
  `term::TermSize` is one (`src/codegen/builtins/term/mod.rs`, in `register`);
  `canvas::Point`/`Size`/`Paint`/… are ordinary value records.
- **Enums** — `pkg.add_enum(RegistryEnum { … variants: vec![EnumVariant{…}] })`,
  rendered into the injected `<builtin-term>` / `<builtin-canvas>` source by
  `get_mfb`. `term::LineStyle`/`FillStyle` and `canvas::BlendMode`/`GradientKind`/
  `CapStyle` are these. **Variant ordinals follow declaration order**, so a
  variant declared first is ordinal 0.

### 2.2 The seam list (measured against HEAD)

Adding a call to an existing registry package touches **four** places, not the
seven the pre-migration draft of this plan listed:

1. **A new `func_<name>.rs`** in `src/codegen/builtins/<pkg>/` with the member's
   `INTRO`/`DESC`/`EX` prose, its `lower_*` body, and a `register(pkg)`; plus the
   `mod func_<name>;` line and the `func_<name>::register(&mut pkg);` call in the
   package's `mod.rs`. New types are `add_record`/`add_enum` calls in `mod.rs`.
2. **Per-target supported-call lists** — `src/target/macos_aarch64/mod.rs:186`,
   `src/target/linux_common/mod.rs:201` (all 3 linux arches share it),
   `src/target/win_x86_64/mod.rs:217` (the `"term.terminalSize"` neighbourhood in
   each). Missing one → build fails `native backend does not support runtime call
   '<pkg>.<call>'` (`src/target/shared/validate/capabilities.rs`,
   `validate_capabilities`).
3. **The shared code-layer arm** — the `match call` in
   `src/codegen/term/core/term.rs` (`lower_term_helper`, the `"term.terminalSize"`
   arm at `:306` is the shape), reached from `gen_shared::lower_term_helper`.
   Canvas members that need a code-layer emitter add theirs under
   `src/codegen/builtins/canvas/`.
4. **`plan.rs runtime_imports`** (`src/target/*/plan.rs`) **only if** the call
   makes a libc call. A pure state read/write falls through to no imports.

Three seams the migration **deleted** — do not re-add them:

- **No `term_specs.rs` / `catalog.rs` row.** Term's runtime specs are *derived*
  from the registry (`src/target/shared/runtime/catalog.rs:27`: "`term` is
  migrated: its 24 native OS-seam helpers … are DERIVED from the registry"). A
  hand-written `TERM_*_SPEC` row would be wrong.
- **No companion `.mfb` file.** `src/builtins/term_package.mfb` is gone; enums are
  `add_enum` and the injected source is rendered by `get_mfb`.
- **No `src/docs/man/builtins/term/*.md` page.** Built-in package, function and
  type man pages are rendered from the registry descriptors
  (`src/cli/man.rs`); the prose lives in the `func_*.rs` consts. Verify by
  rendering: `mfb man term pollMouse`, `mfb man term types`.

Two chores the seam list does not imply but the tree requires:

- The hard-coded member counts in `src/codegen/builtins/term/mod.rs:11`,
  `src/codegen/builtins/term/gen_shared.rs:11` and
  `src/target/shared/runtime/catalog.rs:27` all say "24 members"; adding two
  makes it 26.
- `src/docs/spec/memory/08_program-startup.md` documents the arena-state region
  chain and `TERM_STATE_SLOTS` (27); the new region (§4.4) belongs there.

### 2.3 App-mode dispatch and the presentation-mode gate

Each app backend has its own `emit_app_term_helper`
(`src/target/macos_aarch64/app/app_io.rs:551`,
`src/target/linux_gtk/app_io.rs:10`, `src/target/win_x86_64/app/mod.rs:2633`) that
returns `None` to delegate a call to the shared console backend.

**New since the pre-migration draft:** when a backend returns `Some`,
`gen_shared::lower_term_helper` prepends a presentation-mode gate
(`prepend_wrong_mode_gate`, `src/codegen/app/hook/app.rs:76`) with
`ModeRequirement::Console` — so `term::` raises the trappable `ErrWrongMode`
outside `Console`, **including in `Canvas`** (a canvas surface is pixels, not
cells). Calls that return `None` fall through to the console backend un-gated.

In A, `enableMouse`/`pollMouse` are **not** added to any app dispatch → they
delegate to the shared no-op stub, uniform on every mode. §4.5 records what B–E
must decide about the gate.

### Measured populations

| What | Count | Command |
|---|---|---|
| `term::` members before this feature | 24 | `grep -c '^    func_.*::register(&mut pkg);' src/codegen/builtins/term/mod.rs` → 24 |
| `term::` builtin types before this feature | 3 | `TermSize` record + `LineStyle`/`FillStyle` enums, in `register` (`src/codegen/builtins/term/mod.rs`) |
| Per-target supported-call lists to edit | 3 | `grep -rln '"term.terminalSize"' src/target/*/mod.rs` → `macos_aarch64`, `linux_common`, `win_x86_64` |
| Arena-state regions in the chain today | 2 | `term_state_offset` / `presentation_mode_offset` (`src/codegen/engine/builder/mod.rs:1527`, `:1543`) |

### Verified properties

- **A registry record MAY carry a companion-enum field.** `canvas` does it three
  times: `Gradient.kind AS GradientKind` (`src/codegen/builtins/canvas/mod.rs:495`),
  `DrawItem.blend AS BlendMode` (`:548`) and `Line.cap AS CapStyle` (`:667`), with
  those enums registered by `add_enum` in the same package. **The pre-migration
  draft's `Integer`-ordinal fallback is therefore unnecessary and is deleted**,
  along with its Open Decision. `MouseEvent.kind`/`button` are enum-typed.
- **`MouseEvent` is an ordinary value record, not a read-only one.**
  `term::TermSize` is read-only because the runtime allocates it
  (`is_read_only_record`), but nothing requires that of `MouseEvent`, and the
  canvas value-record shape is simpler. A program may construct one.
- **No-arg and one-arg members both exist** (`term::isOn` no-arg,
  `term::setBold(Boolean)` one-arg), so `pollMouse()`/`enableMouse(Boolean)` need
  no new arity machinery.
- **Overloads are supported** — a `RegistryFunction` carries a `Vec<Implementation>`
  (`canvas::getSize` has two, `src/codegen/builtins/canvas/func_get_size.rs:134,146`).
  Not needed here, but it is the escape valve if a coordinate-unit overload is
  ever wanted.
- **`term::on` already puts the tty in raw/cbreak mode** (`~ICANON`/`~ECHO`/
  `VMIN=1`/`VTIME=0`, bug-149), recorded in `TERM_STATE_RAW_ACTIVE_OFFSET`
  (`src/codegen/error/constants/error_constants.rs:318`). Mouse reporting needs
  raw mode, and it is already there — a prerequisite this plan used to carry
  implicitly and no longer has to. **But** `io::input`/`io::readLine` temporarily
  restore cooked mode around their read (`:322`/`:325`), which is a hazard B must
  handle (§4.6).

## 3. Design Overview (shared across plan-94 A–E)

The feature has four independent pieces; A builds only the first.

1. **Language surface (A).** Four functions + six types + all seams + the two new
   storage regions, no-op stubs.
2. **The unified input decoder / "pump" (B).** One stdin filter at the single
   per-byte read choke point that recognizes complete escape sequences,
   side-channels events into a worker-local queue, and passes non-event bytes
   through as characters. Fed by real tty bytes in CLI; reused verbatim for item 7
   (decoded keys).
3. **The event queue (B).** A per-thread (per-arena) fixed-size **timestamped
   overwrite-on-full ring**; `pollMouse` returns the oldest event ≤100 ms old.
   **One ring serves both packages** — `term::pollMouse` reads its coordinates as
   cells, `canvas::pollMouse` as pixels; the producer chose the unit.
4. **Per-backend byte injectors (C/D/E).** Each app backend converts a native
   mouse event to surface coordinates and **injects the SGR bytes into the same
   worker input pipe** it already uses for keystrokes — so the one decoder in (2)
   serves all four modes and both surfaces with no per-backend event queue.

**Where design uncertainty concentrates (schedule first, in B):** the decoder's
placement in the read path and its interaction with `io::pollInput` and the
cooked-mode restore (§4.6). B is the cheapest experiment that falsifies the "one
pump, all modes" premise.

**Where correctness risk concentrates (schedule last, C/D/E):** hand-written app
mouse handlers + px→surface conversion. These are compile/assembly-verified only
(no headless window), same limitation as `didResize` app mode.

**Byte-identity is NOT this feature's gate.** Every phase legitimately changes
emitted code and runtime behavior; the gates are rt-behavior tests (CLI) and
compile+assembly inspection (app). Byte-identity/`.ncode` goldens for term and
canvas fixtures are **expected to diff** whenever a phase changes codegen (A adds
the stub bodies; B adds the decoder/enable; C changes the macOS TermView) — a diff
there is the plan working; regenerate and confirm the diff is only the intended
change. The one place byte-identity *is* the right gate is the mouse-off read
path (B §Byte-identity note).

Rejected alternatives:

- **Per-backend native event queues + per-backend `pollMouse` arms** (instead of
  byte injection). Rejected: triples the hand-written per-backend surface and adds
  a thread-safe queue per backend. Byte injection reuses the existing keystroke
  pipe as the cross-thread channel and needs one decoder. (§4.3.)
- **Callback/handler API.** Rejected: neither `term::` nor `canvas::` has a
  closure-as-handler idiom; a poll model fits the existing draw-loop shape.
- **Dynamic/growable event queue.** Rejected: overwrite-on-full fixed ring gives
  bounded memory and "newest wins" backpressure for free (§4.2).
- **One shared `MouseEvent` in a third package** (or `canvas` referencing
  `term.MouseKind`). Rejected: `term::` is `Console`-gated, so a canvas program
  must never be made to `IMPORT term`. Two independent per-package type sets cost
  a little duplication and keep `IMPORT` at zero bytes — exactly the shape
  `term::didResize` and `canvas::didResize` already have. (§4.1.)
- **Growing `TERM_STATE_SLOTS` to hold the mouse slots.** Rejected on two counts:
  it shifts `presentation_mode_offset` for every program that uses both `term::`
  and `app::` (needless golden churn), and a canvas-only program that never
  imports `term` would get no mouse storage at all. (§4.4.)

## 4. Detailed Design

### 4.1 Surface (this sub-plan)

`term` members (bodies delegate to `gen_shared::lower_term_helper`, like every
other `term::` member):

```
term::enableMouse(enabled AS Boolean)   ' returns Nothing
term::pollMouse() AS MouseEvent
```

`term` types:

```
' value record (mod.rs add_record)
TYPE MouseEvent { kind AS MouseKind, button AS MouseButton,
                  row AS Integer, column AS Integer,
                  shift AS Boolean, ctrl AS Boolean, alt AS Boolean }

' enums (mod.rs add_enum; ordinal = declaration order)
ENUM MouseKind   { None, Down, Up, Move, Drag, ScrollUp, ScrollDown }
ENUM MouseButton { None, Left, Middle, Right }
```

`canvas` members and types, mirroring them in the canvas coordinate system:

```
canvas::enableMouse(enabled AS Boolean) ' returns Nothing
canvas::pollMouse() AS MouseEvent

TYPE MouseEvent { kind AS MouseKind, button AS MouseButton,
                  position AS Point,
                  shift AS Boolean, ctrl AS Boolean, alt AS Boolean }

ENUM MouseKind   { None, Down, Up, Move, Drag, ScrollUp, ScrollDown }
ENUM MouseButton { None, Left, Middle, Right }
```

`term`'s `row`/`column` are `Integer` 0-based cell coords, consistent with
`term::moveTo`/`terminalSize`. `canvas`'s `position` is the existing
`canvas::Point` (`x`/`y` `Float`, **top-left origin, Y increasing downward** —
`src/codegen/builtins/canvas/mod.rs:213`), consistent with every other canvas
coordinate. `shift`/`ctrl`/`alt` are `Boolean` in both.

**`MouseKind.None = 0` is load-bearing** in both packages: the zero record from an
uninitialized/empty poll must read as "no event", so `None` MUST be declared
first. Same for `MouseButton.None`.

A-only lowering (stubs):

- `<pkg>.enableMouse` → no-op returning `Ok(Nothing)` (ignore the arg).
- `<pkg>.pollMouse` → build a `MouseEvent` record with all fields zero
  (`kind = None = 0`, `button = None = 0`, coords 0, flags false) and return it.
  The `None`-kind zero record is the permanent "no event" sentinel; B replaces the
  body.

### 4.2 Event queue (design for B; frozen here so A's record layout matches)

Per-thread (per-arena) fixed-size ring in an arena block; pointer + head/tail in
the new mouse-state region (§4.4). Slot = `(kind, button, coordA, coordB,
modifier bits, u64 monotonic_stamp)`. Overwrite-on-full (advance tail when head
catches it). `pollMouse` returns the oldest entry with `now − stamp ≤ 100 ms`,
advancing tail past staler entries (prefix-skip; stamps monotonic in enqueue
order), else the `None` record. Ring size 64 (~1–2 KB).

**One ring, two readers.** `coordA`/`coordB` hold whatever the producer put
there; the *unit* is a property of the mouse mode (§4.4), not of the slot.
`term::pollMouse` reads them as `row`/`column` `Integer`; `canvas::pollMouse`
reads them as `Point.x`/`Point.y` `Float`. A program cannot have both surfaces
live at once — `term::` traps outside `Console` and `canvas::` outside `Canvas` —
so there is no ambiguity to resolve at poll time.

Monotonic clock: `emit_read_monotonic_nanos`
(`src/codegen/builtins/perf/perf.rs:202` is the one existing call site) and the
`datetime::monotonicNanos` OS-seam body
(`src/codegen/builtins/datetime/func_monotonic_nanos.rs::lower_monotonic_nanos`)
are the two candidates. B's first task picks one and records why; `net` has a
third, `emit_monotonic_nanos` (`src/codegen/builtins/net/gen_ping.rs:1220`), which
is the evidence that this primitive already travels outside its home package.

### 4.3 Unified decoder / byte injection (design for B/C-E)

See §3(2)(4). CLI: real tty bytes flow through the pump. App: each backend formats
the native event to surface coordinates and writes SGR bytes
(`\x1b[<b;x;yM` press / `m` release; enabled via `\x1b[?1000h\x1b[?1002h\x1b[?1006h`,
1006 SGR extended coords mandatory) into the worker input pipe it already uses for
keystrokes.

**SGR carries pixels as happily as cells.** The 1006 extended form encodes
coordinates as plain decimal integers with no 223-value ceiling — that ceiling is
exactly what 1006 exists to remove — so a 1920-pixel x fits with no new encoding
and no private escape hatch. Coordinates are 1-based on the wire; subtract 1 on
decode, in both units.

**The ring is worker-local — fed only by the pump, never written by the UI
thread** — so overwrite needs no atomics (the pipe is the cross-thread boundary).
Do not let anyone write the ring cross-thread (that is a lock-free SPSC-overwrite
hazard). This is the same rule `.ai/canvas-threading.md` §2 states for the scene:
arena state is per-thread, and the explicit shared channel is the design.

### 4.4 Storage: where mouse state lives

Two pieces of state, in two different places, for two different reasons.

**(a) The per-arena mouse region — the ring, its head/tail, and the parse state.**

The arena-state chain is a sequence of conditionally-reserved regions
(`src/codegen/engine/builder/mod.rs:1527`): module globals, then `term::` state
if `uses_term`, then the `app::` presentation-mode word if `uses_app`. Add a
**third region, `uses_mouse`**, appended *past* the presentation-mode word:

```
globals | term state (uses_term) | presentation mode (uses_app) | mouse state (uses_mouse)
```

`uses_mouse` is true when the module references either package's `enableMouse` or
`pollMouse` helper symbol (the `uses_term`/`uses_app` model — scan
`runtime_symbols`, as `uses_app` does). Appending keeps `term_state_offset` and
`presentation_mode_offset` byte-identical for every existing program, and gives a
canvas-only program the region without forcing it to carry `TERM_STATE_SLOTS`.

Slots: `MOUSE_STATE_RING_PTR`, `MOUSE_STATE_HEAD`, `MOUSE_STATE_TAIL`,
`MOUSE_STATE_PARSE_*` (the partial-sequence buffer and its length). B fixes the
exact list; A reserves the region and zeroes it.

**(b) The process-global mouse-mode word — read by UI-thread callbacks.**

An app backend's mouse handler runs on the **UI thread**, which does not have the
worker's arena state (`.ai/canvas-threading.md` §2). It therefore cannot read (a).
It needs to know two things: whether to emit at all, and in which unit. One
process-global word answers both:

```
_mfb_rt_mouse_mode :  0 = off   1 = cells (term)   2 = pixels (canvas)
```

Written by `term::enableMouse` / `canvas::enableMouse` on the worker, read by
every UI-thread handler on every backend. This is the same escape the canvas scene
takes — a writable process-global data symbol, like `CANVAS_SCENE_SYMBOL` /
`GRAPHICS_STATE_SYMBOL` (`src/codegen/runtime/canvas/mod.rs:80`) — and it is what
lets C, D and E each drop the "where does the flag live?" Open Decision the
pre-migration draft carried three copies of.

A word rather than a byte so a plain 8-byte load/store works on every backend with
no sub-word addressing; a single writer (the worker) and readers that tolerate a
one-event-stale answer, so no atomics.

### 4.5 Threads, modes, and who receives events (verified; frozen for B–E)

Stdin buffering today:

- **Console mode**: a global broadcast log `_mfb_rt_stdin_log` (zero-init
  non-arena data section, mutex/cursors + a 128-entry subscriber registry
  inlined); 8 KiB log byte blocks `malloc`/`free`'d, never per-arena; a per-thread
  4 KiB local copy buffer lazily arena-allocated, lock-free fast path
  (`src/codegen/io/stdin/stdin_broadcast.rs`,
  `src/codegen/error/constants/error_constants.rs`).
- **App mode**: **no broadcast log** — stdin is the window input pipe and reads are
  direct per-byte `read(0,…,1)` (`src/codegen/builtins/io/gen_read_family.rs:38-41`).

Both branches meet at **one function**: `emit_stdin_byte_read`
(`src/codegen/builtins/io/gen_read_family.rs:43`), which chooses the source by
`app_mode` and is called by every read helper. **That is the pump's placement**,
and it is a better answer than the pre-migration draft's "wrap
`_mfb_rt_stdin_next_byte`", which would have covered console only and left every
app backend undecoded. B §3a is written against this.

Threads opt into console stdin via `thread::openStdIn` (main auto-subscribes at
entry); an unsubscribed stdin read traps `ErrInvalidContext` naming
`thread::openStdIn`. The mouse filter rides the per-thread byte source, so **a
thread must `openStdIn` to get mouse events in console mode** — inherited for
free; the existing trap enforces it.

- **MUST document (B):** broadcast (per-subscriber) semantics — each
  subscribed+mouse-enabled thread independently decodes the same bytes and gets
  its own copy of the events (not consumed once).
- **MUST decide (B, explicit task):** whether `term::pollMouse`/`canvas::pollMouse`
  should carry a `prepend_wrong_mode_gate` (§2.3). A returns `None` from app
  dispatch so the stub is un-gated; once B–E implement real behavior,
  `term::pollMouse` in `Canvas` mode and `canvas::pollMouse` in `Console` mode are
  both nonsense and should trap like their siblings.

### 4.6 The two read-path hazards B inherits

Neither existed when this plan was first written; both are load-bearing.

**`io::pollInput` reports readiness the pump will consume.** `io::pollInput`
(`src/codegen/builtins/io/func_poll_input.rs`) answers TRUE when the broadcast log
has a byte staged for this thread, or when `poll(fd 0)` says fd 0 is readable —
*without consuming*. It cannot know the pending bytes are a mouse report the pump
will swallow, so with mouse on, `pollInput()` → TRUE followed by `readChar()` can
block. B must pick one and test it: teach `pollInput` to drain-and-recheck through
the pump, or document that `pollInput` means "bytes are ready" and not "a
character is ready" while mouse mode is on. The first is the honest answer; the
second is cheaper. **Recommend draining through the pump** — a `pollInput` that
lies is a worse defect than the one it replaces.

**`io::input`/`io::readLine` temporarily restore cooked mode.** They bracket their
read with a cooked-mode restore and re-apply the raw termios afterwards
(`TERM_STATE_COOKED_TERMIOS_OFFSET` / `TERM_STATE_RAW_TERMIOS_OFFSET`,
`src/codegen/error/constants/error_constants.rs:322,325`). During that window the
tty line-buffers and **echoes**, so mouse reports arriving mid-`readLine` would be
echoed onto the screen as visible garbage and delivered a line at a time. B must
either withdraw mouse tracking around the cooked window (emit the 1000/1002/1006
disable before the restore and re-enable after) or document the interaction.
**Recommend withdrawing** — it is a few bytes, it is symmetric with the restore
already there, and an echoed `\x1b[<0;40;12M` on a user's screen is indefensible.

## Compatibility / Format Impact

- **Additive only.** Four new call names, six new types, one new conditionally
  reserved arena region (appended — no existing offset moves), one new
  process-global word, one new arena block per mouse-enabled thread (B). No
  existing signature, record layout, term-state offset, or ANSI output changes.
- `term::on`/`off` output is unchanged until mouse is enabled (opt-in).
- A program that never mentions mouse emits the same bytes it does today; that is
  the one byte-identity claim this feature does make.

## Phases

### Phase 1 — `term::` surface + seams + no-op stubs

- [x] `src/codegen/builtins/term/func_enable_mouse.rs` and `func_poll_mouse.rs`:
      prose (`INTRO`/`DESC`/`EX`), `lower_*` bodies delegating to
      `gen_shared::lower_term_helper`, and `register(pkg)`. Add `mod` lines +
      `::register(&mut pkg)` calls in `src/codegen/builtins/term/mod.rs`.
      — both files created; `grep -c '^    func_.*::register(&mut pkg);'
      src/codegen/builtins/term/mod.rs` → 26.
- [x] `src/codegen/builtins/term/mod.rs`: `add_record` for `MouseEvent`,
      `add_enum` for `MouseKind` and `MouseButton` (`None` declared **first** in
      both). Extend the package `DESC` to mention the new surface — including the
      sentence at `src/codegen/builtins/term/mod.rs:204` ("The package defines one
      built-in record type and two enums"), which the new types falsify.
      — done; that sentence now reads "two built-in record types and four enums".
      `mfb man term types` renders `MouseEvent` with all seven props and both
      enums with `None` listed first.
- [x] Shared arm in `src/codegen/term/core/term.rs` `lower_term_helper`:
      `"term.enableMouse"` no-op; `"term.pollMouse"` building the zeroed
      `MouseEvent`. — `emit_enable_mouse` (tag-only) and `emit_poll_mouse`
      (56-byte `arena_alloc`, seven zero stores, `ErrOutOfMemory` on failure).
- [x] Per-target supported lists: add `"term.enableMouse"`, `"term.pollMouse"` to
      `macos_aarch64/mod.rs`, `linux_common/mod.rs`, `win_x86_64/mod.rs`.
      — `grep -n '"term.enableMouse"\|"term.pollMouse"' src/target/*/mod.rs` → 6
      hits, 2 per file.
- [x] Update the "24 members" counts in `term/mod.rs:11`, `term/gen_shared.rs:11`,
      `src/target/shared/runtime/catalog.rs:27` → 26. — all three now say 26.
- [x] Verify the rendered docs: `mfb man term pollMouse`, `mfb man term
      enableMouse`, `mfb man term types`; run the examples
      (`scripts/man-run-examples.sh term --run`).
      — all three pages render. `MFB=./target/debug/mfb
      scripts/man-run-examples.sh term --run` → `examples: 47 built: 47 ran: 37
      not run: 0 failed: 10`; both `term::pollMouse` examples and both
      `term::enableMouse` examples ran, and all 10 failures are the pre-existing
      no-controlling-terminal entries already listed in
      `scripts/man-examples-not-run.txt` (10 `term::` lines there). Census:
      `MFB=./target/debug/mfb scripts/man-census.sh --memory-scope term` → **0**
      unclassified hits (see Corrections C1).

Acceptance: `cargo build` clean on all five targets; a program calling
`term::enableMouse(TRUE)` + `term::pollMouse()` builds and runs; `cargo test --bin
mfb` passes.
**Met.** `cargo build` → `Finished` (exit 0). A `term::enableMouse(TRUE)` +
`term::pollMouse()` project builds for `macos-aarch64` and cross-builds for
`linux-{aarch64,riscv64,x86_64}` (glibc + musl each) and `windows-x86_64` — all
five targets, `Wrote executable to …` each time. Run on the host it prints
`kind=None / button=None / row=0 column=0 / shift=FALSE ctrl=FALSE alt=FALSE`,
exit 0, and `od -c` finds **no** `\033` in its output (the "no new ANSI bytes"
non-goal, measured). `cargo test --bin mfb` → see Phase 1 commit note.
Commit: —

### Phase 2 — The two storage regions

- [x] `src/codegen/error/constants/error_constants.rs`: the `MOUSE_STATE_*`
      offsets and `MOUSE_STATE_SLOTS`, plus `MOUSE_MODE_SYMBOL`
      (`_mfb_rt_mouse_mode`) and its three documented values (§4.4).
      — `MOUSE_STATE_{RING_PTR,HEAD,TAIL,PARSE_LEN,PARSE_BUF}_OFFSET` +
      `PARSE_BUF_BYTES` (32, sized for the 21-byte worst-case SGR report) →
      `MOUSE_STATE_SLOTS = 8`; `MOUSE_MODE_{OFF,CELLS,PIXELS} = 0/1/2`.
- [x] `src/codegen/engine/builder/mod.rs` (~`:1527`): `uses_mouse` scan and the
      third region, appended past `presentation_mode_offset`; thread it onto
      `AbiCtx` as `mouse_state_offset: Option<usize>` alongside
      `term_state_offset`. — done, plus the matching `ArenaLayout` field and the
      four `AbiCtx` construction sites. `uses_mouse` keys on the member symbol
      suffix, not the package prefix (see Corrections C2).
- [x] Emit `_mfb_rt_mouse_mode` as a zero-init writable global whenever
      `uses_mouse`. — confirmed in the `.ncode` dump:
      `{"symbol": "_mfb_rt_mouse_mode", … "value": "0000000000000000"}`, and
      absent from a non-mouse program's `dataObjects`.
- [x] Tests: an entry-frame test asserting a non-mouse program's arena layout is
      unchanged, and a mouse program's region lands past the presentation-mode
      word. — `tests/codegen/codegen_mouse_arena_region.rs`, two cases,
      `cargo test --test codegen_mouse_arena_region` → `2 passed; 0 failed`.
      It asserts the **prefix** property (every pre-existing slot at its exact
      old offset), not merely a size delta, which is the claim that actually
      matters.

Acceptance (**corrected — see Corrections C3; strengthened, not weakened**):
`scripts/artifact-gate.sh <exe> all` shows **no `.ncode` diff at all**, and the
only `.ir`/`.nir` diffs are pure ADDITIONS of the three new type declarations
plus `"line": N` renumbering of the injected `<builtin-term>` source — nothing
removed, no emitted instruction changed.
**Met.** `./scripts/artifact-gate.sh ./target/release/mfb all` → `1466 tests,
1637 build(s), 2058 golden(s) checked, 20 diff(s)`; all 20 are `.ir`/`.nir` under
`term`/`app`, **zero** `.ncode`. A tree-wide scan of every regenerated
`.ir`/`.nir` against its golden found **0** removed lines other than `"line": N`
renumbering. Measured directly: the entry frame grows by exactly 64 bytes
(`sub_sp` 4000 → 4064 = `MOUSE_STATE_SLOTS * 8`) and the non-mouse program's 28
arena zero-store offsets are a strict prefix of the mouse program's 36, the new
8 appended at 4000…4056.
Commit: —

### Phase 3 — `canvas::` surface + seams + no-op stubs

- [x] `src/codegen/builtins/canvas/func_enable_mouse.rs` and `func_poll_mouse.rs`
      + the `mod`/`register` lines in `src/codegen/builtins/canvas/mod.rs`.
      — both carry `prepend_wrong_mode_gate(ModeRequirement::Canvas)`, like every
      other surface-touching canvas member (see Corrections C4 for why the canvas
      gate is settled here while the `term::` one is still B's call).
- [x] `src/codegen/builtins/canvas/mod.rs`: `add_record` for `MouseEvent`
      (`position AS Point`), `add_enum` for `MouseKind`/`MouseButton`.
      — `position AS Point` makes it an **inlined** prop, which the A stub has to
      build by hand; see Corrections C5.
- [x] Per-target supported lists: add `"canvas.enableMouse"`,
      `"canvas.pollMouse"`. — `grep -n '"canvas.enableMouse"\|"canvas.pollMouse"'
      src/target/*/mod.rs` → 6 hits, 2 per file.
- [x] Verify rendered docs: `mfb man canvas pollMouse`, `mfb man canvas types`.
      — both render; the `types` page shows all six props with `position` typed
      `Point`. `MFB=./target/debug/mfb scripts/man-census.sh --memory-scope
      canvas` → **0** unclassified hits.

Acceptance: an `--app` program calling `canvas::enableMouse(TRUE)` +
`canvas::pollMouse()` builds on macOS/GTK/Windows and returns `kind = None`.
**Met.** The fixture builds `-app` for `macos-aarch64` (`.app`), `linux-x86_64`
(glibc + musl `.AppImage`) and `windows-x86_64` (`.exe`). Run on macOS it prints
`kind=None / button=None / pos=0.00,0.00 / mods=FALSEFALSEFALSE` — and that
`pos=0.00,0.00` is the load-bearing line: reading `event.position.x` through the
hand-built inline-offset layout is what proves the layout right (a wrong offset
word reads garbage or faults, it does not print zeros).
Commit: —

### Phase 4 — Tests, spec, goldens

- [ ] `tests/runtime/rt_native_term_runtime.rs`: a
      `native_term_poll_mouse_is_none_stub` case — build+run (piped and pty),
      assert `pollMouse().kind` prints `None` and `enableMouse(TRUE)` emits no
      escape bytes.
- [ ] Spec: `src/docs/spec/language/18_builtin-functions.md` lines for the four
      members; `src/docs/spec/app/04_term-backend.md` a note that the two term
      calls exist as stubs; `src/docs/spec/memory/08_program-startup.md` the new
      arena region in the chain. Cite with `[[path:Symbol]]` per
      `.ai/specifications.md`.
- [ ] Regenerate goldens: `scripts/artifact-gate.sh <exe> term` and the canvas
      fixtures ×5, plus any `.app.ncode`/`.ncodesum` that shift; confirm each diff
      is only the additive surface.

Acceptance: `cargo test --bin mfb`; `scripts/test-accept.sh <exe> /tmp/out
'*term*'`; `scripts/artifact-gate.sh <exe> term`;
`scripts/man-examples-gate.sh`.
Commit: —

## Validation Plan

- Tests: the rt stub case above; registry/descriptor tests extended for the new
  names and types.
- Coverage check: the rt case actually calls all four new members (in the suite
  denominator), not just references them.
- Runtime proof: `mfb build` + run a 5-line program printing `pollMouse().kind`
  → `None`, under a pty, with no `\x1b` in output.
- Doc sync: rendered man pages (`mfb man term pollMouse`, `mfb man canvas
  pollMouse`, `mfb man <pkg> types`) + spec §18 + the term-backend and
  program-startup spec topics. `.ai/man-content.md` bans C/Rust memory vocabulary
  on a man page — check with `scripts/man-census.sh --memory-scope` (must report
  0 unclassified hits).
- Acceptance: `cargo test --bin mfb`; `scripts/test-accept.sh <exe> /tmp/out
  '*term*'`; `scripts/artifact-gate.sh <exe> term`.

## Open Decisions

- **`enableMouse` return** — `Nothing` (recommended, matches every `term::`
  setter) vs. a `Result` reporting "terminal does not support mouse". Recommend
  `Nothing`; best-effort, like every other `term::` setter.
- **Whether `canvas::MouseEvent.position` is a `Point` or two bare `Float`s.**
  Recommend `Point` — every other canvas coordinate is one, and it makes
  `event.position` hand straight to a hit test.

*(Two Open Decisions the pre-migration draft carried are now closed by evidence:
the enum-typed record field is proven to work (§2 Verified properties), and the
UI-thread flag storage is settled by the process-global mode word (§4.4).)*

## Corrections

**C1 — "pointer" is a banned man-page word, so the mouse prose says "mouse".**
The Validation Plan requires `scripts/man-census.sh --memory-scope` to report 0
unclassified hits. The first draft of the four new pages used *pointer* in its
ordinary GUI sense ("report pointer activity", "the row the pointer was over")
and the census reported **19** unclassified hits, every one of them mine and
every one of them the word `pointer` — which is on the canonical banned memory
vocabulary (`BANNED_CORE`, `scripts/man-census.sh:61`), alongside `heap`,
`allocate` and `ownership`. The instrument is not wrong to flag it: it matches
whole words and has no way to tell a mouse pointer from a memory one.

Fixed by rewording the prose to say *mouse* rather than *pointer* everywhere
(`grep -rn 'pointer' src/codegen/builtins/term/` → 0 hits).
**Deliberately NOT fixed by adding a fifth carve-out to the census**: the script
is the shared instrument every package is measured by, and widening it for one
package's convenience is exactly the "weaken the check to make the phase pass"
move. Re-measured: `MFB=./target/debug/mfb scripts/man-census.sh --memory-scope
term` → `unclassified memory-vocabulary hits: 0`.

This is a standing constraint for B–E, which all add mouse prose: **say "mouse",
never "pointer"** on any rendered page.

**C2 — `uses_mouse` keys on the member-symbol SUFFIX, not a package prefix.**
§4.4a says to scan `runtime_symbols` "as `uses_app` does". `uses_app` matches the
prefix `_mfb_rt_app_`, and copying that shape literally would have been wrong
here: the mouse members live inside `term` and `canvas`, whose prefixes
(`_mfb_rt_term_`, `_mfb_rt_canvas_`) are already true for any program that draws
a box or presents a scene. Every `term::` program would have paid for a
mouse region it never touches — and the plan's whole argument for appending the
region is that programs which do not use mouse are unaffected.

Implemented as `symbol.ends_with("_enableMouse") || symbol.ends_with("_pollMouse")`,
measured against the real spellings (`mfb build --mir` on a mouse fixture →
`_mfb_rt_term_term_enableMouse`, `_mfb_rt_term_term_pollMouse`; canvas mints
`_mfb_rt_canvas_canvas_*` the same way). The suffix form covers both packages
without a four-symbol list that would silently rot if a member were renamed.

**C3 — Phase 2's byte-identity acceptance was miscalibrated, and is now
stronger.** As written it demanded "**no** diff for fixtures that never mention
mouse". Measured, `artifact-gate.sh … all` reports 20 diffs, all in `term`/`app`
fixtures that never mention mouse. This is **not** the premise failing: §3 of
this very plan already predicts it — "A adds the stub bodies … a diff there is
the plan working". The cause is Phase **1**, not Phase 2: registering three new
types renders them into the injected `<builtin-term>` source, which appears in
every `term::` program's `.ir`/`.nir` type table and renumbers the lines after
it. No arena offset and no instruction is involved.

Root-caused one fixture rather than theorising
(`tests/byte-identity/term/term_codegen_cover_rt.ir`): the entire diff is the
three type declarations added, plus two `"line": N` values shifting. So the
criterion was measuring the wrong thing — it could never have passed once Phase 1
landed, whatever Phase 2 did.

Replaced with a criterion that is **narrower and harder to pass**, and that
actually tests what Phase 2 changes:

> no `.ncode` diff **at all**, and every `.ir`/`.nir` diff is a pure addition
> plus `"line": N` renumbering — nothing removed, no emitted instruction changed.

Both halves measured: 0 of the 20 diffs are `.ncode`, and a tree-wide scan of
every regenerated `.ir`/`.nir` against its golden found 0 removed lines other
than `"line": N`. The goldens were regenerated only after that scan and after the
full suite, per `AGENTS.md`.

**C4 — the canvas mouse members are `Mode.Canvas`-gated from the start; the
`term::` gate stays B's decision.** §4.5 lists "whether `pollMouse` should carry a
`prepend_wrong_mode_gate`" as a **B** decision, and for `term::` it still is — A
adds nothing to any app dispatch, so the term stubs fall through to the shared
console backend un-gated, exactly as §2.3 describes. But that reasoning does not
transfer to `canvas::`, and treating the decision as one question for both
packages would have been a mistake: a canvas member is not reached through
`emit_app_term_helper` at all. It writes its own body and calls
`prepend_wrong_mode_gate(ModeRequirement::Canvas)` itself — `canvas::didResize`
is the precedent, and every surface-touching canvas member does it. An ungated
`canvas::pollMouse` would answer "where is the mouse on the surface" in a mode
that has no surface. So the gate is in from A, and B inherits one open question
(term's) rather than two.

**C5 — `canvas::MouseEvent.position AS Point` is an INLINED prop, and the A stub
builds that layout by hand.** The plan's Open Decision recommends `Point` over two
bare `Float`s and that recommendation is kept — but it has a cost §4.1 does not
mention. A record-typed prop is *inlined* (`record_field_is_inlined`): its 8-byte
slot holds the block-relative byte offset of a sub-block that follows the fixed
slots, **not** a pointer, and `0` in that slot is the "sub-block absent" sentinel.
A stub that simply zeroed the whole block would therefore have produced a
`MouseEvent` whose `position` reads as absent.

Layout measured against what the compiler emits for a hand-written record of the
identical shape (6 props, a nested 2-`Float` record at index 2): fixed region
`8 * 6 = 48`, `position`'s slot at `+16` holds `48`, the `Point` sub-block is
memcpy'd to `base + 48`, total block 64 bytes. `func_poll_mouse.rs` reproduces
exactly that: zero all 64 bytes, then overwrite the `position` slot with 48.

Verified by running, not by reading: the macOS canvas fixture prints
`pos=0.00,0.00`, which requires the offset word to be right — a wrong one reads
garbage or faults rather than printing zeros. **B–E inherit this**: anything that
writes a real position into a `canvas::MouseEvent` writes it at `base + 48`, not
into the `position` slot.

## Summary

A is pure surface + seams + storage with inert bodies — low risk, fully verifiable
on every target, and it freezes the API and record layout so B–E implement behind
a fixed contract. It now covers both surfaces, which is what forced the two design
changes that most improve the feature: one mouse-mode word reachable from every
UI-thread callback, and an independently reserved arena region that neither
shifts an existing offset nor makes a canvas program carry `term::` state. The
engineering risk is entirely downstream: the decoder/queue (B) and the
per-backend injectors (C–E).
