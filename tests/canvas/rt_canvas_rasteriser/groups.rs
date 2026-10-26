//! plan-176-D: `rt_canvas_rasteriser`'s groups: presents, rendering, buffer retirement
//! (including the `draws=` assertions) and resources destroyed mid-frame. The render
//! helpers live in the root, `rt_canvas_rasteriser.rs`.

use super::*;

/// Build and run a program, returning its stdout — with no frame dump required.
///
/// `render` insists on a dump, which is right for every test that looks at pixels and
/// wrong for one whose program is *supposed* to fail before presenting anything. Those
/// tests assert on what the program printed from its `TRAP` handler, so the frame is
/// not merely unnecessary, it is the thing that must not exist.
fn render_stdout(name: &str, source: &str) -> (String, String) {
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    let run = Command::new(&binary)
        .env("MFB_HEADLESS", "1")
        // The GTK one too. Omitting it is invisible on the macOS dev host, where the
        // MACAPP flag is the one that matters; on a Linux box the program tries to open
        // a display, fails, and exits 1 -- which these tests then report as a
        // use-after-free or a missing raise. `render` above has always set all three.
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let err = String::from_utf8_lossy(&run.stderr).to_string();
    let _ = std::fs::remove_dir_all(&project);
    (out, err)
}

/// A group's contents changing must produce a second frame, and that frame must show
/// the **new** contents — even though the scene list handed to `present` is
/// byte-for-byte the one already published.
///
/// This is plan-116-G §4.4's least obvious requirement, written before any of the
/// feature exists. `publishScene` decides whether to redraw by comparing the raw bytes
/// of the `DrawItem` list's data region (`emit_compare_bytes_branch` in
/// `gen_present.rs`). A `Group` node is `dx`, `dy` and `name` — and after
/// `setGroup("panel", …)` installs *different* items under the same name, all three are
/// unchanged. Two presents of the same list therefore compare equal, the second is
/// skipped, and the program draws the OLD panel forever with nothing raised: a stale
/// picture reported as success.
///
/// §4.4's answer is a parallel resolved-groups signature — `(slotIndex, revision)` per
/// resolved group node, in scene order — published and compared alongside the items.
/// This test does not care how it is done; it pins the observable consequence.
///
/// **Two assertions, and the second is the one with teeth.** The frame count alone
/// would pass for a fix that republishes but resolves to the old buffer. `A` is a red
/// box at the top-left and `A'` a green box far away at the bottom-right, so the dump —
/// which is the **last** frame, because `__canvas_presentSurface` writes it with
/// `fs::writeBytes` and that overwrites — must show green where `A'` is and background
/// where `A` was.
///
/// Un-ignored by **Phase 4**, which landed the resolution pass.
#[test]
fn a_group_replaced_between_two_identical_presents_redraws_with_the_new_contents() {
    let (frame, stats) = render(
        "canvas_group_revision",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [a])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             canvas::present([node])\n  \
             LET b AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [b])\n  \
             canvas::present([node])\n",
        ),
    );

    assert_eq!(
        stats.len(),
        2,
        "replacing a group's contents must redraw, even though the scene list handed \
         to the second `present` is byte-identical to the first: the group's revision \
         has to reach the content comparison, or the program draws the old panel \
         forever and nothing raises. {stats:?}",
    );
    assert_eq!(
        pixel(&frame, 720, 520),
        (0, 255, 0, 255),
        "the last frame does not show the REPLACED group. A revision that reaches the \
         skip comparison but not the resolved buffer republishes and then redraws the \
         old contents — which the frame count above cannot distinguish, and this can.",
    );
    assert_ne!(
        pixel(&frame, 30, 30),
        (255, 0, 0, 255),
        "the old group's box is still on the surface, so the second frame drew the \
         previous contents on top of, or instead of, the new ones",
    );
}

/// Its sibling, and the half a careless fix breaks: with no `setGroup` between them,
/// three identical presents still draw **once**.
///
/// The cheap way to pass the test above is to stop comparing, or to fold something
/// per-frame-varying into the signature — either of which turns every group program
/// into an unconditional redraw and silently undoes the skip that
/// `rt_canvas_graphics_thread.rs`'s `an_identical_re_present_draws_no_second_frame`
/// protects for group-free scenes. Pairing the two is what makes the requirement
/// two-sided.
///
/// Un-ignored by **Phase 4**, with its sibling.
#[test]
fn three_identical_presents_of_an_unchanged_group_draw_one_frame() {
    let (_, stats) = render(
        "canvas_group_no_revision",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [a])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             canvas::present([node])\n  \
             canvas::present([node])\n  \
             canvas::present([node])\n",
        ),
    );

    assert_eq!(
        stats.len(),
        1,
        "three presents of an unchanged group must draw once: the resolved-groups \
         signature has to compare EQUAL when nothing changed, or every group program \
         redraws on every present forever. {stats:?}",
    );
}

/// One `name=value` field of one stats line.
fn stat(line: &str, name: &str) -> String {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(name).map(str::to_string))
        .unwrap_or_else(|| panic!("no {name} field in {line:?}"))
}

/// `setGroup` deep-copies its item list, and `removeGroup` of an absent name is a
/// no-op — both read off `MFB_CANVAS_STATS`, which `.ai/canvas-threading.md` §11 makes
/// the only window a test has onto worker-owned state (plan-116-G Phase 3).
///
/// Four frames, each asserting one thing the phase promises:
///
/// 1. nothing installed — `groups=0 groupBytes=0`, so the later numbers are deltas from
///    a known zero rather than from whatever a previous test left behind;
/// 2. one group installed — `groups=1` and `groupBytes` non-zero;
/// 3. **the caller's list mutated after installing** — `groupBytes` must be *unchanged*.
///    This is the deep copy. Publishing the caller's block would be cheaper and would
///    pass frames 1, 2 and 4; appending to that list afterwards is the only thing that
///    tells the two apart from outside;
/// 4. the group removed — `groups` drops to 0 while `groupBytes` **stays**, because
///    nothing is freed until the drain gate.
///
/// The fourth is an assertion that this phase **leaks, by construction**, and it is
/// deliberate: Phase 5's acceptance is that this number falls, and a gate with no
/// measured "before" cannot show that it moved. When Phase 5 lands, this assertion is
/// the one that has to change, and its message says so.
///
/// `removeGroup("absent")` is called in the same run rather than in a test of its own:
/// if it were not a no-op the program would raise and every assertion below would fail
/// at once, which is a clearer signal than a separate test asserting nothing happened.
#[test]
fn set_group_deep_copies_and_remove_group_frees_nothing_yet() {
    let (_, stats) = render(
        "canvas_group_deep_copy",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET green AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 80.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET blue AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 150.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::removeGroup(\"absent\")\n  \
             canvas::present([red])\n  \
             MUT items AS List OF canvas::DrawItem = [red]\n  \
             canvas::setGroup(\"panel\", items)\n  \
             canvas::present([red, green])\n  \
             items = collections::append(items, green)\n  \
             items = collections::append(items, blue)\n  \
             canvas::present([red, green, blue])\n  \
             canvas::removeGroup(\"panel\")\n  \
             canvas::present([green])\n",
        ),
    );

    assert_eq!(stats.len(), 4, "expected four frames: {stats:?}");
    let groups: Vec<String> = stats.iter().map(|l| stat(l, "groups=")).collect();
    let bytes: Vec<String> = stats.iter().map(|l| stat(l, "groupBytes=")).collect();

    assert_eq!(
        groups,
        vec!["0", "1", "1", "0"],
        "the table should hold nothing, then one group, then still one, then none \
         again after `removeGroup`: {stats:?}",
    );
    assert_eq!(bytes[0], "0", "the table owns nothing before any setGroup");
    assert_ne!(
        bytes[1], "0",
        "installing a group must charge its copied block to the table",
    );
    assert_eq!(
        bytes[1], bytes[2],
        "the caller appended two items to the list it passed to `setGroup` and the \
         installed group followed it — `setGroup` published the caller's block instead \
         of copying it. Nothing else in this test can tell those apart: frames 1, 2 and \
         4 pass either way.",
    );
    assert_eq!(
        bytes[3], bytes[2],
        "`removeGroup` freed the items immediately. It must not: a frame may be mid-copy \
         of that block, so it is retired and the free waits for the drain gate. \
         \n\nPhase 5 landed that gate and this assertion did **not** need to change, \
         which is worth stating because the phase expected it would: the drain runs at \
         the TOP of a present and requires a frame to have completed since the \
         retirement, so the earliest it can release this buffer is the present after \
         the one below. `a_removed_groups_buffer_is_retired_not_freed` and \
         `the_group_drain_does_not_depend_on_the_scene_changing` cover the release \
         itself.",
    );
}

/// A nested group renders at the **composed** offset, a diamond renders twice, and an
/// absent name renders nothing (plan-116-G Phase 4).
///
/// One scene rather than three, because the interesting failures are ones that would
/// pass a test of any single case: an implementation that ignored `dx`/`dy` entirely
/// draws everything at the origin, and one that used the innermost offset instead of
/// the accumulated one puts the nested group in the right *row* and the wrong column.
/// Probing four disjoint places at once separates them.
///
/// The `entries=` assertion is the other half, and it is the performance claim this
/// whole letter rests on: a group drawn at several offsets is **one** geometry cache
/// entry, because the offset is applied at draw time and never enters the cache key.
#[test]
fn a_group_renders_at_its_offset_nested_diamond_and_absent() {
    let (frame, stats) = render(
        "canvas_group_offsets",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"leaf\", [red])\n  \
             canvas::setGroup(\"outer\", [canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"leaf\"]])\n  \
             LET a AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"leaf\"]\n  \
             LET b AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 300.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"leaf\"]\n  \
             LET nested AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 500.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"outer\"]\n  \
             LET absent AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 700.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"nope\"]\n  \
             canvas::present([a, b, nested, absent])\n",
        ),
    );

    assert_eq!(
        pixel(&frame, 110, 110),
        (255, 0, 0, 255),
        "the group did not render at its own (100,100) offset",
    );
    assert_eq!(
        pixel(&frame, 310, 110),
        (255, 0, 0, 255),
        "the SAME group did not render at a second offset — one node's offset is being \
         applied to both, or the walk emits a group once however often it is named",
    );
    assert_eq!(
        pixel(&frame, 510, 310),
        (255, 0, 0, 255),
        "the nested group did not render at the COMPOSED offset (500,100)+(0,200). Red \
         at (510,110) instead would mean the inner group's own offset was dropped; red \
         at (10,310) would mean the outer's was.",
    );
    assert_eq!(
        pixel(&frame, 710, 110),
        (0, 0, 0, 255),
        "a `Group` naming a group that was never installed must draw nothing — and must \
         not raise, which it would have before reaching this assertion",
    );
    assert_eq!(
        pixel(&frame, 10, 10),
        (0, 0, 0, 255),
        "something drew at the origin, which is where every group lands if `dx`/`dy` \
         are ignored entirely",
    );

    let entries = stats
        .first()
        .and_then(|l| {
            l.split_whitespace()
                .find_map(|f| f.strip_prefix("entries="))
        })
        .expect("an entries= field");
    assert_eq!(
        entries, "1",
        "one rectangle drawn at three offsets must be ONE geometry cache entry: the \
         offset is applied at draw time and is not part of the cache key. That is the \
         performance claim groups exist for — {stats:?}",
    );
}

/// A group nested past the depth limit raises, and so does one that references itself
/// (plan-116-G Phase 4).
///
/// The two are the same error deliberately: a cycle *is* unbounded depth, and a program
/// with a cycle and one with 65 honest levels need the same fix. Asserting they report
/// identically is what pins that decision rather than leaving it to look like an
/// accident.
///
/// The self-reference is the sharper of the two — it is one `setGroup` call, so an
/// implementation with no depth bound at all hangs or overflows the stack here rather
/// than failing an assertion.
#[test]
fn a_group_cycle_and_an_over_deep_chain_both_raise() {
    for (name, body) in [
        (
            "canvas_group_cycle",
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 4.0, height := 4.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"self\", [red, canvas::Group[id := \"\", position := vector::Float2[x := 1.0, y := 1.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"self\"]])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"self\"]]) TRAP(e)\n  \
               io::print(\"raised \" & toString(e.code))\n  \
               RETURN\n  \
             END TRAP\n  \
             io::print(\"NO RAISE\")\n",
        ),
        (
            "canvas_group_deep",
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 4.0, height := 4.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"g0\", [red])\n  \
             MUT i AS Integer = 1\n  \
             WHILE i <= 70\n  \
               canvas::setGroup(\"g\" & toString(i), [canvas::Group[id := \"\", position := vector::Float2[x := 1.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\" & toString(i - 1)]])\n  \
               i = i + 1\n  \
             END WHILE\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g70\"]]) TRAP(e)\n  \
               io::print(\"raised \" & toString(e.code))\n  \
               RETURN\n  \
             END TRAP\n  \
             io::print(\"NO RAISE\")\n",
        ),
    ] {
        let (stdout, _) = render_stdout(name, &scene(body));
        assert!(
            stdout.contains("raised 77050024"),
            "{name} must raise ErrDepthExceeded (7-705-0024) from `present`; got: \
             {stdout:?}. \"NO RAISE\" means the depth bound is missing, and for the \
             cycle case that would otherwise recurse until the stack ends.",
        );
    }
}

/// A clipped item inside a translated group carries its clip with it: the clip is in the
/// item's own coordinates, so the group's translation moves both (plan-163-C §3, which
/// reverses plan-116-G's G5 on purpose — a surface-rectangle clip was right for
/// `Bounds`, and wrong for an SVG `clip-path`, which moves with its element).
///
/// A 200x200 square in a group translated by (300, 100), clipped to its own left half,
/// (0, 0)-(100, 200). The square lands at (300, 100)-(500, 300) and the clip with it at
/// (300, 100)-(400, 300): the left half is painted and the right half is not. Had the
/// clip stayed on the surface it would cut (0, 0)-(100, 200), which the square never
/// reaches, and nothing would be painted.
#[test]
fn a_clip_inside_a_translated_group_moves_with_it() {
    let (frame, _) = render(
        "canvas_group_clip",
        &scene(
            "  LET clipped AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 200, 255)) { clip := [[vector::Float2[x := 0.0, y := 0.0], vector::Float2[x := 100.0, y := 0.0], vector::Float2[x := 100.0, y := 200.0], vector::Float2[x := 0.0, y := 200.0]]] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"g\", [clipped])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 300.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"]])\n",
        ),
    );
    assert_eq!(
        pixel(&frame, 350, 200),
        (0, 200, 255, 255),
        "the square's left half, inside its moved clip, is painted",
    );
    assert_eq!(
        pixel(&frame, 450, 200),
        (0, 0, 0, 255),
        "the square's right half is outside its clip, which moved with the group",
    );
}

/// A removed group's buffer is **retired, not freed** (plan-116-G Phase 5).
///
/// The free cannot happen inside `removeGroup`: `canvas::groupItems` copies out of that
/// block on the graphics thread, so a render may be part-way through it. The block is
/// therefore handed to the drain gate, which releases it once a frame has completed —
/// the same rule `.ai/canvas-threading.md` §3 gives for retired scene blocks and §7 for
/// textures.
///
/// Observable because both states produce a frame: the name disappears immediately
/// (`groups=` drops), while the bytes stay charged (`groupBytes=` does not).
#[test]
fn a_removed_groups_buffer_is_retired_not_freed() {
    let (_, stats) = render(
        "canvas_group_retire",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET keep AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             canvas::present([keep, node])\n  \
             canvas::removeGroup(\"panel\")\n  \
             canvas::present([keep, node])\n",
        ),
    );

    assert_eq!(stats.len(), 2, "expected two frames: {stats:?}");
    let groups: Vec<String> = stats.iter().map(|l| stat(l, "groups=")).collect();
    let bytes: Vec<String> = stats.iter().map(|l| stat(l, "groupBytes=")).collect();

    assert_eq!(
        groups,
        vec!["1", "0"],
        "the name must go the moment `removeGroup` is called: {stats:?}",
    );
    assert_ne!(bytes[0], "0", "the installed group must own bytes");
    assert_eq!(
        bytes[1], bytes[0],
        "the buffer was freed the instant `removeGroup` was called. It must be RETIRED \
         instead — a render may be mid-copy of exactly that block — and released only \
         once a frame has completed since: {stats:?}",
    );
}

/// The drain does not depend on the scene ever changing again (**G7**).
///
/// This is the row the placement of the gate is decided by, and it is written as a
/// **memory bound** rather than as a single free, because a single free cannot tell the
/// two placements apart: whichever present eventually publishes will run either gate.
///
/// The loop installs and removes a group under a name **the presented scene never
/// references**, so the scene is byte-identical every time and the signature never
/// moves — every one of those presents is skipped, and no publish happens at all. With
/// the free placed beside the scene ring's `emit_reclaim_retired`, which sits after the
/// publish label, nothing would ever be released and the table would accumulate one
/// buffer per iteration. With the gate at the top of `present`, each iteration's
/// predecessor is drained on the next call.
///
/// The final present changes the scene only so that a stats line is written to read the
/// answer off; by then the bound has already been established or lost.
#[test]
fn the_group_drain_does_not_depend_on_the_scene_changing() {
    let (_, stats) = render(
        "canvas_group_drain_bound",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET keep AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([keep])\n  \
             MUT i AS Integer = 0\n  \
             WHILE i < 200\n  \
               canvas::setGroup(\"scratch\" & toString(i), [red, red, red, red, red, red, red, red])\n  \
               canvas::present([keep])\n  \
               canvas::removeGroup(\"scratch\" & toString(i))\n  \
               canvas::present([keep])\n  \
               i = i + 1\n  \
             END WHILE\n  \
             LET last AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([keep, last])\n",
        ),
    );

    let bytes: i64 = stat(stats.last().expect("a final frame"), "groupBytes=")
        .parse()
        .expect("groupBytes is a number");
    let groups = stat(stats.last().unwrap(), "groups=");
    assert_eq!(groups, "0", "every scratch group was removed: {stats:?}");
    // One eight-item group is 4914 bytes (measured, plan-163-D: 4082 before the gradient
    // gained five fields, 104 bytes an item), so 200 undrained ones would be ~1 MB. The
    // bound allows a single outstanding buffer and not two: the last iteration's may
    // legitimately still be retired, since the gate needs a frame to complete after it.
    assert!(
        bytes < 8192,
        "the table still owns {bytes} bytes after 200 install/remove cycles, so the \
         buffers were not drained. Every present in that loop showed an unchanged scene \
         and was skipped, which is exactly the case a free placed on the publish path \
         never reaches — the frame skip works and the memory is held anyway. {stats:?}",
    );
}

/// Replacing a live group frees the **old** buffer and not the new one, once a frame
/// has completed (plan-116-G Phase 5).
///
/// The sharp part is the second half: after the drain, `groupBytes` must equal what one
/// copy of the new contents costs — not zero (which would mean the live buffer was
/// freed too) and not the sum of both (which would mean the old one never was). The new
/// group is deliberately a *different size* from the old, so "one copy of the new
/// contents" is a number only the correct implementation produces.
///
/// The last present puts the same two nodes in the other order: an identical
/// re-present publishes nothing and draws no frame (bug-968), and the drain needs one.
#[test]
fn replacing_a_group_frees_only_the_displaced_buffer() {
    let (_, stats) = render(
        "canvas_group_replace_drain",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET b AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], radius := 10.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             canvas::setGroup(\"one\", [a])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 400.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"one\"]])\n  \
             canvas::setGroup(\"panel\", [a, a, a])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 400.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"one\"], node])\n  \
             canvas::setGroup(\"panel\", [b])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 400.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"one\"], node])\n  \
             canvas::present([node, canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 400.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"one\"]])\n",
        ),
    );

    assert_eq!(stats.len(), 4, "expected four frames: {stats:?}");
    let n: Vec<i64> = stats
        .iter()
        .map(|l| stat(l, "groupBytes=").parse().unwrap())
        .collect();
    let groups: Vec<String> = stats.iter().map(|l| stat(l, "groups=")).collect();

    assert_eq!(
        groups,
        vec!["1", "2", "2", "2"],
        "the replacement reuses the slot, so the name count must not move: {stats:?}",
    );
    assert!(
        n[2] > n[1],
        "replacing a group must charge the new copy before the old one drains — both \
         are held for one frame, which is what the drain gate costs: {stats:?}",
    );
    assert!(
        n[3] < n[2],
        "the displaced buffer was never freed: {stats:?}",
    );
    assert!(
        n[3] > 0,
        "everything was freed, including the LIVE buffer — a replace must retire only \
         the block it displaced: {stats:?}",
    );
    // The three-item group is gone and the one-item group replaced it, so the total
    // must be below what the frame carrying both cost, and below the pre-replace total.
    assert!(
        n[3] < n[1],
        "after the drain the table should hold the one-item replacement plus the \
         unrelated group, which is less than the three-item version it replaced: {stats:?}",
    );
}

/// A group removed while a **parent group** still names it keeps drawing.
///
/// The parent holds no pointer into the child's buffer — a `canvas::Group` node carries
/// a name, and the renderer resolves it per frame — so "still referenced" here means the
/// name resolves, and removing the child makes the parent's node a silent no-op like any
/// other unresolved name. This asserts that outcome rather than a refcount, because the
/// refcount is what the design does not have (**G24**).
#[test]
fn removing_a_group_a_parent_names_makes_the_parents_node_a_no_op() {
    let (frame, _) = render(
        "canvas_group_child_removed",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET mark AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"child\", [red])\n  \
             canvas::setGroup(\"parent\", [canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"child\"], mark])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"parent\"]\n  \
             canvas::present([node])\n  \
             canvas::removeGroup(\"child\")\n  \
             canvas::present([node])\n",
        ),
    );

    assert_eq!(
        pixel(&frame, 130, 130),
        (0, 0, 0, 255),
        "the removed child still drew. Removing a name makes every `canvas::Group` that \
         referenced it a no-op, including one inside another group.",
    );
    assert_eq!(
        pixel(&frame, 830, 630),
        (0, 0, 255, 255),
        "the parent group's OTHER item stopped drawing, so removing the child took the \
         parent down with it",
    );
}

/// A group removed while the graphics thread is **mid-frame** over it: the in-flight
/// frame completes normally and draws what it started with (plan-116-G Phase 5).
///
/// This is the row **G9** was written about, and it is deterministic rather than
/// probabilistic because this phase built the affordance G9 asked for.
/// `MFB_CANVAS_FRAME_HOLD_MS` parks the graphics thread inside `__canvas_renderFrame`,
/// immediately after `__canvas_sceneOffsets` has resolved every group name and copied
/// out its items — so the `removeGroup` below lands while a frame is demonstrably still
/// working from that block. Before the affordance, every proven "mid-render" row got
/// there through `MFB_CANVAS_RESIZE_W`/`_H` firing while the worker slept, which is
/// resize-specific; a row needing a different worker action was tested by luck.
///
/// `MFB_CANVAS_SYNC` is deliberately **off**: the whole point is that `present` returns
/// while the frame is still being drawn, so the worker can reach `removeGroup`.
///
/// What it asserts is that the frame completes and the picture is right — a
/// use-after-free here shows up as a crash, a torn frame, or nothing drawn where the
/// group was. It cannot assert the *timing* it arranged, so the hold is long enough
/// (600ms against a frame measured in single-digit ms) that the ordering is not in
/// doubt.
///
/// The worker's 120ms sleep before `removeGroup` is the other half of that ordering and
/// is not padding. Without it the worker races the graphics thread to the *start* of the
/// frame and usually wins — `present` returns as soon as it has signalled — so the group
/// is removed before it is ever resolved and the frame correctly draws nothing. That
/// tests the absent-name path, not this one, and it is what the first run of this test
/// measured.
#[test]
fn removing_a_group_mid_frame_lets_the_frame_finish() {
    let project = common::temp_project(
        "canvas_group_race",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 200.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             LET mark AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 30.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([mark, node])\n  \
             os::sleep(120)\n  \
             canvas::removeGroup(\"panel\")\n  \
             os::sleep(1200)\n",
        )
        .replace("IMPORT collections", "IMPORT collections\nIMPORT os"),
    );
    let frame_path = project.join("frame.rgba");
    let binary = common::build_app_debug(&project, "canvas_group_race");
    let run = Command::new(&binary)
        .env("MFB_HEADLESS", "1")
        // The GTK one too. Omitting it is invisible on the macOS dev host, where the
        // MACAPP flag is the one that matters; on a Linux box the program tries to open
        // a display, fails, and exits 1 -- which these tests then report as a
        // use-after-free or a missing raise. `render` above has always set all three.
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame_path)
        .env("MFB_CANVAS_FRAME_HOLD_MS", "600")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "the program did not exit cleanly — a use-after-free on the retired group buffer \
         is what this row exists to catch, and it presents as a signal here. exit {:?}\n{}\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    let frame =
        std::fs::read(&frame_path).expect("the in-flight frame must still have been written");
    assert_eq!(
        frame.len(),
        WIDTH * HEIGHT * 4,
        "the frame was torn or truncated",
    );
    assert_eq!(
        pixel(&frame, 240, 240),
        (255, 0, 0, 255),
        "the frame that was already drawing when `removeGroup` arrived did not finish \
         drawing the group. It had already copied the items, so removing the name must \
         not affect it — the retired block stays valid until a frame completes.",
    );
    assert_eq!(
        pixel(&frame, 700, 500),
        (0, 255, 0, 255),
        "the rest of the frame was lost",
    );
    let _ = std::fs::remove_dir_all(&project);
}

/// The program exits while a frame is still drawing a group: no use-after-free
/// (plan-116-G Phase 5 — R12's group analogue).
///
/// R12's own hazard is that the scene slots live in the worker's arena and the worker's
/// arena state lives on its *stack frame*, so a graphics thread still rendering after
/// the entry returns reads freed stack. A group's buffer is in the same arena, and this
/// phase gave the graphics thread a new reason to be inside one — `canvas::groupItems`
/// copies out of it — so the row is re-run against a group rather than assumed to be
/// covered.
///
/// `MFB_CANVAS_FRAME_HOLD_MS` is what makes it a real test rather than a hopeful one:
/// the frame is *guaranteed* to still be in progress when `main` returns, because the
/// graphics thread is parked in the middle of it.
#[test]
fn exiting_while_a_frame_draws_a_group_is_clean() {
    let project = common::temp_project(
        "canvas_group_exit_race",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 200.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]])\n",
        ),
    );
    let binary = common::build_app_debug(&project, "canvas_group_exit_race");
    let run = Command::new(&binary)
        .env("MFB_HEADLESS", "1")
        // The GTK one too. Omitting it is invisible on the macOS dev host, where the
        // MACAPP flag is the one that matters; on a Linux box the program tries to open
        // a display, fails, and exits 1 -- which these tests then report as a
        // use-after-free or a missing raise. `render` above has always set all three.
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_FRAME_HOLD_MS", "400")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "the program did not exit cleanly with a frame in flight over a group. Shutdown \
         must drain the pending frame and join the graphics thread before the worker's \
         entry unwinds — its arena, which holds the group's buffer, is on that stack \
         frame. exit {:?}\n{}\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("rendered"),
        "the program did not reach the end of `main`",
    );
    let _ = std::fs::remove_dir_all(&project);
}

/// A group offset far outside the surface draws nothing and does not **raise**
/// (plan-116-G).
///
/// `dx`/`dy` are user-supplied `Float`s that this letter feeds into two new integer
/// conversions: `__canvas_hashFloat` folds them into the draw hash as
/// `toInt(value * 65536.0)`, and the glyph path takes `toInt(gdx)` to move a run's
/// origin. A conversion whose result does not fit raises `7-705-0010` — arithmetic
/// overflow or numeric conversion outside the destination range — and it would raise
/// from `canvas::present`, which no caller expects to fail because a shape was placed
/// off-screen.
///
/// Written after a peer found exactly that error class in the canvas font path on
/// linux-aarch64, where it raises rather than corrupting. That is not this code and this
/// test does not chase it; it pins that *this* letter's new conversions are not another
/// instance, on a value a program can hand them directly.
///
/// `1.0e9` is chosen to be absurd rather than borderline: multiplied by 65536 it is
/// ~6.5e13, comfortably past a 32-bit destination and comfortably inside a 64-bit one,
/// so the test states which of those the conversion actually uses.
#[test]
fn a_group_offset_far_off_surface_draws_nothing_and_does_not_raise() {
    let (frame, stats) = render(
        "canvas_group_huge_offset",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET here AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             LET huge AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 1.0e9, y := 0.0 - 1.0e9], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             LET big AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 100000.0, y := 100000.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             canvas::present([here, huge, big])\n",
        ),
    );

    // Reaching here at all is most of the assertion: a raise from `present` fails the
    // harness in `render`, which asserts the program exited successfully.
    assert_eq!(stats.len(), 1, "expected one frame: {stats:?}");
    assert_eq!(
        pixel(&frame, 110, 110),
        (0, 255, 0, 255),
        "the in-surface item was lost, so the off-surface groups did more than draw \
         nothing",
    );
    assert_eq!(
        pixel(&frame, 10, 10),
        (0, 0, 0, 255),
        "something was drawn at the origin — an offset that overflowed its conversion \
         and wrapped would land somewhere arbitrary, and the origin is the most likely \
         somewhere",
    );
}

/// A `Polygon` inside a translated group draws at the offset (plan-116-G).
///
/// Every other group test here uses a fixed-tail kind — `Rectangle`, `Circle`, `Text`.
/// A polygon is the only shape whose geometry record has a **variable-length tail**: its
/// points live past the 47-slot header as edges, and `__canvas_geoDistance` reads them
/// from `offset + __CANVAS_GEO_HEADER`. The group offset moves the query point rather
/// than the record, so those edge coordinates are consumed in *shape* space while the
/// bounds that select the pixels are in *surface* space — the one place this letter's
/// two directions meet on the same item.
///
/// Two polygons, deliberately: `__canvas_hashItem` folds a polygon's points in by hand
/// because two different polygons can share a header (same bounds, same count, same
/// paint), so a scene with one polygon cannot tell a correct per-item tail from a shared
/// one. Here they differ only in shape, not in bounding box, which is exactly the
/// collision that motivated the hand-folding.
#[test]
fn a_polygon_inside_a_translated_group_draws_at_the_offset() {
    let (frame, stats) = render(
        "canvas_group_polygon",
        &scene(
            "  LET up AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 0.0, y := 0.0], vector::Float2[x := 100.0, y := 0.0], vector::Float2[x := 50.0, y := 100.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET down AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 0.0, y := 100.0], vector::Float2[x := 100.0, y := 100.0], vector::Float2[x := 50.0, y := 0.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"tri\", [up])\n  \
             canvas::setGroup(\"tri2\", [down])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 200.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"tri\"], canvas::Group[id := \"\", position := vector::Float2[x := 500.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"tri2\"]])\n",
        ),
    );

    // `up` is widest at its top edge: at (250, 205) — 5px down from the apex row — the
    // triangle spans roughly x 202..298, so its centre column is inside.
    assert_eq!(
        pixel(&frame, 250, 210),
        (255, 0, 0, 255),
        "the up-pointing polygon did not draw inside its group at (200,200). Its edges \
         live in the record's variable-length tail and are read in shape space, so a \
         group offset applied to the record rather than to the query point puts the \
         shape somewhere else entirely.",
    );
    assert_eq!(
        pixel(&frame, 550, 290),
        (0, 255, 0, 255),
        "the down-pointing polygon did not draw inside its group at (500,200)",
    );
    // The two are different shapes in the same 100x100 box, so a per-item tail is the
    // only thing that keeps them apart: `up` is empty near its bottom corners, `down` is
    // filled there.
    assert_eq!(
        pixel(&frame, 210, 290),
        (0, 0, 0, 255),
        "the up-pointing polygon is filled at its bottom-left corner, so it drew the \
         OTHER polygon's tail — two polygons can share a header, which is why \
         `__canvas_hashItem` folds a polygon's points in by hand",
    );
    assert_eq!(
        pixel(&frame, 510, 290),
        (0, 255, 0, 255),
        "the down-pointing polygon is empty at its bottom-left corner, so the tails were \
         crossed the other way",
    );

    // Two distinct polygons, two cache entries — and the offset is not part of the key.
    let entries = stats
        .first()
        .and_then(|l| {
            l.split_whitespace()
                .find_map(|f| f.strip_prefix("entries="))
        })
        .expect("an entries= field");
    assert_eq!(
        entries, "2",
        "expected one cache entry per distinct polygon: {stats:?}",
    );
}

/// Installing and removing groups under **many distinct names** does not grow the arena
/// without bound (plan-116-G).
///
/// `setGroup` copies the caller's name into the arena, because the table outlives the
/// caller's binding. That copy has to be released too — and it was not: `removeGroup`
/// zeroed the name pointer and a replacing `setGroup` overwrote it, leaving the block
/// unreachable either way.
///
/// **`groupBytes=` could not see this until the fix widened it**, which is why the
/// existing drain test passed against the leak: the counter charged the *items* block
/// only, so a run that leaked 200 name copies reported a table owning nothing. Making
/// the counter mean "every byte this table is responsible for" is half the fix, and it
/// is what turns this test into a direct measurement rather than an indirect one — a
/// counter that does not cover everything the table owns cannot detect the table owning
/// too much.
///
/// Measured both ways before being trusted: with the retire suppressed this reports
/// **16,530 bytes** still owned after the loop, and with it restored, zero.
///
/// Names are long and distinct on purpose. A short name shares the arena's small-block
/// behaviour with everything else in the frame; a 67-byte one leaked 400 times (each
/// name is installed, replaced, then removed) is the ~16 KB above, on a path a
/// long-running program hits every time it rebuilds its groups.
#[test]
fn installing_and_removing_many_named_groups_does_not_grow_without_bound() {
    let (_, stats) = render(
        "canvas_group_name_churn",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET keep AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET pad AS String = \"a-deliberately-long-group-name-so-a-leaked-copy-is-worth-measuring-\"\n  \
             canvas::present([keep])\n  \
             MUT i AS Integer = 0\n  \
             WHILE i < 200\n  \
               LET n AS String = pad & toString(i)\n  \
               canvas::setGroup(n, [red])\n  \
               canvas::present([keep])\n  \
               canvas::setGroup(n, [red, red])\n  \
               canvas::present([keep])\n  \
               canvas::removeGroup(n)\n  \
               canvas::present([keep])\n  \
               i = i + 1\n  \
             END WHILE\n  \
             LET last AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([keep, last])\n",
        ),
    );

    let last = stats.last().expect("a final frame");
    assert_eq!(
        stat(last, "groups="),
        "0",
        "every name was removed, so the table should hold none: {stats:?}",
    );
    let bytes: i64 = stat(last, "groupBytes=").parse().expect("a number");
    assert!(
        bytes < 4096,
        "the table still owns {bytes} bytes after 200 install/replace/remove cycles. \
         The items drain on their own gate, so this is almost certainly the interned \
         NAME copies: `setGroup` copies the caller's name into the arena because the \
         table outlives the caller's binding, and both `removeGroup` and a replacing \
         `setGroup` have to retire that copy rather than just overwrite the pointer. \
         Suppressing the retire reports 16530 here: {stats:?}",
    );
}

/// `__canvas_sceneDraws` produces the expected `(base, count, dx, dy)` sequence, and a
/// **diamond's two draws name the same item base** (plan-116-H Phase 1).
///
/// That last property is what this whole letter is arranged around, and it is the
/// decision §4.3 asked Phase 1 to make: a shared group's blocks are written **once** and
/// referenced, not once per reference. The alternative would duplicate the blocks, at
/// which point a per-draw offset earns nothing — the translation could simply be baked
/// into each copy as it is written, and the push constant this letter adds would be
/// unnecessary machinery.
///
/// Read off `MFB_CANVAS_STATS`, which is the only window onto a structure built on the
/// graphics thread and handed straight to an emitter. `blocks=` is how many item blocks
/// the frame uploads; `draws=` is `base:count:dx:dy:mode:repeat` per entry, `|`-separated, with the
/// offsets as the float32 bit patterns stored (`__canvas_float32Bits`) — 100.0 is
/// `1120403456`.
///
/// The four cases are one test rather than four because the interesting failures are
/// relative: a walk that emitted per-reference blocks passes the flat and single-group
/// cases unchanged and only diverges on the diamond, and a walk that dropped the
/// composed offset passes everything except the nested case.
#[test]
fn scene_draws_shares_one_base_between_a_diamonds_two_draws() {
    let d = |body: &str| -> (String, String) {
        let (_, stats) = render("canvas_scene_draws", &scene(body));
        let line = stats.last().expect("a frame").clone();
        (stat(&line, "blocks="), stat(&line, "draws="))
    };
    const RED: &str = "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  ";

    // 1. A flat scene: one run, no offset.
    let (blocks, draws) = d(&format!(
        "{RED}LET b AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([red, b])\n"
    ));
    assert_eq!(
        (blocks.as_str(), draws.as_str()),
        ("2", "0:2:0:0:0:1"),
        "a group-free scene must be one run of every item at no offset — the shape this \
         letter must not change for scenes that use no groups. The fifth field is the \
         run's BLEND MODE, which Phase 2 added to `__canvas_drawsText` because it selects \
         the pipeline: a draw list can read correct in base, count and offset and still \
         bind the wrong program. The sixth is the row's REPEAT (plan-175-B), 1: one draw \
         over the row's own node",
    );

    // 2. One group at (100, 100): its two items written once, one draw at the offset.
    let (blocks, draws) = d(&format!(
        "{RED}canvas::setGroup(\"g\", [red, red])\n  canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"]])\n"
    ));
    assert_eq!(
        (blocks.as_str(), draws.as_str()),
        ("2", "0:2:1120403456:1120403456:0:1"),
        "one group should be its own items once, drawn at its offset as float32 bits, in \
         Normal blend mode, repeat 1",
    );

    // 3. A nested group: the inner run is drawn at the COMPOSED offset.
    let (blocks, draws) = d(&format!(
        "{RED}canvas::setGroup(\"inner\", [red])\n  \
         canvas::setGroup(\"outer\", [canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"inner\"]])\n  \
         canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 500.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"outer\"]])\n"
    ));
    assert_eq!(
        blocks, "1",
        "the outer group has no items of its own — only the inner group's one block \
         should be laid out: {draws}",
    );
    assert!(
        draws.contains(&format!("{}:{}", 500.0f32.to_bits(), 300.0f32.to_bits())),
        "the inner run must be drawn at the composed offset (500,100)+(0,200) = \
         (500,300) = {}:{} as float32 bits; got {draws}",
        500.0f32.to_bits(),
        300.0f32.to_bits(),
    );

    // 4. The diamond: one group, two references. THE case.
    let (blocks, draws) = d(&format!(
        "{RED}canvas::setGroup(\"g\", [red, red])\n  \
         canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"], canvas::Group[id := \"\", position := vector::Float2[x := 300.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"]])\n"
    ));
    assert_eq!(
        blocks, "2",
        "a diamond must write the shared group's blocks ONCE. {blocks} blocks means they \
         were duplicated per reference, which is the decision §4.3 asked Phase 1 to make \
         and this test to enforce: draws={draws}",
    );
    let bases: Vec<&str> = draws
        .split('|')
        .map(|e| e.split(':').next().unwrap_or(""))
        .collect();
    assert_eq!(
        bases,
        vec!["0", "0"],
        "the diamond's two draws must name the SAME item base — that is the \
         buffer-sharing property the per-draw offset exists to make possible: {draws}",
    );
    assert_ne!(
        draws.split('|').next(),
        draws.split('|').nth(1),
        "the two draws are identical, so the offsets did not differ: {draws}",
    );

    // 5. A group whose items have DIFFERENT blend modes must split into two draws
    //    (H8). §4.1's rule -- a run ends at a group node or a Text item -- misses this,
    //    and the four cases above all use one blend mode, which is why they do not
    //    catch it. Each BlendMode is a separate pipeline and a pipeline is bound per
    //    draw, so one draw spanning both would render whichever was bound: a wrong
    //    colour, not a missing shape.
    let (blocks, draws) = d(&format!(
        "{RED}LET mul AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 20.0, y := 20.0], radius := 10.0, paint := WITH canvas::fill(color::rgb(0, 255, 0)) {{ blend := canvas::BlendMode.Multiply }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
           canvas::setGroup(\"g\", [red, mul])
           canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"]])
"
    ));
    assert_eq!(
        blocks, "2",
        "the group still lays out both blocks once: draws={draws}",
    );
    assert_eq!(
        draws.split('|').count(),
        2,
        "a group whose two items have different blend modes must become TWO draws, one \
         per pipeline. One draw spanning both renders the Multiply item with whichever \
         pipeline was bound -- a plausible wrong colour: {draws}",
    );
    assert!(
        draws.starts_with("0:1:") && draws.contains("|1:1:"),
        "the split must fall between the two blocks, at bases 0 and 1: {draws}",
    );
}

/// An image destroyed while a scene still names it: the frame renders, the item
/// contributes nothing, and nothing raises (plan-116-I Phase 3).
///
/// The reachable shape, and the *only* one. `canvas::destroyImage` consumes its
/// binding, so a program cannot name the image again afterwards — "destroy, then build
/// a new `Picture` from it" is a compile error (`2-203-0055 TYPE_USE_AFTER_MOVE`), not
/// a runtime case. What remains expressible is this: build the item while the image is
/// live, destroy the image, and present the item that already holds it. The scene's
/// copy outlives the binding, which is exactly the lifetime question this letter opened
/// by putting a resource in a record field.
///
/// Asserted as "a frame came back, black" rather than as "the picture is absent",
/// because a `Picture` draws nothing today anyway (bug-484). What would fail here is a
/// raise or a crash reaching through a resource the program has finished with — and
/// that is the property worth pinning now, before `Picture` learns to draw.
#[test]
fn an_image_destroyed_while_a_scene_names_it_renders_a_frame_and_does_not_raise() {
    let (frame, stats) = render(
        "canvas_image_destroyed_in_scene",
        &scene(
            "  LET px AS List OF Byte = [toByte(255), toByte(0), toByte(0), toByte(255)]\n  \
             RES img AS canvas::Image = canvas::createImage(1, 1, px) TRAP(e)\n  \
             RETURN\n  \
             END TRAP\n  \
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
             canvas::destroyImage(img)\n  \
             canvas::present([tile])\n",
        ),
    );
    assert!(
        !frame.is_empty(),
        "no frame came back at all — presenting a scene that names a destroyed image \
         must still render: {stats:?}"
    );
    assert!(
        frame.chunks(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0),
        "an item naming a destroyed image contributed ink",
    );
}

/// Destroying a font while the graphics thread is mid-frame is benign (plan-116-I
/// Phase 3, §3's claim — asserted rather than argued).
///
/// `MFB_CANVAS_FRAME_HOLD_MS` is what makes this a real test rather than a hopeful one:
/// it parks the graphics thread inside `__canvas_renderFrame` so the `destroyFont`
/// below lands *while a frame holding that font is being drawn*, not before or after.
/// Without it the two never overlap and the run proves nothing.
///
/// Placed here rather than in `rt_canvas_graphics_thread.rs`, where plan-116-I Phase 3
/// names it, because the hold affordance and the two sibling mid-frame race tests live
/// in this file — a third race test next to them is easier to keep honest than one that
/// has to re-derive the timing (**I6**).
///
/// What would fail: the graphics thread reads the item's resource record to get the
/// backend id, and the worker frees that record. §3 argues the read is safe because the
/// record lives in the worker's arena and is retained for the thread's lifetime — this
/// is that argument, run.
#[test]
fn destroying_a_font_mid_frame_is_clean() {
    let project = common::temp_project(
        "canvas_font_destroy_race",
        &scene(
            "  RES face AS canvas::Font = canvas::loadFont(\"fixture.ttf\") TRAP(e)\n  \
             RETURN\n  \
             END TRAP\n  \
             LET label AS canvas::DrawItem = canvas::Text[id := \"\", position := vector::Float2[x := 100.0, y := 200.0], text := \"AA\", fonts := [face], fontSize := 90.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET mark AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 30.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([mark, label])\n  \
             os::sleep(120)\n  \
             canvas::destroyFont(face)\n  \
             os::sleep(1200)\n",
        )
        .replace("IMPORT collections", "IMPORT collections\nIMPORT os"),
    );
    std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
        .expect("write the font fixture");
    let frame_path = project.join("frame.rgba");
    let binary = common::build_app_debug(&project, "canvas_font_destroy_race");
    let run = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame_path)
        .env("MFB_CANVAS_FRAME_HOLD_MS", "600")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "destroying a font while a frame holding it was mid-render did not exit \
         cleanly. The graphics thread reads the item's resource record for the backend \
         id; if the worker can free that record out from under it, this is where it \
         shows. exit {:?}\n{}\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    assert!(
        frame_path.exists(),
        "no frame was written, so the race never actually ran",
    );
}

/// A `setGroup` under a name a LAYERED scene already draws is seen by the next
/// `presentLayers`, as `present` has always seen it for a flat one — and a group cycle
/// under a layer raises, as it does under `present`.
///
/// Found by plan-150-A: `presentLayers` published the layers and never took the
/// resolved-groups signature. The layers themselves are identical before and after the
/// `setGroup` — a `Group` node is a name — so the second call reported "unchanged" and
/// the new group was never drawn: one frame where there should be two, and the old
/// 20 px box still on screen. The same missing walk is the one that refuses a group
/// nested past the depth limit, which a layered scene therefore never raised for.
#[test]
fn present_layers_sees_a_group_replaced_under_the_same_name() {
    const BOX: &str = "canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := WIDTH, height := 20.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]";
    const NODE: &str = "canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"NAME\"]";
    let node = NODE.replace("NAME", "g");
    let (frame, stats) = render(
        "canvas_layers_group_replaced",
        &scene(&format!(
            "  canvas::setGroup(\"g\", [{}])\n  canvas::presentLayers([canvas::DrawLayer[items := [{node}]]])\n  canvas::setGroup(\"g\", [{}])\n  canvas::presentLayers([canvas::DrawLayer[items := [{node}]]])\n",
            BOX.replace("WIDTH", "20.0"),
            BOX.replace("WIDTH", "200.0"),
        )),
    );
    assert_eq!(
        stats.len(),
        2,
        "the second presentLayers, after setGroup replaced the group, drew no frame"
    );
    // (150, 20) is inside the 200 px box and outside the 20 px one.
    let at = ((20 * 900 + 150) * 4) as usize;
    assert_eq!(&frame[at..at + 4], &[255, 0, 0, 255]);

    let cycle = NODE.replace("NAME", "self");
    let (stdout, _) = render_stdout(
        "canvas_layers_group_cycle",
        &scene(&format!(
            "  canvas::setGroup(\"self\", [{cycle}])\n  canvas::presentLayers([canvas::DrawLayer[items := [{cycle}]]]) TRAP(e)\n    io::print(\"raised \" & toString(e.code))\n    RETURN\n  END TRAP\n  io::print(\"NO RAISE\")\n"
        )),
    );
    assert!(
        stdout.contains("raised 77050024"),
        "a group cycle under a layer must raise ErrDepthExceeded from `presentLayers`; \
         got: {stdout}"
    );
}
