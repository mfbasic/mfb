# UI package and embedded canvases

Introduce `ui::` for native widgets and tree-based layout. Keep `app::` responsible
for presentation modes and `canvas::` responsible for drawing.

## Presentation modes

- `app::Mode.UI` presents a native widget tree, including embedded canvases.
- `app::Mode.Canvas` retains the existing canvas-only window.

Enter UI mode explicitly:

    app::setMode(app::Mode.UI)

## Widget tree

A layout contains an ordered list of elements and nested layouts. `Layout` and
`Element` are data values; `ElementResource` is a union of native widget resource
types. The declarations below describe the proposed shape within `ui::`.

    UNION ElementResource
        ui::Button
        ui::Canvas
        ui::Input
        ui::Table
        ...
    END UNION

    TYPE Element
        disabled AS Boolean
        visible AS Boolean
        element AS RES ElementResource
    END TYPE

    UNION Widget
        ui::Element
        ui::Layout
    END UNION

    TYPE Layout
        disabled AS Boolean
        visible AS Boolean
        justification AS LayoutJustification
        align AS LayoutAlign
        ...
        widgets AS List OF Widget
    END TYPE

The `RES` field holds a reference to the widget resource. `List OF Widget` is an
ordinary data collection whose elements can contain those references.

## Widget creation

Native widget creation belongs to `ui::`, including canvas widgets:

    ui::createButton(...) AS RES ui::Button
    ui::createCanvas(...) AS RES ui::Canvas
    ui::createInput(...) AS RES ui::Input
    ui::createTable(...) AS RES ui::Table

Creating a widget does not attach it to a window. Attachment happens when a tree
containing that widget is presented. Each constructor produces a new resource
following the standard MFB resource rules. Widgets have no explicit close function.

## Resource lifetime and attachment

A widget resource belongs to its MFB cleanup scope. Bindings, record fields, and
collection slots hold references to that resource. Storing a reference in an
outer-scope layout follows the existing ownership-floats-up rules; the UI adds no
new ownership model.

The native presentation holds references to widgets without taking ownership or
extending their MFB resource lifetime. Attachment and resource lifetime are separate:

- Removing a widget from the MFB layout does not close it or immediately change
  the displayed tree.
- Presenting the updated layout detaches the omitted widget from the native tree.
  The resource and native widget still exist and can be included in a later layout.
- Removing references does not move cleanup inward. The resource closes when its
  owning scope exits, even if it was removed from the layout earlier.
- If scope cleanup closes a widget still attached to the last presented tree, the
  native side removes it without waiting for another `ui::present`. Its old slot
  remains a blank hole until the next presentation; the removed widget no longer
  draws or receives input events.

The user may never present an updated layout, so cleanup must remove a closed
widget independently of presentation.

These ownership rules are defined in §15.6 of
`src/docs/spec/language/15_resource-management.md`. The implementation uses
`emit_resource_float` in `src/codegen/resource/owner.rs` and
`emit_owned_list_drain` in `src/codegen/cleanup/owned/builder_owned_cleanup.rs`
(inspect with `rg -n 'emit_resource_float|emit_owned_list_drain' src/codegen/resource/owner.rs src/codegen/cleanup/owned/builder_owned_cleanup.rs`).

## Presenting the UI

    ui::present(view AS ui::Layout) AS Nothing

Requires `app::Mode.UI`.

Presents the layout as the main application window’s widget tree. Nested layouts
determine the arrangement of native controls and embedded canvases.

Proposed tree-update semantics:

- Validate the complete tree before installing it.
- Each widget handle may appear only once in the installed tree.
- Reusing a handle preserves the widget’s identity and state.
- Moving a handle within the tree reparents the existing widget.
- Omitting a previously attached handle from the presented tree detaches it
  without closing its resource.
- Layout responds to window resizing without another `ui::present` call.

## Drawing into embedded canvases

Add explicit-target overloads:

    canvas::present(
        target AS ui::Canvas,
        items AS List OF canvas::DrawItem
    ) AS Nothing

    canvas::presentLayers(
        target AS ui::Canvas,
        layers AS List OF canvas::DrawLayer
    ) AS Nothing

These overloads require `app::Mode.UI` and raise a wrong-mode error in
`app::Mode.Canvas`.

Each canvas has independent drawing content. Updating a canvas does not require
republishing the UI tree.

Proposed behavior: publishing to a detached canvas retains its latest content,
which becomes visible when the canvas is attached.

## Existing canvas-only API

Keep the existing implicit-target forms:

    canvas::present(items AS List OF canvas::DrawItem) AS Nothing

    canvas::presentLayers(layers AS List OF canvas::DrawLayer) AS Nothing

These require `app::Mode.Canvas` and raise a wrong-mode error in
`app::Mode.UI`. Their target is the mode’s window-filling canvas.

The implicit canvas and embedded canvases should share the same underlying
canvas-instance implementation.

## Multi-canvas API consistency

Canvas-specific operations must identify their target in UI mode. Review size
queries, redraw requests, input routing, coordinate conversion, and rendering
state alongside the new presentation overloads.

Do not introduce a mutable “current canvas.”
