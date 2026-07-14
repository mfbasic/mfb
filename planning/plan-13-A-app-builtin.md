# plan-13-A: `app::` built-in GUI package

Last updated: 2026-07-09
Overall Effort: huge (the whole plan-13 `app::` feature: A + B + C)
Effort: x-large

Part **A** of plan-13 (the `app::` GUI feature). Companion phase documents:
**plan-13-B** (`app::TextArea` + `text::AttributeString`) and **plan-13-C**
(the widget-cell grid `app::Table`), both additive to the §8 host-protocol seam
defined here — land this document's phases first.

Status: **design**. Target: a cross-platform **native-widget** GUI package, `app::`,
usable from `mfb build -app` programs. Builds directly on the existing `-app` mode
(plan-04 macOS / plan-05 Linux): the native event loop owns the main thread and the
program's entry runs on a worker.

## 1. Goals & non-goals

- **Native widgets**, not self-drawn: `NSView`/`NSButton`/`NSTextField` on macOS,
  `GtkFixed`/`GtkButton`/`GtkLabel` on Linux. "App, not a game."
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

Non-goals (v1): **multi-line** text areas, menus, native dialogs, images, scrolling
containers, animation/timers, theming. (A single-line `Input` *is* in v1 — §3/§5.) The
host-protocol seam (§8) is designed so the rest slot in later.

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

**Every widget type has a registered close op — but a registered close op need not be
user-callable.** The language closes a `RES` binding at scope-drop by calling *that resource
type's registered close op*, and a resource union's drop is **tag-dispatched to the active
variant's registered close op** (`mfb spec language resource-management`). So `Window`,
`Container`, `Button`, `Label`, and `Input` each register one — not just `Window`. The
registration is a compiler-internal fact (a `ResourceInfo.close_function` name, resolved
straight to a runtime helper by drop lowering); it is independent of whether the same name is
also published in the user-callable builtin surface. Only `app::close` is:

| Resource | Registered close op | User-callable? |
| --- | --- | --- |
| `app::Window` | `app::close(win)` | **yes** — a window has OS-visible identity and an observable close; "close the window" is a real UI verb |
| `app::Container` | `app::destroy(c)` | no — internal close op only |
| `app::Button` | `app::destroy(b)` | no — internal close op only |
| `app::Label` | `app::destroy(l)` | no — internal close op only |
| `app::Input` | `app::destroy(i)` | no — internal close op only |

**Widgets have no early-release op, and none is needed.** `app::destroy` is registered as
each child widget type's internal close op — a **return-type-free** entry on the concrete
type (never on the `Widget` union — a close op must name a concrete type, §10) — so scope-drop
and resource-union tag-dispatch have a target to call. It is **not exported**: `app::destroy(b)`
in user code is an unknown function. This is deliberate, for two reasons that together mean an
exported `destroy` would buy nothing:

- *It never named a widget you could legally destroy.* Early release only matters for a
  runtime-created widget, and by §6/plan-13-C the only way to keep such a widget alive past
  the loop that made it is to park it in an outer `List OF RES` (`mfb spec language
  resource-management` §15.6). Once ownership floats to that collection the binding is
  **borrow-only** and may not close, RETURN, or transfer it (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`)
  — so `app::destroy` on it is a compile error. The widgets you *could* pass it are the ones
  with a live local `RES` binding, which scope-drop already closes correctly on every path.
- *It reeks of `free`.* MFB has no user-visible lifetime construct; a resource is released by
  the same ownership and drop rules as any other owned value (`mfb spec language
  resource-management`). Release is **scope-drop of the owning binding or collection, on every
  exit path** — never an explicit call. A long-lived app that churns table rows does not
  accumulate: it structures the churn so the owning collection's scope exits per cycle (a
  per-refresh block or helper `SUB`), at which point every widget it owns is destroyed exactly
  once. The trigger is scope exit of the owner, not overwriting its value — so an
  ever-growing program-scope list is a data-structure choice, not a language limitation.

Contrast `app::close(win)`: a `Window` is the one handle with OS-visible identity and an
observable close, the same category as a file descriptor or socket. `app::isOpen`/`app::close`
is a UI verb, not a memory verb, so it stays exported — the user may close a window early and
observe it, and scope-drop closes any window not closed by hand.

Should a future need for genuine mid-scope widget release appear, the honest fix is **not**
re-exporting `destroy` (a `free`); it is a *consuming* `app::remove(container, w)` that takes
ownership of `w` — naming the UI act ("take this off the screen for good") with deallocation as
a consequence. Not in v1.

**`app::Widget` is a parameter-only type.** It is never `RES`-bound
(`RES w AS app::Widget = ...` is not part of the surface), never returned, and never
consumed. It exists solely to name "any child widget" in a borrow position (§3).

**Opaque state vs. user `STATE`.** Any "state" in this document is the package's *private*
opaque resource state (dirty flags, the click counter, the property shadow). It is **not**
the language-level MFB Resource `STATE`, which the user may still attach to any of these
handles and use freely for their own data, e.g. `RES ok AS app::Button STATE RowRef`.
(A resource *union* carries no `STATE` per the language rules; that is fine — `Widget` is
never bound, only passed.)

## 3. Resources

```
app::Window        ' RES — the lifetime anchor; its drop/close tears down the native window
app::Container     ' RES — a layout box (flex)
app::Button        ' RES
app::Label         ' RES
app::Input         ' RES — single-line editable text field (NSTextField editable / GtkEntry)

UNION Widget       ' app::Widget — a child widget of any kind (NOT Window, which is the root)
    app::Container
    app::Button
    app::Label
    app::Input
END UNION
```

`app::Widget` is a **resource union** (all variants are resources), used **only as a
borrowed parameter type**: naming an existing child generically for `slot`/`attach` and for
the widget-wide `getVisible`/`setVisible`/`getSize`/`setSize`/`frame`. No `app::` function
consumes a `Widget`; the consuming ops are `app::close` (a `Window`) and `app::destroy`
(a concrete widget type). Accepting a variant in a union parameter position is **not**
something the language supports today — §10 specifies the compiler work and the spec
amendment that makes it legal, and the ABI it lowers to.

## 4. Types & enums

```
TYPE Size
    width  AS Integer    ' < 0 means "fill available width"
    height AS Integer    ' < 0 means "fill available height"
END TYPE

TYPE Rect                ' a laid-out frame, in the parent's coordinate space
    x      AS Integer
    y      AS Integer
    width  AS Integer
    height AS Integer
END TYPE

TYPE Spacing
    top    AS Integer    ' < 0 clamps to 0
    bottom AS Integer    ' < 0 clamps to 0
    left   AS Integer    ' < 0 clamps to 0
    right  AS Integer    ' < 0 clamps to 0
END TYPE

app::Direction      Row, Column, Stack    ' Stack = z-overlay (children share the content rect)
app::Justification  Start, End, Center, Between, Around, Even   ' main-axis distribution (ignored when dir = Stack)
app::Align          Start, End, Center, Stretch                 ' cross-axis alignment (both axes when dir = Stack)
app::ClickMode      Immediate, Exclusive    ' Button: Immediate (default) fires single at once; Exclusive defers single until a double is ruled out
```

`Justification` defaults to **`Start`**, matching CSS `justify-content: flex-start`. (An
earlier draft defaulted to `Between`, which silently pushes a two-child `Row` to opposite
edges — a surprising default.)

## 5. Function surface

Construction / structure functions run **entirely on the worker** — they only mutate the
MFB-side shadow tree (cheap, no thread hop). Native objects are realized lazily on the next
`app::sync` (§7).

### 5.0 The argument rule (read before the signatures)

**Omitted arguments must be trailing.** `app::` follows the existing builtin convention —
`strings::padLeft(value, width[, padChar])`, `strings::find(value, needle[, start])` — in
which optional parameters are *trailing* and omission selects a shorter arity. There are no
AST-inserted default expressions for builtins: the value shown as `= …` below is what the
**implementation** supplies when the argument is absent.

Concretely, two rules the signatures obey and every example respects:

1. **A middle parameter cannot be skipped, even by name.** `app::window(title := "Demo")`
   is legal only because `title` is the *first* parameter. Skipping a parameter and naming a
   later one is rejected by
   `normalize_builtin_call_arguments` (`src/syntaxcheck/builtins.rs`) with
   `TYPE_CALL_ARITY_MISMATCH`: *"omits parameter `X` before a later supplied argument."*
2. **Overload sets are arity/type sets, not defaults.** `mfb spec language functions`:
   *"Default arguments do not combine with overloading… a name therefore either uses
   default/omitted arguments or is overloaded, not both."* For builtins the two are
   reconciled by declaring **every arity as its own overload**: `app::window` is four
   overloads (`()`, `(title)`, `(title, size)`, `(title, size, resizable)`), and named-arg
   binding selects among them by count-and-names
   (`builtins::select_param_name_overload`). Two overloads that share an arity must either
   share their parameter names (so name binding is unambiguous) or differ in argument types
   so `resolve_call` can separate them.

Parameters are therefore **ordered most-likely-supplied first**. Named arguments remain
available and are the recommended style; they just may not skip.

### Layout / lifetime

```
app::window(title AS String = "MFB Application",
            size AS Size = Size[width := 800, height := 600],
            resizable AS Boolean = TRUE) AS RES app::Window

' Destroys the window. Live descendants detach and remain valid until their own scope
' drop (re-attachable to a new window). This IS app::Window's registered close op, so an
' explicit app::close and the binding's scope-drop never double-fire.
app::close(win AS RES app::Window) AS Nothing

' The registered close op of each child widget type (§2). Detaches, destroys the native
' peer + shadow node, consumes the handle. NOT exported — this is the internal target that
' scope-drop and union tag-dispatch call; it is not user-callable (`app::destroy(...)` in
' user code is an unknown function). Shown here only to name the close op registered per type.
app::destroy(c AS RES app::Container) AS Nothing
app::destroy(b AS RES app::Button) AS Nothing
app::destroy(l AS RES app::Label) AS Nothing
app::destroy(i AS RES app::Input) AS Nothing

' A Window holds EXACTLY ONE root child. Calling addContainer/attach on a window that
' already has a root child fails with ErrInvalidArgument; detach the old root first.
app::addContainer(parent AS RES app::Window,
                  dir AS app::Direction = app::Direction.Row,
                  align AS app::Align = app::Align.Center,
                  justify AS app::Justification = app::Justification.Start,
                  padding AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Container

app::addContainer(parent AS RES app::Container,
                  dir AS app::Direction = app::Direction.Row,
                  align AS app::Align = app::Align.Center,
                  justify AS app::Justification = app::Justification.Start,
                  padding AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Container

app::addButton(parent AS RES app::Container,
               label AS String = "",
               margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Button

app::addLabel(parent AS RES app::Container,
              label AS String = "",
              margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Label

app::addInput(parent AS RES app::Container,
              value AS String = "",
              placeholder AS String = "",
              margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Input

' Current zero-based slot of `widget` in `container`, or -1 if not a child.
app::slot(container AS RES app::Container, widget AS RES app::Widget) AS Integer

' Detach the child in `index` (current child-list position; positions compact after a
' remove). Detached widget + descendants stay valid until their own scope drop and keep
' their own subtree; they can be re-attached elsewhere.
app::remove(container AS RES app::Container, index AS Integer) AS Nothing

' Detach a window's root child (the only way to replace it without closing the window).
' No-op when the window has no root child.
app::remove(win AS RES app::Window) AS Nothing

' Re-attach an existing (detached) widget. index = -1 appends; otherwise inserts at slot.
app::attach(container AS RES app::Container, widget AS RES app::Widget, index AS Integer = -1) AS Nothing

' Re-attach a detached container as a window's root child (ErrInvalidArgument if occupied).
app::attach(win AS RES app::Window, container AS RES app::Container) AS Nothing
```

Note `addContainer`'s parameter order — `dir`, `align`, `justify`, `padding`. `align` sits
ahead of `justify` because the common "stretch every row to the full width" case needs
`align` alone; under §5.0 rule 1 a `justify` sitting in front of it would have to be
supplied just to reach it.

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
'   timeout < 0  : ErrInvalidArgument
' Parks the WORKER only; the main thread / window stay live throughout (§7). poll waits;
' it does not drain — sync drains.
' Before this window's FIRST sync, poll returns FALSE immediately (regardless of
' timeout): no native window exists yet, so no event source can ever fire — parking
' would deadlock a poll-first loop (§9, locked).
app::poll(win AS RES app::Window, timeout AS Integer = 0) AS Boolean

' FALSE once the user has closed the window (a native->shadow event, like clicked).
app::isOpen(win AS RES app::Window) AS Boolean
```

`poll` takes the window it waits on. (An earlier draft made it a free function while the
seam call `host_wait_events(window, timeout)` took one — the asymmetry left "what does
`poll` do with zero windows?" undefined.)

### Window properties

```
app::getVisible(win AS RES app::Window) AS Boolean
app::setVisible(win AS RES app::Window, visible AS Boolean) AS Nothing
app::getTitle(win AS RES app::Window) AS String
app::setTitle(win AS RES app::Window, title AS String) AS Nothing
app::getResizable(win AS RES app::Window) AS Boolean
app::setResizable(win AS RES app::Window, resizable AS Boolean) AS Nothing
app::getSize(win AS RES app::Window) AS Size
app::setSize(win AS RES app::Window, size AS Size) AS Nothing
```

### Any widget (`Container` / `Button` / `Label` / `Input`)

One overload each, taking the `app::Widget` union — covers every child widget now and any
future variant automatically. (`Window` is not a `Widget`; it has its own pair above.)

```
app::getVisible(w AS RES app::Widget) AS Boolean
app::setVisible(w AS RES app::Widget, visible AS Boolean) AS Nothing
app::getSize(w AS RES app::Widget) AS Size
app::setSize(w AS RES app::Widget, size AS Size) AS Nothing   ' width/height < 0 = fill (§8)

' The widget's most recently computed native frame, in its parent's coordinates, mirrored
' native->shadow at sync (§7). Rect[0, 0, 0, 0] before the widget has a realized native
' peer. Read-only observed geometry — the program cannot write a frame.
app::frame(w AS RES app::Widget) AS Rect
```

`getSize` on a widget returns the **configured** `Size` (the `setSize`/constructor value,
including `< 0` fill) — never a laid-out frame. `app::frame` is the laid-out rect: an
observation, pulled at `sync`, never an input to layout (§7). The window's `getSize` is
likewise pulled native→shadow at `sync`, because the user can drag-resize it.

`app::frame` exists for three reasons: hit-testing and geometry-dependent program logic;
making the layout solver's output directly assertable from a test program (§8, §11 Phase
2); and giving the eventual scrolling/canvas work a place to hang.

### Button

```
app::getLabel(button AS RES app::Button) AS String
app::setLabel(button AS RES app::Button, label AS String) AS Nothing
app::clicked(button AS RES app::Button) AS Boolean        ' single-click, frame-latched (§7)
app::doubleClicked(button AS RES app::Button) AS Boolean  ' native double-click gesture, frame-latched (§7)
app::getClickMode(button AS RES app::Button) AS app::ClickMode
app::setClickMode(button AS RES app::Button, mode AS app::ClickMode) AS Nothing
```

`clicked` and `doubleClicked` are **independent** native-detected events, not "1 vs 2 clicks in
a frame" (§7). In the default **`Immediate`** click mode a double-click also raises `clicked`
once (its leading press is a single click, unknowable as a double until the second press) — so
**check `doubleClicked` first** and skip `clicked` if you handled the double. If single and
double must be mutually exclusive on a button, set **`Exclusive`** mode: the native side then
defers each single click by the *system* double-click interval and cancels it if a double
completes — at the cost of that much latency on every single click of that button, so it is
opt-in per button (§7).

### Label

```
app::getLabel(label AS RES app::Label) AS String
app::setLabel(label AS RES app::Label, text AS String) AS Nothing
```

### Input

```
app::getValue(input AS RES app::Input) AS String
app::setValue(input AS RES app::Input, value AS String) AS Nothing
app::valueChanged(input AS RES app::Input) AS Boolean   ' user edited since last sync, frame-latched (§7)
app::submitted(input AS RES app::Input) AS Boolean      ' user pressed Enter since last sync, frame-latched (§7)
```

The `value` is **bidirectional** (both the user and `setValue` write it), unlike every other
property — so it needs a precedence rule and a careful event semantic; see §7. `valueChanged`
reflects **user** edits only: a `setValue` never raises it (no feedback loop). `submitted` is an
independent edge event (Enter) — it does not imply `valueChanged`, and vice versa.

`getX`/`setX` overload on handle type. Properties shared by *all* child widgets —
`getVisible`/`setVisible`/`getSize`/`setSize` — take the `app::Widget` union once (plus the
separate `Window` overload); `getLabel`/`setLabel` stay per-type because `Container` has no
text. **Visibility is CSS `display:none`**: a hidden widget (`setVisible(h, FALSE)`) occupies
no space and its siblings reflow. Read-only observed state keeps `is*`/event/noun naming, *not*
`get*` — `app::isOpen` (no setter; close via `app::close`), `app::clicked` (an event), and
`app::frame` (observed geometry). The `Widget`-union acceptance is **not free**: §10 specifies
the three compiler sites and the ABI it needs.

## 6. Canonical program

```
RES win  = app::window(title := "Demo")
RES root = app::addContainer(win, dir := app::Direction.Column)   ' RES: a container is a resource
RES ok   = app::addButton(root, label := "OK")
RES out  = app::addLabel(root, label := "ready")

WHILE app::isOpen(win)
    app::poll(win, 16)        ' timed tick: wait <= 16 ms, waking early on any event. (Before the
                              '   first sync, poll returns FALSE immediately — it never parks.)
    app::sync(win)            ' non-blocking: drain events -> shadow, flush dirty props -> native
    IF app::clicked(ok) THEN app::setLabel(out, "clicked!")   ' clicked reads the shadow drained by sync
WEND
' scope exit drops in reverse declaration order: out, ok, root freed first (each via its
' registered app::destroy close op); win drops last -> its registered app::close op tears
' down the (now-empty) native window
```

**Why a timed `poll(win, 16)`, not a bare event-driven `poll(win)`:** `sync` is the only flush
point (§7/§9 — `poll` waits, `sync` reconciles; they are deliberately separate concerns). A
property written *after* this frame's `sync` — like the `setLabel` above — reaches native at the
**next** `sync`. With a fully-event-driven `poll(win)`, that next sync waits for the next user
event, so a handler's write would not become visible until the user does something else. The
rule: a loop whose handlers **write** properties paces with a timed `poll(win, t)` so writes
flush within `t` ms; a pure `poll(win)` suits loops that only read/observe.

## 7. Architecture: shadow tree + sync

**Threading.** Same split as the transcript `-app` mode: the native loop owns the main
thread (`[NSApp run]` / `g_application_run`); the program runs on a worker. **`app::sync`
is the one and only main↔worker sync point.** If `sync` is never called, no window is
created/shown — only the native loop runs.

**Cross-thread mechanics: reuse the two channels `-app` already has.** The existing transcript
`-app` mode (plan-04 macOS, plan-05 Linux) already runs a native main thread and an MFB
worker, and it already moves data both ways. `app::` adds **no new concurrency primitive** —
it generalizes the two existing ones. Crucially, the current `-app` mode uses **no mutex and
no atomics**, and neither does this design. (It could not: the compiler emits **no atomic or
exclusive-load instructions** on any backend — `src/arch/{aarch64,x86_64,riscv64}` have no
`ldaxr`/`stlxr`/`cas`/`lock`-prefixed encoders.)

| Direction | Existing transcript mechanism | `app::` generalization |
| --- | --- | --- |
| main → worker | `pipe(2)`; keystrokes written by the native key handler; read end `dup2`'d onto fd 0; worker blocks in `read()` (`macos_aarch64/app/bootstrap.rs:335`, `mod.rs:165-189`; `linux_gtk/bootstrap.rs:204`) | an **event pipe** (its own fd pair, *not* dup'd onto fd 0) carrying fixed-size **event records** |
| worker → main | macOS `performSelectorOnMainThread:withObject:waitUntilDone:YES` (`mod.rs:101`, `bootstrap.rs:614`); Linux `g_idle_add` (`linux_gtk/app_io.rs:138`) | the same two calls, carrying a **command batch** |

- **Command channel (worker → main), used by `sync`.** `app::sync` serializes every dirty
  node's properties and every structure edit into one heap-allocated **command batch**, then
  hands the pointer to the main thread — `performSelectorOnMainThread:…waitUntilDone:**NO**`
  on macOS, `g_idle_add` on Linux — and returns. Ownership of the batch transfers with the
  pointer; the main thread applies it and frees it. Nothing is shared, so nothing is locked.
  Posts from one thread to a run loop are FIFO, so batch ordering is preserved.
  (Today's `io::print` uses `waitUntilDone:YES` because it hands over an autoreleased
  `NSString` whose lifetime it does not control; a batch we `malloc` ourselves has no such
  constraint, which is precisely why `sync` can be non-blocking where `io::print` is not.)
- **Event channel (main → worker), drained by `sync`, waited on by `poll`.** Native handlers
  write fixed-size event records — `{kind, nodeId, payload}` for click, double-click,
  value-changed, submit, resize, close, frame-report — directly into the event pipe, exactly
  as the key handler writes keystrokes today. `app::sync` drains it with non-blocking reads
  until `EAGAIN` and folds the records into the shadow (click records bump that node's frame
  counter, a resize record updates the window-size shadow, a frame-report record updates that
  node's `app::frame` mirror). Variable-length payloads (an `Input`'s text) are
  length-prefixed.
- **`app::poll(win, t)` is `poll(2)`/`ppoll(2)` on the event pipe's read fd** with timeout `t`
  (`0` → infinite). It returns TRUE on `POLLIN`. The wait primitive is therefore free: the
  kernel already gives us "block until readable, with a timeout." No condvar, no mutex.
- **Backpressure.** The pipe's write end is `O_NONBLOCK`. On `EAGAIN` (a worker that has not
  `sync`ed in a long time) the native side folds the record into a **main-thread-only** pending
  struct — click counts accumulate, frame reports and enter/leave coalesce last-wins — and
  retries on the next run-loop turn. Nothing is lost and nothing is shared: the pending struct
  is touched only by the main thread. A blocking write end (what the transcript uses for
  keystrokes today) would risk stalling the UI behind a busy worker.

The upshot: **there is no shared mutable memory between the two threads** in the steady state,
so the words "atomic drain" from earlier drafts are not just unimplementable, they are
unnecessary. Two message channels, both already load-bearing in shipped `-app` code.

**Mode selection — GUI vs transcript (static, whole-program).** Whether a program calls
`app::window` is detected at **build time** (the same mechanism as the existing `uses_term`
flag — a runtime-symbol-presence scan in `src/target/shared/code/mod.rs`), selecting between
two sub-modes of `-app`:

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
- **Property writes** (`setLabel`, `title`, `size`, `setVisible`, …) mutate the shadow and
  set a **dirty flag** on that node. `sync` flushes dirty nodes to native and clears the
  flags. (Per-node single dirty flag is fine for v1 — re-push that node's props.)
- **Property reads** (`getVisible`, …) read the shadow — correct even before the first
  `sync` (they return the intended value), and never block.
- **Structure** (`add*`/`remove`/`attach`) edits the shadow tree's parent/child links and
  marks structure dirty; `sync` applies inserts/detaches to the native tree.
- **The shadow holds no *authoritative* geometry.** No computed frame is ever an **input**
  to anything worker-side: layout parameters flow shadow→native, and geometry is computed
  and owned exclusively on the native (main-thread) side (§8). The two geometry values the
  worker can *observe* are pulled native→shadow at `sync` as read-only mirrors: the window's
  own size (the user can drag-resize it) and each realized widget's last computed frame
  (`app::frame`, one 16-byte `Rect` per realized node). Neither is ever read back by the
  solver.

**Events (pull direction), e.g. `clicked`:**

- The native side (main thread) emits **one event record per click** into the event pipe as it
  happens (not a bool — rapid clicks aren't lost). Under pipe backpressure the records fold
  into a main-thread counter and flush later, so a click is never dropped.
- `app::sync` **drains** the pipe and folds each record into the worker-visible shadow: a
  click record increments that button's per-frame counter.
- `app::clicked(btn)` reads worker-local shadow state: stable for the whole frame, no
  cross-thread access at read time, no wall-clock timing. (v1 may collapse the counter to
  "clicked since last sync"; keep it a counter in the shadow to allow richer event reads
  later.)

**Double-click is a separate, natively-classified counter — NOT "counter == 2".** A
double-click is an OS gesture (a time *and* space threshold), so it must be detected where the
OS detects it, at native event time — `NSEvent.clickCount` / `GtkGestureClick` `n_press` — and
accumulated in its **own** counter alongside the single-click one. Counting "two clicks drained
in one `sync`" would be wrong: that depends on the worker's frame pacing, not the user's
gesture — an event-driven or 60fps loop splits the two presses into separate frames (never
sees 2), while a laggy loop coalesces two *unrelated* single clicks into one frame (false
double). With native classification, `app::doubleClicked` drains the double-click counter and
is correct regardless of `poll`/`sync` cadence. The leading press of a double-click is also a
clickCount==1 single click, so it increments the single counter too — hence both events can
latch in the same frame; `doubleClicked` is checked first by the caller (§5). The default mode
does **no** suppression by delay (that would add the OS double-click interval of latency to
*every* single click).

**Exclusive click mode (opt-in) — suppress the stray single.** A button set to
`ClickMode.Exclusive` (§5) makes `clicked`/`doubleClicked` mutually exclusive: on a
`clickCount==1` press the native side starts a timer of the **system double-click interval**
(`NSEvent.doubleClickInterval` / GtkSettings `gtk-double-click-time` — user-configurable,
typically ~250–500 ms; **do not hardcode**, and note ~75 ms is far too short to catch the
second press), increments the single counter only when it expires, and cancels it if a
`clickCount==2` arrives first (bumping the double counter instead). This is the only way to drop
the leading single click, and it costs the full interval of latency on every single click — so
it stays **per-button opt-in**, default `Immediate`, never global. It adds a narrow main-thread
resolution timer for `Exclusive` buttons only; the default path stays timer-free.

**Input value is the one bidirectional property — it needs a precedence rule.** Every other
property is written only by the program (shadow→native); an `Input`'s `value` is written by
**both** the user (native→shadow, pulled at `sync`) and `setValue` (shadow→native, pushed at
`sync`). The conflict — user typed *and* program `setValue`d the same field in one frame — is
resolved by **program-set-wins-this-frame**: if `value` is dirty (a `setValue` since the last
`sync`), `sync` pushes the shadow value to native and discards that frame's user edit;
otherwise it pulls the native field's text into the shadow so `getValue` reflects typing. This
is the standard "controlled input" rule and keeps a single source of truth per frame.
`valueChanged` is a frame-latched edge event like `clicked`: TRUE iff the **user** edited the
field since the last `sync` (native `controlTextDidChange` / GtkEditable `changed`, collapsed
to a since-sync flag — not a per-keystroke stream). A programmatic `setValue` clears/does not
set the latch, so it never triggers `valueChanged` (no echo loop). `submitted` is a second
frame-latched edge event on the same field — TRUE iff the user pressed **Enter** since the last
`sync` (NSTextField action on Return / GtkEntry `activate`) — independent of `valueChanged`
(Enter need not change the text, and typing need not submit).

**Window close** is the same mechanism: the native close handler sets a shadow flag drained
at `sync`; `app::isOpen` reads it.

**`sync`, mutators, and getters never block; `app::poll` is the one explicit wait.**
`app::sync` never waits on the main thread: it drains the event pipe with non-blocking reads
until `EAGAIN`, then **posts** an owned command batch of dirty property/structure changes to
the main thread (async — an enqueued apply the native loop picks up on its next iteration) and
returns. Property/event reads are plain shadow reads. So there is up to one frame of latency
between a `sync` and the pixels changing — fine for a retained UI.

Waiting is **`app::poll(win, timeout)`**, the only blocking call (and it blocks by default).
It parks the **worker** in `poll(2)` on the event pipe's read fd: `poll(win)` waits
indefinitely for the next event record, `poll(win, t>0)` waits up to `t` ms with early wakeup.
It parks the worker only — the main thread's native loop keeps the window responsive
throughout. Note `poll` only *waits*; `sync` is what drains events into the shadow, so there is
no need for a zero-wait `poll` — if you don't want to wait, don't call it (a bare `sync`-only
loop drains fine but busy-spins, so cap it with `poll(win, t)` or go event-driven with
`poll(win)`). Before this window's **first** `sync` no native window exists and nothing can
ever write to the pipe, so `poll` returns FALSE immediately instead of parking (§9). And
because `sync` is the only flush point, a property written *after* this frame's `sync` reaches
native at the **next** one — loops whose handlers write properties pace with a timed
`poll(win, t)` (§6, §9).

**`close` orphan handling.** On `app::close(win)`, the backend must **retain + unparent**
every live descendant native widget (reparent to an offscreen holder) *before* destroying
the native window, so detached widgets survive per §2. Each orphan is freed when its own
binding drops (its registered `app::destroy` close op).

## 8. Host-protocol seam (per-platform)

Everything above the seam is shared, platform-independent code (the `app::` surface, the
shadow tree, dirty tracking, the sync driver, **and the layout solver**). Everything below is
`src/target/<platform>/widgets` written against the existing `Asm`/`abi` builder (so it is
CPU-neutral — a future `arch/x64` reuses it). Keep the seam **small and stable** — its size
is the cost of every future platform.

```
host_create_window(size, title, resizable) -> handle
host_destroy_window(handle)                              ' detaches live children first
host_window_set_title / set_size / set_resizable / set_visible(handle, …)
host_window_is_open(handle) -> bool                      ' drained at sync

host_create_container() -> handle             ' plain positioning view — NO layout params
host_create_button(label) -> handle           ' native leaf; keeps native look + a11y
host_create_label(text) -> handle
host_create_input(value, placeholder) -> handle          ' editable NSTextField / GtkEntry
host_set_text(handle, s)                                 ' button + label
host_set_value(handle, s)                                ' Input: programmatic write (shadow->native, push at sync)
host_input_drain(handle) -> (text, userChanged, submitted)  ' Input: pull text + did-user-edit + did-user-submit (Enter) since last drain
host_set_visible(handle, bool)                           ' display:none semantics (engine skips hidden nodes)

host_insert_child(parent, child, index)                  ' view-hierarchy parenting + z-order (Stack); add / attach
host_detach_child(parent, child)                         ' remove (no destroy)
host_destroy(handle)                                     ' fired by RES drop / a registered close op

host_measure(handle, avail) -> Size                      ' main-thread intrinsic size of a leaf (button/label)
host_set_frame(handle, rect)                             ' absolute position+size of a child in its parent's coords

host_set_click_mode(handle, mode)                        ' Immediate | Exclusive; Exclusive defers single by the system double-click interval
host_present(window)                                     ' calls the shared solver, then set_frame; returns immediately (never blocks)
host_wait_events(window, timeout) -> bool                ' app::poll: poll(2) on the event-pipe read fd (timeout 0 = forever)
host_drain_events(window) -> records                     ' app::sync: non-blocking reads until EAGAIN
host_post_batch(window, batch)                           ' app::sync: hand an owned command batch to the main thread
```

Note there is no `host_button_take_clicks`. Clicks are not polled out of native state — the
native click handler *pushes* an event record into the pipe when it fires (§7). That removes
the per-button main-thread counter, and with it the shared memory the counter would have
needed. Same for double-clicks, value-changed, submit, resize, close, and frame reports:
one record kind each.

### 8.0 What the existing `-app` mode already provides

`app::` is **the same architecture as today's transcript `-app` mode, with a user-defined
widget tree in place of the fixed transcript view.** Almost nothing below the seam is new
technique; it is new *content* built with techniques already shipped. Before estimating any
backend phase, read `src/target/macos_aarch64/app/` (~3 400 lines) and
`src/target/linux_gtk/` (~2 600 lines), which already do all of this:

| `app::` needs | Already done, here |
| --- | --- |
| Native window + run loop owning the main thread; MFB entry on a `pthread` worker | `macos_aarch64/app/bootstrap.rs:458` (`pthread_create`, `[NSApp run]`); `linux_gtk/bootstrap.rs:73` (`g_application_run`) |
| **Creating an Obj-C class at runtime, from codegen**, with a method the toolkit calls back into — precisely what a `Button` target/action needs | `objc_allocateClassPair` + `class_addMethod` + `objc_registerClassPair` for `MFBTextView.keyDown:` and `TermView` (`macos_aarch64/app/bootstrap.rs:109-124`, `:208-213`); per-instance state via the extra-bytes / `object_getIndexedIvars` path (`app/mod.rs:317`) |
| Connecting a GTK signal to a codegen-emitted callback | `g_signal_connect_data` for `activate` / `key-pressed` (`linux_gtk/bootstrap.rs:73-198`); already in the symbol table (`linux_gtk/mod.rs:662`) |
| Calling `objc_msgSend` / GTK from emitted code, with selector interning | `app/mod.rs:502` (`sel_registerName` helper), `app/mod.rs:101` (selector table) |
| Marshalling work onto the main thread from the worker | `performSelectorOnMainThread:withObject:waitUntilDone:` (`app/mod.rs:101`); `g_idle_add` (`linux_gtk/app_io.rs:138`) |
| Delivering native events to a blocked worker, with a timeout | the input `pipe(2)` written by the native key handler (`macos_aarch64/app/bootstrap.rs:335`, `linux_gtk/bootstrap.rs:204`) |
| Toolkit symbols resolved lazily at load | the `-app` symbol tables (`linux_gtk/mod.rs:616-662`; `LIB_OBJC`/`LIB_SYSTEM` on macOS) |
| GTK4 specifically (not GTK3) | `linux_gtk/mod.rs:188` binds `libgtk-4.so.1` |
| `-app` on x86-64 as well as aarch64 | the GTK module is shared at `src/target/linux_gtk` and box-verified |

So the genuinely new work below the seam is: creating leaf widgets (`NSButton`/`NSTextField`,
`GtkButton`/`GtkLabel`/`GtkEntry`) instead of one text view; a `setFrame`/`GtkFixed` placement
call; and one more pipe carrying richer records than keystrokes. The risky, unprecedented item
in this plan is **not** the backends — it is the solver (§8.1).

Two constraints inherited from the existing mode, both easy to get wrong:

- **The transcript's input pipe is `dup2`'d onto fd 0** so the console `readLine` helpers can
  be reused verbatim. `app::`'s event pipe must be a **separate fd pair and must not touch fd
  0**: in GUI sub-mode (§7) `io::` uses console lowering, so fd 0 is the program's real stdin.
- **`performSelectorOnMainThread:` with `waitUntilDone:YES`** is what today's `io::print` uses,
  and it makes each write a synchronous main-thread round-trip. `app::sync` must post with
  `waitUntilDone:NO` and an owned batch, or it inherits that blocking behavior and violates §9.

### 8.1 The layout solver: what it is made of (locked)

**Layout is one shared flex engine, run on the main thread — not native stacks.**
`Direction`/`Justification`/`Align`/`Size`/`Spacing` are computed by a single
platform-independent solver, so every platform lays out *identically* and the full
`Justification × Align` matrix is just arithmetic. Native containers are demoted to dumb
positioning canvases (`host_create_container` makes a plain `NSView` / `GtkFixed`); only
the leaf widgets (`NSButton`/`NSTextField`, `GtkButton`/`GtkLabel`) stay native, so native
look + a11y are preserved. This deliberately replaces the earlier "delegate to native
stacks" idea: `NSStackView.distribution` and `GtkBox` *both* fail to express the full
justify-content set (`Start`/`End`/`Center`/`Around`/`Even` need spacers or priority hacks
on *both* platforms, and spacer widgets desync the shadow↔native child-index model) — so
the shared solver is both less total code and the only way to be feature-complete +
identical.

**The solver is an emitted MIR runtime helper, `_mfb_rt_app_layout`.** This is the single
most consequential implementation fact in the plan and an earlier draft left it unstated.
This compiler has **no runtime library**: every routine that executes inside a built program
is either (a) hand-emitted machine code produced by a Rust emitter under
`src/target/shared/code/` (the precedent: `float_format.rs`, ~600 lines of Rust emitting the
one `_mfb_rt_float_to_string` helper), or (b) an MFBASIC-source package compiled with the
user's program (the precedent: `regex`, `csv`, `datetime`, `vector`). A Rust function cannot
run at program runtime, so "shared, platform-independent solver" *must* resolve to one of
those two, and the choice is forced by which thread it runs on:

- **(b) is unavailable.** MFBASIC-source code needs an arena (allocation, RNG state) and runs
  on the worker. But §8.2 locks that layout re-runs **autonomously on the main thread during a
  native drag-resize, with no worker round-trip**. The main thread has no arena. Moving layout
  to the worker would put resize latency behind the program's `poll` cadence and would force
  computed geometry back into the worker shadow — losing two locked properties.
- **(a) is the answer.** `_mfb_rt_app_layout` is a hand-emitted, **allocation-free**,
  main-thread-callable helper. It takes a pointer to the native side's flat node array (the
  same array `host_present` already walks), an indirect `host_measure` function pointer, and
  the content rect; it writes one `Rect` per node back into that array. No arena, no MFB
  calling convention, no per-platform code — one emitter, three backends, identical output.

Budget this honestly: a full flex pass (3 directions × 6 justifications × 4 aligns ×
flex-grow × padding/margin/`display:none` skipping) emitted as MIR is on the order of
1500–2500 lines of Rust emitter — several times `float_format.rs`, and comparable to the
transcendental kernels. It is the largest single work item in plan-13-A, and it lands in
Phase 2, before any window exists.

**Testing it headlessly: the `headless` host backend.** The old Phase-2 plan — "unit tests
injecting a fake measure callback" — describes testing a Rust function, which is not what the
solver is. Instead, add a **third backend behind the same seam**:
`src/target/shared/widgets/headless`. Its `host_measure` returns a deterministic synthetic
size (`width = 8 * scalarLen(text) + 16`, `height = 20` for leaves), its `host_set_frame`
appends `id kind x y w h` to stdout, `host_wait_events` returns FALSE immediately, and
`host_window_is_open` goes FALSE after the first `sync`. It is selected by
`mfb build -app --app-host headless`, needs no display server, and runs on every platform
including the riscv64 box.

This gives Phase 2 exactly the property the old plan wanted — deterministic frames from an
injected measure function, no window — while testing the **real emitted solver** rather than a
Rust model of it that could silently diverge. Layout tests become ordinary
`tests/rt-behavior/app/layout-*` golden `.run` files. `app::frame` (§5) makes the same frames
assertable from inside a normal GUI program too.

### 8.2 Layout ownership is native (locked)

The native-side container layer owns layout *execution*: it calls `_mfb_rt_app_layout`
(a) inside `host_present` after a `sync` applies config changes, and (b) **autonomously on a
native resize** — a user drag-resize re-lays-out immediately on the main thread using the
layout config it already holds, with no worker round-trip and no `sync`. This works because
the worker-side shadow holds no authoritative geometry (§7): the native side keeps the full
layout config per node (pushed at `sync`), and all positional state lives natively. The
resize still enqueues a wakeup for `app::poll` and updates the window-size shadow and the
`app::frame` mirrors at the next `sync`, but pixels never wait on the worker. The solver
remains one shared, platform-independent routine; "native ownership" is about who *invokes*
it and where its outputs live, not per-platform layout code.

Because the solver is invoked from the native side at arbitrary times (present, resize, and
in plan-13-C, scroll), it must be **re-entrant-safe on the main thread and allocation-free** —
that is a hard constraint on the emitter, not a nice-to-have.

### 8.3 Solver semantics

- **`Direction.Stack`**: children overlap, sharing the container's content rect; z-order =
  child order (last child on top). `Justification` is ignored; each child is placed by
  `Align` on *both* axes (default `Stretch` fills). One container type and one solver cover
  Row / Column / Stack — the solver branches on `Direction`.
- **`Size` `< 0` = fill** is flex-grow (binary; no basis/min/max/wrap in v1 — the contract is
  written down so the solver's limits are explicit). Reachable per-widget via
  `app::setSize(w AS RES app::Widget, …)` (§5); an empty `Container` given a fill `Size` is
  the flexible-spacer idiom (`flex:1` strut) — there is **no dedicated `Spacer` widget in
  v1** (justify-content covers uniform distribution; the fill-container covers asymmetric
  pushes). A `Spacer` convenience can slot in later over the unchanged seam.
- **Measurement model — v1 single-pass, multi-pass-ready.** v1 calls `host_measure` **once
  per dirty leaf** and treats the returned `Size` as a fixed intrinsic; no height-for-width,
  no wrapping. But the seam is already shaped for the multi-pass future: `host_measure(handle,
  avail)` *takes* the available-space constraint, so a later solver can re-measure a leaf
  under a resolved width (e.g. natural width first, then wrapped height) without a signature
  change. Two contract rules make that safe and must hold from v1: (1) `host_measure` is a
  **pure, side-effect-free query** — same `(handle, avail)` ⇒ same `Size`, repeatable any
  number of times within one `host_present`; (2) the solver treats measurement as a **function
  it may call N times**, never a one-shot value cached on the node. So going multi-pass later
  changes only the solver's internal pass count — not the API, not the seam, not the backends.
  v1 passes `avail` = unconstrained (or the container's main-axis extent) and ignores
  re-measurement; that's the only thing a future version relaxes.
- **Per-platform `host_set_frame` cost.** macOS is a direct `view.frame = rect`. GTK4 has no
  direct setFrame — use `GtkFixed` (`put`/`move` + measured allocation); a custom
  `GtkLayoutManager` is "correct" but needs GObject subclassing from codegen, and unlike the
  Obj-C side (where `objc_allocateClassPair` is already used from codegen — §8.0) GTK has no
  equally cheap runtime-subclassing path, so `GtkFixed` is the pragmatic pick. Prototype
  `host_set_frame` on GTK4 before committing — it is the one genuinely awkward seam call. (The
  existing `-app` GTK backend already targets GTK4: `src/target/linux_gtk/mod.rs` binds
  `libgtk-4.so.1`.)

## 9. Resolved: non-blocking sync + `app::poll` for pacing

**Decision locked.** `app::sync`, all mutators, and all getters are non-blocking (non-blocking
pipe drain + async batch post; shadow reads). The single blocking function is
**`app::poll(win, timeout)`**, which blocks **by default**: `poll(win)` / `timeout = 0` blocks
until the next event (event-driven); `poll(win, t>0)` waits up to `t` ms with early wakeup
(frame tick). It parks the worker in `poll(2)` on the event pipe's read fd (§7). The canonical
loop (§6) is `poll` then `sync`; no external `sleep` primitive is required.

**Also locked — three poll/sync rules:**

- **`poll` before that window's first `sync` returns FALSE immediately**, regardless of
  timeout. Native objects realize lazily at the first `sync` (§7), so before it nothing can
  ever write to the event pipe; parking would deadlock a poll-first loop's first
  iteration. `poll` stays a pure wait — it never flushes or drains (sync and events are
  separate concerns).
- **Handler writes flush at the *next* `sync`.** `sync` is the only flush point, so a loop
  whose handlers write properties (§6's `setLabel`, §13's display) uses a timed
  `poll(win, t)` to bound the write-to-pixels latency at ~`t` ms; a fully-event-driven
  `poll(win)` would defer the flush to the next user event. Pure `poll(win)` is for loops
  that only read/observe.
- **`poll(win, t < 0)` raises `ErrInvalidArgument`.** `0` already means wait-forever, and a
  zero-wait check is unnecessary: `poll` does not drain, so "check without waiting" is
  exactly "don't call `poll`". Reserving `-1` for a non-blocking check was considered and
  rejected — it would be a second way to spell "call `sync` and look".

## 10. Language-support checkpoints (verify before/while building)

> **Note on citations.** An earlier draft of this section cited `src/typecheck/*`. That
> directory does not exist; the type checker lives in **`src/syntaxcheck/`** (and per plan-20,
> a large body of semantic rules moved to `src/ir/verify/`). Line numbers below were
> re-verified on 2026-07-09.

- **RES params are borrow-only — VERIFIED in code.** `is_resource_type` ⇒ `ExprMode::Borrow`
  for every resource arg (`src/syntaxcheck/types.rs:328`; spec `15_resource-management.md`).
  The *only* consuming modes are a registered close op's first arg and `thread::transfer`. So
  every `app::` function except `app::close`/`app::destroy` borrows its handle —
  `slot`/`attach`/`getVisible`/`setVisible`/`getSize`/`setSize`/`frame`/`getLabel`/`setLabel`
  never consume, and re-attach-after-`remove` / orphan-after-`close` (§2) is sound.
- **Variant→union widening in `compatible()` — VERIFIED, but only for *bindings*.**
  `compatible()` implements union subsumption (`src/syntaxcheck/types.rs:145-170`): an actual
  whose bare name is one of a `UNION`'s variants is accepted. `tests/rt-behavior/resources/
  resource-union-valid` exercises `File→Stream` — **at a `RES` binding initializer
  (`RES s AS Stream = fs::createTempFile()`), never at a call site.** No test in the tree
  passes a concrete resource into a resource-union *parameter*.
- **Resource-union parameters are currently FORBIDDEN by the spec — this plan must amend it.**
  `mfb spec language resource-management` states: *"A resource value may be passed only to a
  function whose parameter is declared RES and explicitly names that **concrete** resource
  type… There is no generic resource supertype, no structural matching of handles, and no
  implicit conversion between resource types."* `app::Widget` as a parameter type is exactly
  what that sentence prohibits. Landing `app::` therefore requires a **deliberate, specified
  language change**, not an incidental one:
  - **Amendment (narrow, directional):** a `RES` parameter may name a **resource union**; an
    actual of any variant type widens to it. Widening is only ever variant→union, never the
    reverse, so ops that need a concrete handle — every **registered close op**,
    `thread::transfer`, `thread::accept` — keep concrete-typed parameters and are unaffected
    with **no blocklist and no exemption table**. `compatible()` already enforces exactly this
    direction. Binding a union (`RES w AS app::Widget = …`), returning one, and consuming one
    stay as they are today; only the borrow-parameter position opens up.
  - Update `src/docs/spec/language/15_resource-management.md` in the same commit, and add
    `tests/syntax/resources/resource-union-param-valid` (variant into a union param) plus
    `resource-union-param-invalid` (a union actual into a concrete param — must still be
    rejected, proving directionality).
- **Builtin union params — THREE sites, not one.** The earlier draft called this "a global fix
  at the `term::` seam." It is not: `term::` has no overloads and no union parameters
  (`builtins::term::param_types` is one flat type list, `builtins::term::arity` one
  `(min, max)` pair). Arg types reach three independent checkers, and all three must learn
  the same rule:
  1. **`src/syntaxcheck/builtins.rs`** — the type checker. Follow the `term::` shape
     (`check_term_builtin_call`, `src/syntaxcheck/builtins.rs:879`) but generalize
     `param_types` from one flat list to a **per-overload table**, and select the overload
     whose params are all `expression_compatible()` with the actuals. Note `term::` infers its
     args in `ExprMode::Read`; `app::` must use `ExprMode::Borrow` for resource params.
  2. **`src/builtins/app.rs::resolve_call(name, arg_types: &[String])`** — called from
     `src/ir/lower.rs` (~:2131 onward) with **no access to the type registry**; every existing
     package does context-free `exact()` string matching (`src/builtins/net.rs:172`). It cannot
     see that `app.Button` is a variant of `app.Widget`. Fix: have `app.rs` own a **static
     variant table** (`WIDGET_VARIANTS: &[&str]`) and match with a `widget_or(name)` predicate
     instead of `exact`. This is a small, package-local, rot-proof duplication of one fact
     (the union's variant list) that a `#[test]` pins against the registered union.
  3. **`src/ir/verify/mod.rs`** — per plan-20 the sole rejecter on both paths; its own
     `compatible()` (`src/ir/verify/mod.rs:3411`) must accept the same widening for builtin
     call args.
- **ABI of a `RES app::Widget` argument — locked: the raw handle, not a tagged union.** A
  bound resource union carries a tag the compiler writes from the statically-known initializer
  type. A *parameter* has no such site, and materializing a tagged temporary per call would be
  pure waste. Since `app::` widget handles are pointers to shadow nodes and **a shadow node
  already carries its own kind byte**, a `RES app::Widget` argument lowers to exactly the same
  single pointer as a `RES app::Button` argument. The native side reads the kind from the
  node. Nothing is erased and nothing is boxed. Write this into the amendment: *widening a
  resource variant to a resource union in a borrow-parameter position is representation-neutral.*
- **A registered close op per widget type** (§2): `app::close` for `Window`; `app::destroy`
  overloads for `Container`/`Button`/`Label`/`Input`. Without them, scope-drop of a `RES ok AS
  app::Button` has nothing to call, and `app::Widget`'s tag-dispatched union drop has no
  per-variant target. A close op must name a **concrete** type, so there is deliberately no
  `app::destroy(w AS RES app::Widget)`. The `app::destroy` overloads are registered as internal
  close ops **only** — they are not added to the user-callable builtin surface (§2), so
  `app::destroy(...)` in user code is an unknown function. Register the close-op name in the
  resource registry (`ResourceInfo.close_function`, resolved by drop lowering) without a matching
  entry in the `app::` call table; `app::close` is the sole exported close op.
- **Omitted arguments must be trailing; defaults do not combine with overloading** (§5.0).
  `mfb spec language functions` line 38, and `normalize_builtin_call_arguments`
  (`src/syntaxcheck/builtins.rs:1701`) which reports `TYPE_CALL_ARITY_MISMATCH` for a skipped
  middle parameter. Every signature in §5 and every call in §6/§13 obeys this. The named-arg
  overload table (`builtins::call_param_name_overloads` /
  `builtins::select_param_name_overload`, `src/builtins/mod.rs:411-437`) selects by
  count-and-names, so **two `app::` overloads sharing an arity must share parameter names**
  (`addContainer`'s `Window` and `Container` forms do) **or differ in argument type** so
  `resolve_call` separates them. A `#[test]` must assert this property over the whole `app::`
  table.
- **Builtin record type IDs** for `Size`/`Rect`/`Spacing`/`Widget` must use the high reserved
  range (see the `term::` `TermColor`/`TermSize` precedent — `FIRST_TABLE_TYPE_ID` collision).
- **Static `app::window` detection** reuses the `uses_term` whole-program flag pattern
  (`src/target/shared/code/mod.rs:590`, a runtime-symbol-presence scan) to select GUI vs
  transcript sub-mode at build time (§7).
- **Cross-thread primitives** (§7, §8.0): there are **no atomic instruction encoders** in
  `src/arch/*`, and none are needed. Reuse the existing `-app` channels: a `pipe(2)` for
  main→worker events (`app::poll` = `poll(2)` on its read fd, giving the timeout for free) and
  `performSelectorOnMainThread:…waitUntilDone:NO` / `g_idle_add` for the worker→main command
  batch. No shared mutable memory ⇒ no mutex, no condvar. Two hazards to check per backend:
  the event pipe must **not** be `dup2`'d onto fd 0 (that is the transcript's input path, and
  in GUI sub-mode fd 0 is the real stdin), and its write end must be `O_NONBLOCK` with a
  main-thread-only coalescing fallback on `EAGAIN` so a slow worker can never stall the UI.
- **Main-thread layout** (§8): the solver is an emitted, allocation-free, re-entrant MIR
  helper `_mfb_rt_app_layout` invoked from `host_present` and from the native resize handler.
  Confirm each backend exposes a synchronous intrinsic-size query (`fittingSize` /
  `gtk_widget_measure`) callable during present/resize.

## 11. Phases

Ordered lowest-risk / independently-landable first. Each phase lists its concrete tasks and
the acceptance criterion that must be verified before it is done; fill in `Commit:` with the
hash(es) that land it.

### Phase 0 — Language amendment: resource-union parameters

The one prerequisite that is a *language* change rather than a package.

- [ ] Amend `src/docs/spec/language/15_resource-management.md`: a `RES` parameter may name a resource union; widening is variant→union only, in borrow position only, and is representation-neutral (§10).
- [ ] Teach the three checkers (§10): `src/syntaxcheck/builtins.rs` (per-overload `param_types` + `expression_compatible` selection, `ExprMode::Borrow` for resource args), the package's `resolve_call` variant predicate, `src/ir/verify/mod.rs::compatible`.
- [ ] Tests: `tests/syntax/resources/resource-union-param-valid` (variant → union param) and `resource-union-param-invalid` (union actual → concrete param still rejected; a close op still refuses a union) — proving the widening is directional.

Acceptance: a user-declared `UNION Stream { File Socket }` can be passed a `File` in a `RES s AS Stream` **parameter** on all three paths, while `fs::close(s AS Stream)` remains a compile error; the existing resource suite is unchanged.
Commit: —

### Phase 1 — Package skeleton & types

Register the `app::` surface so programs typecheck against it; no native backend yet.

- [ ] Register the `app::` builtin package; declare resources `Window`/`Container`/`Button`/`Label`/`Input`, the `Widget` resource union, `Size`/`Rect`/`Spacing` records, and enums `Direction`/`Justification`/`Align`/`ClickMode` (§3, §4).
- [ ] Reserve builtin record type IDs for `Size`/`Rect`/`Spacing`/`Widget` in the high reserved range (§10 — `term::` `FIRST_TABLE_TYPE_ID` collision precedent).
- [ ] Declare every function as an explicit **arity × type overload set** with all optional parameters trailing (§5.0); add the `#[test]` asserting no two same-arity overloads disagree on parameter names unless their argument types separate them.
- [ ] Register `app::close` as `app::Window`'s (exported) close op and the four `app::destroy` overloads as the child widgets' **internal** close ops — registry-only (`ResourceInfo.close_function`), NOT added to the user-callable `app::` call table — so scope-drop and union tag-dispatch have a target while `app::destroy(...)` in user code stays an unknown function (§2, §10).
- [ ] Stub every function against the (not-yet-built) shadow tree.
- [ ] Tests: `tests/func_app_*_valid/**` and `_invalid/**` covering arity, overload resolution (`Window` vs `Widget`, `Window` vs `Container`), variant→union widening acceptance, RES-borrow rejection, use-after-`close` rejection (on a `Window`), `app::destroy(...)` in user code rejected as an **unknown function** (the close op is internal, not exported), and the skipped-middle-argument rejection (`TYPE_CALL_ARITY_MISMATCH`).

Acceptance: programs typecheck against the full `app::` surface — incl. `Widget`-union overload resolution, per-widget close ops, and RES-params-are-borrow-only — with no native window created.
Commit: —

### Phase 2 — Shadow tree + dirty model + layout solver + headless host

The worker-side model, the emitted solver, and the backend that makes both testable without a display.

- [ ] Build the worker-side shadow tree: `window`/`add*`/`remove`/`attach`/`slot` create and mutate nodes with parent/child links, per-node dirty + structure-dirty flags, and per-node property shadows (§7). Enforce the window's one-root-child rule (`ErrInvalidArgument`).
- [ ] Emit `_mfb_rt_app_layout` (`src/target/shared/code/app_layout.rs`): allocation-free, re-entrant, main-thread-callable; walks a flat node array + an indirect `host_measure` fn-ptr; produces one `Rect` per node for Row / Column / Stack across the full `Justification × Align × Size`(`<0` fill) matrix, honoring padding/margin and skipping `display:none` nodes (§8.1, §8.3). **Budget: the largest single item in this plan** (~1500–2500 lines of emitter).
- [ ] Keep the solver multi-pass-ready: it *calls* the measure fn-ptr (single-pass per leaf in v1) rather than caching a baked size (§8.3 measurement contract).
- [ ] Implement the `headless` host backend (`src/target/shared/widgets/headless`) + `mfb build -app --app-host headless`: synthetic deterministic `host_measure`, `host_set_frame` printing `id kind x y w h`, immediate-FALSE `host_wait_events`, window closes after the first `sync` (§8.1).
- [ ] Tests: `tests/rt-behavior/app/layout-*` golden `.run` files driving the **real emitted solver** through the headless host — one per `Direction × Justification × Align` combo, plus `<0`-fill flex-grow, padding/margin, and hidden-sibling reflow. Plus headless model tests that shadow-tree mutations (`add`/`remove`/`attach`/`slot`) update links + dirty flags as specified.

Acceptance: the emitted solver produces correct frames for the full matrix under the headless host on macOS **and** Linux (no display server), byte-identical between them; shadow-tree mutations behave as specified.
Commit: —

### Phase 3 — Mode selection + host-protocol seam + macOS backend

Bring up a real macOS window through the §8 seam and the Phase-2 solver.

- [ ] Add static whole-program detection of an `app::window` call (the `uses_term` mechanism) to select GUI vs transcript sub-mode; in GUI sub-mode skip the transcript window and keep `io::`/`term::` on console lowering (§7).
- [ ] Define the §8 host-protocol seam (`src/target/<platform>/widgets`, against the `Asm`/`abi` builder).
- [ ] Stand up the **event pipe**: its own fd pair (never `dup2`'d onto fd 0 — §8.0), `O_NONBLOCK` write end, fixed-size records, main-thread coalescing fallback on `EAGAIN`.
- [ ] Implement the macOS backend: `NSWindow` + plain `NSView` containers + `NSButton` + `NSTextField` (non-editable = `Label`, editable = `Input`). Button target/action reuses the runtime-class recipe already used for `MFBTextView.keyDown:` — `objc_allocateClassPair` + `class_addMethod` + `objc_registerClassPair` (`macos_aarch64/app/bootstrap.rs:109-124`) — with the node id in the instance's indexed ivars (`app/mod.rs:317`).
- [ ] Implement `app::sync` (non-blocking pipe drain + `performSelectorOnMainThread:…waitUntilDone:NO` post of an owned command batch, main thread frees it) and `host_present` (call `_mfb_rt_app_layout` with `fittingSize`/`intrinsicContentSize` as the measure fn-ptr, `setFrame` every node, return immediately — non-blocking).
- [ ] Native resize re-invokes `_mfb_rt_app_layout` autonomously on the main thread (layout ownership is native; no worker `sync` involved — §8.2); the worker shadow gains only the read-only `frame` mirror.

Acceptance: on-device, a `window` + `Column`/`Row` + `Button`/`Label` program lays out to the same frames the headless host produced for the same tree (asserted via `app::frame`), stays live under `[NSApp run]`, and re-flows on drag-resize without the worker running; no transcript window appears and `io::print` goes to console stdio.
Commit: —

### Phase 4 — Events + pacing + Input I/O

Native events drained at `sync`; the `poll` wait primitive; the `Input` round-trip.

- [ ] Single/double click event records: the button's target/action (macOS) and `GtkGestureClick` (GTK) classify each press by native `clickCount`/`n_press` and push a `Click` or `DoubleClick` record into the event pipe; `sync` folds them into per-node shadow counters (§7).
- [ ] Optional `ClickMode.Exclusive` (`host_set_click_mode`): defer the single via a main-thread timer of the *system* double-click interval; default path stays timer-free (§7).
- [ ] Window-close record → `app::isOpen`; resize record → window-size shadow; frame-report records → the `app::frame` mirror.
- [ ] Implement `app::poll` / `host_wait_events` as `poll(2)`/`ppoll(2)` on the event pipe's read fd; returns FALSE immediately (never parks) before that window's first `sync`; `ErrInvalidArgument` on a negative timeout (§9).
- [ ] `Input` I/O: `host_input_drain` pulls text + user-edited + Enter-submit latches into the shadow at `sync` (→ `getValue`/`valueChanged`/`submitted`), with program-set-wins-this-frame precedence on a dirty `value` (§7).
- [ ] Backpressure: fill the pipe from a program that stops calling `sync`, confirm the UI stays responsive (non-blocking write end) and that no click is lost once `sync` resumes (main-thread coalescing fallback — §7).
- [ ] Tests: `tests/func_app_*_valid/**` for the event/Input functions.

Acceptance: on-device, `clicked`/`doubleClicked`/`isOpen` behave correctly under an event-driven `poll(win)` loop (no busy-spin; `doubleClicked` correct where "two clicks per frame" never happens; `Exclusive` drops the stray single); `setValue` does not echo as `valueChanged`; `submitted` latches on Enter independently of `valueChanged`; a stalled worker never freezes the window and loses no clicks.
Commit: —

### Phase 5 — Linux/GTK4 backend

Same seam, GTK4 widgets, the Phase-2 solver reused unchanged.

- [ ] Implement the GTK4 backend against the §8 seam: `GtkFixed` containers + `GtkButton`/`GtkLabel`/`GtkEntry`; `host_measure` via `gtk_widget_measure`; `host_set_frame` via `GtkFixed` put/move + allocation; `host_present` via GTK main-context iteration.
- [ ] Wire events with `g_signal_connect_data` (already in the `-app` symbol table, `linux_gtk/mod.rs:662`) and post the command batch with `g_idle_add` (`linux_gtk/app_io.rs:138`) — the same calls the transcript mode uses (§8.0).
- [ ] Reuse the emitted `_mfb_rt_app_layout` unchanged (identical layout to macOS).
- [ ] Prototype `host_set_frame` on GTK4 first — the one genuinely awkward seam call (§8.3).

Acceptance: the canonical program runs on the Debian aarch64 box (per plan-05) with frames identical to macOS and to the headless host.
Commit: —

### Phase 6 — Lifetime & detach correctness

The §2 detach-not-destroy model and orphan handling, end to end.

- [ ] `remove`/`close` detach rather than destroy; on `close`, retain + reparent every live descendant to an offscreen holder before destroying the native window (§2, §7).
- [ ] Verify each registered close op fires exactly once: `app::close` (exported) + a window's scope-drop never double-fire, and each internal `app::destroy` overload fires once at its widget's scope-drop (there is no explicit-call path to double it); support re-attach of a detached widget to a new window; verify `app::Widget`'s tag-dispatched union drop is never reached (the type is parameter-only).
- [ ] Tests: remove→re-attach; close-window-then-reuse-widget; widget scope-drop destroys the native peer + shadow node exactly once (no explicit `destroy` call exists — the op is internal); scope-drop teardown ordering (reverse declaration order); churn-in-a-scoped-collection frees every widget at the owning collection's scope exit with no accumulation (§2).

Acceptance: every handle is destroyed exactly once at its own binding drop or explicit close op; detached/orphaned widgets stay valid and re-attach correctly; no double-free across `close` + scope-drop; no leaks under the leak checker.
Commit: —

### Phase 7 — Polish, examples, docs

- [ ] `setVisible` reflow (display:none), `size`/`resizable`/`title` live updates; `<0` fill/clamp semantics for `Size`/`Spacing`; `app::frame` mirror.
- [ ] Ship the §13 calculator worked example; add spec/man docs for the `app::` surface (`.ai/man_template.md`, `.ai/man_type_template.md`, `.ai/man_package_template.md`).
- [ ] Tests: acceptance (`scripts/test-accept.sh`) green; runtime proof via the worked example.

Acceptance: the calculator example builds and runs on both backends; docs/spec updated (incl. the §10 resource-union-parameter amendment); acceptance suite passes. Remove this plan's `Last updated` line in the commit that lands this phase.
Commit: —

## 12. Naming (locked: get/set)

All read/write properties use uniform `getX`/`setX` pairs overloaded on handle type:
`getVisible`/`setVisible`, `getTitle`/`setTitle`, `getSize`/`setSize`,
`getResizable`/`setResizable`, `getLabel`/`setLabel`, `getClickMode`/`setClickMode`. The
earlier `show`/`hide` verbs are folded into `setVisible(h, TRUE/FALSE)`; the earlier
prefix-less setters (`title`/`size`/`resizable`) and the bare `visible` getter are gone.

Read-only observed state is deliberately *not* `get*` — it keeps `is*`/event/noun naming
because it has no setter: `app::isOpen` (window closed by the user; close via `app::close`),
`app::clicked` (an edge event drained at `sync`), and `app::frame` (computed geometry, owned
natively). Rule of thumb: a property the program both reads and writes → `getX`/`setX`; state
the program can only observe → `isX` / event verb / bare noun.

The one exported lifetime op is a verb: `app::close(win)`. Widgets have no exported lifetime
op — `app::destroy` is the internal close op scope-drop calls, never user code (§2).

## 13. Worked example: a (crappy) calculator

A complete `mfb build -app` program — the macOS-calculator layout from the mock: a big
`Label` display over a 5×4 grid of `Button`s, no `Input` widget. It exercises the whole v1
surface that matters: `window` → `Column` root → `Row` containers → `Button`/`Label`, the
`poll`/`sync` event loop, `clicked`, and `setLabel`. Styling (fonts, colors, the orange
operator column, button radii) is all absent — that's fine; this is about the structure and
the control flow, not the pixels.

All calculator state lives in one `Calc` record updated functionally with `WITH` (mirroring
the §20 worked-example style), so the button handlers are pure and the only side effects are
`setLabel` on the display.

```basic
IMPORT app
IMPORT io
IMPORT strings

TYPE Calc
  display AS String     ' what the screen shows
  acc     AS Float      ' running accumulator
  op      AS String     ' pending operator: "+" "-" "*" "/" or "" for none
  fresh   AS Boolean    ' TRUE => the next digit starts a new number
END TYPE

' NOTE: the single-line `IF ... THEN <stmt>` form admits only ELSE, never ELSEIF or
' END IF (`mfb spec language control-flow`). Multi-branch chains use the block form.
FUNC compute(acc AS Float, op AS String, x AS Float) AS Float
  IF op = "+" THEN
    RETURN acc + x
  ELSEIF op = "-" THEN
    RETURN acc - x
  ELSEIF op = "*" THEN
    RETURN acc * x
  ELSEIF op = "/" THEN
    RETURN acc / x     ' x = 0 is guarded by settle() before we get here
  END IF
  RETURN x
END FUNC

' Fold the pending operator against the number currently on screen.
FUNC settle(c AS Calc) AS Calc
  LET text = strings::stripSuffix(c.display, ".")        ' tolerate a mid-entry "12."
  IF NOT isNumeric(text) THEN
    RETURN Calc[display := "0", acc := 0.0, op := "", fresh := TRUE]
  END IF
  LET x = toFloat(text)
  IF c.op = "" THEN RETURN WITH c { acc := x }
  IF c.op = "/" AND x = 0.0 THEN
    RETURN Calc[display := "Error", acc := 0.0, op := "", fresh := TRUE]
  END IF
  LET r = compute(c.acc, c.op, x)
  RETURN WITH c { acc := r, display := toString(r) }
END FUNC

FUNC pressDigit(c AS Calc, d AS String) AS Calc
  IF c.fresh THEN RETURN WITH c { display := d, fresh := FALSE }
  IF c.display = "0" THEN RETURN WITH c { display := d }       ' no leading zeros
  RETURN WITH c { display := c.display & d }
END FUNC

FUNC pressDot(c AS Calc) AS Calc
  IF c.fresh THEN RETURN WITH c { display := "0.", fresh := FALSE }
  IF strings::contains(c.display, ".") THEN RETURN c          ' only one point
  RETURN WITH c { display := c.display & "." }
END FUNC

FUNC pressOp(c AS Calc, newOp AS String) AS Calc
  LET s = settle(c)
  IF NOT isNumeric(s.display) THEN RETURN s                   ' "Error" sticks
  RETURN WITH s { op := newOp, fresh := TRUE }
END FUNC

FUNC pressEquals(c AS Calc) AS Calc
  IF c.op = "" THEN RETURN c
  LET s = settle(c)
  RETURN WITH s { op := "", fresh := TRUE }
END FUNC

FUNC pressNegate(c AS Calc) AS Calc
  IF c.display = "0" OR NOT isNumeric(c.display) THEN RETURN c
  IF strings::startsWith(c.display, "-") THEN
    RETURN WITH c { display := strings::stripPrefix(c.display, "-") }
  END IF
  RETURN WITH c { display := "-" & c.display }
END FUNC

FUNC pressPercent(c AS Calc) AS Calc
  LET text = strings::stripSuffix(c.display, ".")
  IF NOT isNumeric(text) THEN RETURN c
  RETURN WITH c { display := toString(toFloat(text) / 100.0), fresh := TRUE }
END FUNC

FUNC pressClear(c AS Calc) AS Calc
  RETURN Calc[display := "0", acc := 0.0, op := "", fresh := TRUE]
END FUNC

SUB main()
  ' `title` is app::window's FIRST parameter, so naming it skips nothing (§5.0).
  RES win  = app::window(title := "Calculator")
  ' addContainer's order is (parent, dir, align, justify, padding) — `align` precedes
  ' `justify` so the common stretch-the-rows case needs no filler argument (§5.0).
  RES root = app::addContainer(win, dir := app::Direction.Column,
                               align := app::Align.Stretch)
  RES disp = app::addLabel(root, label := "0")

  RES row1 = app::addContainer(root, dir := app::Direction.Row)
  RES bAC  = app::addButton(row1, label := "AC")
  RES bNeg = app::addButton(row1, label := "+/-")
  RES bPct = app::addButton(row1, label := "%")
  RES bDiv = app::addButton(row1, label := "÷")

  RES row2 = app::addContainer(root, dir := app::Direction.Row)
  RES b7   = app::addButton(row2, label := "7")
  RES b8   = app::addButton(row2, label := "8")
  RES b9   = app::addButton(row2, label := "9")
  RES bMul = app::addButton(row2, label := "×")

  RES row3 = app::addContainer(root, dir := app::Direction.Row)
  RES b4   = app::addButton(row3, label := "4")
  RES b5   = app::addButton(row3, label := "5")
  RES b6   = app::addButton(row3, label := "6")
  RES bSub = app::addButton(row3, label := "−")

  RES row4 = app::addContainer(root, dir := app::Direction.Row)
  RES b1   = app::addButton(row4, label := "1")
  RES b2   = app::addButton(row4, label := "2")
  RES b3   = app::addButton(row4, label := "3")
  RES bAdd = app::addButton(row4, label := "+")

  RES row5 = app::addContainer(root, dir := app::Direction.Row)
  RES b0   = app::addButton(row5, label := "0")
  RES bDot = app::addButton(row5, label := ".")
  RES bEq  = app::addButton(row5, label := "=")

  MUT state AS Calc = Calc[display := "0", acc := 0.0, op := "", fresh := TRUE]

  WHILE app::isOpen(win)
    app::poll(win, 10)       ' timed tick (<= 10 ms, waking early on a click): the setLabel
                             '   below flushes on the NEXT sync, so the display updates
                             '   within 10 ms of a press (§9). poll waits; sync reconciles.
    app::sync(win)           ' drain native events into the shadow; flush the dirty label

    IF app::clicked(bAC) THEN
      state = pressClear(state)
    ELSEIF app::clicked(bNeg) THEN
      state = pressNegate(state)
    ELSEIF app::clicked(bPct) THEN
      state = pressPercent(state)
    ELSEIF app::clicked(bDiv) THEN
      state = pressOp(state, "/")
    ELSEIF app::clicked(b7) THEN
      state = pressDigit(state, "7")
    ELSEIF app::clicked(b8) THEN
      state = pressDigit(state, "8")
    ELSEIF app::clicked(b9) THEN
      state = pressDigit(state, "9")
    ELSEIF app::clicked(bMul) THEN
      state = pressOp(state, "*")
    ELSEIF app::clicked(b4) THEN
      state = pressDigit(state, "4")
    ELSEIF app::clicked(b5) THEN
      state = pressDigit(state, "5")
    ELSEIF app::clicked(b6) THEN
      state = pressDigit(state, "6")
    ELSEIF app::clicked(bSub) THEN
      state = pressOp(state, "-")
    ELSEIF app::clicked(b1) THEN
      state = pressDigit(state, "1")
    ELSEIF app::clicked(b2) THEN
      state = pressDigit(state, "2")
    ELSEIF app::clicked(b3) THEN
      state = pressDigit(state, "3")
    ELSEIF app::clicked(bAdd) THEN
      state = pressOp(state, "+")
    ELSEIF app::clicked(b0) THEN
      state = pressDigit(state, "0")
    ELSEIF app::clicked(bDot) THEN
      state = pressDot(state)
    ELSEIF app::clicked(bEq) THEN
      state = pressEquals(state)
    END IF

    app::setLabel(disp, state.display)   ' dirty-flagged; pushed to native on the next sync
  WEND
  ' scope exit drops in reverse: every button/row/label first (each via its registered
  ' app::destroy close op), then `win` last — `win`'s app::close op tears down the
  ' native window.
  EXIT SUB

  TRAP(err)
    io::print("calculator error: " & err.message)
    EXIT SUB
  END TRAP
END SUB
```

Notes tying it back to the design:

- **Layout** is one `Column` root (`align := Stretch` so each row fills the width) holding the
  display `Label` and five `Row` containers. No flex justification gymnastics are needed for a
  uniform grid; the solver (§8) sizes the rows from the buttons' intrinsic sizes.
- **Every named argument names a parameter no later than the last supplied one** — the §5.0
  rule. `app::window(title := ...)` works because `title` moved to first; `addContainer(root,
  dir := ..., align := ...)` works because `align` precedes `justify`.
- **The loop is a timed tick**: `poll(win, 10)` parks the worker up to 10 ms, waking early on
  any press. The display's `setLabel` is written *after* this frame's `sync`, so it flushes on
  the **next** sync — at most 10 ms later. A fully-event-driven `poll(win)` would defer that
  flush to the next user event (the display would lag one press behind, §9); the cost of the
  tick is one cheap shadow-scan per 10 ms while idle, still no busy-spin.
- **`clicked` is read off the shadow** (§7), so the long `ELSEIF` chain is just cheap
  shadow reads — at most one fires per frame, and none of them touch native.
- **No `Input`** — the display is a `Label` the program owns and writes; the user only ever
  drives state through buttons. (Swapping in `app::addInput` + `submitted` would turn this into
  an expression-entry calculator, but that's beyond the mock.)
- **`setLabel` every frame** is intentionally naive — it re-marks the node dirty each loop. For
  v1 that's harmless (one node, re-pushed at sync); a tidier version would set it only when
  `state.display` changed.
