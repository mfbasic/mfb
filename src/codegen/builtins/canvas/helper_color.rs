//! The canvas colour pipeline: sRGB <-> linear, and the one place a pixel is written.
//!
//! **Every value here is deterministic on purpose.** The software rasteriser is the
//! oracle plan-98-E/F compare their GPU output against, so it must produce the same
//! bytes on every target. That rules out `pow` and every other libm transcendental,
//! whose results differ across platforms — hence the literal table below rather than
//! evaluating the sRGB transfer function at run time.
//!
//! IEEE-754 `+`, `-`, `*`, `/` and `sqrt` ARE exactly specified and so are safe; only
//! transcendentals are not. The rasteriser is written to that line.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// sRGB -> linear, as 256 literal entries scaled to `0..65535`.
///
/// Generated from the standard transfer function (`c/12.92` below `0.04045`, else
/// `((c+0.055)/1.055)^2.4`) and pasted, because computing it at run time would put
/// `pow` on the path and make the oracle platform-dependent. A module-level `LET` so
/// it is built once at program start rather than per pixel — the same reason
/// `crypto` hoists its round-constant tables.
#[rustfmt::skip]
const SRGB_TABLE: &str =
r#"FUNC __canvas_srgbTable() AS List OF Integer
  RETURN [0, 20, 40, 60, 80, 99, 119, 139, 159, 179, 199, 219, 241, 264, 288, 313, 340, 367, 396, 427, 458, 491, 526, 562, 599, 637, 677, 718, 761, 805, 851, 898, 947, 997, 1048, 1101, 1156, 1212, 1270, 1330, 1391, 1453, 1517, 1583, 1651, 1720, 1790, 1863, 1937, 2013, 2090, 2170, 2250, 2333, 2418, 2504, 2592, 2681, 2773, 2866, 2961, 3058, 3157, 3258, 3360, 3464, 3570, 3678, 3788, 3900, 4014, 4129, 4247, 4366, 4488, 4611, 4736, 4864, 4993, 5124, 5257, 5392, 5530, 5669, 5810, 5953, 6099, 6246, 6395, 6547, 6700, 6856, 7014, 7174, 7335, 7500, 7666, 7834, 8004, 8177, 8352, 8528, 8708, 8889, 9072, 9258, 9445, 9635, 9828, 10022, 10219, 10417, 10619, 10822, 11028, 11235, 11446, 11658, 11873, 12090, 12309, 12530, 12754, 12980, 13209, 13440, 13673, 13909, 14146, 14387, 14629, 14874, 15122, 15371, 15623, 15878, 16135, 16394, 16656, 16920, 17187, 17456, 17727, 18001, 18277, 18556, 18837, 19121, 19407, 19696, 19987, 20281, 20577, 20876, 21177, 21481, 21787, 22096, 22407, 22721, 23038, 23357, 23678, 24002, 24329, 24658, 24990, 25325, 25662, 26001, 26344, 26688, 27036, 27386, 27739, 28094, 28452, 28813, 29176, 29542, 29911, 30282, 30656, 31033, 31412, 31794, 32179, 32567, 32957, 33350, 33745, 34143, 34544, 34948, 35355, 35764, 36176, 36591, 37008, 37429, 37852, 38278, 38706, 39138, 39572, 40009, 40449, 40891, 41337, 41785, 42236, 42690, 43147, 43606, 44069, 44534, 45002, 45473, 45947, 46423, 46903, 47385, 47871, 48359, 48850, 49344, 49841, 50341, 50844, 51349, 51858, 52369, 52884, 53401, 53921, 54445, 54971, 55500, 56032, 56567, 57105, 57646, 58190, 58737, 59287, 59840, 60396, 60955, 61517, 62082, 62650, 63221, 63795, 64372, 64952, 65535]
END FUNC

LET __CANVAS_SRGB AS List OF Integer = __canvas_srgbTable()"#;

/// linear -> sRGB, by binary search over the forward table.
///
/// A reverse *table* would need 65536 entries; searching the 256-entry forward one
/// costs 8 comparisons and is exactly as deterministic. The result is the sRGB byte
/// whose linear value is nearest, so `linear(srgb(x)) == x` holds for every one of
/// the 256 representable outputs.
#[rustfmt::skip]
const LINEAR_TO_SRGB: &str =
r#"FUNC __canvas_linearToSrgb(value AS Integer) AS Byte
  MUT lo AS Integer = 0
  MUT hi AS Integer = 255
  WHILE lo < hi
    LET mid AS Integer = (lo + hi) / 2
    LET midLow AS Integer = collections::getOr(__CANVAS_SRGB, mid, 0)
    LET midHigh AS Integer = collections::getOr(__CANVAS_SRGB, mid + 1, 65535)
    IF value > (midLow + midHigh) / 2 THEN
      lo = mid + 1
    ELSE
      hi = mid
    END IF
  END WHILE
  RETURN toByte(lo)
END FUNC"#;

/// One channel of the over-operator, in linear space.
///
/// `dst + (src - dst) * alpha`, evaluated on the linear values and converted back.
/// Rounding is round-to-nearest via the `+ 127` before the divide, so
/// `blendChannel(d, s, 255) == s` holds exactly rather than drifting a step low.
#[rustfmt::skip]
const BLEND_CHANNEL: &str =
r#"FUNC __canvas_blendChannel(dst AS Byte, src AS Byte, alpha AS Integer) AS Byte
  LET dstLin AS Integer = collections::getOr(__CANVAS_SRGB, toInt(dst), 0)
  LET srcLin AS Integer = collections::getOr(__CANVAS_SRGB, toInt(src), 0)
  LET mixed AS Integer = dstLin + ((srcLin - dstLin) * alpha + 127) / 255
  RETURN __canvas_linearToSrgb(mixed)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_srgbTable", SRGB_TABLE));
    pkg.add_helper(RegistryHelper::always(
        "canvas_linearToSrgb",
        LINEAR_TO_SRGB,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_blendChannel", BLEND_CHANNEL));
}

#[cfg(test)]
mod tests {
    use super::SRGB_TABLE;

    /// Parse the literal list out of `__canvas_srgbTable`'s body.
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

    /// `__canvas_linearToSrgb` binary-searches this table, which is only correct if it
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
            assert_eq!(entry, expected, "sRGB table entry {index}");
        }
    }
}
