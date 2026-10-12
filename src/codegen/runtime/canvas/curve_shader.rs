//! The curve shapes' shader functions, as ONE text for both GPU backends (plan-164-C).
//!
//! `shaders/curves.tpl` is the fill test, the nearest point of a piece, the cap cut and
//! the walk that draws a `canvas::PolyCurve` and a `canvas::CurvePolygon` from their
//! pieces — the software renderer's functions in `builtins/canvas/helper_curves.rs`,
//! in fp32. It is written once, in GLSL's spelling with four markers, and [`curve_shader`]
//! turns it into each language:
//!
//! | Marker | MSL | GLSL |
//! |---|---|---|
//! | `@S ` before a function | `static ` | nothing |
//! | `@D ` in a parameter list | the item and the edge buffer, as parameters | nothing: both are globals |
//! | `@C ` in an argument list | `item, edges, ` | nothing |
//! | `@E[` | `edges[` | `edges.values[` |
//!
//! and the vector types (`vec2` → `float2`, …) for MSL. Metal compiles its source at run
//! time, so the MSL is spliced into `METAL_SHADER_SOURCE` where the body says
//! [`CURVE_SHADER_MARK`]. Vulkan takes checked-in SPIR-V, so the GLSL has to be in the
//! `.frag` file itself: `tools/canvas-curve-shader/generate.py` writes it between two
//! comment lines there, and `the_fragment_shader_holds_the_generated_curve_functions`
//! holds that copy equal to what this module produces.
//!
//! One text is the point. "Line for line" between two hand-kept shaders is a promise a
//! reviewer keeps; here a difference between the two is a difference in this table.

/// The template, in GLSL's spelling with the markers above.
pub(crate) const CURVE_SHADER_TEMPLATE: &str = include_str!("shaders/curves.tpl");

/// Where the Metal shader body takes the expansion.
pub(crate) const CURVE_SHADER_MARK: &str = "@CURVE_SHADER@";

/// The two comment lines the GLSL copy sits between in `mfb_canvas.frag`. Test-only:
/// the compiler never reads the copy, it embeds the SPIR-V compiled from it.
#[cfg(test)]
const CURVE_GLSL_BEGIN: &str =
    "// @CURVES-BEGIN generated from curves.tpl by tools/canvas-curve-shader/generate.py";
#[cfg(test)]
const CURVE_GLSL_END: &str = "// @CURVES-END";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShaderLanguage {
    Msl,
    /// Test-only, for the same reason: the build's GLSL is the checked-in file's.
    #[cfg(test)]
    Glsl,
}

/// One coordinate of an edge-region slot — word `w` of slot `slot` — as the 16.16 word
/// plus its residual, and a control slot's weight, which is fp32 bits: the two functions
/// of the block that are spelled per language, because the two backends lay the region
/// out differently.
fn curve_coord(language: ShaderLanguage) -> &'static str {
    match language {
        ShaderLanguage::Msl => {
            "static float2 curveCoord(constant MfbItem &item, constant int *edges, int slot, int w) {\n  \
             int i = slot * 4 + w;\n  \
             return edgeCoord(edges, i + METAL_EDGE_BASE, i + METAL_EDGE_RESIDUAL_BASE);\n}\n\
             static float curveWeight(constant MfbItem &item, constant int *edges, int slot) {\n  \
             return as_type<float>(edges[slot * 4 + 2 + METAL_EDGE_BASE]);\n}\n"
        }
        #[cfg(test)]
        ShaderLanguage::Glsl => {
            "vec2 curveCoord(int slot, int w) {\n  \
             int i = slot * 4 + w;\n  \
             return edgeCoord(i, i + EDGE_RESIDUAL_BASE);\n}\n\
             float curveWeight(int slot) {\n  \
             return intBitsToFloat(edges.values[slot * 4 + 2]);\n}\n"
        }
    }
}

/// The curve functions in `language`.
pub(crate) fn curve_shader(language: ShaderLanguage) -> String {
    let body = match language {
        ShaderLanguage::Msl => CURVE_SHADER_TEMPLATE
            .replace("@S ", "static ")
            .replace("@D ", "constant MfbItem &item, constant int *edges, ")
            .replace("@C ", "item, edges, ")
            .replace("@E[", "edges[")
            // The longer names first: `ivec2` and `bvec2` both end in `vec2`.
            .replace("ivec2", "int2")
            .replace("bvec2", "bool2")
            .replace("vec2", "float2")
            .replace("vec3", "float3")
            .replace("vec4", "float4"),
        #[cfg(test)]
        ShaderLanguage::Glsl => CURVE_SHADER_TEMPLATE
            .replace("@S ", "")
            .replace("@D ", "")
            .replace("@C ", "")
            .replace("@E[", "edges.values["),
    };
    format!("{}{body}", curve_coord(language))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No marker survives either expansion, and each language got its own spelling.
    #[test]
    fn the_curve_shader_expands_for_both_languages() {
        let msl = curve_shader(ShaderLanguage::Msl);
        let glsl = curve_shader(ShaderLanguage::Glsl);
        for text in [&msl, &glsl] {
            assert!(!text.contains('@'), "a marker survived:\n{text}");
        }
        assert!(msl.contains("static float2 pieceNearest(MfbPiece p)"));
        assert!(msl.contains("curvePiece(constant MfbItem &item, constant int *edges, int e,"));
        assert!(!msl.contains("vec2") && !msl.contains("vec3") && !msl.contains("vec4"));
        assert!(glsl.contains("\nvec2 pieceNearest(MfbPiece p)"));
        assert!(glsl.contains("edges.values[band.x + j]"));
        assert!(!glsl.contains("float2") && !glsl.contains("static "));
    }

    /// The GLSL copy in the fragment shader is this module's output, to the byte: the
    /// `.spv` is compiled from that file, so a copy that drifted would be a Vulkan
    /// renderer that no longer draws what Metal's does. Regenerate with
    /// `python3 tools/canvas-curve-shader/generate.py`, then `scripts/regen-spirv.sh`.
    #[test]
    fn the_fragment_shader_holds_the_generated_curve_functions() {
        let frag = include_str!("shaders/mfb_canvas.frag");
        let begin = frag
            .find(CURVE_GLSL_BEGIN)
            .expect("mfb_canvas.frag has the curve block's opening line");
        let start = begin + CURVE_GLSL_BEGIN.len() + 1;
        let end = frag[start..]
            .find(CURVE_GLSL_END)
            .expect("mfb_canvas.frag has the curve block's closing line")
            + start;
        assert_eq!(
            &frag[start..end],
            curve_shader(ShaderLanguage::Glsl),
            "mfb_canvas.frag's curve block is stale: run \
             `python3 tools/canvas-curve-shader/generate.py`"
        );
    }

    /// The step count the shaders loop to is the oracle's `__CANVAS_PIECE_STEPS`, and a
    /// piece's dash row is its `__CANVAS_CURVE_DASH_ROW` floats of `__CANVAS_CURVE_ARC_STEPS`
    /// steps. The template spells the three as literals — a shader has no access to the
    /// MFBASIC constants — so they are pinned here.
    #[test]
    fn the_curve_shader_literals_are_the_oracles_constants() {
        let source = include_str!("../../builtins/canvas/helper_curves.rs");
        let constant = |name: &str| -> usize {
            let needle = format!("LET {name} AS Integer = ");
            let at = source.find(&needle).unwrap_or_else(|| panic!("{name}")) + needle.len();
            source[at..]
                .lines()
                .next()
                .expect("a value")
                .trim()
                .parse()
                .expect("a number")
        };
        let steps = constant("__CANVAS_PIECE_STEPS");
        assert!(CURVE_SHADER_TEMPLATE.contains(&format!(
            "for (int i = 0; i < {steps}; ++i) {{\n    float f = pcF(k, order, t);"
        )));
        let arc = constant("__CANVAS_CURVE_ARC_STEPS");
        let row = constant("__CANVAS_CURVE_DASH_ROW");
        assert_eq!(
            row,
            arc + 2,
            "a row is arc + 1 positions and the contour's length"
        );
        assert!(CURVE_SHADER_TEMPLATE.contains(&format!("int row = data + e * {row};")));
        assert!(CURVE_SHADER_TEMPLATE.contains(&format!("float f = t * {arc}.0;")));
        assert!(CURVE_SHADER_TEMPLATE.contains(&format!("s1 = dashWord(@C row + {arc});")));
        assert!(CURVE_SHADER_TEMPLATE.contains(&format!("ring = dashWord(@C row + {});", arc + 1)));
    }

    /// The four places a shader hands a curve shape to this block, in the GLSL. Metal's
    /// twins are held by `the_metal_fragment_clips_and_samples_in_shape_space`; the two
    /// lists are the same calls, less the item and the buffer Metal passes by hand.
    #[test]
    fn the_fragment_shader_draws_curves_from_the_shape_space_point() {
        let frag = include_str!("shaders/mfb_canvas.frag");
        for call in [
            "if ((bk != 4 && bk != 10 && bk != 11 && bk != 12) || nb <= 0)",
            "curveWalk(band, x, y, kind == 12 ? 0 : (item.strokeStyle.w > 0 ? 3 : 1),",
            "curveWalk(band, cqx, cqy, dashed ? 3 : 2, dfF(half2), item.arc.w)",
            "item.misc.x == 2 || item.misc.x == 3 || item.misc.x == 11",
        ] {
            assert!(frag.contains(call), "mfb_canvas.frag has lost `{call}`");
        }
        let glsl = curve_shader(ShaderLanguage::Glsl);
        assert!(!glsl.contains("gl_FragCoord"));
    }
}
