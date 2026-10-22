//! The canvas `abi_function` bodies read their arguments positionally, and a body
//! handed fewer than it reads names the first one it is missing — never a register
//! nothing set. The front end checks every call's arity first, so each body is
//! driven directly, with a prefix of its arguments.

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::registry::abi_function_lower;
use crate::types::ParameterType;

/// The refusal `canvas.<member>`'s body gives when handed `count` arguments.
fn refusal(member: &str, count: usize) -> String {
    let harness = BuilderHarness::default();
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_canvas_probe", &platform);
    let ctx = harness.abi_ctx(&platform);
    let args: Vec<ValueResult> = (0..count)
        .map(|index| ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(builder.temporary_vreg()),
            text: format!("arg{index}"),
        })
        .collect();
    let (lower, _) = abi_function_lower(&format!("canvas.{member}"))
        .unwrap_or_else(|| panic!("canvas.{member} is an abi_function member"));
    match lower(&mut builder, &args, &ctx) {
        Ok(value) => panic!(
            "canvas.{member} with {count} argument(s) lowered to {}",
            value.text
        ),
        Err(error) => error,
    }
}

/// Each prefix of `members`' arguments is refused naming the next one.
fn assert_each_prefix_refused(member: &str, expected: &[&str]) {
    for (count, what) in expected.iter().enumerate() {
        assert_eq!(
            refusal(member, count),
            format!("'_mfb_canvas_probe' expects {what}"),
            "canvas.{member} with {count} argument(s)"
        );
    }
}

#[test]
fn blit_surface_names_each_missing_argument() {
    assert_each_prefix_refused(
        "blitSurface",
        &[
            "the frame buffer argument",
            "the width argument",
            "the height argument",
        ],
    );
}

#[test]
fn font_from_bytes_names_each_missing_argument() {
    assert_each_prefix_refused("fontFromBytes", &["the font bytes", "the face index"]);
}

/// The three scene renderers read the same bank — the surface (absent from the
/// present form), the size, the geometry and the draw tables.
#[test]
fn the_scene_renderers_name_their_first_missing_argument() {
    for member in ["metalDrawScene", "vulkanDrawScene"] {
        assert_eq!(
            refusal(member, 0),
            "'_mfb_canvas_probe' expects the surface argument",
            "canvas.{member}"
        );
    }
    for member in ["vulkanPresentScene", "metalPresentScene"] {
        assert_eq!(
            refusal(member, 0),
            "'_mfb_canvas_probe' expects the width argument",
            "canvas.{member}"
        );
    }
}
