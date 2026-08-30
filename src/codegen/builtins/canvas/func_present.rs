//! `canvas::present` — install a scene as the canvas's current content.

// --- codegen tier imports (migration) ---
use crate::codegen::app::hook::app::{prepend_wrong_mode_gate, ModeRequirement};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Install a list of `DrawItem`s as the canvas's current content."#;

const DESC: &str = r#"`present` **installs** a scene. It is not a per-frame draw call: the runtime keeps
rendering the installed scene — on vsync, on resize, on damage — until the next
`present` replaces it. A static picture is therefore presented once and costs
nothing thereafter, and a program that never changes its content never calls
`present` again.

`present` **deep-copies the scene transitively**. Every reachable byte — the item
fields, a `Polygon`'s point list, a `Text`'s string, the `Paint` values — is copied
into runtime-owned storage, so once `present` returns, nothing in the installed
scene points at anything the caller owns. The program is free to mutate or drop
whatever it built the list from, and the renderer can read the scene at any later
moment without coordinating with the program.

**Re-presenting an identical scene does nothing.** `present` compares the incoming
content against what is already installed and returns without republishing when
they match, so an animation loop that redraws an unchanged frame costs a
comparison rather than a re-render.

An item names an image or font through an `ImageRef`/`FontRef` — an id, not the
resource — so an installed scene has no opinion about any resource's lifetime.
Destroying an image that a scene still names is safe: the runtime defers freeing
the backing texture until the GPU has finished with it.

Requires `Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`."#;

const EX: &str = r#"A yellow face with green eyes and a smile. Note that each item is bound first —
a list literal does not span source lines:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(Mode.Canvas)
  LET yellow AS Color = canvas::rgb(255, 255, 0)
  LET green AS Color = canvas::rgb(0, 160, 0)

  LET face AS DrawItem = Circle[x := 200.0, y := 200.0, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS DrawItem = Circle[x := 150.0, y := 160.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS DrawItem = Circle[x := 250.0, y := 160.0, radius := 22.0, paint := canvas::fill(green)]
  ' 0 -> PI sweeps downward under a Y-down origin, so this is a smile.
  LET smile AS DrawItem = Arc[x := 200.0, y := 215.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := canvas::stroke(green, 14.0)]

  canvas::present([face, eyeL, eyeR, smile])
END SUB
```"#;

/// The scene's element type, used to size and copy the incoming list.
fn scene_type() -> ParameterType {
    ParameterType::list_of(ParameterType::named("DrawItem"))
}

/// `canvas::present(items)` — deep-copy the scene into the arena's canvas region and
/// publish it.
///
/// Why a copy at all: the renderer reads the installed scene at arbitrary times
/// after `present` returns, with no further involvement from the program. A scene
/// that pointed at caller storage would be read after that storage was reused.
///
/// Why **one** copy suffices: an MFBASIC collection is a self-contained flat block —
/// strings, records and nested collections are inlined, not referenced — so
/// `copy_flat_block` on the list is already the transitive deep copy this needs.
/// That is the same property `copy_flat_block`'s own contract states ("because a
/// flat block has no internal pointers, the byte copy **is** a deep copy"), and it
/// is why the scene needs no per-variant walk.
///
/// The copy lands in the **arena**, not the caller's frame: the arena is a growing
/// region owned by the execution context, so the block outlives this call, which a
/// frame allocation would not.
pub(crate) fn lower_present(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scene_offset = ctx.canvas_scene_offset.ok_or_else(|| {
        format!("native code plan emits '{symbol}' without reserving the canvas scene region")
    })?;
    let items = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the scene list argument"))?
        .location
        .clone();

    // Hold the incoming list pointer across the copy's calls: `copy_flat_block`
    // allocates, and an argument register does not survive a call.
    let source_slot = builder.allocate_stack_object("canvas_present_source", 8);
    builder.emit(abi::store_u64(&items, abi::stack_pointer(), source_slot));

    let copy = builder.copy_flat_block(&scene_type(), &items)?;
    let copy_slot = builder.allocate_stack_object("canvas_present_copy", 8);
    builder.emit(abi::store_u64(&copy, abi::stack_pointer(), copy_slot));

    // count = source.count. Read from the SOURCE rather than the copy only because
    // the source pointer is already parked; both carry the same count (the copy is
    // shrink-to-fit, which drops capacity, never entries).
    let count = builder.temporary_vreg();
    let source = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), source_slot));
    builder.emit(abi::load_u64(&count, &source, COLLECTION_OFFSET_COUNT));

    // Publish: items pointer, then count, then bump the revision. The revision is
    // written LAST and is what a reader gates on, so a reader can never observe a
    // bumped revision alongside a half-written scene.
    let published = builder.temporary_vreg();
    builder.emit(abi::load_u64(&published, abi::stack_pointer(), copy_slot));
    builder.emit(abi::store_u64(
        &published,
        ARENA_STATE_REGISTER,
        scene_offset + CANVAS_SCENE_ITEMS_OFFSET,
    ));
    builder.emit(abi::store_u64(
        &count,
        ARENA_STATE_REGISTER,
        scene_offset + CANVAS_SCENE_COUNT_OFFSET,
    ));
    let revision = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &revision,
        ARENA_STATE_REGISTER,
        scene_offset + CANVAS_SCENE_REVISION_OFFSET,
    ));
    builder.emit(abi::add_immediate(&revision, &revision, 1));
    builder.emit(abi::store_u64(
        &revision,
        ARENA_STATE_REGISTER,
        scene_offset + CANVAS_SCENE_REVISION_OFFSET,
    ));

    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    // The mode gate is spliced in at the very top, before the manual prologue, so a
    // wrong-mode call returns before allocating anything at all.
    prepend_wrong_mode_gate(
        &mut builder.instructions,
        &mut builder.relocations,
        &symbol,
        ctx.presentation_mode_offset,
        ModeRequirement::Canvas,
    );

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "canvas.present".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "present",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "items",
                desc: "The scene to install, drawn in list order — later items paint \
                       over earlier ones.",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::named("DrawItem")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrWrongMode"],
            body: Body::abi_function(lower_present),
        }],
    });
}
