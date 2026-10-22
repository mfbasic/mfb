//! The native `DrawItem` hash and id pass are generated from the type model, and refuse
//! a layout they cannot walk.
//!
//! `canvas::itemHash`, `canvas::sceneHashes` and `canvas::sceneIdBits`
//! (`canvas/func_item_hash.rs`) are not written against the canvas records: they walk
//! whatever fields the type model says each `DrawItem` variant has, so a field a
//! variant gains is folded without anyone remembering to. The other half of that
//! contract is the module's promise that "a field of a type it does not know how to
//! fold fails the BUILD", and the id pass's that a variant whose first field is not
//! `id AS String` fails it too — rather than hashing or reading the wrong slot.
//!
//! The canvas package's own records satisfy every rule, so no canvas program reaches a
//! refusal. These tests hand the lowerings the type model of a program's OWN
//! `DrawItem` union, with the canvas package's records taken out of it, so the bare
//! names are the ones `record_type`/`variant` find (their fallback for a helper
//! compiled inside the package), and change one variant's fields at a time.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::builder::{TypeModel, ValueResult};
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::registry::abi_function_lower;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

/// Every `DrawItem` variant the lowerings look up, in union order.
const VARIANTS: [&str; 15] = [
    "Rectangle",
    "Circle",
    "Line",
    "Arc",
    "Polygon",
    "Picture",
    "Ellipse",
    "PolyLine",
    "PolyCurve",
    "CurvePolygon",
    "Tile",
    "Text",
    "Group",
    "AutoPaint",
    "ParticleSystem",
];

/// A program declaring a `DrawItem` union of every variant, each `TYPE` holding
/// `id AS String` and `n AS Integer` unless `fields` overrides it, plus the shape
/// union and a `Holder` record whose function value makes it pointer-held.
fn program(fields: &[(&str, &str)]) -> String {
    let mut source = String::from(
        "IMPORT io\n\nUNION AutoShape\n  Circle\n  Ellipse\nEND UNION\n\n\
         TYPE Holder\n  f AS FUNC() AS Integer\nEND TYPE\n\n",
    );
    for name in VARIANTS {
        let body = fields
            .iter()
            .find(|(variant, _)| *variant == name)
            .map(|(_, body)| body.to_string())
            .unwrap_or_else(|| "  id AS String\n  n AS Integer\n".to_string());
        source.push_str(&format!("TYPE {name}\n{body}END TYPE\n\n"));
    }
    source.push_str("UNION DrawItem\n");
    for name in VARIANTS {
        source.push_str(&format!("  {name}\n"));
    }
    source.push_str(
        "END UNION\n\nFUNC main() AS Integer\n  io::print(\"ok\")\n  RETURN 0\nEND FUNC\n",
    );
    source
}

/// Lower the `canvas.<member>` body against `fields`' model, with one argument of
/// each of `arguments`.
fn lower(
    member: &str,
    fields: &[(&str, &str)],
    arguments: &[ParameterType],
) -> Result<Vec<CodeInstruction>, String> {
    let module = nir_for_src(
        &program(fields),
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    // The model carries the canvas package's own records whether or not the program
    // imports it, and the lowerings try the package-qualified spelling first. Drop
    // them, so the program's own `DrawItem` and its variants are the ones walked.
    let mut type_model = TypeModel::from_module(&module).expect("the model builds");
    let canvas = |ty: &ParameterType| ty.name().starts_with("canvas.");
    type_model.record_fields.retain(|ty, _| !canvas(ty));
    type_model.union_variant_tags.retain(|ty, _| !canvas(ty));
    let harness = BuilderHarness {
        type_model,
        ..BuilderHarness::default()
    };
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_canvas_probe", &platform);
    let ctx = harness.abi_ctx(&platform);
    let args: Vec<ValueResult> = arguments
        .iter()
        .enumerate()
        .map(|(index, type_)| ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(builder.temporary_vreg()),
            text: format!("arg{index}"),
        })
        .collect();
    let (lower, _) = abi_function_lower(&format!("canvas.{member}"))
        .unwrap_or_else(|| panic!("canvas.{member} is an abi_function member"));
    lower(&mut builder, &args, &ctx)?;
    Ok(builder.instructions)
}

fn item_hash(fields: &[(&str, &str)]) -> Result<Vec<CodeInstruction>, String> {
    lower("itemHash", fields, &[ParameterType::named("DrawItem")])
}

fn scene_id_bits(fields: &[(&str, &str)]) -> Result<Vec<CodeInstruction>, String> {
    lower(
        "sceneIdBits",
        fields,
        &[ParameterType::list_of(ParameterType::named("DrawItem"))],
    )
}

fn count(instructions: &[CodeInstruction], op: CodeOp) -> usize {
    instructions.iter().filter(|i| i.op == op).count()
}

/// A list of a fixed-width element folds each element at its own width: a byte
/// list's elements as bytes, a `Scalar` list's as 32-bit words.
#[test]
fn a_fixed_width_list_folds_each_element_at_its_width() {
    let plain = item_hash(&[]).expect("the plain model hashes");
    let lists = item_hash(&[(
        "Rectangle",
        "  id AS String\n  bytes AS List OF Byte\n  marks AS List OF Scalar\n",
    )])
    .expect("byte and scalar lists hash");
    assert_eq!(
        count(&plain, CodeOp::LdrU32),
        0,
        "nothing in the plain model is 32 bits wide"
    );
    assert!(
        count(&lists, CodeOp::LdrU32) > 0,
        "a Scalar element is folded as a 32-bit load"
    );
    assert!(
        count(&lists, CodeOp::LdrU8) > count(&plain, CodeOp::LdrU8),
        "a Byte element is folded as a byte load"
    );
}

/// A field of a type the walk does not fold fails the build, naming the field.
#[test]
fn a_field_the_hash_cannot_fold_fails_the_build() {
    let err = item_hash(&[(
        "Rectangle",
        "  id AS String\n  m AS Map OF String TO Integer\n",
    )])
    .err()
    .expect("a Map field is not folded");
    assert_eq!(
        err,
        "canvas::itemHash: 'Rectangle.m' has type 'Map OF String TO Integer', which the \
         structural hash does not fold"
    );
    let err = item_hash(&[("Rectangle", "  id AS String\n  names AS List OF String\n")])
        .err()
        .expect("a list of Strings is not folded");
    assert_eq!(
        err,
        "canvas::itemHash: a list of 'String' is neither fixed-width, a list nor a record"
    );
}

/// The walk goes round the dispatch once, for a picture's one shape union; a record
/// holding two would have its second silently skipped, so it fails the build.
#[test]
fn a_record_holding_two_shape_unions_fails_the_build() {
    let err = item_hash(&[(
        "Picture",
        "  id AS String\n  shape AS AutoShape\n  backdrop AS AutoShape\n",
    )])
    .err()
    .expect("two shape unions");
    assert_eq!(
        err,
        "canvas::itemHash: 'Picture' holds two shape unions; the walk goes round once"
    );
}

/// `sceneHashes` takes the scene and the carried hashes.
#[test]
fn scene_hashes_without_the_carried_hashes_is_refused() {
    let err = lower(
        "sceneHashes",
        &[],
        &[ParameterType::list_of(ParameterType::named("DrawItem"))],
    )
    .err()
    .expect("one argument of two");
    assert!(
        err.ends_with("expects the scene and the carried hashes"),
        "{err}"
    );
}

/// The id pass reads every hashed variant's id from its record's FIRST slot, and a
/// picture's shape's id one load further: a variant that breaks either assumption
/// fails the build instead of answering from the wrong slot.
#[test]
fn the_id_pass_refuses_a_variant_whose_first_field_is_not_its_id() {
    scene_id_bits(&[("Picture", "  id AS String\n  shape AS AutoShape\n")])
        .expect("every variant starts with its id, and the picture holds a shape");
    let err = scene_id_bits(&[("Circle", "  n AS Integer\n  id AS String\n")])
        .err()
        .expect("Circle's id is not first");
    assert_eq!(
        err,
        "canvas::sceneIdBits: 'Circle' does not start with `id AS String`"
    );
    let err = scene_id_bits(&[("Picture", "  id AS String\n  shape AS Integer\n")])
        .err()
        .expect("the shape is not the shape union");
    assert_eq!(
        err,
        "canvas::sceneIdBits: 'Picture.shape' is 'Integer', not the shape union"
    );
}

/// A composite that cannot be copied as a flat block -- here anything holding a
/// function value -- is held in its record by POINTER rather than inlined, and both
/// walks follow the pointer: into a nested record, into each element of a list of
/// such records, and through a picture's shape union. The function value itself is
/// then refused, and the refusal naming the INNER field is what shows the walk got
/// there.
#[test]
fn composites_held_by_pointer_are_followed() {
    let unfoldable = |field: &str| {
        format!(
            "canvas::itemHash: '{field}' has type 'FUNC() AS Integer', which the structural \
             hash does not fold"
        )
    };
    for rectangle in [
        "  id AS String\n  holder AS Holder\n",
        "  id AS String\n  holders AS List OF Holder\n",
    ] {
        let err = item_hash(&[("Rectangle", rectangle)])
            .err()
            .expect("the holder's function value is not folded");
        assert_eq!(err, unfoldable("Holder.f"), "{rectangle:?}");
    }
    // A function value in `Ellipse` makes `AutoShape` pointer-held. The picture's arm
    // follows its shape through the pointer; `Ellipse`'s own arm, emitted after it,
    // then meets the function value.
    let pointer_shape = [
        ("Picture", "  id AS String\n  shape AS AutoShape\n"),
        ("Ellipse", "  id AS String\n  f AS FUNC() AS Integer\n"),
    ];
    let err = item_hash(&pointer_shape)
        .err()
        .expect("Ellipse's function value is not folded");
    assert_eq!(err, unfoldable("Ellipse.f"));
    scene_id_bits(&pointer_shape).expect("a pointer-held shape's id is read through the pointer");
}
