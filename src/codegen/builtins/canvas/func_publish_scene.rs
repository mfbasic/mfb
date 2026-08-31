//! `canvas::publishScene` / `canvas::publishLayers` — the internal native publishers.
//!
//! These are the `abi_function` half of `canvas::present` / `canvas::presentLayers`:
//! they deep-copy the incoming list into the arena's scene region, publish it, and
//! report whether they actually published. They are **internal-only** — resolvable
//! from the package's own injected source and nothing else — because a program has
//! no use for a publish that does not also render.
//!
//! Splitting the members this way is what lets the frame skip pay for itself. The
//! publish is three stores; the render is the whole scene. A skip that saved only the
//! stores would save nothing, so the publisher returns a `Boolean` and the public
//! member renders only when it is `TRUE`.
//!
//! The `Mode.Canvas` gate lives here rather than on the public member, so it fires
//! before the deep copy allocates anything.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

use super::gen_present::{emit_publish, SceneShape};

pub(crate) fn lower_publish_scene(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    emit_publish(builder, args, ctx, SceneShape::Flat)
}

pub(crate) fn lower_publish_layers(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    emit_publish(builder, args, ctx, SceneShape::Layered)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "publishScene",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "items",
                desc: "",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::named("DrawItem")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec!["ErrWrongMode"],
            body: Body::abi_function(lower_publish_scene),
        }],
    });

    pkg.add_function(RegistryFunction {
        name: "publishLayers",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "layers",
                desc: "",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::named("DrawLayer")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec!["ErrWrongMode"],
            body: Body::abi_function(lower_publish_layers),
        }],
    });
}
