//! plan-176-D: `rt_canvas_rasteriser`'s ellipses, the `#[ignore]`d Newton iteration
//! measurement, and the corner rule (radii and binding). The render helpers live in the
//! root, `rt_canvas_rasteriser.rs`.

use super::*;

/// plan-116-E Phase 1: how many Newton steps does the ellipse SDF need?
///
/// Kept and `#[ignore]`d rather than deleted, for the same reason
/// `measure_the_transformed_distance_correction` is: it measures a *design* question
/// whose answer is written into `planning/completed/plan-116-E-canvas-ellipse.md` §4.2.
/// Re-run it with
/// `cargo test --release --test rt_canvas_rasteriser -- --ignored --nocapture` if the
/// solve or the iteration count is ever revisited.
///
/// **Why the count has to be fixed rather than convergence-tested.** A
/// `WHILE |Δ| > ε` loop makes the number of steps depend on the input, which is fine
/// numerically and fatal for an oracle: the software rasteriser, Metal and Vulkan
/// would take different numbers of steps on the same pixel on different hardware, and
/// the software path would stop being predictive of the other two. So the count is
/// pinned by this measurement and shared by all three.
///
/// The ground truth is the true distance from the pixel centre to the ellipse,
/// obtained by dense sampling of the curve — not by another closed form, because the
/// closed forms this letter rejected are exactly what is being avoided. The error
/// reported is in **coverage steps of 1/255**, since that is the only thing a
/// difference in `d` can actually change: `clamp(0.5 - d, 0, 1)` quantised to 0..255.
#[test]
#[ignore = "a design measurement, not a regression gate; see plan-116-E §4.2"]
fn measure_the_ellipse_newton_iteration_count() {
    /// The true distance from `q` to the ellipse `(rx, ry)`, by dense sampling.
    ///
    /// 1 << 22 samples over the first quadrant, which at rx = 300 puts adjacent
    /// samples ~1e-4 px apart — two orders below the 1/255 coverage step this is
    /// measured against, so the ground truth is not the thing being measured.
    fn truth(qx: f64, qy: f64, rx: f64, ry: f64) -> f64 {
        // Coarse sweep to bracket the minimum, then twelve golden-section-style
        // halvings around it. Brute force at the resolution this needs (~1e-4 px at
        // rx = 300) is 4M samples per query and there are ~10^5 queries; bracket-then-
        // refine reaches the same place in ~4096 + 12·2. The distance along the curve
        // is unimodal in the first quadrant, so the bracket is sound.
        const COARSE: usize = 4096;
        let at = |t: f64| {
            let (s, c) = t.sin_cos();
            let dx = qx - rx * c;
            let dy = qy - ry * s;
            (dx * dx + dy * dy).sqrt()
        };
        let mut best_i = 0usize;
        let mut best = f64::INFINITY;
        for i in 0..=COARSE {
            let d = at(std::f64::consts::FRAC_PI_2 * (i as f64) / (COARSE as f64));
            if d < best {
                best = d;
                best_i = i;
            }
        }
        let step = std::f64::consts::FRAC_PI_2 / (COARSE as f64);
        let mut lo = (best_i.saturating_sub(1)) as f64 * step;
        let mut hi = ((best_i + 1).min(COARSE)) as f64 * step;
        for _ in 0..60 {
            let m1 = lo + (hi - lo) / 3.0;
            let m2 = hi - (hi - lo) / 3.0;
            if at(m1) < at(m2) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        let best = at((lo + hi) / 2.0).min(best);
        let inside = (qx / rx) * (qx / rx) + (qy / ry) * (qy / ry) < 1.0;
        if inside {
            -best
        } else {
            best
        }
    }

    /// §4.2's solve at `n` steps: Newton on the unit pair, never on an angle.
    fn solve(qx: f64, qy: f64, rx: f64, ry: f64, n: usize) -> f64 {
        // The seed is the gradient direction, exact in the folded first quadrant.
        let l = ((qx * rx) * (qx * rx) + (qy * ry) * (qy * ry)).sqrt();
        if l == 0.0 {
            // The exact centre. The sign test answers this without iterating.
            return -rx.min(ry);
        }
        let mut c = qx * rx / l;
        let mut s = qy * ry / l;
        for _ in 0..n {
            // Nearest-point residual: the component of (q - P) along the tangent,
            // over the second-order term. Ratios of dot products — `+ - * /` only.
            let px = rx * c;
            let py = ry * s;
            let ex = -rx * s;
            let ey = ry * c;
            let num = (qx - px) * ex + (qy - py) * ey;
            let den = ex * ex + ey * ey + (qx - px) * (rx * c) + (qy - py) * (ry * s);
            if den == 0.0 {
                break;
            }
            let delta = num / den;
            // Rotate the pair by the small-angle form and renormalise: an exact
            // rotation by atan(delta) rather than delta, i.e. a slightly damped step.
            let cp = c - s * delta;
            let sp = s + c * delta;
            let nn = (cp * cp + sp * sp).sqrt();
            c = cp / nn;
            s = sp / nn;
        }
        let dx = qx - rx * c;
        let dy = qy - ry * s;
        let d = (dx * dx + dy * dy).sqrt();
        let inside = (qx / rx) * (qx / rx) + (qy / ry) * (qy / ry) < 1.0;
        if inside {
            -d
        } else {
            d
        }
    }

    /// §4.2's **named fallback**: fixed-count bisection on the folded quadrant.
    ///
    /// Bisects the sign of `g(t) = (q - P(t)) · P'(t)`, the derivative of the squared
    /// distance. After the `|q|` fold, `g(0) = qy·ry ≥ 0` and `g(π/2) = −qx·rx ≤ 0`, so
    /// the bracket is guaranteed by construction rather than by a property of the
    /// input — which is the whole reason to prefer it over Newton here.
    ///
    /// The halving is the plan's midpoint-renormalise on the `(c, s)` pair, so no
    /// trigonometry appears: the angular midpoint of two unit vectors is their sum,
    /// normalised. Every operation is `+ - * /` and `sqrt`.
    fn bisect(qx: f64, qy: f64, rx: f64, ry: f64, n: usize) -> f64 {
        let g = |c: f64, s: f64| (qx - rx * c) * (-rx * s) + (qy - ry * s) * (ry * c);
        // The quadrant's endpoints, as (c, s) pairs.
        let (mut c0, mut s0) = (1.0f64, 0.0f64);
        let (mut c1, mut s1) = (0.0f64, 1.0f64);
        let (mut cm, mut sm) = (c0, s0);
        for _ in 0..n {
            let (cs, ss) = (c0 + c1, s0 + s1);
            let nn = (cs * cs + ss * ss).sqrt();
            cm = cs / nn;
            sm = ss / nn;
            if g(cm, sm) > 0.0 {
                c0 = cm;
                s0 = sm;
            } else {
                c1 = cm;
                s1 = sm;
            }
        }
        let dx = qx - rx * cm;
        let dy = qy - ry * sm;
        let d = (dx * dx + dy * dy).sqrt();
        let inside = (qx / rx) * (qx / rx) + (qy / ry) * (qy / ry) < 1.0;
        if inside {
            -d
        } else {
            d
        }
    }

    fn steps(a: f64, b: f64) -> f64 {
        let ca = (0.5 - a).clamp(0.0, 1.0) * 255.0;
        let cb = (0.5 - b).clamp(0.0, 1.0) * 255.0;
        (ca - cb).abs()
    }

    // Radii chosen at both ends of the plan's range, at the four eccentricities it
    // names. The 10:1 row is the one that decides `N`: the flat ends of a very
    // eccentric ellipse are where a seed from the gradient direction is furthest from
    // the true nearest point.
    //
    // 450 and 900 are past the plan's stated range deliberately: the bisection error
    // scales with the radius (the angular bracket after k halvings is
    // `(pi/2)/2^k`, so the arc it spans is proportional to r), and a canvas is 900 px
    // wide — an ellipse can legitimately be larger than the 300 the plan sampled. A
    // count chosen at 300 and deployed at 900 would be a third as accurate.
    let cases: &[(f64, f64)] = &[
        (5.0, 5.0),
        (5.0, 2.5),
        (5.0, 1.25),
        (5.0, 0.5),
        (300.0, 300.0),
        (300.0, 150.0),
        (300.0, 75.0),
        (300.0, 30.0),
        (450.0, 45.0),
        (900.0, 90.0),
    ];

    eprintln!("worst coverage error in 1/255 steps, over the antialiased band:");
    eprintln!(
        "  rx     ry     N=1      N=2      N=4      N=6      N=8     bis16    bis20    bis24"
    );
    let mut worst_by_n = [0.0f64; 9];
    let mut worst_bisect = [0.0f64; 3];
    for &(rx, ry) in cases {
        let mut row = format!("{rx:6.1} {ry:6.2}");
        // Sample the band where coverage is not saturated — |d| < 1 — since that is
        // the only place an error in `d` can move a pixel. Walk the curve and step off
        // it perpendicular. The ground truth is computed ONCE per query point and
        // reused across every N, which is what makes this run in seconds.
        const M: usize = 800;
        let mut queries = Vec::new();
        for i in 0..=M {
            let t = std::f64::consts::FRAC_PI_2 * (i as f64) / (M as f64);
            let (st, ct) = t.sin_cos();
            let bx = rx * ct;
            let by = ry * st;
            let nx = ct / rx;
            let ny = st / ry;
            let nl = (nx * nx + ny * ny).sqrt();
            for k in -4..=4 {
                let off = k as f64 * 0.25;
                let qx = (bx + nx / nl * off).abs();
                let qy = (by + ny / nl * off).abs();
                let t = truth(qx, qy, rx, ry);
                queries.push((qx, qy, t));
            }
        }
        for n in [1usize, 2, 4, 6, 8] {
            let mut worst = 0.0f64;
            for &(qx, qy, t) in &queries {
                let e = steps(solve(qx, qy, rx, ry, n), t);
                if e > worst {
                    worst = e;
                }
            }
            let slot = match n {
                1 => 1,
                2 => 2,
                4 => 3,
                6 => 4,
                _ => 5,
            };
            if worst > worst_by_n[slot] {
                worst_by_n[slot] = worst;
            }
            row.push_str(&format!(" {worst:8.4}"));
        }
        for (slot, n) in [16usize, 20, 24].iter().enumerate() {
            let mut worst = 0.0f64;
            for &(qx, qy, t) in &queries {
                let e = steps(bisect(qx, qy, rx, ry, *n), t);
                if e > worst {
                    worst = e;
                }
            }
            if worst > worst_bisect[slot] {
                worst_bisect[slot] = worst;
            }
            row.push_str(&format!(" {worst:8.4}"));
        }
        eprintln!("{row}");
    }
    eprintln!();
    for (slot, n) in [1usize, 2, 4, 6, 8].iter().enumerate() {
        eprintln!(
            "Newton N={n}: worst over all cases = {:.4} steps",
            worst_by_n[slot + 1]
        );
    }
    for (slot, n) in [16usize, 20, 24].iter().enumerate() {
        eprintln!(
            "bisection {n} halvings: worst over all cases = {:.4} steps",
            worst_bisect[slot]
        );
    }

    // The seed-basin check the plan asks for: a fixed-count Newton that starts in the
    // wrong quadrant does not converge and does not announce it. After the |q| fold
    // the seed is in the first quadrant by construction, so what is checked is that
    // the solved point stays there.
    //
    // **This assertion started as `d.is_finite() && d > 0.0` and was useless.** A
    // Newton step that converges to the stationary point on the FAR side of the
    // ellipse returns a distance that is finite and positive — it is just the wrong
    // one, six times too large. What has to be checked is agreement with the truth.
    let mut basin_failures = 0usize;
    for &(rx, ry) in cases {
        for i in 0..=200 {
            let t = std::f64::consts::FRAC_PI_2 * (i as f64) / 200.0;
            let (st, ct) = t.sin_cos();
            let qx = (rx * ct * 1.7).abs();
            let qy = (ry * st * 1.7).abs();
            let got = solve(qx, qy, rx, ry, 8);
            let want = truth(qx, qy, rx, ry);
            if (got - want).abs() > 0.01 {
                if basin_failures == 0 {
                    eprintln!(
                        "basin: Newton(8) converged to the wrong stationary point at \
                         rx={rx} ry={ry} q=({qx:.4}, {qy:.4}): got {got:.4}, want {want:.4}"
                    );
                }
                basin_failures += 1;
            }
            let got = bisect(qx, qy, rx, ry, 16);
            assert!(
                (got - want).abs() <= 0.01,
                "bisection(16) missed the nearest point at rx={rx} ry={ry} \
                 q=({qx}, {qy}): got {got}, want {want}"
            );
        }
    }
    eprintln!("basin: Newton(8) wrong-stationary-point failures = {basin_failures} of 1608");

    // The `rx == ry` seam. The question the plan asks — "is the guard introducing a
    // visible discontinuity?" — is NOT answered by comparing the solve at `ry != rx`
    // against a circle of radius `rx`: those are different shapes, and they differ by
    // about `|ry - rx|` in distance whatever the solve does. Measured that way, `ry =
    // rx·(1 + 1/4096)` at `rx = 300` reads as 18.7 steps, which is just `300/4096 ·
    // 255` and says nothing about the guard.
    //
    // What matters is (a) that the two arms agree AT the guard, where the exact float
    // compare hands over, and (b) that the difference off it goes to zero linearly
    // rather than jumping. Both are measured here.
    for &rx in &[5.0f64, 300.0, 900.0] {
        let mut worst_at = 0.0f64;
        for i in 0..=800 {
            let t = std::f64::consts::FRAC_PI_2 * (i as f64) / 800.0;
            let (st, ct) = t.sin_cos();
            for k in -4..=4 {
                let off = k as f64 * 0.25;
                let qx = (rx * ct + ct * off).abs();
                let qy = (rx * st + st * off).abs();
                let guard = (qx * qx + qy * qy).sqrt() - rx;
                let e = steps(bisect(qx, qy, rx, rx, 24), guard);
                if e > worst_at {
                    worst_at = e;
                }
            }
        }
        eprint!("seam rx={rx}: at the guard, solve vs circle arm = {worst_at:.4} steps;");
        // And off it, at three separations, to show the difference shrinks with the
        // shape difference rather than sitting at a step.
        for &denom in &[1024.0f64, 4096.0, 16384.0] {
            let ry = rx * (1.0 + 1.0 / denom);
            let mut worst = 0.0f64;
            for i in 0..=800 {
                let t = std::f64::consts::FRAC_PI_2 * (i as f64) / 800.0;
                let (st, ct) = t.sin_cos();
                for k in -4..=4 {
                    let off = k as f64 * 0.25;
                    let qx = (rx * ct + ct * off).abs();
                    let qy = (ry * st + st * off).abs();
                    let guard = (qx * qx + qy * qy).sqrt() - rx;
                    let e = steps(bisect(qx, qy, rx, ry, 24), guard);
                    if e > worst {
                        worst = e;
                    }
                }
            }
            eprint!(" 1/{denom:.0}: {worst:.3};");
        }
        eprintln!();
    }
}

/// An `Ellipse` with equal radii is **byte-identical** to the `Circle` of that radius.
///
/// plan-116-E §1's load-bearing case, and the cheapest available check on the solve
/// being right: if a 24-halving bisection and the closed-form circle distance disagree
/// anywhere, one of them is wrong about what "distance to this curve" means.
///
/// It holds *by construction* rather than by convergence — `__canvas_ellipseDistance`
/// short-circuits `rx = ry` to `sqrt(qx² + qy²) - rx`, literally the circle arm. A
/// fixed-count solve is never algebraically exact, and a last-bit residual can flip the
/// `clamp(0.5 - d, 0, 1)` quantisation on whichever edge pixel lands nearest a 1/255
/// step, so equality could not have been promised from the iteration. Phase 1 measured
/// the handover clean (0.0072 steps at `rx = 300`), which is what says the guard hides
/// nothing.
///
/// Whole frames rather than sampled pixels: the difference this guards against is a
/// one-step shift on the antialiased rim, which is exactly what spot checks miss.
#[test]
fn an_ellipse_with_equal_radii_is_identical_to_a_circle() {
    let paint = "canvas::fillStroke(color::rgb(90, 200, 255), color::rgb(255, 255, 255), 9.0)";
    let (circle, _) = render(
        "canvas_ellipse_as_circle_ref",
        &scene(&format!(
            "  LET a AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radius := 120.0, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([a])\n"
        )),
    );
    let (ellipse, _) = render(
        "canvas_ellipse_as_circle",
        &scene(&format!(
            "  LET a AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radiusX := 120.0, radiusY := 120.0, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a])\n"
        )),
    );
    assert!(
        circle.iter().any(|&b| b != 0),
        "the reference circle drew nothing, so the comparison would be vacuous"
    );
    assert_eq!(
        circle, ellipse,
        "an Ellipse with radiusX = radiusY and angle 0 must render byte-for-byte as \
         the Circle of that radius — a difference here is the rx == ry guard not \
         firing, or firing on a different value than the circle arm computes"
    );
}

/// An axis-aligned 3:1 ellipse covers its own extent and nothing beyond it.
///
/// The four extreme points are the ones a wrong SDF gets wrong first: an approximate
/// ellipse distance (the `(‖p/r‖ − 1)·min(rx, ry)` form this letter rejected) is worst
/// at the *flat* ends, where the curvature is lowest and the approximation's error is
/// largest. So the assertions bracket both ends of both axes, one pixel either side.
#[test]
fn an_axis_aligned_ellipse_covers_its_extent() {
    let (frame, _) = render(
        "canvas_ellipse_axis",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radiusX := 300.0, radiusY := 100.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a])\n",
        ),
    );
    // Inside, just short of each extreme.
    for (x, y, what) in [
        (698usize, 300usize, "the +X extreme"),
        (102, 300, "the -X extreme"),
        (400, 398, "the +Y extreme"),
        (400, 202, "the -Y extreme"),
        (400, 300, "the centre"),
    ] {
        assert_eq!(
            pixel(&frame, x, y),
            (255, 255, 255, 255),
            "{what} should be inside a 300x100 ellipse centred at (400, 300)"
        );
    }
    // Outside, just past each extreme.
    for (x, y, what) in [
        (702usize, 300usize, "past the +X extreme"),
        (98, 300, "past the -X extreme"),
        (400, 402, "past the +Y extreme"),
        (400, 198, "past the -Y extreme"),
        // The corner of the bounding box, well outside the curve — the case an
        // implementation that filled its bounds rather than its shape would fail.
        (650, 230, "the bounding box's corner region"),
    ] {
        assert_eq!(
            pixel(&frame, x, y),
            (0, 0, 0, 255),
            "{what} should be outside"
        );
    }
}

/// Rotating an ellipse by 90° swaps which axis is long.
///
/// The cheapest exact statement available about `angle`: a 3:1 ellipse turned a quarter
/// turn is the 1:3 ellipse, so the two frames must be identical. That catches a
/// rotation applied with the wrong sign, applied to the point instead of the frame, or
/// not applied at all — none of which a "does it look rotated" check would separate.
#[test]
fn rotating_an_ellipse_by_a_quarter_turn_swaps_its_axes() {
    let ell = |rx: f64, ry: f64, angle: &str| {
        format!(
            "  LET a AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radiusX := {rx:.1}, radiusY := {ry:.1}, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[a := math::cos({angle}), b := math::sin({angle}), c := 0.0 - math::sin({angle}), d := math::cos({angle}), tx := 0.0, ty := 0.0]]\n  \
             canvas::present([a])\n"
        )
    };
    let (turned, _) = render(
        "canvas_ellipse_turned",
        &scene(&ell(300.0, 100.0, "1.5707963267948966")),
    );
    let (swapped, _) = render("canvas_ellipse_swapped", &scene(&ell(100.0, 300.0, "0.0")));
    assert!(
        swapped.iter().any(|&b| b != 0),
        "the reference ellipse drew nothing, so the comparison would be vacuous"
    );

    // Localized rather than `assert_eq!` on the two buffers: a whole-frame equality
    // failure prints two 2.3 MB byte vectors, which is 16 MB of output and no
    // information. Count what differs and report the first one.
    let mut differing = 0usize;
    let mut worst = 0u8;
    let mut first = None;
    for i in 0..turned.len() / 4 {
        let (a, b) = (&turned[i * 4..i * 4 + 4], &swapped[i * 4..i * 4 + 4]);
        if a != b {
            differing += 1;
            let delta = (0..4).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0);
            if delta > worst {
                worst = delta;
            }
            if first.is_none() {
                first = Some((i % WIDTH, i / WIDTH, a.to_vec(), b.to_vec()));
            }
        }
    }
    // **Not byte-equality, and that is not a loosened assertion — byte-equality was
    // never the right claim here.** It would assert a property of the *trigonometry*
    // rather than of the rotation: `__canvas_cos(PI/2)` is the deterministic Taylor
    // pair's answer, not an exact zero, and it cannot be one. Its worst error over the
    // circle is 4.6e-7 (see `TRIG` in `helper_shapes.rs`), which at radius 300
    // displaces the rim by 1.4e-4 px — 0.035 of a 1/255 coverage step. A rim pixel
    // sitting within that of a quantisation boundary flips, and the sRGB encode turns
    // one coverage step into up to two output steps near mid-grey.
    //
    // Measured: **16 of 576000 pixels differ, worst channel delta 2** — first at
    // (432, 16), 211 against 210. The bound below is that measurement plus headroom,
    // and it keeps every discriminating power the test was written for: a rotation with
    // the wrong sign, applied to the shape instead of the query point, or not applied
    // at all each move *whole regions* — a 3:1 ellipse against a 1:3 one differs over
    // roughly 100,000 pixels, four orders of magnitude past this.
    assert!(
        worst <= 2 && differing * 500 < turned.len() / 4,
        "a 3:1 ellipse turned a quarter turn must be the 1:3 ellipse: {differing} \
         pixels differ (worst channel delta {worst}), first at {first:?}"
    );
    eprintln!(
        "quarter-turn vs swapped axes: {differing} of {} pixels differ by at most {worst}",
        turned.len() / 4
    );
}

/// plan-172-C: an ellipse has no `angle`; it is turned by its `transform`.
///
/// A 100 x 20 ellipse at (300, 300) turned half a radian about its own centre. Its long
/// axis now runs along (cos 0.5, sin 0.5) = (0.8776, 0.4794), so 92 px out along it is
/// (380.7, 344.1) on one side and (219.3, 255.9) on the other — both well inside the
/// ellipse. The control is the pixel 92 px out along +X, (392, 300): inside the ellipse
/// had it NOT turned, and 44 px off the turned axis against a short radius of 20.
#[test]
fn an_ellipse_is_turned_by_its_transform() {
    let (frame, _) = render(
        "canvas_ellipse_turned",
        &scene(
            "  LET co AS Float = 0.8775825618903728\n  \
             LET si AS Float = 0.479425538604203\n  \
             LET turn AS canvas::Transform = canvas::Transform[a := co, b := si, c := 0.0 - si, d := co, tx := 0.0, ty := 0.0]\n  \
             LET egg AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radiusX := 100.0, radiusY := 20.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := turn]\n  \
             canvas::present([egg])\n",
        ),
    );
    assert_eq!(
        pixel(&frame, 380, 344),
        (255, 255, 255, 255),
        "92 px along the turned long axis, towards the bottom-right"
    );
    assert_eq!(
        pixel(&frame, 219, 255),
        (255, 255, 255, 255),
        "92 px along the turned long axis, towards the top-left"
    );
    assert_eq!(
        pixel(&frame, 300, 300),
        (255, 255, 255, 255),
        "the centre does not move: the turn is about it"
    );
    assert_eq!(
        pixel(&frame, 392, 300),
        (0, 0, 0, 255),
        "92 px along +X is inside the UNTURNED ellipse only; drawing here means the \
         transform did not turn it"
    );
}

// ---- plan-172-E: a rectangle's corners ---------------------------------------------
//
// The rule is in the package's own MFBASIC (`__canvas_cornerRadii`), which no unit test
// can call, so it is proved by what it draws. `corner_reach` is the rule written a
// second time, in Rust, straight from `planning/new_canvas.md`: the oracle the pixels
// are held to.

/// The horizontal and vertical radius a corner value `r` resolves to on a `w` x `h`
/// rectangle.
fn corner_reach(w: f64, h: f64, r: f64) -> (f64, f64) {
    if r <= 0.0 {
        (0.0, 0.0)
    } else if r <= 1.0 {
        (r * w / 2.0, r * h / 2.0)
    } else {
        let c = r.min(w / 2.0).min(h / 2.0);
        (c, c)
    }
}

/// A rectangle at (100, 100) of the given size whose TOP-LEFT corner has radius `r`
/// and whose other three are square.
fn top_left_rounded(w: f64, h: f64, r: &str) -> String {
    scene(&format!(
        "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := {w:?}, height := {h:?}], radiusTL := {r}, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([box])\n"
    ))
}

/// The seven cases of the corner rule, each held to `corner_reach`.
///
/// For a resolved corner (rx, ry) the outline's corner is the ellipse quadrant centred
/// (100 + rx, 100 + ry). Two probes sit on the diagonal from that centre towards the
/// rectangle's corner: one 2 px short of the curve (inside, white) and one 2 px past it
/// (outside the shape but inside the rectangle's box, black). A square corner has no
/// curve; its probe is the corner pixel itself, which is white. Two pixels clears the
/// antialiased edge and the 0.05 px flattening error by a wide margin, so each probe is
/// a fact about which side of the corner it is on.
#[test]
fn corner_rule_every_meaning_of_a_radius_draws_the_corner_it_names() {
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    for (name, w, h, r, text) in [
        ("zero", 100.0, 40.0, 0.0, "0.0"),
        ("negative", 100.0, 40.0, -1.0, "0.0 - 1.0"),
        ("half", 100.0, 40.0, 0.5, "0.5"),
        ("one", 100.0, 40.0, 1.0, "1.0"),
        ("just_past_one", 100.0, 40.0, 1.01, "1.01"),
        ("clamped", 100.0, 40.0, 500.0, "500.0"),
        ("pixels", 100.0, 40.0, 12.0, "12.0"),
    ] {
        let (rx, ry) = corner_reach(w, h, r);
        let (frame, _) = render(
            &format!("canvas_corner_{name}"),
            &top_left_rounded(w, h, text),
        );
        // The far corner is square in every case: the rectangle is really there.
        assert_eq!(
            pixel(&frame, 100 + w as usize - 1, 100 + h as usize - 1),
            white,
            "{name}: the bottom-right corner pixel"
        );
        if rx < 3.0 || ry < 3.0 {
            // No curve to speak of (a square corner, or the one-pixel corner of 1.01):
            // 3 px in along both sides is inside whatever corner there is.
            assert_eq!(
                pixel(&frame, 103, 103),
                white,
                "{name}: just inside the top-left corner"
            );
            if rx == 0.0 {
                assert_eq!(
                    pixel(&frame, 100, 100),
                    white,
                    "{name}: a square corner's own pixel"
                );
            }
            continue;
        }
        let (cx, cy) = (100.0 + rx, 100.0 + ry);
        // The point of the ellipse on the diagonal towards the corner, parametrically at
        // 225 degrees, and the outward normal there.
        let k = std::f64::consts::FRAC_1_SQRT_2;
        let (ex, ey) = (cx - rx * k, cy - ry * k);
        let (nx, ny) = (-ry * k, -rx * k);
        let len = (nx * nx + ny * ny).sqrt();
        let (nx, ny) = (nx / len, ny / len);
        let inside = ((ex - 2.0 * nx) as usize, (ey - 2.0 * ny) as usize);
        let outside = ((ex + 2.0 * nx) as usize, (ey + 2.0 * ny) as usize);
        assert_eq!(
            pixel(&frame, inside.0, inside.1),
            white,
            "{name} (r = {r}, corner {rx} x {ry}): 2 px inside the curve at {inside:?}"
        );
        assert_eq!(
            pixel(&frame, outside.0, outside.1),
            black,
            "{name} (r = {r}, corner {rx} x {ry}): 2 px outside the curve at {outside:?}"
        );
    }
}

/// A 1 x 1 rectangle with a fractional radius: both radii are half a pixel. It must
/// draw its pixel and nothing else, and must not fail — the degenerate end of the rule.
#[test]
fn corner_rule_a_one_pixel_rectangle_is_still_one_pixel() {
    let (frame, _) = render("canvas_corner_unit", &top_left_rounded(1.0, 1.0, "1.0"));
    let lit = frame.chunks_exact(4).filter(|p| p[0] != 0).count();
    assert!(lit <= 1, "a 1 x 1 rectangle lit {lit} pixels");
    assert_eq!(
        pixel(&frame, 102, 100),
        (0, 0, 0, 255),
        "right of it is background"
    );
}

/// A `radiusTL`-only rectangle has one rounded corner and three square ones.
#[test]
fn corner_rule_only_the_named_corner_is_rounded() {
    let (frame, _) = render(
        "canvas_corner_only_tl",
        &top_left_rounded(200.0, 120.0, "40.0"),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 102, 102),
        black,
        "top-left: cut away by the 40 px corner"
    );
    assert_eq!(pixel(&frame, 297, 102), white, "top-right: square");
    assert_eq!(pixel(&frame, 102, 217), white, "bottom-left: square");
    assert_eq!(pixel(&frame, 297, 217), white, "bottom-right: square");
    assert_eq!(
        pixel(&frame, 130, 130),
        white,
        "inside the rounded corner's curve"
    );
}

/// `BindTo.Center` puts the rectangle's CENTRE at `position`; `TopLeft` puts its corner
/// there. The same 100 x 60 rectangle at (300, 300) both ways.
#[test]
fn corner_rule_bind_names_the_point_position_is() {
    let body = |bind: &str| {
        scene(&format!(
            "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], bind := canvas::BindTo.{bind}, size := canvas::Size[width := 100.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([box])\n"
        ))
    };
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    let (centre, _) = render("canvas_bind_center", &body("Center"));
    assert_eq!(
        pixel(&centre, 300, 300),
        white,
        "Center: the centre pixel is at position"
    );
    assert_eq!(
        pixel(&centre, 251, 271),
        white,
        "Center: 49 left and 29 up is inside"
    );
    assert_eq!(
        pixel(&centre, 348, 328),
        white,
        "Center: 48 right and 28 down is inside"
    );
    assert_eq!(
        pixel(&centre, 352, 300),
        black,
        "Center: 52 right is outside a 100-wide box"
    );
    assert_eq!(
        pixel(&centre, 300, 332),
        black,
        "Center: 32 down is outside a 60-high box"
    );

    let (corner, _) = render("canvas_bind_top_left", &body("TopLeft"));
    assert_eq!(
        pixel(&corner, 300, 300),
        white,
        "TopLeft: position is the corner pixel"
    );
    assert_eq!(
        pixel(&corner, 398, 358),
        white,
        "TopLeft: the box runs right and down"
    );
    assert_eq!(
        pixel(&corner, 298, 298),
        black,
        "TopLeft: nothing above or left of position"
    );
}

/// A `Center`-bound rectangle turns in place: a quarter turn of a 200 x 40 bar centred
/// (300, 300) is a 40 x 200 bar centred (300, 300).
#[test]
fn corner_rule_a_centre_bound_rectangle_spins_in_place() {
    let (frame, _) = render(
        "canvas_bind_center_turn",
        &scene(
            "  LET quarter AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 0.0, ty := 0.0]\n  \
             LET bar AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], bind := canvas::BindTo.Center, size := canvas::Size[width := 200.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := quarter]\n  \
             canvas::present([bar])\n",
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(pixel(&frame, 300, 300), white, "the centre stays put");
    assert_eq!(
        pixel(&frame, 300, 210),
        white,
        "90 px above the centre: along the turned bar"
    );
    assert_eq!(
        pixel(&frame, 300, 390),
        white,
        "90 px below the centre: along the turned bar"
    );
    assert_eq!(
        pixel(&frame, 390, 300),
        black,
        "90 px right: where the UNTURNED bar was"
    );
    assert_eq!(
        pixel(&frame, 210, 300),
        black,
        "90 px left: where the UNTURNED bar was"
    );
}

/// The flattened outline against the exact one, measured.
///
/// Four equal 24 px corners are the exact rounded-rectangle record. Making one of them
/// 24.000001 px makes the four unequal, which sends the same shape — to a millionth of
/// a pixel — down the flattened path. The two frames are compared pixel by pixel.
///
/// The comparison is of COVERAGE, not of frame bytes. A frame byte is sRGB-encoded, and
/// near black the encoding is steep: a coverage of 0.034 is byte 53, so two frames that
/// agree to a thirtieth of a pixel differ by 53 of 255 there. Decoded, the difference is
/// what the design says it is — a chord stays within 0.05 px of its arc, so a pixel's
/// coverage moves by at most 0.05. Measured: 656 pixels differ, all on the four corners
/// (the straight sides agree to one rounding step), and the worst is 0.0499.
#[test]
fn corner_rule_the_flattened_outline_is_within_its_bound_of_the_exact_one() {
    let body = |last: &str| {
        scene(&format!(
            "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.25, y := 100.5], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 300.0, height := 180.0], radiusTL := 24.0, radiusTR := 24.0, radiusBL := 24.0, radiusBR := {last}, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([box])\n"
        ))
    };
    // White on black, so a byte decoded to linear light IS the coverage.
    let coverage = |byte: u8| -> f64 {
        let c = f64::from(byte) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (exact, _) = render("canvas_corner_exact", &body("24.0"));
    let (flat, _) = render("canvas_corner_flat", &body("24.000001"));
    let mut worst = 0.0f64;
    let mut differing = 0usize;
    for (a, b) in exact.chunks_exact(4).zip(flat.chunks_exact(4)) {
        if a[0] != b[0] {
            differing += 1;
            worst = worst.max((coverage(a[0]) - coverage(b[0])).abs());
        }
    }
    println!(
        "flattened vs exact rounded rectangle: {differing} pixels differ, worst coverage delta {worst:.4}"
    );
    assert!(
        exact.chunks_exact(4).filter(|p| p[0] == 255).count() > 40_000,
        "the rectangle drew, so the comparison is of something"
    );
    assert!(
        differing > 0,
        "the two frames are identical, so the second rectangle did not take the flattened path"
    );
    assert!(
        worst <= 0.055,
        "the flattened outline's coverage differs from the exact one's by {worst:.4} in some \
         pixel; a chord within 0.05 px of its arc allows 0.05"
    );
}

/// `canvas::Size` holds Floats, and `canvas::getSize()` returns the surface's whole
/// pixel dimensions in them.
#[test]
fn corner_rule_the_surface_size_is_whole_pixels_held_as_floats() {
    let source = "IMPORT app\nIMPORT canvas\nIMPORT io\n\nFUNC main() AS Nothing\n  \
                  app::setMode(app::Mode.Canvas)\n  \
                  LET size AS canvas::Size = canvas::getSize()\n  \
                  LET half AS Float = size.width / 2.0\n  \
                  io::print(\"size \" & toString(size.width) & \" x \" & toString(size.height) & \" half \" & toString(half) & \" ints \" & toString(toInt(size.width)) & \" \" & toString(toInt(size.height)))\n  \
                  canvas::present([])\n  io::print(\"rendered\")\nEND FUNC\n";
    let project = common::temp_project("canvas_size_floats", source);
    let binary = common::build_app_debug(&project, "canvas_size_floats");
    let run = Command::new(&binary)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .output()
        .expect("run the size program");
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        out.contains("size 900.00 x 640.00 half 450.00 ints 900 640"),
        "the surface is 900 x 640, as Floats:\n{out}"
    );
}

/// A degenerate radius draws nothing, the same rule `Circle` follows.
#[test]
fn an_ellipse_with_a_zero_radius_draws_nothing() {
    for (name, rx, ry) in [
        ("canvas_ellipse_zero_rx", "0.0", "80.0"),
        ("canvas_ellipse_zero_ry", "80.0", "0.0"),
        ("canvas_ellipse_neg_rx", "0.0 - 5.0", "80.0"),
    ] {
        let (frame, _) = render(
            name,
            &scene(&format!(
                "  LET a AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radiusX := {rx}, radiusY := {ry}, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
                 canvas::present([a])\n"
            )),
        );
        assert!(
            frame
                .chunks_exact(4)
                .all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0),
            "{name}: a degenerate radius must draw nothing at all"
        );
    }
}

/// A stroked ellipse draws a band, hollow inside and bounded outside.
///
/// The stroke rides the same `|d| - half` band every other primitive uses, so what this
/// checks is that `d` is a true *signed* distance: an unsigned or wrongly-signed one
/// would fill the interior instead of leaving it hollow, and every fill-only test above
/// would still pass.
#[test]
fn a_stroked_ellipse_is_hollow() {
    let (frame, _) = render(
        "canvas_ellipse_stroked",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radiusX := 200.0, radiusY := 100.0, paint := canvas::stroke(color::rgb(255, 255, 255), 20.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a])\n",
        ),
    );
    assert_eq!(
        pixel(&frame, 400, 300),
        (0, 0, 0, 255),
        "the centre of a stroked-only ellipse must be background — ink here means the \
         distance is unsigned, and the band swallowed the interior"
    );
    // The band at the +X extreme: 200 +/- 10 from the centre at x = 400.
    assert_eq!(
        pixel(&frame, 600, 300),
        (255, 255, 255, 255),
        "the band's centre line at the +X extreme"
    );
    assert_eq!(
        pixel(&frame, 585, 300),
        (0, 0, 0, 255),
        "15 px inside the +X extreme is inside the hollow"
    );
    assert_eq!(
        pixel(&frame, 615, 300),
        (0, 0, 0, 255),
        "15 px outside the +X extreme is past the band"
    );
    // And at the +Y extreme, where the curvature is highest — the band's width there
    // is the case an approximate distance gets wrong.
    assert_eq!(
        pixel(&frame, 400, 400),
        (255, 255, 255, 255),
        "the band's centre line at the +Y extreme"
    );
    assert_eq!(
        pixel(&frame, 400, 385),
        (0, 0, 0, 255),
        "15 px inside the +Y extreme is inside the hollow"
    );
}
