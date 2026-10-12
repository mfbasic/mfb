//! `canvas::setGroup` — install a named sub-scene.

use super::gen_group::emit_set_group;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Install a list of items under a name, to be drawn by `canvas::Group`."#;

const DESC: &str = r#"`setGroup` installs `items` under `name`. A scene then draws them by including a
`canvas::Group` node — a position, a transform and a name — instead of the items
themselves:

```
canvas::setGroup("panel", panelItems)
canvas::present([canvas::Group[id := "", position := vector::Float2[x := 10.0, y := 20.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"]])
```

`canvas::present` copies the whole scene every time it is called. A large static
sub-picture — a map, a sprite sheet, a UI panel — is therefore copied on every frame
even though nothing about it changed. Installing it once and referencing it means each
`present` copies the *node* and not the picture, and the same group can be referenced
from several scenes and at several positions without being copied for each.

**It takes effect at the next `canvas::present`, not immediately.** Calling `setGroup`
does not repaint; `present` is the install point for everything a scene draws, and a
group is no exception. Presenting the same scene list after a `setGroup` *does* redraw,
even though the list is unchanged — the change is inside the group, and it is seen.

Calling it again with the same name replaces what that name draws. A name you have not
installed, or one you removed, draws nothing and does not raise — so a scene may
reference a group before it is built.

Items in the list may themselves be `canvas::Group` nodes. A nested group stays a
reference: replacing the inner group changes what the outer one draws, without
reinstalling the outer. Nesting is allowed to a depth of **64**, and `canvas::present`
raises `ErrDepthExceeded` past it — which is also how a group that eventually references
itself is reported, since a cycle is unbounded depth. Both need the same fix, so both get
the same error.

A group node **places and turns everything it draws as one thing.** The group's own
origin — the point its items call `(0, 0)` — lands at the node's `position`, and the
node's `transform` turns, scales or shears the whole group about that point. So build a
figure around its own origin, install it once, and each node that names it can put it
somewhere else, at another angle and another size. An item's own `transform` still
shapes that item inside the group; the group then carries the result. Strokes scale
with the group, and each item's clip and gradient go with it.

Nested groups compose from the inside out: an inner node places its group inside the
outer group, and the outer node then places all of it. A wheel turning on a cart that
is itself moving is two nodes, each with one job.

A `transform` left at all zeros — which is what `MUT node AS canvas::Group` gives you —
turns nothing, and the node only places the group at `position`.

A node's `id` names that one use of the group. It draws nothing; it is what
`canvas::MouseEvent.overID` puts in front of an item's own `id` when the mouse is over
something this node drew, so two nodes drawing the same group can be told apart. Leave
it `""` and the items report their own `id`s alone.

The items are copied, so the list you pass is yours to change afterwards — the
installed group does not follow it. The group stays installed until you replace it or
call `canvas::removeGroup`.

**An image or font you name in the items becomes the group's to close, and you do not
close it yourself.** A group is drawn long after the `present` that installed it, so it
keeps what its items name usable for as long as it is installed — even if the binding you
built the items from has gone out of scope. That is the difference from `canvas::present`,
where naming an image in a scene keeps nothing usable.

The compiler holds you to it: after `setGroup`, naming the same image again is a compile
error rather than a picture that quietly goes wrong later. If you need the same image in
two groups, build it twice.

`canvas::removeGroup`, or a `setGroup` that replaces the items, closes what the old items
named — unless something still on screen names it too, in which case it stays usable. So
a group you rebuild every frame from the same font keeps drawing."#;

const EX: &str = r#"A panel installed once and drawn at two positions:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET body AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 160.0, height := 90.0], radiusTL := 12.0, radiusTR := 12.0, radiusBL := 12.0, radiusBR := 12.0, paint := canvas::fillStroke(color::rgb(40, 60, 90), color::rgb(120, 170, 230), 3.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dot AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 24.0, y := 24.0], radius := 8.0, paint := canvas::fill(color::rgb(255, 200, 60)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("panel", [body, dot])

  LET left AS canvas::DrawItem = canvas::Group[id := "", position := vector::Float2[x := 40.0, y := 40.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"]
  LET right AS canvas::DrawItem = canvas::Group[id := "", position := vector::Float2[x := 260.0, y := 40.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"]
  canvas::present([left, right])
END FUNC
```

One figure, drawn upright and again turned and enlarged. The arrow is built around
its own origin, pointing right; the second node turns it a quarter turn about its
`position` and doubles it, so it points down:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET ink AS canvas::Paint = canvas::fill(color::rgb(255, 200, 60))
  MUT shaft AS canvas::Rectangle
  shaft = WITH shaft { position := vector::Float2[x := 0.0 - 40.0, y := 0.0 - 6.0], size := canvas::Size[width := 60.0, height := 12.0], paint := ink }
  MUT head AS canvas::Polygon
  head = WITH head { position := vector::Float2[x := 20.0, y := 0.0], contours := [[ vector::Float2[x := 0.0, y := 0.0 - 18.0], vector::Float2[x := 30.0, y := 0.0], vector::Float2[x := 0.0, y := 18.0]]], paint := ink }
  canvas::setGroup("arrow", [shaft, head])

  MUT plain AS canvas::Group
  plain = WITH plain { name := "arrow", position := vector::Float2[x := 120.0, y := 120.0] }
  MUT turned AS canvas::Group
  turned = WITH turned { name := "arrow", position := vector::Float2[x := 320.0, y := 120.0], transform := canvas::Transform[a := 0.0, b := 2.0, c := 0.0 - 2.0, d := 0.0, tx := 0.0, ty := 0.0] }
  canvas::present([plain, turned])
END FUNC
```

Changing what the name draws. The scene list is not rebuilt and `present` is handed
exactly what it was handed before — the redraw happens because the group changed:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT os

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET node AS canvas::DrawItem = canvas::Group[id := "", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "light"]

  LET off AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 0.0, y := 0.0], radius := 30.0, paint := canvas::fill(color::rgb(60, 60, 60)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("light", [off])
  canvas::present([node])
  os::sleep(500)

  LET on AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 0.0, y := 0.0], radius := 30.0, paint := canvas::fill(color::rgb(255, 220, 80)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("light", [on])
  canvas::present([node])
END FUNC
```

One group drawn twice, each draw with its own `id`, so a click says which of the two
it landed on. The key cap is named once, inside the group; the `id` on each
`canvas::Group` goes in front of it, and `canvas::MouseEvent.overID` reports
`"yes::cap"` over the left key and `"no::cap"` over the right:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT cap AS canvas::Rectangle
  cap = WITH cap { id := "cap", size := canvas::Size[width := 90.0, height := 40.0], radiusTL := 8.0, radiusTR := 8.0, radiusBL := 8.0, radiusBR := 8.0, paint := canvas::fill(color::rgb(70, 110, 190)) }
  canvas::setGroup("key", [cap])

  MUT yes AS canvas::Group
  yes = WITH yes { id := "yes", name := "key", position := vector::Float2[x := 60.0, y := 80.0] }
  MUT no AS canvas::Group
  no = WITH no { id := "no", name := "key", position := vector::Float2[x := 180.0, y := 80.0] }
  canvas::present([yes, no])

  canvas::enableMouse(TRUE)
  LET event AS canvas::MouseEvent = canvas::pollMouse()
  canvas::enableMouse(FALSE)
  IF event.kind = canvas::MouseKind.Down THEN
    io::print("pressed [" & event.overID & "]")
  END IF
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setGroup",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "name",
                    desc: "What to call this group. A `canvas::Group` naming it draws \
                           these items; installing again under the same name replaces \
                           them.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "items",
                    desc: "The sub-scene, drawn in list order — later items paint over \
                           earlier ones, exactly as in `canvas::present`. Copied, so \
                           the list you pass stays yours to change. An image or font \
                           named here becomes the group's to close: it stays usable for \
                           as long as the group is installed, and you do not close it \
                           yourself.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::named("DrawItem")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrWrongMode", "ErrCanvasGroupLimit", "ErrOutOfMemory"],
            body: Body::abi_function(emit_set_group),
        }],
    });

    // plan-116-J: `items` consumes the resources reachable from it. A group outlives the
    // `present` that draws it, so it has to keep the images and fonts its items name
    // alive — and it cannot, while the caller still owns them: scope-drop closes a `RES`
    // its binding still owns, and the group holds only an alias into the same record.
    // Marking the parameter consuming is what moves those bindings, so the caller's
    // scope-drop no longer fires and the group's free path is the one close.
    //
    // The rendered signature is unchanged; what changed is what passing an item list
    // DOES to the resources inside it, which is why `DESC` says so.
    pkg.add_consuming_parameter("setGroup", "items");
}
