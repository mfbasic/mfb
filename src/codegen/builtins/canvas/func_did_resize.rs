//! `canvas::didResize` — has the surface changed size since you last asked?

use crate::codegen::app::hook::app::{prepend_wrong_mode_gate, ModeRequirement};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::codegen::runtime::canvas::{emit_did_resize, GraphicsScratch};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"TRUE once after the canvas surface changes size."#;

const DESC: &str = r#"`didResize` answers TRUE the first time it is called after the surface's dimensions
change, and FALSE every time after that until the next change. Asking is what
acknowledges it, so a program polls it once per frame and lays out again only when it
says so:

The answer is per *change*, not per event. A window dragged across ten sizes between
two calls reports one resize, because that is what the program has to react to; and a
platform event that re-publishes the size it already had reports none at all, which
AppKit in particular sends routinely.

There is no resize *callback*, and that is deliberate: a callback would run on the
platform's UI thread, where a program cannot safely touch its own state. This is a
plain read from the worker, in the program's own loop, where everything else it owns
is reachable.

`canvas::getSize` reports the current dimensions; this reports only that they moved.
A program that lays out from scratch each frame does not need this at all — the
surface size is already an input to that. It is for programs that cache a layout and
want to know when the cache is stale.

Requires `Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`."#;

const EX: &str = r#"A banner that always spans the window: laid out once, and again only when the
surface changes size.

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT items AS List OF canvas::DrawItem = []
  MUT frame AS Integer = 0
  WHILE frame < 600
    IF canvas::didResize() OR len(items) = 0 THEN
      LET size AS canvas::Size = canvas::getSize()
      LET banner AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := toFloat(size.width), h := toFloat(size.height) / 8.0, paint := canvas::fill(canvas::rgb(30, 90, 200))]
      items = [banner]
    END IF
    canvas::present(items)
    frame = frame + 1
  END WHILE
END SUB
```"#;

/// `canvas::didResize() AS Boolean`.
///
/// Read-and-acknowledge against a counter the platform bumps, rather than a flag it
/// sets: only the main thread writes the counter and only the worker writes the
/// acknowledged value, so the two never race for a word. A single read-and-clear flag
/// would lose a resize that landed between the reader's load and its store — on the one
/// path whose entire job is to report edges.
pub(crate) fn lower_did_resize(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_did_resize(
        &symbol,
        &scratch,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    // The same gate every surface-touching member takes: outside `Mode.Canvas` there is
    // no surface, so "did it resize" has no answer to give and trapping is the honest
    // one.
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
        text: symbol,
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "didResize",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec!["ErrWrongMode"],
            body: Body::abi_function(lower_did_resize),
        }],
    });
}
