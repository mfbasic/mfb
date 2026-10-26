//! plan-175-C: adjacent rows that draw the same blocks merge into one instanced draw —
//! a head of repeat `R` and `R - 1` node-only rows — and every frame still matches the
//! software oracle. Each case is drawn on Metal and in software and compared; `draws=`
//! (`base:count:dx:dy:mode:repeat` per row) is read off the Metal run's stats line.

use super::*;

/// A program of `body`, with `IMPORT collections` for the scenes that build node lists.
fn merge_scene(body: &str) -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\nIMPORT collections\n\n\
         FUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n{body}END FUNC\n"
    )
}

/// A square-cornered rectangle literal at `(x, y)`, `w` by `h`, in `rgb`.
fn rect(x: f64, y: f64, w: f64, h: f64, rgb: (u8, u8, u8)) -> String {
    format!(
        "canvas::Rectangle[id := \"\", position := vector::Float2[x := {x:?}, y := {y:?}], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := {w:?}, height := {h:?}], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb({}, {}, {})), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]",
        rgb.0, rgb.1, rgb.2
    )
}

/// A `canvas::Group` node naming `name` at `(x, y)` — MFBASIC expressions.
fn node(name: &str, x: &str, y: &str) -> String {
    format!(
        "canvas::Group[id := \"\", position := vector::Float2[x := {x}, y := {y}], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"{name}\"]"
    )
}

/// The repeat (sixth field) of every row of the stats line's `draws=`.
fn repeats(line: &str) -> Vec<i64> {
    let draws = line
        .split_whitespace()
        .find_map(|w| w.strip_prefix("draws="))
        .unwrap_or_else(|| panic!("no draws= in {line}"));
    if draws == "none" {
        return Vec::new();
    }
    draws
        .split('|')
        .map(|row| {
            row.split(':')
                .nth(5)
                .and_then(|r| r.parse().ok())
                .unwrap_or_else(|| panic!("a row without a repeat: {row}"))
        })
        .collect()
}

/// Build `source`, draw it on Metal and in software, compare, and answer the Metal
/// frame and its last stats line — or `None` on a host with no Metal device.
fn metal_and_oracle(name: &str, source: &str) -> Option<(Frame, String)> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let program = build(name, source);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return None;
    }
    let line = stats
        .lines()
        .next_back()
        .expect("a Metal stats line")
        .to_string();
    assert_eq!(stat(&line, "gpuFrames"), 1, "Metal drew the frame: {line}");
    assert!(
        software.pixels.iter().any(|&b| b != 0),
        "{name}: the software render drew nothing, so the comparison would be vacuous",
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!("{name}: the merged frame differs from the oracle: {diff}");
    }
    Some((gpu, line))
}

/// 1,000 nodes of a one-rectangle group, every one in view: one drawn row of repeat
/// 1,000 and 999 node-only rows.
#[test]
fn a_thousand_adjacent_group_nodes_are_one_draw() {
    let body = format!(
        "  canvas::setGroup(\"dot\", [{}])\n  \
         MUT nodes AS List OF canvas::DrawItem = []\n  \
         MUT k AS Integer = 0\n  \
         WHILE k < 1000\n    \
           nodes = collections::append(nodes, {})\n    \
           k = k + 1\n  \
         END WHILE\n  \
         canvas::present(nodes)\n",
        rect(0.0, 0.0, 10.0, 10.0, (240, 180, 40)),
        node(
            "dot",
            "toFloat(22 * (k - 40 * (k / 40)))",
            "toFloat(25 * (k / 40))"
        ),
    );
    let Some((_, line)) = metal_and_oracle("canvas_merge_thousand", &merge_scene(&body)) else {
        return;
    };
    let r = repeats(&line);
    assert_eq!(r.len(), 1000, "one row per node: {line}");
    assert_eq!(r[0], 1000, "the head draws all 1,000 nodes");
    assert!(
        r[1..].iter().all(|&x| x == 0),
        "the rest are node-only rows"
    );
    assert_eq!(stat(&line, "profDraws"), 1, "one draw call: {line}");
}

/// Two nodes of a red-then-blue group placed so node 1's red covers node 0's blue: one
/// draw of repeat 2 must still draw node 0's instances before node 1's, so the overlap is
/// red. Any other instance order is wrong in exactly the overlap.
#[test]
fn a_merged_draw_keeps_instance_order_where_nodes_overlap() {
    let source = include_str!("../scenes/group_merge_overlap.mfb");
    let Some((gpu, line)) = metal_and_oracle("canvas_merge_overlap", source) else {
        return;
    };
    assert_eq!(repeats(&line), vec![2, 0], "the two nodes merged: {line}");
    let at = |x: u32, y: u32| {
        let i = ((y * WIDTH + x) * 4) as usize;
        (gpu.pixels[i], gpu.pixels[i + 1], gpu.pixels[i + 2])
    };
    assert_eq!(
        at(150, 120),
        (255, 0, 0),
        "node 1's red is drawn over node 0's blue"
    );
}

/// Nodes of one group with another item between them do not merge across it: that
/// item draws between them.
#[test]
fn group_nodes_with_an_item_between_them_stay_separate_draws() {
    let body = format!(
        "  canvas::setGroup(\"dot\", [{}])\n  \
         canvas::present([{}, {}, {}])\n",
        rect(0.0, 0.0, 30.0, 30.0, (255, 0, 0)),
        node("dot", "100.0", "100.0"),
        rect(110.0, 110.0, 30.0, 30.0, (0, 255, 0)),
        node("dot", "120.0", "120.0"),
    );
    let Some((_, line)) = metal_and_oracle("canvas_merge_between", &merge_scene(&body)) else {
        return;
    };
    assert_eq!(
        repeats(&line),
        vec![1, 1, 1],
        "no merge across the item: {line}"
    );
}

/// 70,000 adjacent nodes: the run is cut at the node region (65,536), so the frame is two
/// heads in two passes.
#[test]
fn a_run_longer_than_the_node_region_is_cut_and_drawn_in_two_passes() {
    let source = include_str!("../scenes/group_merge_cut.mfb");
    let Some((_, line)) = metal_and_oracle("canvas_merge_cut", source) else {
        return;
    };
    let r = repeats(&line);
    assert_eq!(r.len(), 70_000, "one row per node: {line:.200}");
    assert_eq!(r[0], 65_536, "the first run is a node region long");
    assert_eq!(r[65_536], 70_000 - 65_536, "the rest is the second run");
    assert_eq!(r.iter().filter(|&&x| x > 0).count(), 2, "two heads");
    assert_eq!(stat(&line, "profPasses"), 2, "two passes: {line:.200}");
}

/// Culling around a merge (plan-173): a node wholly offscreen writes no row, so the
/// nodes on either side of it are adjacent and merge; a node half across the right edge
/// writes only its visible block, and merges with nothing.
#[test]
fn culled_nodes_merge_across_and_a_half_visible_node_stands_alone() {
    let body = format!(
        "  canvas::setGroup(\"two\", [{}, {}])\n  \
         canvas::present([{}, {}, {}, {}, {}])\n",
        rect(0.0, 0.0, 20.0, 20.0, (255, 0, 0)),
        rect(40.0, 0.0, 20.0, 20.0, (0, 0, 255)),
        node("two", "100.0", "100.0"),
        node("two", "2000.0", "100.0"),
        node("two", "200.0", "100.0"),
        node("two", "870.0", "100.0"),
        node("two", "300.0", "100.0"),
    );
    let Some((_, line)) = metal_and_oracle("canvas_merge_culled", &merge_scene(&body)) else {
        return;
    };
    assert_eq!(
        repeats(&line),
        vec![2, 0, 1, 1],
        "the nodes at 100 and 200 merge across the culled one; the one at 870 is its \
         visible block alone, and the one at 300 follows it: {line}"
    );
}

/// A merged run whose blocks a pass boundary cuts: two nodes of a group of 70,000
/// rectangles, more than one pass's item region (65,536), so the head's blocks span two
/// passes. A repeat cannot reach across passes (each pass has its own node table), so
/// the run's rows draw SOLO from the cut on (`__CANVAS_PASS_SOLO_END`), and the frame
/// still matches the oracle.
#[test]
fn a_merged_run_cut_by_a_pass_boundary_draws_each_node_alone() {
    let source = include_str!("../scenes/group_merge_solo.mfb");
    let Some((_, line)) = metal_and_oracle("canvas_merge_solo", source) else {
        return;
    };
    assert_eq!(
        repeats(&line),
        vec![2, 0],
        "the frame's list merges the two nodes: {line:.300}"
    );
    assert!(
        stat(&line, "profPasses") >= 2,
        "the group alone is past one pass: {line:.300}"
    );
}
