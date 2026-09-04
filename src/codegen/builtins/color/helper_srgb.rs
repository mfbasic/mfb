//! The sRGB <-> linear-light transfer table, and the `color` package's colour
//! pipeline seam.
//!
//! **Every value here is deterministic on purpose.** canvas's software rasteriser
//! is the oracle its GPU backends are compared against, so it must produce the
//! same bytes on every target. That rules out `pow` and every other libm
//! transcendental, whose results differ across platforms — hence the literal table
//! below rather than evaluating the sRGB transfer function at run time.
//!
//! IEEE-754 `+`, `-`, `*`, `/` and `sqrt` ARE exactly specified and so are safe;
//! only transcendentals are not. Everything in `color` is written to that line.
//!
//! plan-122-B moved this table out of `canvas`, where it was the private
//! `__CANVAS_SRGB`. A package cannot reach another package's private `__` members,
//! so the seam is exposed as the public `color::toLinear`/`color::fromLinear` pair
//! and canvas now calls that. Duplicating the table instead would have left two
//! copies that must agree forever — with `color::luminance` disagreeing with what
//! canvas renders as the failure nobody could see.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// sRGB -> linear, as 256 literal entries scaled to `0..65535`.
///
/// Generated from the standard transfer function (`c/12.92` below `0.04045`, else
/// `((c+0.055)/1.055)^2.4`) and pasted, because computing it at run time would put
/// `pow` on the path and make the oracle platform-dependent. A module-level `LET`
/// so it is built once at program start rather than per pixel — the same reason
/// `crypto` hoists its round-constant tables.
///
/// **Two GPU shaders reproduce this table's ROUNDING by hand, and neither mentions
/// it by name.** `srgbTable(i)` in `runtime/canvas/shaders/mfb_canvas.frag` and in
/// the MSL string in `target/macos_aarch64/app/metal.rs` both recompute
/// `floor(srgbToLinear(i) * 65535 + 0.5)` — specifically to match the *quantisation*
/// here, because a gradient lerp happens in this table's integer space and a
/// midpoint that rounds differently from the oracle lands a step off. A grep for
/// this constant or for `__color_srgbTable` finds neither copy.
///
/// So changing the values or the rounding here silently desynchronises both GPU
/// backends. The test that catches it is
/// `the_gpu_draws_the_gradient_scene_the_reference_shows` in
/// `tests/rt_canvas_golden.rs` — the *gradient* path, not the blend tests, because
/// blending never enters that quantised space.
///
/// The plan-122-B move preserved both: the 256 literals were copied by machine and
/// the binary search verbatim, so the shaders needed no change and that test passed
/// untouched.
///
/// The graphics thread depends on this global being initialised on *its own*
/// thread — the trampoline runs the module's `LINK` and global initialisers there
/// for exactly this reason, and an unpopulated table renders every antialiased
/// pixel black rather than failing (`codegen::runtime::canvas`).
#[rustfmt::skip]
const SRGB_TABLE: &str =
r#"FUNC __color_srgbTable() AS List OF Integer
  RETURN [0, 20, 40, 60, 80, 99, 119, 139, 159, 179, 199, 219, 241, 264, 288, 313, 340, 367, 396, 427, 458, 491, 526, 562, 599, 637, 677, 718, 761, 805, 851, 898, 947, 997, 1048, 1101, 1156, 1212, 1270, 1330, 1391, 1453, 1517, 1583, 1651, 1720, 1790, 1863, 1937, 2013, 2090, 2170, 2250, 2333, 2418, 2504, 2592, 2681, 2773, 2866, 2961, 3058, 3157, 3258, 3360, 3464, 3570, 3678, 3788, 3900, 4014, 4129, 4247, 4366, 4488, 4611, 4736, 4864, 4993, 5124, 5257, 5392, 5530, 5669, 5810, 5953, 6099, 6246, 6395, 6547, 6700, 6856, 7014, 7174, 7335, 7500, 7666, 7834, 8004, 8177, 8352, 8528, 8708, 8889, 9072, 9258, 9445, 9635, 9828, 10022, 10219, 10417, 10619, 10822, 11028, 11235, 11446, 11658, 11873, 12090, 12309, 12530, 12754, 12980, 13209, 13440, 13673, 13909, 14146, 14387, 14629, 14874, 15122, 15371, 15623, 15878, 16135, 16394, 16656, 16920, 17187, 17456, 17727, 18001, 18277, 18556, 18837, 19121, 19407, 19696, 19987, 20281, 20577, 20876, 21177, 21481, 21787, 22096, 22407, 22721, 23038, 23357, 23678, 24002, 24329, 24658, 24990, 25325, 25662, 26001, 26344, 26688, 27036, 27386, 27739, 28094, 28452, 28813, 29176, 29542, 29911, 30282, 30656, 31033, 31412, 31794, 32179, 32567, 32957, 33350, 33745, 34143, 34544, 34948, 35355, 35764, 36176, 36591, 37008, 37429, 37852, 38278, 38706, 39138, 39572, 40009, 40449, 40891, 41337, 41785, 42236, 42690, 43147, 43606, 44069, 44534, 45002, 45473, 45947, 46423, 46903, 47385, 47871, 48359, 48850, 49344, 49841, 50341, 50844, 51349, 51858, 52369, 52884, 53401, 53921, 54445, 54971, 55500, 56032, 56567, 57105, 57646, 58190, 58737, 59287, 59840, 60396, 60955, 61517, 62082, 62650, 63221, 63795, 64372, 64952, 65535]
END FUNC

LET __COLOR_SRGB AS List OF Integer = __color_srgbTable()"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("color_srgbTable", SRGB_TABLE));
}

#[cfg(test)]
mod tests {
    use super::SRGB_TABLE;

    /// Parse the literal list out of `__color_srgbTable`'s body.
    fn table() -> Vec<i64> {
        let open = SRGB_TABLE.find('[').expect("table literal");
        let close = SRGB_TABLE.find(']').expect("table literal");
        SRGB_TABLE[open + 1..close]
            .split(',')
            .map(|entry| entry.trim().parse().expect("integer entry"))
            .collect()
    }

    /// The table is indexed by a `Byte`, so anything short of 256 entries leaves
    /// `collections::getOr` falling back to its `0` default for the high channels —
    /// which does not fail, it silently blends towards black. A hand-pasted literal
    /// that lost its tail did exactly that, and only a pixel dump revealed it, so the
    /// length is pinned here where `cargo test` can see it.
    #[test]
    fn srgb_table_covers_every_byte() {
        assert_eq!(table().len(), 256);
    }

    /// The endpoints must be exact: `0` keeps black black, and `65535` is what makes
    /// `blendChannel(d, s, 255) == s` hold for an opaque white source.
    #[test]
    fn srgb_table_endpoints_are_exact() {
        let entries = table();
        assert_eq!(entries[0], 0);
        assert_eq!(entries[255], 65535);
    }

    /// `__color_fromLinear` binary-searches this table, which is only correct if it
    /// is strictly increasing.
    #[test]
    fn srgb_table_is_strictly_increasing() {
        let entries = table();
        for pair in entries.windows(2) {
            assert!(pair[0] < pair[1], "not increasing at {pair:?}");
        }
    }

    /// Every entry matches the standard sRGB transfer function.
    ///
    /// This catches the subtler paste error the length test cannot: entries that are
    /// present but wrong. The truncated table was also wrong from index 121 onward.
    /// It is also what proves the plan-122-B move arrived intact — the literal was
    /// copied by machine rather than retyped, and this is the test that says so.
    #[test]
    fn srgb_table_matches_the_transfer_function() {
        for (index, &entry) in table().iter().enumerate() {
            let c = index as f64 / 255.0;
            let linear = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
            let expected = (linear * 65535.0).round() as i64;
            assert_eq!(entry, expected, "index {index}");
        }
    }
}
