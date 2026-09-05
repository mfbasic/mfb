//! `__color_nameTable` — the CSS Color Level 4 `<named-color>` table.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The 148 CSS Color Level 4 `<named-color>` entries, lower-cased, as
/// `0xAARRGGBB` packed values with alpha `255`.
///
/// **Generated from the specification, not transcribed.** The table was parsed out
/// of the published CSS Color 4 document
/// (`https://www.w3.org/TR/css-color-4/`, the `named-color-table` rows) by
/// `/tmp/p122c/extract_named_colors.py`, matching each `<dfn>` and its `<td>#RRGGBB`
/// in a single paired regex — matching names and hexes as two independent lists
/// misaligned 3 against 181 on the first attempt, which is precisely the failure a
/// hand transcription makes silently.
///
/// One map rather than two: `fromName` reads it forward and `nameOf` walks it
/// backward. A second reverse map would double the shipped data to speed up a call
/// that is in no hot path, and this table is already the bulk of the package.
///
/// A module-level `LET` so it is built once at program start rather than per
/// lookup, the same shape the sRGB table uses.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_nameTable() AS Map OF String TO Integer
  RETURN Map OF String TO Integer { "aliceblue" := 4293982463, "antiquewhite" := 4294634455, "aqua" := 4278255615, "aquamarine" := 4286578644, "azure" := 4293984255, "beige" := 4294309340, "bisque" := 4294960324, "black" := 4278190080, "blanchedalmond" := 4294962125, "blue" := 4278190335, "blueviolet" := 4287245282, "brown" := 4289014314, "burlywood" := 4292786311, "cadetblue" := 4284456608, "chartreuse" := 4286578432, "chocolate" := 4291979550, "coral" := 4294934352, "cornflowerblue" := 4284782061, "cornsilk" := 4294965468, "crimson" := 4292613180, "cyan" := 4278255615, "darkblue" := 4278190219, "darkcyan" := 4278225803, "darkgoldenrod" := 4290283019, "darkgray" := 4289309097, "darkgreen" := 4278215680, "darkgrey" := 4289309097, "darkkhaki" := 4290623339, "darkmagenta" := 4287299723, "darkolivegreen" := 4283788079, "darkorange" := 4294937600, "darkorchid" := 4288230092, "darkred" := 4287299584, "darksalmon" := 4293498490, "darkseagreen" := 4287609999, "darkslateblue" := 4282924427, "darkslategray" := 4281290575, "darkslategrey" := 4281290575, "darkturquoise" := 4278243025, "darkviolet" := 4287889619, "deeppink" := 4294907027, "deepskyblue" := 4278239231, "dimgray" := 4285098345, "dimgrey" := 4285098345, "dodgerblue" := 4280193279, "firebrick" := 4289864226, "floralwhite" := 4294966000, "forestgreen" := 4280453922, "fuchsia" := 4294902015, "gainsboro" := 4292664540, "ghostwhite" := 4294506751, "gold" := 4294956800, "goldenrod" := 4292519200, "gray" := 4286611584, "green" := 4278222848, "greenyellow" := 4289593135, "grey" := 4286611584, "honeydew" := 4293984240, "hotpink" := 4294928820, "indianred" := 4291648604, "indigo" := 4283105410, "ivory" := 4294967280, "khaki" := 4293977740, "lavender" := 4293322490, "lavenderblush" := 4294963445, "lawngreen" := 4286381056, "lemonchiffon" := 4294965965, "lightblue" := 4289583334, "lightcoral" := 4293951616, "lightcyan" := 4292935679, "lightgoldenrodyellow" := 4294638290, "lightgray" := 4292072403, "lightgreen" := 4287688336, "lightgrey" := 4292072403, "lightpink" := 4294948545, "lightsalmon" := 4294942842, "lightseagreen" := 4280332970, "lightskyblue" := 4287090426, "lightslategray" := 4286023833, "lightslategrey" := 4286023833, "lightsteelblue" := 4289774814, "lightyellow" := 4294967264, "lime" := 4278255360, "limegreen" := 4281519410, "linen" := 4294635750, "magenta" := 4294902015, "maroon" := 4286578688, "mediumaquamarine" := 4284927402, "mediumblue" := 4278190285, "mediumorchid" := 4290401747, "mediumpurple" := 4287852763, "mediumseagreen" := 4282168177, "mediumslateblue" := 4286277870, "mediumspringgreen" := 4278254234, "mediumturquoise" := 4282962380, "mediumvioletred" := 4291237253, "midnightblue" := 4279834992, "mintcream" := 4294311930, "mistyrose" := 4294960353, "moccasin" := 4294960309, "navajowhite" := 4294958765, "navy" := 4278190208, "oldlace" := 4294833638, "olive" := 4286611456, "olivedrab" := 4285238819, "orange" := 4294944000, "orangered" := 4294919424, "orchid" := 4292505814, "palegoldenrod" := 4293847210, "palegreen" := 4288215960, "paleturquoise" := 4289720046, "palevioletred" := 4292571283, "papayawhip" := 4294963157, "peachpuff" := 4294957753, "peru" := 4291659071, "pink" := 4294951115, "plum" := 4292714717, "powderblue" := 4289781990, "purple" := 4286578816, "rebeccapurple" := 4284887961, "red" := 4294901760, "rosybrown" := 4290547599, "royalblue" := 4282477025, "saddlebrown" := 4287317267, "salmon" := 4294606962, "sandybrown" := 4294222944, "seagreen" := 4281240407, "seashell" := 4294964718, "sienna" := 4288696877, "silver" := 4290822336, "skyblue" := 4287090411, "slateblue" := 4285160141, "slategray" := 4285563024, "slategrey" := 4285563024, "snow" := 4294966010, "springgreen" := 4278255487, "steelblue" := 4282811060, "tan" := 4291998860, "teal" := 4278222976, "thistle" := 4292394968, "tomato" := 4294927175, "turquoise" := 4282441936, "violet" := 4293821166, "wheat" := 4294303411, "white" := 4294967295, "whitesmoke" := 4294309365, "yellow" := 4294967040, "yellowgreen" := 4288335154 }
END FUNC

LET __COLOR_NAMES AS Map OF String TO Integer = __color_nameTable()"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("color_nameTable", BODY));
}

#[cfg(test)]
mod tests {
    use super::BODY;

    /// Parse the `"name": value` pairs out of the map literal.
    fn table() -> Vec<(String, i64)> {
        let open = BODY.find('{').expect("map literal");
        let close = BODY.rfind('}').expect("map literal");
        BODY[open + 1..close]
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let (name, value) = entry.split_once(":=").expect("name := value");
                (
                    name.trim().trim_matches('"').to_string(),
                    value.trim().parse().expect("integer value"),
                )
            })
            .collect()
    }

    /// The entry count, so a truncated paste is caught.
    #[test]
    fn name_table_has_every_css_named_colour() {
        assert_eq!(table().len(), 148);
    }

    /// Every key is lower-case ASCII: `fromName` lower-cases its argument before
    /// looking up, so an upper-case key would be unreachable rather than wrong —
    /// a failure no round-trip test would notice.
    #[test]
    fn name_table_keys_are_lowercase_ascii() {
        for (name, _) in table() {
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase()),
                "not lower-case ASCII: {name}"
            );
        }
    }

    /// Every value is opaque. `nameOf` matches only on alpha `255`, so an entry
    /// with any other alpha would be permanently unreachable in reverse.
    #[test]
    fn name_table_values_are_opaque() {
        for (name, value) in table() {
            assert_eq!(
                (value >> 24) & 0xFF,
                255,
                "{name} is not opaque: {value:#010x}"
            );
            assert!((0..=0xFFFF_FFFF).contains(&value), "{name} out of range");
        }
    }

    /// The values most often misremembered, spot-checked against the specification.
    ///
    /// This is the `srgb_table_matches_the_transfer_function` lesson applied to bulk
    /// data: a length test catches a truncated paste, but only a value test catches
    /// entries that are present and wrong. `green` is the one that matters most —
    /// CSS `green` is `#008000`, and the colour a reader expects, `#00ff00`, is
    /// `lime`.
    #[test]
    fn name_table_spot_checks_match_the_specification() {
        let entries = table();
        let get = |want: &str| {
            entries
                .iter()
                .find(|(name, _)| name == want)
                .unwrap_or_else(|| panic!("missing {want}"))
                .1
        };
        assert_eq!(
            get("green"),
            0xFF008000,
            "CSS green is #008000, not #00ff00"
        );
        assert_eq!(get("lime"), 0xFF00FF00);
        assert_eq!(get("gray"), 0xFF808080);
        assert_eq!(get("grey"), 0xFF808080, "both spellings, same colour");
        assert_eq!(get("purple"), 0xFF800080);
        assert_eq!(get("rebeccapurple"), 0xFF663399);
        // The four colours CSS spells both ways must agree with each other.
        for (a, b) in [
            ("gray", "grey"),
            ("darkgray", "darkgrey"),
            ("lightgray", "lightgrey"),
            ("slategray", "slategrey"),
        ] {
            assert_eq!(get(a), get(b), "{a} and {b} must be the same colour");
        }
        // The two pairs CSS spells both ways for non-grey colours.
        assert_eq!(get("aqua"), get("cyan"));
        assert_eq!(get("fuchsia"), get("magenta"));
    }
}
