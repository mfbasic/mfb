//! A `Text` item's `Paint.clip` on the GPU.
//!
//! The GPU backends build a glyph run's item block on a path of its own, and that path
//! once skipped the clip step every other item takes: the block's clip words held
//! whatever was left on the graphics thread's stack, so an unclipped run drew nothing
//! on some runs, and a clipped one was never clipped. This compares a clipped run
//! against the software renderer, which has always honoured the clip.

use super::*;

/// The fixture's `AAAA` at size 120 — four 39×39 squares standing on the baseline —
/// clipped to a triangle, in the item's own coordinates (the baseline origin at 0, 0),
/// whose hypotenuse cuts through every glyph, so the clip decides
/// the edge of each square and no glyph is wholly inside or outside it. A triangle and
/// not a rectangle, because a ring that is not one axis-aligned rectangle takes the
/// path-clip arm: the clip's edges ride the frame buffer's edge region, as a polygon's
/// do.
const CLIPPED_TEXT: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET ink AS canvas::Paint = canvas::fill(color::rgb(220, 40, 160))
  LET cut AS List OF vector::Float2 = [vector::Float2[x := 0.0 - 10.0, y := 0.0 - 50.0], vector::Float2[x := 320.0, y := 0.0 - 50.0], vector::Float2[x := 0.0 - 10.0, y := 20.0]]
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 300.0], text := "AAAA", fonts := [face], fontSize := 120.0, letterSpacing := 0.0, paint := WITH ink { clip := [cut] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([label])
END FUNC
"#;

/// The unclipped run draws 4,536 pixels (`text_on_the_gpu_matches_the_software_oracle`).
const UNCLIPPED_PIXELS: usize = 4536;

fn lit(pixels: &[u8]) -> usize {
    pixels
        .chunks(4)
        .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
        .count()
}

#[test]
fn a_clipped_text_run_on_the_gpu_matches_the_software_oracle() {
    let software = render_with("canvas_text_clip_sw", CLIPPED_TEXT, false);
    let (gpu, stats) = render_env(
        "canvas_text_clip_hw",
        CLIPPED_TEXT,
        &[("MFB_CANVAS_GPU", "1")],
    );
    if !stats.contains("metalReady=TRUE") {
        eprintln!("skip: this host reports no Metal device");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the clipped run fell back to software, so the GPU's clip went unexercised: {stats}"
    );
    let drawn = lit(&software);
    assert!(
        drawn > 0 && drawn < UNCLIPPED_PIXELS,
        "the software render lit {drawn} pixels; the clip should leave some of the \
         run's {UNCLIPPED_PIXELS} and not all of them, or the comparison says nothing \
         about the clip"
    );

    let height = (software.len() / 4 / WIDTH) as u32;
    let want = Frame {
        width: WIDTH as u32,
        height,
        pixels: software,
    };
    let got = Frame {
        width: WIDTH as u32,
        height,
        pixels: gpu,
    };
    if let Err(diff) = compare_within_tolerance(&got, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "the GPU's clipped glyph run disagrees with the oracle: {diff:?}\n\
             A run drawn whole is a glyph block without the item's clip words; a run \
             missing is clip words read from somewhere else."
        );
    }
}
