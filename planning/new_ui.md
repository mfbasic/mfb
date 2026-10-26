# UI package: native widget trees (`ui::`)

Introduce `ui::` for native widgets laid out by a flex-based `Layout`. `app::` stays
responsible for presentation modes; `canvas::` stays responsible for drawing and is
**not touched** by the MVP (see "Future work").

## Scope

MVP:

- `app::Mode.UI` presenting a native widget tree in the main application window.
- One layout mechanism: `ui::Layout`, a CSS-flexbox-style container. There is no
  other positioning model, now or later.
- Widgets are the **target's native widgets** (AppKit / GTK4 / Win32). MVP set:
  `Button`, `Label`, `Input`.
- `ui::present` and `ui::pollEvent` as the only two channels between the program
  and the screen.

Deferred (own plans later): `Table`, an embedded `ui::Canvas` widget, any
`canvas::` integration. riscv64 has no app mode today
(`AppSupport::Unsupported`) and `Mode.UI` does not change that.

## Design laws

These are the invariants everything below follows from:

1. **Two channels, both value-copying.** The worker sends intent out through
   `ui::present` (a deep copy of the tree, scene-ring style) and receives facts
   back through `ui::pollEvent` (a bounded native→worker event queue). Nothing
   else crosses the thread boundary. This is what makes `ui::` thread-safe by
   construction.
2. **The screen changes only at `present`, and `present` is retained-mode, not a
   frame loop.** A presented tree stays up indefinitely: the normal pattern is
   present once (or only when something changes), then just poll for events. A
   static presented UI costs zero ongoing work — no timer, no repaint loop; the
   main thread runs reconcile only at a `present`, a close-wake, or a window
   resize, and the native toolkit handles all drawing (hover, repaint, expose)
   itself. Mutating a descriptor or the tree is invisible until the next
   `ui::present`. The single exception: a widget whose resource is closed is
   removed from the native tree at the next reconcile without waiting for a
   `present` (see lifetime rules below).
3. **Descriptors are the complete desired state.** Every `present` applies the
   descriptors as truth. The native side keeps no MFB-visible state of its own
   beyond what it reports through events.
4. **The program must fold event values back into its descriptors before the next
   `present` — if there is one.** An `Input` the user has typed into reports the
   text via `InputChanged`; a program that re-presents without updating the
   input's descriptor presents the old text, and the old text is what the
   reconcile installs. A program that never re-presents never has to fold
   anything: the native input keeps the typed text and the program just reads
   events. This is the MFB developer's contract, stated in `mfb man ui`.
5. **No getters.** The worker never asks the native side "what is the input's
   text" or "is the button hovered". Everything the program can learn arrives as
   an event; a missing fact is a missing event type, not a missing getter.
6. **Interaction state is native.** Hover, pressed, focus, caret, selection belong
   to the native widget. The MFB program expresses what it wants (a button here,
   with this label) and polls for what happened (`ButtonClicked`).

## Presentation modes

- `app::Mode.UI` presents the native widget tree.
- `app::Mode.Canvas` retains the existing canvas-only window, unchanged.

Enter UI mode explicitly:

    app::setMode(app::Mode.UI)

Mechanics: `Mode.UI` is **appended** to the `app::Mode` enum (values come from
declaration order, `src/codegen/builtins/app/mod.rs`); `ui::` members are gated by
a new `ModeRequirement` variant in `src/codegen/app/hook/app.rs` (the existing
Canvas gates test `mode == 2` exactly and are untouched). `ui::` members in any
other mode raise `ErrWrongMode`.

## Threading model

Two threads — simpler than canvas's three. There is no GPU side and no graphics
thread: native toolkits require widget work on the main thread, and the flex
solver is cheap enough to run there.

| Thread | Owns |
|---|---|
| **main** (UI) | the window, the native widgets, the flex solver, the reconcile pass, the event pump, producing events into the queue |
| **worker** (language) | the program, the descriptor records, `ui::present`, `ui::pollEvent` |

Hand-off, scene-ring style (`.ai/canvas-threading.md` §3 is the template, minus
hashes and the frame counter — the "frame" here is the reconcile pass):

- `ui::present` validates the tree, deep-copies it into a fresh block **in the
  worker's arena**, publishes the pointer into process-global storage, and signals
  the main thread.
- The main thread's reconcile takes one snapshot of the published pointer and uses
  only it for the whole pass. It never frees a block.
- A displaced block is retired, and the worker frees retired blocks only after a
  reconcile has completed past them (same drain gate as the scene ring).
- The published copy stamps each widget's `WidgetId` and descriptor contents; the
  main thread never reads a live descriptor record.

Closing a widget (scope exit) sets the resource's closed flag on the worker and
signals the main thread; the next reconcile removes the native twin. The closed
flag is read before the native handle, never after (`.ai/canvas-threading.md` §7
discipline).

## Widget tree

A layout contains an ordered list of flex items: elements (native widgets) and
nested layouts. `Layout` and `Element` are data values; `ElementResource` is a
resource union of the native widget descriptor types.

    UNION ElementResource
        ui::Button
        ui::Input
        ui::Label
        ...
    END UNION

    TYPE Element
        grow AS Float
        shrink AS Float
        basis AS Float
        ...
        element AS RES ElementResource
    END TYPE

    UNION Widget
        ui::Element
        ui::Layout
    END UNION

    TYPE Layout
        visible AS Boolean
        direction AS LayoutDirection
        justification AS LayoutJustification
        align AS LayoutAlign
        gap AS Float
        ...
        widgets AS List OF Widget
    END TYPE

Notes:

- **`Element` is load-bearing, not ceremony.** `Widget` cannot be
  `UNION { RES ElementResource | Layout }`: a union mixing resource and data
  variants is rejected (`TYPE_MIXED_RESOURCE_UNION`). `Element` is the data
  record that makes the union legal, and it is the natural home for per-child
  flex-item properties (grow / shrink / basis / align-self).
- **Widget properties live in the descriptor, not the tree.** A button's label,
  an input's text, `disabled` — all fields of the widget resource (law 3). The
  tree carries structure and layout only. `Layout.visible` remains tree-level
  because it applies to a subtree.
- The recursion `Layout → List OF Widget → Layout` is legal: the cycle passes
  through a `List` and a `UNION` (`04_types.md` recursion rules). `Element` and
  `Widget` have no default value (a `RES` field / a union), which is fine — trees
  are built bottom-up.

## Widget creation

Widget creation produces a **descriptor** — a record in worker memory. No native
widget exists until a tree containing the descriptor is presented; the worker
never touches native UI.

    ui::createButton(...) AS RES ui::Button
    ui::createInput(...) AS RES ui::Input
    ui::createLabel(...) AS RES ui::Label

Each constructor produces a new resource following the standard MFB resource
rules and assigns it a process-unique `WidgetId`. Widgets have no user-facing
close function; the registry's required close member is internal-only (the
`process.__drop` pattern), setting the closed flag and waking the main thread.

Programmatic updates mutate the descriptor and take effect at the next `present`
(law 2): set the button resource's label field, present again.

## Resource lifetime and attachment

A widget resource belongs to its MFB cleanup scope. Bindings, record fields, and
collection slots hold references to that resource. Storing a reference in an
outer-scope layout follows the existing ownership-floats-up rules; the UI adds no
new ownership model.

The native presentation holds no reference to the resource at all — only its own
native twin, keyed by `WidgetId`. Attachment and resource lifetime are separate:

- Removing a widget from the MFB layout does not close it or immediately change
  the displayed tree.
- Presenting the updated layout detaches the omitted widget's native twin. The
  resource still exists and can be included in a later layout; reattachment
  creates a fresh native twin configured from the descriptor (native-held
  interaction state — focus, caret — is not preserved across detach).
- Removing references does not move cleanup inward. The resource closes when its
  owning scope exits, even if it was removed from the layout earlier.
- If scope cleanup closes a widget still attached to the last presented tree, the
  native side removes it without waiting for another `ui::present`. Its old slot
  closes up at that reconcile (the flex layout re-solves); the removed widget no
  longer draws or receives input events.

The user may never present an updated layout, so cleanup must remove a closed
widget independently of presentation (the close-wake above).

These ownership rules are defined in §15.6 of
`src/docs/spec/language/15_resource-management.md`. The implementation uses
`emit_resource_float` in `src/codegen/resource/owner.rs` and
`emit_owned_list_drain` in `src/codegen/cleanup/owned/builder_owned_cleanup.rs`
(inspect with `rg -n 'emit_resource_float|emit_owned_list_drain' src/codegen/resource/owner.rs src/codegen/cleanup/owned/builder_owned_cleanup.rs`).

Open bugs on this feature's critical path — the tree is exactly "lists of data
unions holding RES boxes", which stresses: bug-1029 (function value returning a
resource union leaks its box), bug-1073 / bug-1081 (`List OF RES` keeps old
blocks), bug-1022 (templates reject a union variant). Fix before, not during.

## Presenting the UI

    ui::present(view AS ui::Layout) AS Nothing

Requires `app::Mode.UI`.

Presents the layout as the main application window's widget tree.

Tree-update semantics:

- Validate the complete tree before installing it: each widget handle may appear
  only once; a closed handle in the tree is an error at `present`.
- Reusing a handle preserves the native twin's identity (and its native-held
  interaction state).
- Moving a handle within the tree reparents the existing native twin.
- Omitting a previously attached handle detaches its native twin without closing
  the resource.
- Reconcile applies every attached descriptor as truth (laws 3 and 4).
- Layout responds to window resizing without another `ui::present` call: the
  solver re-runs on the main thread on resize, with zero worker involvement —
  the same guarantee the canvas resize handshake gives (a UI lays out correctly
  while the program is blocked in `io::input`).

## Layout model

`ui::Layout` is CSS flexbox: direction, justification, alignment, gap, per-item
grow / shrink / basis. It is the only layout mechanism.

**MFB owns the flex solver; it does not map to native containers.** Win32 has no
native layout, and NSStackView / GtkBox implement different fractions of flex. The
solver runs on the **main thread** inside reconcile and on resize, because it
needs intrinsic sizes (a button's min-content width) and only the main thread may
ask the toolkit to measure. Native widgets are absolutely positioned at
solver-computed frames. One shared solver is also what makes the three platforms
lay out identically and lets layout be tested headless against a fake measurer.

### Solver design

- **The solver is an emitted runtime helper** allocation-free, re-entrant,
  main-thread-callable. It walks a **flat node array** (built by reconcile from
  the published tree) plus an indirect `host_measure` fn-ptr, and produces one
  rect per node. No allocation inside the solver, ever — it runs on the main
  thread during reconcile and resize, and an allocation there is a UI stall.
  Re-entrancy is a reverse dependency on the future `Table`; it cannot be
  retrofitted into an emitted helper cheaply, so build it in from the start.
- **The measurement contract is a fn-ptr, never a baked size.** v1 calls
  `host_measure` once per leaf, but nothing in the contract says once — so a
  future multi-pass flex solver (real flexbox needs two passes for wrapping and
  content-sized cross axes) is a change of strategy, not of contract. Do not
  cache measured sizes per node.
- **It needs a `RuntimeHelperSpec`, a catalog entry, and usage gating** so only
  programs that use `ui::` pay for it..

### Headless host and validation

- A `headless` host backend (`--app-host headless` or equivalent): a synthetic
  deterministic `host_measure`, a `host_set_frame` that prints
  `id kind x y w h` per node, events answered immediately-FALSE, window closes
  after the first sync. It ships, not test-only — it is the only way to test
  layout in CI without a display, and it costs one flag.
- **Goldens drive the real emitted solver, never a Rust-side model of it.** One
  golden per `Direction × Justification × Align` cell, plus grow/shrink
  distribution, padding/margin/gap, and hidden-sibling reflow — byte-identical
  across macOS and Linux with no display server. New golden directories must be
  confirmed in the gate's denominator before being relied on.
- **Correctness risk concentrates in flexible-size distribution across the
  `Justification × Align` matrix**: it has the most interacting rules, every real
  UI exercises it, and an off-by-one there produces a *plausible* layout rather
  than an obviously broken one. The golden matrix is the guard.

### Sizing lesson

The solver is the largest single item in the old feature and its size was the one
number nobody derived ("~1500–2500 emitter lines", baseless). The closest
precedent, `term_grid.rs`, is ~1200 lines for a strictly simpler job (fixed grid,
no measure callback, no nesting) — a floor, not an estimate. Before scheduling
anything behind the solver: build a row-only, single-axis, no-grow spike, count
its emitter lines, extrapolate, and split the work by axis (row/column, then the
justify/align matrix, then grow/shrink + padding/margin) if it lands large.

## Events

    ui::pollEvent(timeoutMs AS Integer) AS ui::Event

Requires `app::Mode.UI`. Blocks up to `timeoutMs` for the next event; this is how
an idle UI program sleeps. Returns a `None` variant on timeout.

`ui::Event` is a data union; MVP members:

    UNION Event
        ui::None                                  ' timeout
        ui::WindowClosed
        ui::ButtonClicked    ' { id AS WidgetId }
        ui::InputChanged     ' { id AS WidgetId, text AS String }
        ui::InputSubmitted   ' { id AS WidgetId, text AS String }
        ...
    END UNION

Correlation is by **`WidgetId`, never by handle**: an event may be polled after
its widget's resource is closed, so events carry the opaque id and

    ui::idOf(w AS ui::Button) AS ui::WidgetId    ' one overload per widget type

lets the program compare. Events for a dead widget simply match nothing.

The queue is bounded; on overflow the oldest event is dropped. Repeated
`InputChanged` for the same widget may be coalesced to the newest.

`WindowClosed` is in the MVP set — without it a UI program cannot quit cleanly.

## Future work (explicitly out of the MVP)

- **Embedded canvases (`ui::Canvas`) and explicit-target
  `canvas::present(target, items)` overloads.** Prerequisite: the per-instance
  canvas refactor — today the scene ring (`CANVAS_SCENE_SYMBOL`), the graphics
  thread, the resize handshake and the worker-side `__CANVAS_*` globals are all
  one-per-process, with a 32-row race matrix proven against that shape
  (`.ai/canvas-threading.md` §8). That refactor is its own plan and lands first.
- **`ui::Table`.** Three unrelated native models (NSTableView data sources,
  GtkColumnView list models, Win32 ListView messages); proves nothing about the
  architecture the MVP doesn't.
- Additional widgets, keyboard focus traversal, accessibility.
- Do not introduce a mutable "current canvas" or any second write channel
  (e.g. an immediate-effect `setText`): the two-channel model is the design.
