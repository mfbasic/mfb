### 1. HSL hue endpoint disagrees with its type page
UNIT:      man-pkg:color
PAGES:     color::toHsl; color::Hsl
CATEGORY:  divergence
QUOTE-A:   color::toHsl: “hue in degrees from 0.0 up to but not including 360.0”
QUOTE-B:   color::Hsl: “The hue in degrees around the colour wheel, 0.0..360.0.”
VERDICT:   `color::toHsl` should win: it states the exclusive upper bound returned by the function, while the type page uses the package’s otherwise inclusive-looking range notation.
SUGGESTED: “The hue in degrees around the colour wheel, from 0.0 inclusive up to but not including 360.0. Reported as 0.0 for a colour with no saturation.”

### 2. CSS alias coverage is undercounted on both lookup pages
UNIT:      man-pkg:color
PAGES:     color::nameOf; color::fromName
CATEGORY:  fact-escaped-iter2
QUOTE-A:   color::nameOf: “Six colours have two CSS spellings”
QUOTE-B:   color::fromName: “CSS spells four greys both ways”
VERDICT:   Both claims are incomplete. `src/codegen/builtins/color/helper_name_table.rs:BODY` includes `dimgray`/`dimgrey`, `darkslategray`/`darkslategrey`, and `lightslategray`/`lightslategrey` in addition to the listed aliases; `src/codegen/builtins/color/func_name_of.rs:BODY` examines every table key and returns the alphabetically first match.
SUGGESTED: “Eight colours have two CSS spellings: six gray/grey pairs, plus aqua/cyan and fuchsia/magenta. `nameOf` returns the alphabetically first spelling.”