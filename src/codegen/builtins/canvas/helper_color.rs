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

/// The same over-operator, with a `BlendMode` applied to the source first.
///
/// **Mode `0` is bit-for-bit `__canvas_blendChannel`.** That is the whole
/// compatibility contract of plan-116-B: `BlendMode.Normal` is the zero value, every
/// `Paint` ever built carries it, and every existing golden renders through it. The
/// `Normal` arm therefore does not merely *compute* the same thing — it is the same
/// expression, so it cannot drift by a rounding step.
///
/// The three others are the standard separable blend functions on **linear** values
/// (`06_canvas.md` §"Rendering conventions" defines them there, not on sRGB bytes),
/// applied to the source and then composited over the destination by the ordinary
/// alpha step. Writing it in that order is what makes a partially-covered pixel right:
/// coverage scales *how much of the blended result lands*, it does not scale the
/// operands.
///
/// The linear table runs `0..65535`, so a product needs the `+ 32767` round-to-nearest
/// and a divide by 65535 — not 65536. Dividing by 65536 (a shift) would make
/// `multiply(x, white)` come out one step below `x`, and white is exactly the
/// destination a `Multiply` item is most often tested against.
#[rustfmt::skip]
const BLEND_CHANNEL_MODE: &str =
r#"FUNC __canvas_blendChannelMode(dst AS Byte, src AS Byte, alpha AS Integer, mode AS Integer) AS Byte
  LET dstLin AS Integer = collections::getOr(__CANVAS_SRGB, toInt(dst), 0)
  LET srcLin AS Integer = collections::getOr(__CANVAS_SRGB, toInt(src), 0)
  MUT blended AS Integer = srcLin
  IF mode = 1 THEN
    blended = (srcLin * dstLin + 32767) / 65535
  END IF
  IF mode = 2 THEN
    blended = srcLin + dstLin - (srcLin * dstLin + 32767) / 65535
  END IF
  ' `Add` is the one mode that does NOT go through the lerp below, and the reason is
  ' reproducibility rather than taste. Additive blending means "add the COVERED source
  ' to the destination, then clamp" -- coverage scales how much source is added, which
  ' is the premultiplied-source form every GPU expresses as the factor pair (One, One).
  '
  ' Lerping towards a pre-clamped sum instead -- `dst + (min(src+dst,1) - dst)*a` --
  ' agrees at full coverage and diverges by up to 0.15 in linear at partial coverage
  ' over a bright destination, which no fixed-function blend can reproduce. That would
  ' have made `Add` the one mode the GPU backends had to decline (plan-116-B C6).
  IF mode = 3 THEN
    LET added AS Integer = dstLin + (srcLin * alpha + 127) / 255
    IF added > 65535 THEN
      RETURN __canvas_linearToSrgb(65535)
    END IF
    RETURN __canvas_linearToSrgb(added)
  END IF
  LET mixed AS Integer = dstLin + ((blended - dstLin) * alpha + 127) / 255
  RETURN __canvas_linearToSrgb(mixed)
END FUNC"#;

/// The colour a gradient shows at `t`, interpolated **in linear light** (plan-116-F).
///
/// The stop walk is linear rather than a binary search: the stop count is small and
/// bounded, and a linear walk is the shape `__canvas_edgeDistance` already uses per
/// pixel for a polygon's edges.
///
/// **Linear light, not sRGB space**, and that is a decision rather than an oversight.
/// It is the space `06_canvas.md` §"Rendering conventions" already composites in, and
/// the space both shaders' `srgbToLinear` puts colours into, so a gradient blends the
/// way everything else on the surface does. It is also the choice that makes a
/// black-to-white ramp look uniformly bright: interpolating the *encoded* bytes spends
/// half the ramp below 22% of the light, which reads as dark-heavy. `gradients.png`'s
/// fourth row is that ramp, so the choice is inspectable rather than asserted.
///
/// Offsets arrive already clamped to `0..1` and monotonic (`__canvas_gradientTail`),
/// so the walk needs no defence against a stop that goes backwards. Before the first
/// stop and after the last, the end stop's colour holds rather than extrapolating.
#[rustfmt::skip]
const GRADIENT_COLOR: &str =
r#"FUNC __canvas_gradientChannel(loSrgb AS Integer, hiSrgb AS Integer, num AS Integer, den AS Integer) AS Byte
  LET loLin AS Integer = collections::getOr(__CANVAS_SRGB, loSrgb, 0)
  LET hiLin AS Integer = collections::getOr(__CANVAS_SRGB, hiSrgb, 0)
  IF den <= 0 THEN
    RETURN __canvas_linearToSrgb(loLin)
  END IF
  RETURN __canvas_linearToSrgb(loLin + (hiLin - loLin) * num / den)
END FUNC

FUNC __canvas_gradientStopColor(at AS Integer) AS Color
  RETURN Color[red := toByte(toInt(collections::getOr(__CANVAS_GEO_DATA, at + 1, 0.0))), green := toByte(toInt(collections::getOr(__CANVAS_GEO_DATA, at + 2, 0.0))), blue := toByte(toInt(collections::getOr(__CANVAS_GEO_DATA, at + 3, 0.0))), alpha := toByte(toInt(collections::getOr(__CANVAS_GEO_DATA, at + 4, 0.0)))]
END FUNC

FUNC __canvas_gradientColor(base AS Integer, count AS Integer, t AS Float) AS Color
  IF count < 2 THEN
    RETURN __canvas_transparent()
  END IF
  MUT tt AS Float = t
  IF tt < 0.0 THEN
    tt = 0.0
  END IF
  IF tt > 1.0 THEN
    tt = 1.0
  END IF
  ' The first stop at or past `tt`. Written as a full walk with a separate index
  ' rather than an early exit, because the obvious early-exit form clobbers the very
  ' index it found: setting the loop counter past `count` to leave the loop loses it,
  ' and the lerp below then reads five slots past the stops into the NEXT record's
  ' header -- which renders as a plausible flat colour rather than as a failure.
  MUT idx AS Integer = count
  MUT i AS Integer = 0
  WHILE i < count
    IF idx = count THEN
      IF collections::getOr(__CANVAS_GEO_DATA, base + i * 5, 0.0) >= tt THEN
        idx = i
      END IF
    END IF
    i = i + 1
  END WHILE
  IF idx >= count THEN
    RETURN __canvas_gradientStopColor(base + (count - 1) * 5)
  END IF
  IF idx <= 0 THEN
    RETURN __canvas_gradientStopColor(base)
  END IF
  LET loAt AS Integer = base + (idx - 1) * 5
  LET hiAt AS Integer = base + idx * 5
  LET loOff AS Float = collections::getOr(__CANVAS_GEO_DATA, loAt, 0.0)
  LET hiOff AS Float = collections::getOr(__CANVAS_GEO_DATA, hiAt, 0.0)
  MUT num AS Integer = 0
  LET span AS Float = hiOff - loOff
  IF span > 0.0 THEN
    num = toInt((tt - loOff) / span * 4096.0)
  END IF
  IF num < 0 THEN
    num = 0
  END IF
  IF num > 4096 THEN
    num = 4096
  END IF
  LET loA AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, loAt + 4, 0.0))
  LET hiA AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, hiAt + 4, 0.0))
  RETURN Color[red := __canvas_gradientChannel(toInt(collections::getOr(__CANVAS_GEO_DATA, loAt + 1, 0.0)), toInt(collections::getOr(__CANVAS_GEO_DATA, hiAt + 1, 0.0)), num, 4096), green := __canvas_gradientChannel(toInt(collections::getOr(__CANVAS_GEO_DATA, loAt + 2, 0.0)), toInt(collections::getOr(__CANVAS_GEO_DATA, hiAt + 2, 0.0)), num, 4096), blue := __canvas_gradientChannel(toInt(collections::getOr(__CANVAS_GEO_DATA, loAt + 3, 0.0)), toInt(collections::getOr(__CANVAS_GEO_DATA, hiAt + 3, 0.0)), num, 4096), alpha := toByte(loA + (hiA - loA) * num / 4096)]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_srgbTable", SRGB_TABLE));
    pkg.add_helper(RegistryHelper::always(
        "canvas_linearToSrgb",
        LINEAR_TO_SRGB,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_blendChannel", BLEND_CHANNEL));
    pkg.add_helper(RegistryHelper::always(
        "canvas_gradientColor",
        GRADIENT_COLOR,
    ));
    pkg.add_helper(RegistryHelper::always(
        "canvas_blendChannelMode",
        BLEND_CHANNEL_MODE,
    ));
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
