# plan-13: `app::` built-in GUI package

Status: **design**. Target: a cross-platform **native-widget** GUI package, `app::`,
usable from `mfb build -app` programs. Builds directly on the existing `-app` mode
(plan-04 macOS / plan-05 Linux): the native event loop owns the main thread and the
program's entry runs on a worker.

## 1. Goals & non-goals

- **Native widgets**, not self-drawn: `NSView`/`NSStackView`/`NSButton`/`NSTextField`
  on macOS, `GtkBox`/`GtkButton`/`GtkLabel` on Linux. "App, not a game."
- **No new external dependency** — system toolkits only (AppKit, GTK4), same as the
  existing transcript `-app` mode.
- **Functional surface**: opaque resource handles + functions, matching the idiom of
  `fs::`, `net::`, `term::`. No callbacks (the language forbids escaping `MUT`-capturing
  closures), no exposed mutable data records.
- **Retained tree, polled events** — *not* immediate mode. The widget tree persists;
  the program reads event state each frame. No reconciler/keyed-diff: structure changes
  through explicit `add*` / `remove` / `attach`, properties through a per-resource dirty
  flag.
- **GUI and transcript are mutually exclusive.** A program that opens a real window
  (`app::window`) suppresses the fallback transcript and leaves `io::`/`term::` on real
  stdio; a program that never opens one keeps today's transcript behavior (§7, Mode
  selection).

Non-goals (v1): text input fields, menus, native dialogs, images, scrolling containers,
animation/timers, theming. The host-protocol seam (§8) is designed so these slot in later.

## 2. The core model (read this first)

Two concerns are **deliberately separated** — this is what makes the resource model sound:

- **Lifetime ownership is flat and per-`RES`-binding.** Every widget is destroyed
  **exactly once**, when *its own* `RES` binding drops (or via its registered close op).
  A parent never destroys a child.
- **Layout parentage is a separate, mutable tree.** `addContainer`/`addButton`/`addLabel`
  attach a *new* widget to a parent for layout; `remove` detaches; `attach` re-attaches an
  existing widget. None of these affect lifetime.

Consequences (all intentional):

- `app::remove(c, i)` **detaches**, it does not destroy. The widget and its descendants
  stay valid until their own scope drop and may be re-attached elsewhere.
- `app::close(win)` destroys the **window** and detaches every live descendant, leaving
  them as valid **orphan** widgets (re-attachable to a new window) that die at their own
  scope drop.
- Because nothing cascades destruction, "closed exactly once" holds for every handle, and
  the compiler's resource analysis stays correct — there is no path where one binding's
  resource is freed by a call that names a different binding.

**Opaque state vs. user `STATE`.** Any "state" in this document is the package's *private*
opaque resource state (dirty flags, the click counter, the property shadow). It is **not**
the language-level MFB Resource `STATE`, which the user may still attach to any of these
handles and use freely for their own data, e.g. `RES ok AS app::Button STATE RowRef`.

## 3. Resources

```
app::Window        ' RES — the lifetime anchor; its drop/close tears down the native window
app::Container     ' RES — a layout box (flex)
app::Button        ' RES
app::Label         ' RES

UNION Widget       ' app::Widget — a child widget of any kind (NOT Window, which is the root)
    app::Container
    app::Button
    app::Label
END UNION
```

`app::Widget` is a **resource union** (all variants are resources). Per the language rules
a resource union carries no `STATE` and matching borrows the active variant — fine, because
`Widget` is only used to pass an existing child generically to `slot`/`attach` (a borrow).

## 4. Types & enums

```
TYPE Size
    width  AS Integer    ' < 0 means "fill available width"
    height AS Integer    ' < 0 means "fill available height"
END TYPE

TYPE Spacing
    top    AS Integer    ' < 0 clamps to 0
    bottom AS Integer    ' < 0 clamps to 0
    left   AS Integer    ' < 0 clamps to 0
    right  AS Integer    ' < 0 clamps to 0
END TYPE

app::Direction      Row, Column
app::Justification  Start, End, Center, Between, Around, Even   ' main-axis distribution
app::Align          Start, End, Center, Stretch                 ' cross-axis alignment
```

## 5. Function surface

Construction / structure functions run **entirely on the worker** — they only mutate the
MFB-side shadow tree (cheap, no thread hop). Native objects are realized lazily on the next
`app::sync` (§7). Defaults use record-literal syntax (square brackets).

### Layout / lifetime

```
app::window(size AS Size = Size[width := 800, height := 600],
            resizable AS Boolean = TRUE,
            title AS String = "MFB Application") AS RES app::Window

' Destroys the window. Live descendants detach and remain valid until their own scope
' drop (re-attachable to a new window). This IS app::Window's registered close op, so an
' explicit app::close and the binding's scope-drop never double-fire.
app::close(win AS RES app::Window) AS Nothing

app::addContainer(parent AS RES app::Window,
                  dir AS app::Direction = app::Direction.Row,
                  justify AS app::Justification = app::Justification.Between,
                  align AS app::Align = app::Align.Center,
                  padding AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Container

app::addContainer(parent AS RES app::Container,
                  dir AS app::Direction = app::Direction.Row,
                  justify AS app::Justification = app::Justification.Between,
                  align AS app::Align = app::Align.Center,
                  padding AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Container

app::addButton(parent AS RES app::Container,
               label AS String = "",
               margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Button

app::addLabel(parent AS RES app::Container,
              label AS String = "",
              margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Label

' Current zero-based slot of `widget` in `container`, or -1 if not a child.
app::slot(container AS RES app::Container, widget AS RES app::Widget) AS Integer

' Detach the child in `index` (current child-list position; positions compact after a
' remove). Detached widget + descendants stay valid until their own scope drop and keep
' their own subtree; they can be re-attached elsewhere.
app::remove(container AS RES app::Container, index AS Integer) AS Nothing

' Re-attach an existing (detached) widget. index = -1 appends; otherwise inserts at slot.
app::attach(container AS RES app::Container, widget AS RES app::Widget, index AS Integer = -1) AS Nothing

' Re-attach a detached container as a (new) window's root child.
app::attach(win AS RES app::Window, container AS RES app::Container) AS Nothing
```

### Frame pump

```
' Reconcile the shadow tree to the native tree and pump one frame of events.
' NON-BLOCKING: posts pending shadow changes to the main thread and drains buffered
' events, then returns immediately. sync never blocks (§7).
app::sync(win AS RES app::Window) AS Nothing

' The loop's wait primitive — the ONLY app:: function that blocks (and it blocks by
' default). Returns TRUE when a native event (click, resize, close) is available for the
' next sync to drain, FALSE on timeout.
'   timeout = 0  : (default) block until the next event — fully event-driven
'   timeout > 0  : wait up to `timeout` ms, returning early as soon as an event arrives
' Parks the WORKER only; the main thread / window stay live throughout (§7). poll waits;
' it does not drain — sync drains.
app::poll(timeout AS Integer = 0) AS Boolean

' FALSE once the user has closed the window (a native->shadow event, like clicked).
app::isOpen(win AS RES app::Window) AS Boolean
```

### Window properties

```
app::show(win AS RES app::Window) AS Nothing
app::hide(win AS RES app::Window) AS Nothing
app::visible(win AS RES app::Window) AS Boolean
app::title(win AS RES app::Window, title AS String) AS Nothing
app::resizable(win AS RES app::Window, resizable AS Boolean) AS Nothing
app::size(win AS RES app::Window, size AS Size) AS Nothing
```

### Button

```
app::show(button AS RES app::Button) AS Nothing
app::hide(button AS RES app::Button) AS Nothing
app::visible(button AS RES app::Button) AS Boolean
app::setLabel(button AS RES app::Button, label AS String) AS Nothing
app::clicked(button AS RES app::Button) AS Boolean      ' edge event, frame-latched (§7)
```

### Label

```
app::show(label AS RES app::Label) AS Nothing
app::hide(label AS RES app::Label) AS Nothing
app::visible(label AS RES app::Label) AS Boolean
app::setLabel(label AS RES app::Label, text AS String) AS Nothing
```

`show`/`hide`/`visible` overload on handle type (the stdlib already overloads `thread::send`
this way). **Visibility is CSS `display:none`**: a hidden widget occupies no space and its
siblings reflow.

## 6. Canonical program

```
RES win  = app::window(title := "Demo")
RES root = app::addContainer(win, dir := app::Direction.Column)   ' RES: a container is a resource
RES ok   = app::addButton(root, label := "OK")
RES out  = app::addLabel(root, label := "ready")

WHILE app::isOpen(win)
    app::poll()               ' block until the next event (use app::poll(16) for a ~60fps tick instead)
    app::sync(win)            ' non-blocking: drain events -> shadow, flush dirty props -> native
    IF app::clicked(ok) THEN app::setLabel(out, "clicked!")   ' clicked reads the shadow drained by sync
END WHILE
' scope exit drops in reverse declaration order: out, ok, root freed first;
' win drops last -> its registered close op tears down the (now-empty) native window
```

## 7. Architecture: shadow tree + sync

**Threading.** Same split as the transcript `-app` mode: the native loop owns the main
thread (`[NSApp run]` / `g_application_run`); the program runs on a worker. **`app::sync`
is the one and only main↔worker sync point.** If `sync` is never called, no window is
created/shown — only the native loop runs.

**Mode selection — GUI vs transcript (static, whole-program).** Whether a program calls
`app::window` is detected at **build time** (the same mechanism as the existing `uses_term`
flag), selecting between two sub-modes of `-app`:

- **GUI sub-mode** — the program calls `app::window` somewhere. The bootstrap brings up
  `NSApplication`/`GtkApplication` and the native run loop **but no transcript window**, and
  `io::*` *and* `term::` use their **console** lowering (real stdout/stderr fds, ANSI for
  `term::`) — exactly as a non-`-app` console build. All UI comes from `app::`. `io::print`
  is then ordinary stdio (visible from a terminal / pipeable; routed to the system log when
  double-launched), not a visible transcript.
- **Transcript sub-mode** — the program never calls `app::window`. Today's behavior: the
  bootstrap creates the transcript window and `io::*` / `term::` redirect into it
  (plan-04/05).

It must be static, not runtime: the transcript window is created eagerly in the bootstrap
*before* the worker runs user code, so a runtime "was `app::window` called?" check could
neither hide an already-shown transcript nor decide where an early `io::print` should go.
Compile-time detection makes it an unambiguous whole-program decision with no ordering
hazard. (Referencing `app::` in a non-`-app` console build is a compile error — `app::`
requires `-app`.)

**Each `app::` resource is a shadow/proxy of a native widget.** The single governing rule:

> All getters read the **shadow** (never native — the worker can't touch native
> synchronously). `app::sync` performs the **bidirectional reconcile**: push dirty
> properties shadow→native, pull events native→shadow.

This makes everything coherent:

- **Construction** (`window`/`add*`) only allocates shadow nodes on the worker. Native
  objects are created lazily during the first `sync` that sees them (everything starts
  dirty). Hence "no `sync` ⇒ no window."
- **Property writes** (`setLabel`, `title`, `size`, `show`/`hide`, …) mutate the shadow and
  set a **dirty flag** on that node. `sync` flushes dirty nodes to native and clears the
  flags. (Per-node single dirty flag is fine for v1 — re-push that node's props.)
- **Property reads** (`visible`, …) read the shadow — correct even before the first `sync`
  (they return the intended value), and never block.
- **Structure** (`add*`/`remove`/`attach`) edits the shadow tree's parent/child links and
  marks structure dirty; `sync` applies inserts/detaches to the native tree.

**Events (pull direction), e.g. `clicked`:**

- The native side (main thread) **buffers** clicks since the last `sync` as a **counter**
  (not a bool — rapid double-clicks aren't lost).
- `app::sync` **drains** the counter into the worker-visible shadow and resets the native
  counter — atomically, because `sync` is already the one main↔worker barrier.
- `app::clicked(btn)` reads worker-local shadow state: stable for the whole frame, no
  cross-thread access at read time, no wall-clock timing. (v1 may collapse the counter to
  "clicked since last sync"; keep it a counter internally to allow richer event reads later.)

**Window close** is the same mechanism: the native close handler sets a shadow flag drained
at `sync`; `app::isOpen` reads it.

**`sync`, mutators, and getters never block; `app::poll` is the one explicit wait.**
`app::sync` syncs through shared memory without waiting on the main thread: it drains the
event counters with an atomic read+reset and **posts** dirty property/structure changes to
the main thread (async — an enqueued apply the native loop picks up on its next iteration),
then returns. Property/event reads are plain shadow reads. So there is up to one frame of
latency between a `sync` and the pixels changing — fine for a retained UI.

Waiting is **`app::poll(timeout)`**, the only blocking call (and it blocks by default).
`poll()` parks the **worker** until the main thread signals the next queued event;
`poll(t>0)` waits up to `t` ms with early wakeup. It parks the worker only — the main
thread's native loop keeps the window responsive throughout. Note `poll` only *waits*;
`sync` is what drains events into the shadow, so there is no need for a zero-wait `poll` —
if you don't want to wait, don't call it (a bare `sync`-only loop drains fine but
busy-spins, so cap it with `poll(t)` or go event-driven with `poll()`).

**`close` orphan handling.** On `app::close(win)`, the backend must **retain + unparent**
every live descendant native widget (reparent to an offscreen holder) *before* destroying
the native window, so detached widgets survive per §2. Each orphan is freed when its own
binding drops.

## 8. Host-protocol seam (per-platform)

Everything above the seam is shared, platform-independent code (the `app::` surface, the
shadow tree, dirty tracking, the sync driver). Everything below is
`src/target/<platform>/widgets` written against the existing `Asm`/`abi` builder (so it is
CPU-neutral — a future `arch/x64` reuses it). Keep the seam **small and stable** — its size
is the cost of every future platform.

```
host_create_window(size, title, resizable) -> handle
host_destroy_window(handle)                              ' detaches live children first
host_window_set_title / set_size / set_resizable / set_visible(handle, …)
host_window_is_open(handle) -> bool                      ' drained at sync

host_create_stack(dir, justify, align, padding) -> handle    ' Container
host_create_button(label, margin) -> handle
host_create_label(text, margin) -> handle
host_set_text(handle, s)                                 ' button + label
host_set_visible(handle, bool)                           ' display:none semantics

host_insert_child(parent, child, index)                  ' add / attach
host_detach_child(parent, child)                         ' remove (no destroy)
host_destroy(handle)                                     ' fired by RES drop / close op

host_button_take_clicks(handle) -> int                   ' atomic drain+reset counter (at sync)
host_present(window)                                     ' realize/apply dirty; returns immediately (never blocks)
host_wait_events(window, timeout) -> bool                ' app::poll: worker-side wait (timeout 0 = forever), signaled by main thread
```

**Layout delegates to native stacks — no flex engine.** `Direction`/`Justification`/`Align`
map onto native stack containers:

- **macOS `NSStackView`**: `orientation`=Direction, `distribution`≈Justification
  (`fillEqually`/`equalSpacing`/`equalCentering`…), `alignment`=Align. Clean fit.
- **Linux `GtkBox`** (GTK4): orientation + child `halign`/`valign` cover Direction + Align
  directly. The richer justifications (`Between`/`Around`/`Even`) have no built-in `GtkBox`
  equivalent — emulate with child `hexpand`/`vexpand` or spacer widgets (or `GtkCenterBox`
  for `Center`). Budget a little manual work here; the common cases are free.

## 9. Resolved: non-blocking sync + `app::poll` for pacing

**Decision locked.** `app::sync`, all mutators, and all getters are non-blocking (shared
memory + async post; shadow reads). The single blocking function is **`app::poll(timeout)`**,
which blocks **by default**: `poll()` / `timeout = 0` blocks until the next event
(event-driven); `poll(t>0)` waits up to `t` ms with early wakeup (frame tick). It parks the
worker on a wait the main thread signals. The canonical loop (§6) is `poll` then `sync`; no
external `sleep` primitive is required.

Minor semantics to confirm: behavior of `app::poll(timeout < 0)`. Since `0` now means
wait-forever, a negative value is free to mean a true **zero-wait check** (drain-nothing,
return whether an event is pending right now) if that's ever wanted — otherwise reject
negatives. Recommend reserving `-1` = non-blocking check, others `< 0` invalid.

## 10. Language-support checkpoints (verify before/while building)

- **Builtin overloading on handle type** for `show`/`hide`/`visible`/`addContainer`/`attach`
  — used already by `thread::send`; confirm the `app::` package can declare these.
- **Resource-union borrow widening**: passing `RES ok AS app::Button` where
  `RES app::Widget` is expected (a borrow, for `slot`/`attach`). Confirm the typechecker
  accepts variant→union widening for a borrowed resource param; if not, fall back to
  per-type `attach`/`slot` overloads.
- **`app::close` as the registered close op** for `app::Window` (like `fs::close` for
  `File`) so explicit close + scope-drop are a single close.
- Builtin record type IDs for `Size`/`Spacing`/`Widget` must use the high reserved range
  (see the `term::` `TermColor`/`TermSize` precedent — `FIRST_TABLE_TYPE_ID` collision).
- **Static `app::window` detection** reuses the `uses_term` whole-program flag pattern to
  select GUI vs transcript sub-mode at build time (§7).
- **Pacing** (§9): handled in-package by `app::poll(timeout)` (worker parks on a wait the
  main thread signals on event-queue) — no external `sleep` needed. Confirm the worker-side
  timed-wait primitive (`host_wait_events`) for each backend.

## 11. Phased implementation plan

1. **Package skeleton & types.** Register the `app::` builtin package, resources
   (`Window`/`Container`/`Button`/`Label`), `Widget` union, `Size`/`Spacing` records, and
   the three enums. Reserve type IDs in the high range. No native backend yet; functions
   stub the shadow tree.
2. **Shadow tree + dirty model.** Worker-side tree: `window`/`add*`/`remove`/`attach`/`slot`
   build and mutate shadow nodes with parent/child links, dirty + structure-dirty flags,
   and per-node property shadows. Pure data structure; unit-testable without a window.
3. **Mode selection + host-protocol trait + macOS backend.** Add static detection of an
   `app::window` call (the `uses_term` mechanism) to pick GUI vs transcript sub-mode (§7):
   in GUI sub-mode skip the transcript window and keep `io::`/`term::` on console lowering.
   Define the §8 seam; implement on macOS with `NSWindow` +
   `NSStackView`/`NSButton`/`NSTextField(label)`. `app::sync` realizes dirty nodes and posts
   prop/structure changes; `host_present` applies them and returns immediately (non-blocking
   — the main thread's `[NSApp run]` keeps the window live independently).
4. **Events + pacing.** Click counter (synthesized button target/action → counter),
   `host_button_take_clicks` drain at sync, window-close flag → `isOpen`. Implement
   `app::poll`/`host_wait_events` (worker parks on a condvar/semaphore the main thread
   signals when it enqueues an event; timed wait + early wakeup). Verify `clicked`/`isOpen`
   and a `poll`-paced loop (no busy-spin) on-device.
5. **Linux/GTK4 backend.** Same seam with `GtkBox`/`GtkButton`/`GtkLabel`; map justifications
   per §8; `host_present` via the GTK main-context iteration. Verify the canonical program
   on the Debian aarch64 box (per plan-05).
6. **Lifetime & detach correctness.** `remove`/`close` detach-not-destroy; orphan retain +
   reparent; registered close op; re-attach to a new window. Test: remove→re-attach,
   close-window-then-reuse-widget, scope-drop teardown ordering.
7. **Polish.** `show`/`hide` reflow (display:none), `size`/`resizable`/`title` live updates,
   `<0` fill/clamp semantics for `Size`/`Spacing`, examples + docs.

## 12. Naming note (non-blocking)

Getters/setters currently mix conventions (`visible` getter; `title`/`size`/`resizable`
setters; `setLabel` with a `set` prefix). Consider standardizing before lock-in — e.g.
arity-overload (`app::title(win)` gets, `app::title(win, s)` sets) or uniform `setX`/`x`
pairs. Cosmetic; does not affect the architecture.
