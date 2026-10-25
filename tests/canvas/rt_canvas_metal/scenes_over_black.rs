//! plan-176-D: `rt_canvas_metal`'s scene-file comparisons against the software oracle:
//! the long-chord stars, fills, strokes, clips and pictures, and the shapes drawn over
//! black. The build/render helpers and `assert_metal_draws_like_software` live in the
//! root, `rt_canvas_metal.rs`.

use super::*;

// ---------------------------------------------------------------------------------------
// bug-687: polygons over black, where one coverage step is several sRGB levels.
//
// The shader and the software oracle quantise coverage identically
// (`int(clamp(0.5 - d, 0, 1) * 255 + 0.5)`), so they agree on a pixel exactly when they
// agree on its distance to within the gap to the nearest quantisation boundary. The
// shaders' fp32 distance misses that on a few pixels of every long edge, and near black
// one step of coverage moves a channel by up to 13 levels -- exactly the one step
// `GPU_DEFAULT` allows, so these scenes are where that allowance is spent, and a frame
// past it is more than one step off. Every scene here draws on the default opaque-black
// surface for that reason. Shared with `scripts/test-canvas-gpu-rows.sh`, which draws
// them on Vulkan.
//
// The "N pixels past `GPU_DEFAULT` before" notes below are history: they count pixels
// past the 2-level limit of the double-float shaders (bug-687, bug-735), which the move
// to fp32 replaced with the one-step limit.
// ---------------------------------------------------------------------------------------

/// The star polygon {251/97}: 251 long chords (about 300 px each). bug-687 measured
/// max delta 4 at `42cea38df`, under the old 256-edge cap.
const STAR_251_97: &str = include_str!("../scenes/star_251_97_on_black.mfb");

#[test]
fn a_long_chord_star_over_black_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_star_251_97",
        STAR_251_97,
        "the {251/97} star polygon over black",
    );
}

/// The star polygon {301/97}: past the old 256-edge cap, and the most edge length of
/// the stars. bug-687 measured max delta 9.
const STAR_301_97: &str = include_str!("../scenes/star_301_97_on_black.mfb");

#[test]
fn a_three_hundred_one_point_long_chord_star_over_black_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_star_301_97",
        STAR_301_97,
        "the {301/97} star polygon over black",
    );
}

/// The star polygon {301/2}: short edges (about 7.5 px each). bug-687 measured max
/// delta 3.
const STAR_301_2: &str = include_str!("../scenes/star_301_2_on_black.mfb");

#[test]
fn a_short_edge_star_over_black_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_star_301_2",
        STAR_301_2,
        "the {301/2} star polygon over black",
    );
}

/// plan-163-A: the {301/2} star under `NonZero`. It winds twice round its centre, so the
/// disc even-odd leaves empty is filled — the GPUs' signed winding count, over hundreds
/// of short edges, against the oracle's.
const STAR_301_2_NONZERO: &str = include_str!("../scenes/star_301_2_nonzero_on_black.mfb");

#[test]
fn a_short_edge_star_filled_non_zero_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_star_301_2_nonzero",
        STAR_301_2_NONZERO,
        "the {301/2} star polygon filled NonZero over black",
    );
}

/// plan-163-A: both fill rules where they differ — a ring with a hole wound each way, a
/// {5/2} pentagram, a stroked translucent three-ring item and a transformed one — at
/// fractional coordinates, so the rims are antialiased under each rule.
const FILL_RULES: &str = include_str!("../scenes/fill_rules_on_black.mfb");

#[test]
fn both_fill_rules_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_fill_rules",
        FILL_RULES,
        "the NonZero and EvenOdd ring scene over black",
    );
}

/// plan-163-B: Square caps on lines (several angles and a zero-length one), arcs and
/// polylines, at fractional coordinates, plus a transformed line.
const SQUARE_CAPS: &str = include_str!("../scenes/square_caps_on_black.mfb");

#[test]
fn square_caps_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_square_caps",
        SQUARE_CAPS,
        "the Square-capped lines, arcs and polylines over black",
    );
}

/// plan-163-B: Miter and Bevel joins on polygons (a pentagram at both miter limits, a
/// concave arrow, two rings), polylines (sharp turns, a repeated point, per-point colours),
/// rectangles, a rotated polygon and a translucent stroke over a fill.
const JOINS: &str = include_str!("../scenes/joins_on_black.mfb");

#[test]
fn joins_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_joins",
        JOINS,
        "the mitered and beveled polygons, polylines and rectangles over black",
    );
}

/// plan-163-D: gradients in the item's own space -- turned by their own transform and by
/// the item's, the three spreads, a 2:1 radial, a focus and a focal disc, the ramp painting
/// strokes under three stroke colours and a Line, and two stroked gradients under
/// Multiply.
const GRADIENTS: &str = include_str!("../scenes/gradients_on_black.mfb");

#[test]
fn gradients_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_gradients",
        GRADIENTS,
        "the gradients over black",
    );
}

/// plan-163-C: path clips -- a triangle, a ring with a hole under each rule, rotated,
/// sheared and grouped items clipped in their own coordinates, a rectangle clip by either
/// path, and a dashed stroke clipped by a triangle.
const PATH_CLIPS: &str = include_str!("../scenes/path_clips_on_black.mfb");

#[test]
fn path_clips_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_path_clips",
        PATH_CLIPS,
        "the path-clipped shapes over black",
    );
}

/// plan-172-E: a rectangle's corners -- the exact square and rounded records beside the
/// flattened outline of per-corner and elliptical corners, filled, stroked and dashed,
/// and `Center`- and `TopLeft`-bound rectangles under a turn.
const CORNERS: &str = include_str!("../scenes/rect_corners_on_black.mfb");

#[test]
fn corners_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_corners",
        CORNERS,
        "the rectangle corners over black",
    );
}

/// bug-995: path clips on transformed items and on items inside groups with a matrix
/// (`scenes/transformed_clips_on_black.mfb`) -- the five-sample clip path, at scales 0.1,
/// 1 and 4, under a shear, banded and unbanded, with a rectangle clip under a group
/// matrix. A pixel whose centre clip sample is past the clip's reach is answered from
/// that sample alone; this row holds that shortcut to the oracle's every pixel.
const TRANSFORMED_CLIPS: &str = include_str!("../scenes/transformed_clips_on_black.mfb");

#[test]
fn transformed_path_clips_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_transformed_clips",
        TRANSFORMED_CLIPS,
        "path clips under item transforms and group matrices",
    );
}

/// bug-1085: a banded polygon and a banded path clip (256 edges each) inside groups that
/// shrink by 20, and a banded polygon with its own transform inside one
/// (`scenes/banded_in_shrinking_groups_on_black.mfb`). The band index is built for the
/// item's inverse alone, so under these matrices a pixel's neighbour samples fall outside
/// the band's reach; the shader has to see that from the band header's recorded norm and
/// walk every edge. Taking the band anyway put 60-odd pixels of each a coverage step off.
const BANDED_IN_SHRINKING_GROUPS: &str =
    include_str!("../scenes/banded_in_shrinking_groups_on_black.mfb");

#[test]
fn banded_shapes_in_shrinking_groups_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_banded_shrinking_groups",
        BANDED_IN_SHRINKING_GROUPS,
        "banded polygons and path clips inside shrinking groups",
    );
}

/// bug-1088: arcs of radius under 1 under transforms that magnify by 60 to 100 -- butt,
/// reflex butt, round, square, turned and dashed (`scenes/magnified_arcs_on_black.mfb`).
/// A butt arc's sweep is two signed half-planes, so its ends are antialiased and its
/// distance is 1-Lipschitz; the shaders' arc distance and the shortcut an arc now takes
/// have to agree with the oracle's, which `rt_canvas_rasteriser`'s
/// `a_magnified_butt_arc_draws_its_band_and_antialiased_ends_only` holds to the band.
const MAGNIFIED_ARCS: &str = include_str!("../scenes/magnified_arcs_on_black.mfb");

#[test]
fn magnified_arcs_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_magnified_arcs",
        MAGNIFIED_ARCS,
        "arcs under magnifying transforms",
    );
}

/// plan-172-L: pictures inside shapes (`scenes/picture_shapes_on_black.mfb`). Every
/// shaped picture is a picture record with a polygonal clip and, where it strokes, the
/// shape itself -- two entries the GPU already draws -- so this is the check that the
/// two walks lay the same entries out: a top-level picture that is two entries is
/// resolved by the draw walk, not the native layout.
const PICTURE_SHAPES: &str = include_str!("../scenes/picture_shapes_on_black.mfb");

#[test]
fn picture_shapes_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_picture_shapes",
        PICTURE_SHAPES,
        "pictures inside shapes over black",
    );
}

/// bug-1079: plain pictures whose texel boundaries are pixel centres, in x and in y
/// (`scenes/picture_texel_edge.mfb`). The texel there used to hang on the last bit of
/// the backend's division; Apple's GPUs divide exactly, so this row passed before the
/// index was made exact and holds Metal to the same frame after.
const PICTURE_TEXEL_EDGE: &str = include_str!("../scenes/picture_texel_edge.mfb");

#[test]
fn texel_boundaries_on_pixel_centres_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_texel_edge",
        PICTURE_TEXEL_EDGE,
        "pictures with texel boundaries on pixel centres",
    );
}

/// The same scene with the pointer on its `AutoPaint` picture, the circle about
/// (770, 470): both of the picture's entries are swapped for the hover paint's -- the
/// image under an orange tint, the outline 6 px of red -- and the GPU draw list must
/// name those two records, not the ones the walk appended.
///
/// (818, 470) is 48.5 px from the centre of a circle of radius 48: on the hover
/// outline, which covers 45 to 51, and red. At rest the circle's own 2 px outline
/// there is white.
#[test]
fn a_hovered_autopaint_picture_matches_the_software_oracle_on_metal() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_picture_auto", PICTURE_SHAPES);
    let pixel = |frame: &Frame, x: usize, y: usize| {
        let at = (y * WIDTH as usize + x) * 4;
        [frame.pixels[at], frame.pixels[at + 1], frame.pixels[at + 2]]
    };
    let envs = [("MFB_CANVAS_POINTER", "0,770,470,0,1")];
    let (software, software_stats) = render_env(&program, false, "sw", &envs);
    let (gpu, stats) = render_env(&program, true, "gpu", &envs);
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        software_stats.contains(" auto=3"),
        "the software run did not hover the picture: {software_stats}"
    );
    assert!(
        !stats.contains("gpuFrames=0"),
        "the hovered scene fell back to the software renderer: {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!("Metal draws the hovered picture differently from the software oracle: {diff}");
    }
    let (rest, _) = render(&program, false, "rest");
    for (name, frame) in [("software", &software), ("Metal", &gpu)] {
        let outline = pixel(frame, 818, 470);
        assert!(
            outline[0] > 200 && outline[1] < 80 && outline[2] < 80,
            "the hover outline is {outline:?} on {name}"
        );
        assert_ne!(
            pixel(frame, 770, 470),
            pixel(&rest, 770, 470),
            "the hover tint did not change the image on {name}"
        );
    }
}

/// plan-163-B: dashed strokes -- polygons whose dashes cross their corners under each cap
/// and under Miter and Bevel joins, an odd pattern with an offset, dashed polylines and
/// lines, a circle, a rotated ellipse, a rounded rectangle, an arc, a rectangle and a
/// rotated polygon.
const DASHES: &str = include_str!("../scenes/dashes_on_black.mfb");

#[test]
fn dashes_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_dashes",
        DASHES,
        "the dashed polygons, polylines, lines, curves and rectangles over black",
    );
}

/// `LARGE_POLYGONS` without its grey ground: filled, translucent, stroked, transformed
/// and self-intersecting polygons of 300-4,000 edges, every rim over black. bug-688
/// measured max delta 4 on Vulkan (lavapipe) at (736, 52), the rotated ring's rim.
const LARGE_POLYGONS_ON_BLACK: &str = include_str!("../scenes/large_polygons_on_black.mfb");

#[test]
fn polygons_past_two_hundred_fifty_six_edges_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_large_polygons_on_black",
        LARGE_POLYGONS_ON_BLACK,
        "filled, stroked, transformed and self-intersecting polygons over black",
    );
}

/// 140 small filled polygons over black (bug-687): every rim pixel of every ring is near
/// black. Before the double-float polygon path Metal put 3 pixels past `GPU_DEFAULT`
/// here, worst 4.
const SMALL_POLYGONS_ON_BLACK: &str = include_str!("../scenes/small_polygons_on_black.mfb");

#[test]
fn small_polygons_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_small_polygons_on_black",
        SMALL_POLYGONS_ON_BLACK,
        "140 small filled polygons over black",
    );
}

/// 140 rectangles over black (bug-687): a guard. Rectangles keep their fp32 distance and
/// measured worst 2 on Metal and on Vulkan -- inside `GPU_DEFAULT`, only just.
const RECTANGLES_ON_BLACK: &str = include_str!("../scenes/rectangles_on_black.mfb");

#[test]
fn rectangles_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_rectangles_on_black",
        RECTANGLES_ON_BLACK,
        "140 filled rectangles over black",
    );
}

// ---------------------------------------------------------------------------------------
// bug-735: every other shape kind over black. bug-687 fixed the filled polygon; these rows
// hold the rest -- every distance input carried at double-float precision and every
// distance evaluated in double-float, in both shaders. Shared with
// `scripts/test-canvas-gpu-rows.sh`, which draws them on Vulkan.
// ---------------------------------------------------------------------------------------

/// 140 filled circles over black (bug-735). Metal put 2 pixels past `GPU_DEFAULT` here before
/// bug-735, worst 6.
const CIRCLES_ON_BLACK: &str = include_str!("../scenes/circles_on_black.mfb");

#[test]
fn circles_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_circles_on_black",
        CIRCLES_ON_BLACK,
        "140 filled circles over black",
    );
}

/// 140 filled rotated ellipses over black (bug-735): the 24-step bisection in fp32 took a
/// different branch than the oracle's f64 one. 8 pixels past `GPU_DEFAULT` before, worst 9.
const ELLIPSES_ON_BLACK: &str = include_str!("../scenes/ellipses_on_black.mfb");

#[test]
fn rotated_ellipses_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_ellipses_on_black",
        ELLIPSES_ON_BLACK,
        "140 filled rotated ellipses over black",
    );
}

/// 140 round-capped 2-px lines over black (bug-735). 1 pixel past `GPU_DEFAULT` before, worst 3.
const LINES_ON_BLACK: &str = include_str!("../scenes/lines_on_black.mfb");

#[test]
fn round_capped_lines_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_lines_on_black",
        LINES_ON_BLACK,
        "140 round-capped lines over black",
    );
}

/// 140 butt-capped 2-px arcs over black (bug-735). 3 pixels past `GPU_DEFAULT` before, worst 9.
const ARCS_ON_BLACK: &str = include_str!("../scenes/arcs_on_black.mfb");

#[test]
fn butt_capped_arcs_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_arcs_on_black",
        ARCS_ON_BLACK,
        "140 butt-capped arcs over black",
    );
}

/// 140 stroked polygons over black whose half-width is off the 16.16 grid (bug-735): the
/// half-width reached the shader as 16.16. 1 pixel past `GPU_DEFAULT` before, worst 3.
const STROKED_POLYGONS_ON_BLACK: &str = include_str!("../scenes/stroked_polygons_on_black.mfb");

#[test]
fn polygon_strokes_off_the_fixed_point_grid_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_stroked_polygons_on_black",
        STROKED_POLYGONS_ON_BLACK,
        "140 stroked polygons over black",
    );
}

/// 140 round-capped 40-point polylines over black (bug-735). 2 pixels past `GPU_DEFAULT`
/// before, worst 6.
const POLYLINES_ON_BLACK: &str = include_str!("../scenes/polylines_on_black.mfb");

#[test]
fn polylines_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_polylines_on_black",
        POLYLINES_ON_BLACK,
        "140 round-capped polylines over black",
    );
}

/// 140 filled polygons over black, each in a group drawn at an offset off the 16.16 grid
/// (bug-735): the draw offset reached the shader as 16.16. 4 pixels past `GPU_DEFAULT`
/// before, worst 4.
const GROUPED_POLYGONS_ON_BLACK: &str = include_str!("../scenes/grouped_polygons_on_black.mfb");

#[test]
fn polygons_in_groups_at_off_grid_offsets_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_grouped_polygons_on_black",
        GROUPED_POLYGONS_ON_BLACK,
        "140 grouped polygons over black",
    );
}

/// Butt-capped lines, round-capped arcs, filled-and-stroked circles, ellipses and rounded
/// rectangles, and butt-capped polylines over black, every stroke width off the 16.16 grid
/// (bug-735). 5 pixels past `GPU_DEFAULT` before, worst 12.
const STROKED_SHAPES_ON_BLACK: &str = include_str!("../scenes/stroked_shapes_on_black.mfb");

#[test]
fn stroked_and_capped_shapes_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_stroked_shapes_on_black",
        STROKED_SHAPES_ON_BLACK,
        "butt lines, round arcs, stroked closed shapes and butt polylines over black",
    );
}

/// 140 rectangles over black whose every edge sits a few 1e-6 px from a coverage step
/// (bug-735). `rectangles_on_black` passes at worst 2 only because its edges happen to
/// land away from one; these do not, and the 16.16 narrowing alone moved 3,623 pixels
/// past `GPU_DEFAULT` before (worst 12).
const RECTANGLES_AT_A_STEP_ON_BLACK: &str =
    include_str!("../scenes/rectangles_at_a_step_on_black.mfb");

#[test]
fn rectangles_with_edges_at_a_coverage_step_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_rectangles_at_a_step",
        RECTANGLES_AT_A_STEP_ON_BLACK,
        "rectangles with every edge beside a coverage step",
    );
}

/// The same edges as `rectangles_at_a_step_on_black`, but as a `Paint.clip` (bug-735):
/// the clip's own coverage is the same rectangle distance. 4,684 pixels past
/// `GPU_DEFAULT` before, worst 12.
const CLIPS_AT_A_STEP_ON_BLACK: &str = include_str!("../scenes/clips_at_a_step_on_black.mfb");

#[test]
fn clips_with_edges_at_a_coverage_step_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_clips_at_a_step",
        CLIPS_AT_A_STEP_ON_BLACK,
        "clip rectangles with every edge beside a coverage step",
    );
}

/// The items of `stroked_shapes_on_black`, each under a rotation and scale about its
/// centre, every other one clipped (bug-735): the five-sample transformed path and the
/// clip in double-float. 5 pixels past `GPU_DEFAULT` before, worst 9.
const TRANSFORMED_SHAPES_ON_BLACK: &str = include_str!("../scenes/transformed_shapes_on_black.mfb");

#[test]
fn transformed_and_clipped_shapes_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_transformed_shapes_on_black",
        TRANSFORMED_SHAPES_ON_BLACK,
        "transformed, clipped, stroked and capped shapes over black",
    );
}

/// A Multiply-blended filled rectangle stroked 1e-5 px wide, then two opaque items
/// (bug-735). The draw list splits it into two records on the f64 `half > 0`; before
/// bug-735 both emitters split on the 16.16 word, 0 here, and published one, so every
/// later draw named the wrong block: 20,632 pixels wrong on Metal, worst 101.
const BLENDED_HAIRLINE_STROKE: &str = include_str!("../scenes/blended_hairline_stroke.mfb");

#[test]
fn a_blended_item_stroked_under_the_fixed_point_step_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_blended_hairline_stroke",
        BLENDED_HAIRLINE_STROKE,
        "a blended rectangle with a 1e-5 px stroke, then two opaque items",
    );
}

/// plan-172-K: `canvas::AutoPaint` on the GPU. The scene's own comment says what each
/// pointer position puts under test.
const AUTOPAINT: &str = include_str!("../scenes/autopaint_on_black.mfb");

/// A hovered `AutoPaint` draws on Metal as the software oracle draws it — for a button
/// in the scene itself, and for one use of a group that is drawn twice.
///
/// The state is the graphics thread's, resolved after the walk, and the two renderers
/// read it by different routes: software from the frame's draw entries, Metal from the
/// GPU draw list, which is laid out from `__CANVAS_TOP_OFFSETS` and, for a group, from
/// a run shared by every node that draws the group. So a hover that reached one route
/// and not the other is a wrong frame on one backend only, and this is where it shows.
///
/// Each case also checks the Metal frame itself — the hovered button is the hover
/// paint's orange and the other use of the group is still blue — because two frames
/// that agree on NO hover agree within tolerance too.
#[test]
fn a_hovered_autopaint_matches_the_software_oracle_on_metal() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_autopaint", AUTOPAINT);
    std::fs::write(
        program.project.join("fixture.ttf"),
        common::fixture_truetype(),
    )
    .expect("write the font fixture");
    let pixel = |frame: &Frame, x: usize, y: usize| {
        let at = (y * WIDTH as usize + x) * 4;
        [frame.pixels[at], frame.pixels[at + 1], frame.pixels[at + 2]]
    };
    let lit = |rgb: [u8; 3]| rgb[0] > 200 && rgb[2] < 120;
    let unlit = |rgb: [u8; 3]| rgb[0] < 100 && rgb[2] > 150;
    for (what, pointer, hovered, other) in [
        // (280, 120) is on the scene's own button; (150, 320) is the group's first use.
        (
            "the scene's own button",
            "0,280,120,0,1",
            (280, 120),
            (150, 320),
        ),
        // (150, 320) is the group's first use; (550, 320) its second.
        (
            "one use of a group",
            "0,150,320,0,1",
            (150, 320),
            (550, 320),
        ),
    ] {
        let envs = [("MFB_CANVAS_POINTER", pointer)];
        let (software, software_stats) = render_env(&program, false, "sw", &envs);
        let (gpu, stats) = render_env(&program, true, "gpu", &envs);
        if !metal_built(&stats) {
            return; // no Metal device on this host (§metal_built)
        }
        assert!(
            software_stats.contains(" auto=300") || software_stats.contains(" auto=030"),
            "{what}: the software run did not hover exactly one button: {software_stats}"
        );
        assert!(
            !stats.contains("gpuFrames=0"),
            "{what}: the hovered scene fell back to the software renderer: {stats}"
        );
        if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
            panic!("Metal draws {what}, hovered, differently from the software oracle: {diff}");
        }
        for (name, frame) in [("software", &software), ("Metal", &gpu)] {
            let on = pixel(frame, hovered.0, hovered.1);
            let off = pixel(frame, other.0, other.1);
            assert!(lit(on), "{what}: the hovered button is {on:?} on {name}");
            assert!(unlit(off), "{what}: the other button is {off:?} on {name}");
        }
    }
}
