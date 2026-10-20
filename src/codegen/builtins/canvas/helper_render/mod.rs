//! `__canvas_renderScene` — the entry point `canvas::present` calls after a publish
//! that actually changed something.
//!
//! The renderer is written in MFBASIC source rather than emitted per architecture,
//! which is the same choice `json`, `regex` and `crypto` make for their algorithmic
//! cores. For a rasteriser it is doubly right: it is the **oracle** the GPU backends
//! (plan-98-E/F) are compared against, so it must produce identical pixels on every
//! target, and one source implementation gives that for free where five hand-written
//! assembly ports would each be a place for the oracle to disagree with itself.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};
use crate::codegen::runtime::canvas::{
    CANVAS_DRAW_ENTRY_WORDS, CANVAS_DRAW_PASS_CONTINUES, CANVAS_DRAW_PASS_MORE,
    CANVAS_DRAW_PASS_WORD, METAL_MAX_FRAME_DASH_WORDS, METAL_MAX_FRAME_EDGES,
    METAL_MAX_FRAME_GLYPH_SAMPLES, METAL_MAX_FRAME_GRADIENT_STOPS, METAL_MAX_FRAME_ITEMS,
    VULKAN_MAX_FRAME_DASH_WORDS, VULKAN_MAX_FRAME_EDGES, VULKAN_MAX_FRAME_GLYPH_SAMPLES,
    VULKAN_MAX_FRAME_GRADIENT_STOPS, VULKAN_MAX_FRAME_ITEMS,
};
use std::sync::LazyLock;

mod gpu_draw_list;
mod gpu_groups;
mod gpu_passes;
mod present;
mod render_loop;
mod scene;
use present::{CLOSE_RETIRED, DIRECT_PRESENT, DIRECT_PRESENT_DEBUG};
use render_loop::{
    ENSURE_GRAPHICS, PHASE_TIMERS, PHASE_TIMERS_DEBUG, POINTER_SCRIPT, POINTER_SCRIPT_DEBUG,
    RENDER_LOOP, RENDER_LOOP_DEBUG, TEST_SWITCHES, TEST_SWITCHES_DEBUG,
};
use scene::{HASH_SCENE, RENDER_SCENE};

/// The GPU renderer (plan-98-E), and the scene walk both backends share.
///
/// `__canvas_sceneOffsets` is the same traversal `__canvas_renderScene` does — flat
/// items then layers in order, one hash index across both — reduced to what a
/// backend actually needs: the cache offset of each item's geometry, in draw order.
/// Generating it costs nothing extra when the Metal path declines, because
/// `__canvas_geometryFor` is the cache and the software walk that follows hits it.
///
/// `__canvas_metalRenderable` and `__canvas_vulkanRenderable` are the honesty gate, each
/// `__canvas_framePasses` with its backend's region sizes: a backend draws a frame only
/// when it reproduces every item of it, and hands the rest back to the oracle rather
/// than drawing it wrongly.
///
/// Phase 2's SDF fragment shader evaluates the same distance functions the software
/// rasteriser does, so every kind passes — including `__CANVAS_GEO_NONE`, which both
/// backends draw as nothing. What a backend cannot do is hold more than its frame
/// BUFFER holds: quads, polygon edges, gradient stops, dash words and glyph/picture
/// texels each take a region of it. That is the size of one PASS and not of a frame
/// (bug-765): a frame whose blocks do not fit is drawn as consecutive runs of its draw
/// list (`__canvas_passNext`), each the longest that fits every region, into one target
/// that the first clears and the rest load, and is presented or read back once. The one
/// frame still declined is one holding a block that does not fit a region by itself.
/// There is no per-polygon edge cap since bug-686: the old one (256 edges, justified by
/// a 4 KB `setFragmentBytes:` payload that plan-116-A removed) sent every detailed
/// polygon to software, and its twin in the emitter drew such a polygon as nothing.
///
/// It reads the geometry header by slot rather than through a helper because that is
/// what the header is for: a fixed layout both backends index directly
/// (`__canvas_headerFor`).
///
/// One sum serves both backends, handed the region sizes of the one drawing the
/// frame. They were two predicates' own sums while the backends' limits differed in KIND (Metal
/// capped each polygon, Vulkan their sum); since bug-688 both keep every per-item
/// payload in a region of one buffer, of the same sizes, and two copies of one sum were
/// two places for a count to differ from the emitter's.
///
/// **A glyph run is bounded rather than refused**: each glyph's bitmap is copied into
/// a region of the buffer and its block names its slice (bug-670 moved Metal onto
/// Vulkan's design — its per-glyph `setFragmentBytes:` payload, capped at 4 KiB, was
/// overwritten before the instanced draw read it). Neither backend truncates: a clipped
/// glyph is a different glyph and would read as a rasteriser bug.
///
/// A kind a shader does not know must be declined HERE, not discovered in the emitter.
/// Both backends declined `__CANVAS_GEO_TEXT` outright for as long as neither could
/// draw one, and that was not caution — the version before it *accepted* a kind neither
/// shader knew, and Metal returned a frame with the text simply missing and no error
/// anywhere: 4,536 pixels wrong, reported as success.
///
/// The region sizes below are not literals in this text: each `LET __CANVAS_*MAX*`
/// line carries an `@NAME@` token that `RENDER_METAL` replaces with the Rust constant
/// of that name (`RENDER_CAPS`), so the pass walk and the emitter it fills a buffer for
/// read ONE definition (bug-686). They used to be two, joined only by a test.
#[rustfmt::skip]
const RENDER_METAL_TEMPLATE: &str = concat!(
    gpu_groups::render_metal_groups!(),
    gpu_draw_list::render_metal_draw_list!(),
    gpu_passes::render_metal_passes!(),
);

/// The frame caps `RENDER_METAL_TEMPLATE` shares with the native emitters, by the
/// `@NAME@` token each `LET` line carries. One entry per token; a token with no entry
/// would reach the MFBASIC compiler as a syntax error, and
/// `every_cap_token_in_the_render_source_is_generated` pins that none is left.
const RENDER_CAPS: &[(&str, usize)] = &[
    ("VULKAN_MAX_FRAME_ITEMS", VULKAN_MAX_FRAME_ITEMS),
    ("METAL_MAX_FRAME_ITEMS", METAL_MAX_FRAME_ITEMS),
    (
        "METAL_MAX_FRAME_GLYPH_SAMPLES",
        METAL_MAX_FRAME_GLYPH_SAMPLES,
    ),
    ("METAL_MAX_FRAME_EDGES", METAL_MAX_FRAME_EDGES),
    (
        "VULKAN_MAX_FRAME_GRADIENT_STOPS",
        VULKAN_MAX_FRAME_GRADIENT_STOPS,
    ),
    (
        "METAL_MAX_FRAME_GRADIENT_STOPS",
        METAL_MAX_FRAME_GRADIENT_STOPS,
    ),
    ("VULKAN_MAX_FRAME_DASH_WORDS", VULKAN_MAX_FRAME_DASH_WORDS),
    ("METAL_MAX_FRAME_DASH_WORDS", METAL_MAX_FRAME_DASH_WORDS),
    ("VULKAN_MAX_FRAME_EDGES", VULKAN_MAX_FRAME_EDGES),
    (
        "VULKAN_MAX_FRAME_GLYPH_SAMPLES",
        VULKAN_MAX_FRAME_GLYPH_SAMPLES,
    ),
    ("DRAW_ENTRY_WORDS", CANVAS_DRAW_ENTRY_WORDS),
    ("DRAW_PASS_WORD", CANVAS_DRAW_PASS_WORD),
    ("DRAW_PASS_CONTINUES", CANVAS_DRAW_PASS_CONTINUES),
    ("DRAW_PASS_MORE", CANVAS_DRAW_PASS_MORE),
];

/// `RENDER_METAL_TEMPLATE` with every cap token replaced by its Rust constant.
static RENDER_METAL: LazyLock<String> = LazyLock::new(|| {
    RENDER_CAPS
        .iter()
        .fold(RENDER_METAL_TEMPLATE.to_string(), |text, (name, value)| {
            text.replace(&format!("@{name}@"), &value.to_string())
        })
});

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always(
        "canvas_ensureGraphics",
        ENSURE_GRAPHICS,
    ));
    for helper in
        RegistryHelper::debug_split("canvas_testSwitches", TEST_SWITCHES, TEST_SWITCHES_DEBUG)
    {
        pkg.add_helper(helper);
    }
    for helper in
        RegistryHelper::debug_split("canvas_phaseTimers", PHASE_TIMERS, PHASE_TIMERS_DEBUG)
    {
        pkg.add_helper(helper);
    }
    for helper in
        RegistryHelper::debug_split("canvas_pointerScript", POINTER_SCRIPT, POINTER_SCRIPT_DEBUG)
    {
        pkg.add_helper(helper);
    }
    for helper in RegistryHelper::debug_split("canvas_renderLoop", RENDER_LOOP, RENDER_LOOP_DEBUG) {
        pkg.add_helper(helper);
    }
    pkg.add_helper(RegistryHelper::always("canvas_hashScene", HASH_SCENE));
    pkg.add_helper(RegistryHelper::always("canvas_renderScene", RENDER_SCENE));
    pkg.add_helper(RegistryHelper::always(
        "canvas_renderMetal",
        RENDER_METAL.as_str(),
    ));
    for helper in
        RegistryHelper::debug_split("canvas_directPresent", DIRECT_PRESENT, DIRECT_PRESENT_DEBUG)
    {
        pkg.add_helper(helper);
    }
    pkg.add_helper(RegistryHelper::always("canvas_closeRetired", CLOSE_RETIRED));
}

#[cfg(test)]
mod tests;
