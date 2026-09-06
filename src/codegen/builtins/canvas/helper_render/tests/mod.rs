use super::*;
use crate::codegen::runtime::canvas::{
    CANVAS_DRAW_ENTRY_COUNT_SHIFT, CANVAS_DRAW_ENTRY_MODE, CANVAS_DRAW_ENTRY_SHIFT,
    CANVAS_DRAW_ENTRY_WORDS, CANVAS_MAX_FRAME_ITEMS, GEO_KIND_POLYGON, GEO_KIND_TEXT, HEADER_AUX0,
    MAX_EDGES, MAX_FRAME_GRADIENT_STOPS, METAL_MAX_FRAME_EDGES, METAL_MAX_GLYPH_SAMPLES,
    VULKAN_MAX_FRAME_EDGES, VULKAN_MAX_FRAME_GLYPH_SAMPLES,
};

/// The body of a `FUNC`/`SUB` in the injected MFBASIC source, by name.
fn body(name: &str) -> &'static str {
    let start = RENDER_METAL
        .find(&format!("__canvas_{name}("))
        .unwrap_or_else(|| panic!("__canvas_{name} is not in RENDER_METAL"));
    let rest = &RENDER_METAL[start..];
    let end = rest
        .find("\nEND ")
        .unwrap_or_else(|| panic!("__canvas_{name} has no END"));
    &rest[..end]
}

/// The draw list is BUILT here in MFBASIC and WALKED by the native emitters, so its
/// entry width lives on both sides of a boundary the Rust compiler cannot see across
/// -- this side is a `&str`.
///
/// plan-116-H13: these two disagreed (the emitter at eight words, this source still
/// appending four) and the result was not a wrong picture. The emitter strode 64
/// bytes through a 32-byte array, so it read every other entry and took the following
/// entry's `base` as a blend mode -- indexing the pipeline table out of range and
/// handing Vulkan a junk `VkPipeline`. It SIGSEGVs inside the driver's JIT-compiled
/// code with no MFBASIC frame in the backtrace, on one remote box, in a harness that
/// is not part of `cargo test`. This assertion is the only cheap way to catch it.
#[test]
fn the_draw_entry_width_agrees_with_the_emitter() {
    let appends = body("pushOneDraw")
        .matches("collections::append(out,")
        .count();
    assert_eq!(
        appends, CANVAS_DRAW_ENTRY_WORDS,
        "__canvas_pushOneDraw appends {appends} words but the emitter strides \
             CANVAS_DRAW_ENTRY_WORDS = {CANVAS_DRAW_ENTRY_WORDS}",
    );
    assert_eq!(
        1usize << CANVAS_DRAW_ENTRY_SHIFT,
        CANVAS_DRAW_ENTRY_WORDS * 8,
        "the byte stride shift and the word count disagree",
    );
    assert_eq!(
        1usize << CANVAS_DRAW_ENTRY_COUNT_SHIFT,
        CANVAS_DRAW_ENTRY_WORDS,
        "the element-count shift and the word count disagree",
    );
    assert!(
        CANVAS_DRAW_ENTRY_MODE < CANVAS_DRAW_ENTRY_WORDS * 8,
        "the blend mode is read from outside the entry",
    );
}

/// `__canvas_blockInstances` predicts how many instances each block publishes, and
/// the draw list's bases are its running sum -- but what is actually WRITTEN into the
/// item buffer is `emit_split_or_publish`'s decision. One case that disagrees shifts
/// every base after it, and the final entry then draws instances that were never
/// published: uninitialised buffer, which reaches the screen as opaque black.
///
/// That is why the symptom appears at the END of a scene rather than at the item that
/// disagreed -- plan-116-H13 chased it through the shaders, the draw list and the ABI
/// before finding it here. The blend split is the case that went missing, so it is
/// the case pinned by name.
#[test]
fn block_instances_keeps_the_blend_split_case() {
    let rule = body("blockInstances");
    assert!(
        rule.contains("RETURN 2"),
        "the blend split case is gone: an item that both strokes and fills under a \
             non-Normal blend mode publishes TWO records in emit_split_or_publish, and \
             this function must predict 2 for it",
    );
    for (slot, what) in [
        ("26", "blend mode"),
        ("7", "strokeHalf"),
        ("11", "fill alpha"),
    ] {
        assert!(
            rule.contains(&format!(", {slot})")),
            "the split rule no longer reads slot {slot} ({what}); \
                 emit_split_or_publish tests all three",
        );
    }
    assert!(
        rule.contains(", 20)"),
        "a text run's instance count is HEADER_AUX0 = slot 20, the field \
             emit_glyph_publish loops over",
    );
}

/// Find `LET <name> AS Integer = <n>` in the injected MFBASIC source.
fn declared(name: &str) -> usize {
    let needle = format!("LET {name} AS Integer = ");
    let start = RENDER_METAL
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} is not declared in RENDER_METAL"))
        + needle.len();
    let rest = &RENDER_METAL[start..];
    let end = rest.find('\n').unwrap_or(rest.len());
    rest[..end].trim().parse().expect("a decimal literal")
}

/// The predicates are written in MFBASIC and the emitters in Rust, so the limits
/// exist twice — with no compiler between them. Drift is not a style problem: if
/// the MFBASIC cap ever exceeds the Rust one, the predicate admits a scene the
/// emitter's buffer cannot hold.
///
/// Since plan-116-A every one of these caps guards a *heap* buffer written through
/// a mapped pointer (the frame buffer), where Metal's edges used to be a stack
/// array. That makes drift worse rather than better: a stack overrun tends to fault
/// near the frame that caused it, and a write past a mapped GPU buffer lands in
/// whatever the driver happened to put next.
#[test]
fn the_two_gpu_edge_budgets_match_the_emitters() {
    assert_eq!(declared("__CANVAS_METAL_MAX_EDGES"), MAX_EDGES);
    assert_eq!(
        declared("__CANVAS_VULKAN_MAX_FRAME_EDGES"),
        VULKAN_MAX_FRAME_EDGES
    );
    assert_eq!(
        declared("__CANVAS_METAL_MAX_FRAME_EDGES"),
        METAL_MAX_FRAME_EDGES,
        "the predicate admits a frame whose polygon edges the Metal frame buffer's \
             edge region cannot hold",
    );
    assert_eq!(
        declared("__CANVAS_MAX_FRAME_ITEMS"),
        CANVAS_MAX_FRAME_ITEMS,
        "the predicate admits a frame with more drawn quads than the item buffer \
             has blocks — on BOTH backends, since they share this one. The emitters \
             stop publishing at capacity, so the surplus items would silently not be \
             drawn",
    );
    assert_eq!(
        declared("__CANVAS_VULKAN_MAX_GLYPH_SAMPLES"),
        VULKAN_MAX_FRAME_GLYPH_SAMPLES,
        "the predicate admits a frame whose glyph bitmaps the buffer's glyph \
             region cannot hold",
    );
    assert_eq!(
        declared("__CANVAS_METAL_MAX_GLYPH_SAMPLES"),
        METAL_MAX_GLYPH_SAMPLES,
        "the predicate admits a glyph bigger than `setFragmentBytes:` will carry",
    );
    // plan-116-F. One number for both backends, because the gradient region is
    // sized identically on each -- and the failure it prevents is worse than a
    // dropped item: past the cap an item's first-stop index runs off the region,
    // and the shader reads whatever the buffer holds there as a colour ramp.
    assert_eq!(
        declared("__CANVAS_MAX_FRAME_GRADIENT_STOPS"),
        MAX_FRAME_GRADIENT_STOPS,
        "the predicate admits a frame whose gradient stops the buffer's third \
             region cannot hold, so one item's stops would be read as another's",
    );
}

/// The predicates read the geometry header by slot. `offset + 20` is
/// `HEADER_AUX0` — the polygon's edge count, and for a glyph run its glyph count —
/// and a renumbered header would leave them summing an arc's start angle instead,
/// which is a plausible-looking number rather than an error.
///
/// The count is a census, so it moves whenever the predicates gain or lose a read;
/// it went 4 → 7 in plan-116-A. Enumerated so the next reader can tell a legitimate
/// growth from a slot that drifted:
///
/// | | Metal | Vulkan |
/// |---|---|---|
/// | the glyph-run walk (`__canvas_runLargestGlyph` / `__canvas_runSamples`) | 1 | 1 |
/// | the per-item `MAX_EDGES` decline | 1 | — (no per-item limit) |
/// | the frame edge sum | 1 | 1 |
/// | ~~the frame quad count, a glyph run's glyphs~~ | — | — |
///
/// It went 7 → 5 in plan-116-H, and the two that left did **not** stop reading the
/// slot — they moved. Both predicates' quad counts became one call to
/// `__canvas_blockInstances`, which is now the single answer to "how many instances
/// does this block become" and is what the draw list asks too; it reads the glyph
/// count as `__canvas_geoAt(offset, 20)`, which this census's `offset + 20` pattern
/// does not match by construction.
///
/// So the invariant is intact and is asserted in two places rather than one:
/// `block_instances_keeps_the_blend_split_case` pins that
/// `__canvas_blockInstances` reads slot 20 (and slots 26, 7 and 11 for the split),
/// and this census covers what is left in the predicates themselves. Lowering the
/// number without checking where the reads went would have been the failure this
/// enumeration exists to prevent.
#[test]
fn the_predicates_read_the_edge_count_slot() {
    assert_eq!(HEADER_AUX0, 20);
    assert_eq!(
        RENDER_METAL
            .matches(&format!("offset + {HEADER_AUX0}"))
            .count(),
        5,
        "every glyph-run walk, edge sum and edge decline in both predicates should \
             read HEADER_AUX0. If this went DOWN, check where the read went before \
             changing the number: the quad counts left in plan-116-H by moving into \
             `__canvas_blockInstances`, which still reads slot 20 — a read that simply \
             vanished would be a predicate summing an arc's start angle instead, which \
             is a plausible number rather than an error"
    );
}

/// A glyph run is a kind the predicates now *admit* rather than refuse, so its
/// spelling has to be as pinned as the polygon's — and by the same argument. It
/// was not always: both predicates declined `__CANVAS_GEO_TEXT` outright until
/// the backends could draw one, and the version before *that* accepted it while
/// neither shader knew the kind, returning a frame with the text missing and
/// calling it success.
#[test]
fn the_text_kind_is_spelled_once() {
    assert_eq!(GEO_KIND_TEXT, "6");
    assert!(RENDER_METAL.contains("= __CANVAS_GEO_TEXT THEN"));
}

/// Both predicates test the same kind the emitters and the shaders branch on.
#[test]
fn the_polygon_kind_is_spelled_once() {
    assert_eq!(GEO_KIND_POLYGON, "4");
    assert!(RENDER_METAL.contains("= __CANVAS_GEO_POLYGON THEN"));
}
