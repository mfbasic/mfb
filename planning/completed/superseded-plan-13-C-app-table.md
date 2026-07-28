# plan-13-C: `app::` GUI package — Table (widget-cell grid)

Last updated: 2026-07-09
Effort: large

Part **C** of plan-13 (the `app::` GUI feature). Extends
[plan-13-A](plan-13-A-app-builtin.md) (the `app::` native-widget package) with a
**widget-cell grid**: `app::Table` is a resource widget that is a *kind of container*
addressed by `(row, col)`. Each cell holds exactly **one ordinary widget** — any
`app::Widget`, including a real `app::Container` for composite cells — placed with
`app::setWidget` (or created directly into a cell with the table `add*` overloads). Header
rows are a second, pinned grid of widget cells over the scrolling data grid.

> **Redesign note (2026-07-02).** This plan previously specified a *template + data*
> virtualized table (prototype row templates, `bindColumn` slots, `tableSetRows` data rows,
> and a table-owned "second lifetime regime" of recycled cell instances). That model is
> **dropped**. Cells are now real user-owned widgets under plan-13-A's one lifetime law;
> there are no templates, no data-row model, and no user-visible recycling. Virtualization
> survives as a purely **native-side** optimization (§4): only visible cells get native
> peers. Large grids remain constructible because resource collections + ownership float
> (`List OF RES app::Label`, `mfb spec language resource-management` §15.6) let a loop keep
> its per-iteration widgets alive.

The single behavioral outcome: a `mfb build -app` program can present a scrolling grid of
widgets — headers plus data cells, including editable cells — that stays responsive at large
row counts because only the visible cells are realized natively, while every cell widget is
an ordinary `RES` under plan-13-A's unchanged lifetime law.

It complements:

- [`planning/plan-13-A-app-builtin.md`](plan-13-A-app-builtin.md) — the base `app::` design
  (§2 lifetime model, §5 surface, §7 shadow/`sync`, §8 host seam, §10 language checkpoints).
  This plan reuses every one of those and adds to them; it changes none.
- [`planning/plan-13-B-app-textarea.md`](plan-13-B-app-textarea.md) — `text::AttributeString`
  + `app::TextArea`. A Table cell hosting a `TextArea` is how a table shows attributed text.
  **Soft dependency only** — nothing here requires plan-13-B to land first.
- `./mfb spec language resource-management` — §15.6 resources in collections + ownership
  float: the mechanism that makes loop-built large grids possible.
- `./mfb spec package` — where the `app::` Table surface and new types are documented.
- `./mfb spec memory` — the widget shadow/lifetime model this plan must not perturb.

## 1. Goal

- **Table**: `app::Table` — a grid container widget. The program places one widget per cell
  (`setWidget` / table `add*` overloads), in a pinned **header** grid and a scrolling
  **data** grid. Cell-level interaction is surfaced as index-tagged, frame-latched events:
  header click, cell click, cell mouse enter/leave, plus row selection/activation. The
  shadow cost scales with the cells the program creates (cheap config-only nodes); the
  native cost scales with the *visible* cells only.

### Non-goals (explicit constraints)

- **No change to plan-13-A's lifetime law — and no second lifetime regime.** Every cell
  widget is an ordinary user `RES`, destroyed exactly once at its own owner-scope drop. The
  table never owns a cell widget: `setWidget` stores a **borrow** (attach-like), `clearCell`
  and cell-widget drop **detach**. What recycles natively are *peers* (native objects), not
  widgets — an invisible extension of plan-13-A §7's lazy realization (§4).
- **No callbacks / no escaping `MUT`-capturing closures.** All interaction is polled,
  frame-latched event state drained at `sync`, exactly like `app::clicked` (plan-13-A §7).
- **No new external dependency.** System toolkits only (AppKit, GTK4), per plan-13-A §1.
- **No change to the layout solver contract.** `Table` is a **leaf** `Widget` to the shared
  flex solver (measured + framed as one box); it scrolls *internally*. Inside a cell, a
  `Container` cell reuses the shared solver for its own subtree. Grid placement itself
  (rows × columns) is the table's own arithmetic, native-side (§4).
- **Editable cells just work.** An `Input` / `TextArea` cell keeps its value in its *own*
  widget shadow, exactly as outside a table; the widget owns its edit lifecycle. Scrolling a
  cell out and back must not lose state (§4).
- **`app::sync`, mutators, getters stay non-blocking; `app::poll` stays the only wait**
  (plan-13-A §9). Table event reads are frame-latched shadow reads.

## 2. Current State

plan-13-A is the base. Relevant precedents:

- **Widget model** (plan-13-A §7): each widget is a worker-side *shadow* node (hierarchy +
  configuration, never positional) with a dirty flag; `sync` reconciles shadow↔native. `Table`
  is a new shadow node kind whose children live in `(region, row, col)` cell slots instead of
  an ordered child list; its events are new frame-latched drains at `sync`.
- **Native-owned geometry** (plan-13-A §7/§8, locked): layout/positional state lives on the
  native side and reacts to native input (resize) without worker involvement. The table's
  scroll position and visible-cell realization follow the same rule: scrolling is fully
  native-autonomous (§4). This is only possible because plan-13-A's solver is an **emitted,
  allocation-free, re-entrant MIR helper** (`_mfb_rt_app_layout`, plan-13-A §8.1) callable from
  the main thread at arbitrary times. The table's scroll handler is a *third* caller of it,
  alongside `host_present` and the resize handler: realizing a `Container` cell means solving
  that cell's subtree right then, on the scroll path, with no `sync` and no worker. Note this
  when reading plan-13-A §8.2 — "re-entrant on the main thread" is a hard constraint that this
  plan, not plan-13-A, is the reason for.
- **Trailing-omission argument rule** (plan-13-A §5.0): builtins have no AST-inserted default
  expressions; optional parameters must be trailing, and a middle parameter cannot be skipped
  even by name. This is why the header/data selector below is a **required `Region` enum**
  rather than a `header AS Boolean = FALSE` parameter (§5.1).
- **Resources in collections** (`mfb spec language resource-management` §15.6): a
  `List OF RES app::Label` slot is a borrow whose resource ownership *floats up* to the
  collection's scope — so a `FOR` loop can create a widget per row and keep it alive by
  appending it to an outer list. This is what makes 100 000-cell grids constructible with
  real `RES` widgets (§6).
- **`app::Widget` resource union + central `compatible()` validation** (plan-13-A §3/§10;
  exercised by plan-13-B's `TextArea` variant): adding the `Table` variant gives it the
  widget-wide ops (`getVisible`/`setVisible`/`getSize`/`setSize`) **for free**, no per-op
  edits. And `setWidget`'s `w AS RES app::Widget` param accepts every variant centrally.
- **Bidirectional widget values** (plan-13-A §7 `Input`; plan-13-B §4.2 `TextArea`): the
  per-widget shadow `value` + drain machinery is what lets an editable cell survive native
  peer recycling — drain on de-realize, push on re-realize (§4).
- **`term::` type-ID reservation** (plan-13-A §10): new record/enum type IDs
  (`TableCellEvent`, `SelectionMode`) use the high reserved range to dodge the
  `FIRST_TABLE_TYPE_ID` collision.

## 3. The model: a grid of ordinary widgets

`app::Table` is a new **`RES` widget** and **`app::Widget` union variant** (widget-wide ops
for free). It is a **leaf** to the flex solver and scrolls internally. It holds two grids of
cell slots:

- the **header** grid — zero or more pinned rows that do not scroll (multiple header rows are
  allowed; header events carry their own `(row, col)` numbering);
- the **data** grid — the scrolling body.

Cell rules (all regions):

- **One widget per cell.** A cell is empty or holds exactly one `app::Widget` — including a
  real `app::Container`, which makes composite cells ordinary flex subtrees (its children are
  normal widgets laid out by the shared solver within the cell's frame, invoked from the
  table's own scroll/present path — §2).
- **A cell may not hold a `Table`.** `app::Table` is itself an `app::Widget` variant, so
  `setWidget(t, r, c, region, someTable)` typechecks; it raises `ErrInvalidArgument` at runtime.
  Nesting a virtualizing scroll view inside a virtualized cell that may de-realize under it is
  not something v1 attempts to make correct, and there is no compile-time way to exclude one
  variant from a union parameter. Same for the `addTable(t, ...)` cell overload: it does not
  exist.
- **The table never owns a cell widget.** A cell slot is a layout attachment — the same
  borrow relationship as a `Container` child (plan-13-A §2). The widget's own `RES`
  binding/owner scope destroys it exactly once; when it drops, its cell empties. `clearCell`
  detaches (never destroys); `setWidget` into an occupied cell **detaches the occupant**
  (which stays valid, re-attachable elsewhere) and attaches the new widget. `app::close` /
  table drop detaches every cell widget, leaving valid orphans — plan-13-A §2 verbatim.
- **Grid extents derive from the cells.** `tableRowCount`/`tableColCount` are
  `max set index + 1` per region; rows/columns with no widgets still occupy space (blank).
- **Widget events keep working inside cells.** A `Button` cell still reports `app::clicked`;
  an `Input` cell still reports `valueChanged` — cell widgets are ordinary widgets. The
  table-level cell events (§5) are an *additional*, index-tagged plane: a click on a button
  cell latches **both** the button's `clicked` and the table's `tableCellClicked (row, col)`;
  the program reads whichever plane it wants.

## 4. Virtualization is native-side only (no user-visible recycling)

plan-13-A §7 already realizes native objects **lazily** (a widget gets a native peer at the
first `sync` that sees it). The table generalizes that to **visibility-driven realization +
de-realization**, entirely below the seam:

- **`sync` mirrors cell config to the main thread.** The worker pushes each dirty cell
  widget's configuration (kind, properties, cell coordinates) as usual; the native table
  keeps a main-thread **config mirror** for all cells, realized or not. This is the same
  "post dirty changes" flow as every widget — the mirror is just retained per-cell.
- **The native table realizes peers for visible cells only.** From its scroll position it
  computes the visible row window and materializes native peers (via the existing leaf host
  calls) for cells entering it, recycling/destroying peers for cells leaving it. Because the
  mirror is main-thread data, **scrolling never involves the worker** — consistent with the
  locked native-geometry rule (plan-13-A §7/§8). A 100 000-row table costs
  ~(visible rows × columns) native peers.
- **De-realize drains, re-realize pushes.** Before a peer with bidirectional state (an
  `Input`/`TextArea` cell) is recycled, the native side drains its pending native state into
  that widget's normal drain buffer (picked up at the next `sync` into the widget's own
  shadow); on re-realization the shadow value is pushed back. Net effect: **editable cells
  just work** — the widget owns its own edit state, and scrolling away and back loses
  nothing. Event counters (clicks) live in per-widget main-thread buffers keyed by the
  widget, not the peer, so they survive recycling the same way.
- **Not a second lifetime regime.** Peers were never user-visible objects in plan-13-A
  either (they already appear late, at first `sync`); the table only adds that they can also
  *disappear* while the widget — shadow node, `RES` handle, value, latches — stays fully
  live. No user-facing rule changes.

Sizing model (v1): **uniform data-row height** (`setRowHeight`, default measured from the
tallest realized cell of data row 0) — the virtualizer needs `content height = headerHeights
+ rowCount × rowHeight` without measuring 100 000 rows. Column widths come from
`setColumnWidth` (`< 0` = flex); unset columns share the flex space equally. Header row
heights are measured (there are few). Within a cell's frame the widget is placed per the
column's `align`-style rules — v1 fills the cell (Stretch).

## 5. Surface, types, and seam

### 5.1 Surface

The header/data selector is a **required `app::Region` enum immediately after `col`**, not a
trailing-defaulted `header AS Boolean`. Under plan-13-A §5.0 a middle parameter cannot be
skipped even by name, so `app::addLabel(t, r, 0, label := "item")` — which the earlier draft's
worked example used — is rejected with `TYPE_CALL_ARITY_MISMATCH` (*"omits parameter `header`
before a later supplied argument"*). Making the region required and positional keeps every call
legal, and `app::Region.Header` reads better at a call site than a bare `TRUE` anyway.

```
app::addTable(parent AS RES app::Container,
              margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Table

' Table's registered close op (plan-13-A §2 — every widget type has one). Internal only,
' NOT exported: scope-drop calls it; `app::destroy(...)` in user code is an unknown function.
app::destroy(t AS RES app::Table) AS Nothing

' Create a widget directly into a cell (the born-attached idiom, plan-13-A §5).
' `region` selects the pinned header grid or the scrolling data grid. Required.
' Parameter order mirrors plan-13-A's container add* (dir, align, justify, padding).
app::addContainer(t AS RES app::Table, row AS Integer, col AS Integer, region AS app::Region,
                  dir AS app::Direction = app::Direction.Row,
                  align AS app::Align = app::Align.Center,
                  justify AS app::Justification = app::Justification.Start,
                  padding AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Container
app::addButton(t AS RES app::Table, row AS Integer, col AS Integer, region AS app::Region,
               label AS String = "",
               margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Button
app::addLabel(t AS RES app::Table, row AS Integer, col AS Integer, region AS app::Region,
              label AS String = "",
              margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Label
app::addInput(t AS RES app::Table, row AS Integer, col AS Integer, region AS app::Region,
              value AS String = "", placeholder AS String = "",
              margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::Input
' (plan-13-B, once landed, adds the matching app::addTextArea(t, row, col, region, ...) overload.)

' Re-attach an existing (detached) widget into a cell. An occupied cell's occupant is
' detached first (stays valid). Same detach-not-destroy law as app::attach (plan-13-A §2).
' Passing a Table as `w` raises ErrInvalidArgument (§3, no nested tables in v1).
app::setWidget(t AS RES app::Table, row AS Integer, col AS Integer, region AS app::Region,
               w AS RES app::Widget) AS Nothing

' Detach the cell's widget (it stays valid until its own owner-scope drop). No-op if empty.
app::clearCell(t AS RES app::Table, row AS Integer, col AS Integer, region AS app::Region) AS Nothing

app::tableRowCount(t AS RES app::Table) AS Integer          ' data rows  = max set row + 1
app::tableColCount(t AS RES app::Table) AS Integer          ' columns    = max set col + 1 (both regions)
app::tableHeaderRowCount(t AS RES app::Table) AS Integer    ' header rows = max set header row + 1

app::setColumnWidth(t AS RES app::Table, col AS Integer, width AS Integer) AS Nothing  ' < 0 = flex (default)
app::setRowHeight(t AS RES app::Table, height AS Integer) AS Nothing   ' uniform data-row height; unset = measure row 0

' Events (frame-latched, drained at sync, index-tagged; (-1, -1) / -1 = none this frame):
app::tableHeaderClicked(t AS RES app::Table) AS app::TableCellEvent   ' (header row, col)
app::tableCellClicked(t AS RES app::Table) AS app::TableCellEvent     ' (data row, col)
app::tableCellEntered(t AS RES app::Table) AS app::TableCellEvent     ' mouse entered a data cell since last sync
app::tableCellLeft(t AS RES app::Table) AS app::TableCellEvent        ' mouse left a data cell since last sync

' Row selection (persistent state, not an edge event; mode gates it):
app::setTableSelectionMode(t AS RES app::Table, mode AS app::SelectionMode) AS Nothing  ' default None
app::tableSelectedRow(t AS RES app::Table) AS Integer                 ' -1 if none / mode None
app::setTableSelectedRow(t AS RES app::Table, row AS Integer) AS Nothing
app::tableRowActivated(t AS RES app::Table) AS Integer                ' double-click/Enter on a row, -1 (frame-latched)
```

**Overload-arity collision check** (plan-13-A §10). Making `region` required pushes every table
`add*` form to arity ≥ 4, which keeps most of them clear of the plan-13-A container forms
(`addLabel`/`addButton`: container 1–3, table 4–6 — disjoint). Two do collide:

| Function | Container arities | Table arities | Overlap |
| --- | --- | --- | --- |
| `addContainer` | 1–5 | 4–8 | **4, 5** |
| `addInput` | 1–4 | 4–7 | **4** |
| `addButton` | 1–3 | 4–6 | none |
| `addLabel` | 1–3 | 4–6 | none |

`builtins::select_param_name_overload` picks among same-arity overloads by count-and-names, and
`resolve_call` then separates them by argument type. At each overlapping arity the two name sets
are **disjoint** (`parent/dir/align/justify/padding` vs `t/row/col/region/…`) and the first
argument type differs (`Container` vs `Table`), so no call is ambiguous on either path. This is
exactly the property plan-13-A §10's `#[test]` asserts over the whole `app::` table — re-run it
after adding these forms, because it is the thing that silently breaks if someone later gives a
table form a parameter named `padding`.

### 5.2 Types

```
TYPE TableCellEvent              ' high reserved type-ID range
    row AS Integer
    col AS Integer
END TYPE

app::Region         Data, Header             ' which cell grid an op addresses (required, §5.1)
app::SelectionMode  None, Single, Multi      ' v1 implements None + Single; Multi reserved
```

Event semantics:

- **Index-tagged, both planes.** Table events report the `(row, col)` the cell occupies at
  event time. They are *additional* to the cell widget's own events (§3) — a button-cell
  click latches both `clicked(btn)` and `tableCellClicked(t)`.
- **Header vs data numbering.** `tableHeaderClicked` rows count within the header grid
  (0-based); all data-cell events count within the data grid.
- **Enter/leave latching (v1).** One latched `(row, col)` per event kind per frame,
  most-recent-wins; a fast enter+leave inside one frame latches both events with their
  respective cells. (Internally counters/queues per plan-13-A §7 so richer reads can come
  later; the v1 read collapses to the latest.)
- **Selection is state, not an edge.** `tableSelectedRow` persists across frames (mirrored
  into the shadow at `sync` when the native selection changes); `tableRowActivated` is a
  frame-latched edge like `clicked`. Selection is **data-row indexed**, so it survives
  scrolling/recycling.

### 5.3 Seam additions (additive to plan-13-A §8)

```
host_create_table() -> handle                            ' virtualizing grid scroll view
host_table_set_extent(handle, headerRows, rows, cols)    ' content extent for the virtualizer
host_table_set_column_width(handle, col, width)          ' < 0 = flex
host_table_set_row_height(handle, height)
host_table_place_cell(handle, region, row, col, widgetId)  ' bind a cell slot to a widget's config-mirror entry
host_table_clear_cell(handle, region, row, col)
host_table_set_selection_mode(handle, mode)
host_table_set_selection(handle, row)
```

There are **no `host_table_take_*` drain calls**. Per plan-13-A §7 the worker never reads native
state; the native table *pushes* an event record into the event pipe when a header/cell click,
cell enter/leave, selection change, or row activation fires, and `app::sync` folds it into the
table's shadow. Enter/leave and selection-change records coalesce last-wins per node on the
main-thread backpressure path, which is exactly the v1 read semantics anyway (§5.2). This is the
same correction as dropping `host_button_take_clicks` in plan-13-A §8: there is no shared mutable
state to drain, and no atomics exist to drain it with.

Mouse enter/leave on a large grid is the one event kind that can plausibly saturate the pipe
during a fast drag across cells. It is inherently coalescible (only the latest matters at v1's
read semantics), so the `EAGAIN` fallback is lossless *for these two kinds specifically* — worth
stating, because it is the reason enter/leave is safe to ship without a rate limiter.

Peer realization/de-realization is **not** a seam call — it is internal to the native table,
driven by its own scroll position against the main-thread config mirror (§4), creating leaf
peers with the existing `host_create_*` calls, invoking `_mfb_rt_app_layout` for `Container`
cells, and draining widget state through the widgets' existing drain paths. `region` is
header|data.

## 6. Worked pattern: a large loop-built grid

Ownership float (`mfb spec language resource-management` §15.6) keeps per-iteration widgets
alive in an outer collection — no template machinery needed:

```basic
RES t = app::addTable(root)

RES hName = app::addLabel(t, 0, 0, app::Region.Header, "Name")
RES hQty  = app::addLabel(t, 0, 1, app::Region.Header, "Qty")

' `cells` is declared BEFORE the loop, so each RES created inside the loop body floats its
' ownership up to `cells`' scope (§15.6) and outlives the iteration.
MUT cells AS List OF RES app::Label = []
FOR r = 0 TO 99999
    RES name = app::addLabel(t, r, 0, app::Region.Data, "item " & toString(r))
    RES qty  = app::addLabel(t, r, 1, app::Region.Data, toString(r * 3))
    cells = collections::append(cells, name)   ' ownership floats to `cells`' scope —
    cells = collections::append(cells, qty)    '   the widgets outlive the loop iteration
NEXT

WHILE app::isOpen(win)
    app::poll(win, 10)
    app::sync(win)
    LET hdr = app::tableHeaderClicked(t)
    IF hdr.col >= 0 THEN io::print("sort by column " & toString(hdr.col))  ' program-side sort
    LET cell = app::tableCellClicked(t)
    IF cell.row >= 0 THEN io::print("clicked row " & toString(cell.row))
WEND
' `cells` scope exit closes every label exactly once (each via app::destroy); `t` — declared
' earlier, so dropped later — detaches them as they drop.
```

Three things this example leans on, each worth verifying before the Phase-2 acceptance bets on
them:

- **Every argument is positional up to the last supplied one.** `app::Region.Data` is passed
  explicitly rather than defaulted away (§5.1), and `poll` takes its window (plan-13-A §5).
- **Ownership float out of a *loop body*.** §15.6 gives the source-level contract (a named `RES`
  binding added to an outer collection floats to that collection's scope), and `cells` is bound
  before the loop as §15.6 requires. But the compiler implements float with "a purely syntactic
  per-function decision procedure" specified in `mfb spec language escape-analysis` — confirm the
  loop-body case against that document, since the headline 100 000-row proof rests on it.
- **`collections::append` on a `MUT` binding is amortized O(1)**, not a fresh copy per iteration
  (the in-place bulk-append path). Without that, 200 000 appends are quadratic and the example is
  a benchmark of the wrong thing.

200 000 shadow nodes are config-only worker data; the native side realizes only the visible
window's peers. Program-side sorting = rewriting the labels (`setLabel`) or re-`setWidget`ing
rows — data stays the program's concern, per the no-callback model.

## Layout / ABI Impact

- **New builtin record/enum type IDs** (`TableCellEvent`, `Region`, `SelectionMode`) in the
  **high reserved type-ID range** (term:: precedent, `FIRST_TABLE_TYPE_ID` collision).
- **`app::Widget` union gains one variant** (`Table`), plus one row in `src/builtins/app.rs`'s
  `WIDGET_VARIANTS` table (plan-13-A §10, site 2). `compatible()`-routed — widget-wide ops
  accept it with no per-op edits (verify in §Validation). Note the union now admits a variant
  that `setWidget` rejects at runtime (§3, no nested tables); a union parameter cannot exclude
  a variant statically.
- **One new registered close op** (`app::destroy(RES app::Table)`), per plan-13-A §2.
- **plan-13-A `add*` gain table overloads** (`(t, row, col, region, ...)`) — *additive*
  overloads resolved by arity+types like the existing `Window` vs `Container` pair; no
  existing signature changes. `addContainer` (arities 4–5) and `addInput` (arity 4) now share an
  arity with their container forms and are separated only by argument type and disjoint parameter
  names; re-run plan-13-A §10's overload-coherence `#[test]` (§5.1).
- **The §8 host seam grows additively** (the `host_table_*` calls above) and gains no drain
  calls; no existing seam call changes signature.
- **No change to the shared layout solver contract.** `Table` is a leaf; grid placement is
  table-internal native arithmetic; a `Container` cell reuses the solver within its frame.
- **No second user-visible lifetime regime.** Native peers may now de-realize as well as
  lazily realize — a native-side-only generalization of plan-13-A §7; the user-facing law
  (`RES`-per-widget, detach-not-destroy, close-exactly-once) is untouched.
- **No change to `String`, `AttributeString`, `Input`, or any plan-13-A/plan-13-B widget.**

## Phases

Depends on plan-13-A being landed — specifically its **Phase 0** (the resource-union-parameter
language amendment, without which `setWidget`'s `w AS RES app::Widget` does not typecheck) and
its **Phase 2** (the emitted `_mfb_rt_app_layout` solver, which this plan's scroll path calls,
and the `headless` host backend this plan's model proofs run on). plan-13-B is a soft dependency
(only the `addTextArea` table overload waits on it). Ordered lowest-risk first; the
virtualizer/config-mirror is the risk concentration and lands behind headless + on-device
tests. Fill in `Commit:` with the hash(es) that land each phase.

### Phase 1 — Table shadow grid + surface + types

The worker-side model, headless.

- [ ] Add the `app::Table` shadow node kind (two cell grids: header/data, `(row, col)` slots holding widget borrows) + the `app::Widget` `Table` variant (+ the `WIDGET_VARIANTS` row) + the `app::destroy(RES app::Table)` close op (registry-only, internal — not added to the user-callable `app::` call table, per plan-13-A §2); `TableCellEvent`/`Region`/`SelectionMode` type IDs in the high reserved range.
- [ ] `addTable`; the table `add*` overloads (born-attached into a cell, required `region`); `setWidget` (detach-occupant-then-attach, `ErrInvalidArgument` on a `Table` argument) / `clearCell` (detach); extents (`tableRowCount`/`tableColCount`/`tableHeaderRowCount`); `setColumnWidth`/`setRowHeight`.
- [ ] Lifetime wiring: cell slots are borrows; cell-widget drop empties its cell; table close/drop detaches all cells (plan-13-A §2 orphan rules).
- [ ] Confirm loop-body ownership float against `mfb spec language escape-analysis` before Phase 2 depends on it (§6).
- [ ] Re-run plan-13-A §10's overload-name-coherence `#[test]` after adding the arity-3 table `add*` forms (§5.1).
- [ ] Tests: `tests/func_app_addTable_*`/`func_app_setWidget_*`/… `_valid`+`_invalid` — arity/types, union-widening (`setWidget` accepts every variant), the skipped-middle-argument rejection, nested-table rejection, extent math, detach-not-destroy, RES-borrow rejection.

Acceptance: the full Table surface typechecks and the shadow grid model (placement, replacement-detach, clear, extents, drop-empties-cell) is verified headless; a 100 000-iteration loop-built grid compiles and drops each widget exactly once.
Commit: —

### Phase 2 — macOS backend: grid + config mirror + virtualizer

The risk concentration.

- [ ] `host_create_table` (scroll view + pinned header band + grid body), extents, column widths / uniform row height.
- [ ] The main-thread **config mirror** (batch-pushed per-cell widget config) and visibility-driven peer realize/de-realize using the existing leaf host calls; scrolling fully native-autonomous (§4). A realized `Container` cell solves its subtree by calling `_mfb_rt_app_layout` from the scroll path — the third call site of the emitted solver, which is why it must be re-entrant and allocation-free (§2, plan-13-A §8.2).
- [ ] De-realize drains / re-realize pushes bidirectional widget state (`Input` first); per-widget (not per-peer) event state, so it survives peer recycling.
- [ ] Push header-click / cell-click / enter / leave / selection / activation **event records** into the event pipe; `sync` folds them into the table shadow. Enter/leave and selection coalesce last-wins under backpressure (§5.3).

Acceptance: on-device, a **100 000-row loop-built grid** (§6) scrolls smoothly with a bounded native-peer count; cell events report correct data-row indices after scrolling; an edited `Input` cell keeps its value after scrolling out and back; both event planes fire for a button cell; a fast drag across cells never stalls the UI or exhausts the pipe.
Commit: —

### Phase 3 — GTK4 backend + lifetime/detach correctness

- [ ] Implement the GTK4 backend against the same seam (scrolled window + fixed/grid body + pinned header); identical virtualization behavior.
- [ ] Verify cell-widget drop / `clearCell` / `setWidget`-replace / table close: every widget destroyed exactly once at its own drop, orphans re-attachable, no leak or double-free of native peers across recycling.

Acceptance: GTK4 matches macOS behavior on the Debian aarch64 box (plan-05); lifetime proofs pass on both backends.
Commit: —

### Phase 4 — Selection + activation

- [ ] `SelectionMode` (`None` default / `Single`; `Multi` reserved), `tableSelectedRow`/`setTableSelectedRow` (persistent, data-row indexed), `tableRowActivated` (double-click/Enter).
- [ ] Verify selection survives scrolling/recycling (indexed, not peer-bound).

Acceptance: selection + activation behave correctly and survive scrolling on both backends.
Commit: —

### Phase 5 — Docs + examples

- [ ] The §6 worked example as a shipped sample (100 000 rows, button cells, header-click program-side sort).
- [ ] Spec/man updates: `mfb spec package` Table topic (incl. the native-peer de-realization note), man pages per the `.ai` templates.

Acceptance: the worked example builds and runs on both backends; docs/spec updated; acceptance suite green.
Commit: —

## Validation Plan

- **Function tests** (every overload), per repo standard: `tests/func_app_addTable_*`,
  `func_app_setWidget_*`, `func_app_clearCell_*`, `func_app_tableRowCount_*`,
  `func_app_tableHeaderClicked_*`, `func_app_tableCellClicked_*`, … each with `_valid/**`
  and `_invalid/**` (arity + type + union-widening coverage, incl. a concrete-type op
  rejecting a wrong variant to prove `compatible()` stays directional).
- **Headless model proofs** (via `--app-host headless`, plan-13-A §8.1): grid slot
  placement/replacement/clear; extent math; borrow semantics (drop empties cell; table drop
  orphans cells); ownership-float loop construction compiles and drops each widget exactly
  once; nested-table `setWidget` raises `ErrInvalidArgument`.
- **Runtime proofs** (real behavior, not just golden output): the 100 000-row program —
  bounded native peers while scrolling, correct post-scroll event indices, editable-cell
  state surviving recycle, both event planes on a button cell, selection surviving scroll,
  exactly-once destruction on teardown; on-device on macOS and the Debian aarch64 box.
- **Doc sync**: `mfb spec package` (Table surface + types + native peer de-realization
  note), `mfb spec language` (if the union-variant addition needs a note); man pages per
  `.ai/man_template.md` / `.ai/man_type_template.md` / `.ai/man_package_template.md`; keep
  this plan's `Last updated` current and remove it in the commit that lands the final phase
  (per `.ai/planning.md`).
- **Acceptance**: `scripts/test-accept.sh target/debug/mfb target/accept-actual` green; the
  plan-13-A canonical program (and plan-13-B TextArea, once landed) still byte-identical.

## Open Decisions

- **Native host: custom grid over a plain scroll view vs `NSTableView`/`GtkColumnView`.** —
  *Recommend the custom grid* (scroll view + pinned header band + recycled cell peers): the
  row-model views constrain arbitrary widget cells and multi-row pinned headers, and we
  already own measurement + placement. **Prototype the scroll-driven realize/de-realize on
  both platforms before committing** — it is this plan's equivalent of plan-13-A's
  `host_set_frame` GTK4 risk.
- **Default data-row height.** — *Recommend: measure the tallest realized cell of data row 0*
  (re-measured when row 0's config changes), with `setRowHeight` as the explicit override.
  Fully per-row variable heights are out (the virtualizer would have to measure every row).
- **Default column width.** — *Recommend equal flex share* for unset columns in v1;
  content-based auto-width would need whole-column measurement (conflicts with
  virtualization). `setColumnWidth` covers fixed layouts.
- **Event multiplicity per frame.** — *Recommend last-wins per event kind per frame* in v1
  (internal counters/queues retained so a richer multi-event read can come later without a
  seam change) — consistent with plan-13-A's "keep it a counter internally".

## Non-Goals (v1)

- **Cell spanning** (row/col span), **frozen columns**, **sticky section headers**,
  **nested/grouped rows** — flat header grid + data grid only.
- **Multi-selection** (`SelectionMode.Multi` reserved; `None`/`Single` implemented).
- **Native column-click sort / drag-reorder / user-resizable columns** — sorting stays
  program-side (rewrite cells on `tableHeaderClicked`); these can slot in over the unchanged
  seam later.
- **Per-row variable heights** (uniform data-row height in v1; header rows are measured).
- Everything plan-13-A already lists as a non-goal (menus, native dialogs, animation/timers,
  theming) remains out of scope.

## Summary

`app::Table` is a grid-addressed container of **ordinary widgets**: one widget per
`(row, col)` cell across a pinned header grid and a scrolling data grid, placed with
`setWidget`/table-`add*`, under plan-13-A's single unchanged lifetime law (cells are borrows;
detach, never destroy). Interaction is index-tagged, frame-latched events — header click,
cell click, cell enter/leave, selection/activation — alongside the cell widgets' own event
plane. Virtualization is purely native-side: `sync` feeds a main-thread config mirror, and
the native table realizes peers for visible cells only, draining/pushing per-widget state
across recycling so editable cells just work. Large grids are constructible with plain loops
via resource-collection ownership float (§15.6). The engineering risk concentrates in one
place — the config mirror + scroll-driven realize/de-realize (Phase 2) — gated by the
100 000-row on-device proof.
