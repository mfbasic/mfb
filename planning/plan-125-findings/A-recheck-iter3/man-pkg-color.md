### 1. withAlpha claims to be the only member that changes alpha; mix changes it too
UNIT:      man-pkg:color
PAGES:     withAlpha, mix
CATEGORY:  divergence
QUOTE-A:   withAlpha: "color::brighten, color::darken and the HSL adjustments all leave alpha alone, so this is the one member that changes it."
QUOTE-B:   mix: "Alpha is interpolated too, unlike color::brighten and color::darken which leave it alone."
VERDICT:   mix wins; probe `color::mix(rgba(0,0,0,0), rgba(0,0,0,255), 0.5).alpha` prints 127 (proj/build/proj.out above). withAlpha's "the one member" is false as a package-wide claim, and it is the one most likely to be read as a rule.
SUGGESTED: withAlpha: "color::brighten, color::darken and the HSL adjustments all leave alpha alone; withAlpha is the one adjustment that changes alpha and nothing else (color::mix interpolates alpha along with the colour)."

### 2. Overview's ErrOutOfMemory scope does not match the derived Errors tables
UNIT:      man-pkg:color
PAGES:     overview, fromHex, fromName
CATEGORY:  broken-seam
QUOTE-A:   overview: "Apart from ErrOutOfMemory on the calls that build text, nothing else in the package raises, and the constructors never do."
QUOTE-B:   fromHex Errors table lists "77010001 │ ErrOutOfMemory"; fromName Errors table lists "77010001 │ ErrOutOfMemory". Both pages also say "ErrOutOfMemory means the program ran out of memory." Neither builds text; both parse it.
VERDICT:   The derived tables win (correct by construction). The overview's characterisation is the summary the reader will trust, and it excludes two pages that list the error.
SUGGESTED: overview: "ErrOutOfMemory can be raised by the members that build or parse text (color::fromHex, color::fromName, color::nameOf, color::toHex, color::toHexAlpha); nothing else in the package raises, and the numeric constructors never do."

### 3. rgb describes its relation to rgba backwards relative to rgba and hsla
UNIT:      man-pkg:color
PAGES:     rgb, rgba, hsla
CATEGORY:  divergence
QUOTE-A:   rgb: "It is color::rgba with the fourth argument supplied, and it is the constructor most programs want"
QUOTE-B:   rgba: "It is the general form of color::rgb, which is the same call with alpha fixed at 255." hsla: "hsla is color::hsl with the alpha channel supplied rather than fixed at fully opaque."
VERDICT:   rgba/hsla win. On hsla "supplied" means the caller passes alpha; on rgb the same word is used for the member where the caller does not. The reader meets both and gets the opposite meaning.
SUGGESTED: rgb: "It is color::rgba with alpha fixed at 255, and it is the constructor most programs want"

### 4. "HSL describes the sRGB colour … deliberate asymmetry with brighten/darken" restated on ten pages
UNIT:      man-pkg:color
PAGES:     overview, hsl, hsla, toHsl, saturate, desaturate, rotateHue, brighten, toLinear, types (Hsl)
CATEGORY:  redundancy
QUOTE-A:   hsl: "HSL describes the sRGB colour, not linear light. That is what CSS hsl() and every design tool mean, so a colour built here matches the hex a designer would write. It is a deliberate asymmetry with color::brighten and color::darken, which work in linear light — bumping lightness here and brightening there give different colours, and neither is wrong."
QUOTE-B:   toHsl: "HSL describes the sRGB colour, not linear light. That is what CSS hsl() and every design tool mean, so the numbers here match what a designer's tool shows for the same hex. It is a deliberate asymmetry with color::brighten and color::darken, which work in linear light: the lightness reported here is not the perceived brightness color::luminance measures, and bumping one is not the same as brightening the other." (saturate: "It is a deliberate asymmetry with color::brighten/color::darken."; brighten: "The two are different spaces on purpose, and a colour brightened here will not match a lightness bumped there."; overview: "The two spaces deliberately disagree".)
VERDICT:   The overview already owns the two-spaces paragraph. hsl and toHsl carry near-verbatim copies of the full argument; saturate, desaturate, rotateHue, hsla, brighten, toLinear and the Hsl record each add a shorter restatement. One owner, one clause elsewhere.
SUGGESTED: Keep the full explanation on the overview only. Every HSL page reduces to: "Saturation/hue/lightness are HSL properties, so this works on the sRGB colour, not in linear light (see mfb man color for the two spaces)." brighten and darken reduce to: "brighten and darken work in linear light; the HSL adjustments do not (see mfb man color)."

### 5. "Alpha is straight, not premultiplied" restated on five pages, two of them near-verbatim
UNIT:      man-pkg:color
PAGES:     overview, rgba, hsla, withAlpha, types (Color)
CATEGORY:  redundancy
QUOTE-A:   overview: "Alpha is straight, not premultiplied. 0 is fully transparent and 255 fully opaque, and a colour's red/green/blue are unaffected by it. The all-zero colour is therefore fully transparent."
QUOTE-B:   rgba: "alpha is straight (not premultiplied): 0 is fully transparent and 255 fully opaque, and the colour's red, green and blue are unaffected by it. The all-zero color::Color is therefore fully transparent." types Color: "alpha is straight, not premultiplied: a colour with alpha 0 is invisible but still keeps its red, green and blue … The all-zero colour is transparent black." hsla: "alpha is straight, not premultiplied, so it does not affect the colour the hue, saturation and lightness describe." withAlpha: "alpha is straight, not premultiplied, so the red, green and blue channels are returned exactly as they were."
VERDICT:   The Color record on the types page owns the channel semantics. rgba's paragraph is a copy of the overview's; hsla and withAlpha each re-derive the consequence.
SUGGESTED: Keep the sentence on the types page (Color) and one line on the overview. rgba, hsla and withAlpha: "alpha is 0..255, 0 fully transparent to 255 fully opaque, and never changes the colour channels (mfb man color types)."

### 6. The clamp rationale is repeated near-verbatim on overview, rgb and rgba
UNIT:      man-pkg:color
PAGES:     overview, rgb, rgba
CATEGORY:  redundancy
QUOTE-A:   overview: "Colours get computed — a base plus a delta, a channel scaled by a fraction — and a value one past an end is a rounding artefact, not a mistake worth stopping the program for. Their component parameters are Integer rather than Byte precisely so an out-of-range value can reach that clamp."
QUOTE-B:   rgba: "Colours are routinely computed — a base plus a delta, a channel scaled by a fraction, an interpolation between two colours — and a result that lands one past an end is a rounding artefact rather than a mistake worth stopping the program for. The components are Integer rather than Byte precisely so that out-of-range arithmetic can reach the clamp instead of failing at the call site." rgb: "The components are Integer rather than Byte precisely so that out-of-range arithmetic can reach the clamp instead of failing at the call site."
VERDICT:   The overview owns the design rationale (it is also where the Color[...] literal contrast lives). rgb and rgba should state the behaviour and link.
SUGGESTED: rgb/rgba: "Components are clamped to 0..255, not rejected; mfb man color explains why the parameters are Integer rather than Byte." Delete the duplicated rationale sentences on both pages.

### 7. rgb's parameter descriptions use a different clamp phrasing from every sibling
UNIT:      man-pkg:color
PAGES:     rgb, rgba, gray, hsla, withAlpha
CATEGORY:  divergence
QUOTE-A:   rgb (red/green/blue): "The red component. Values below 0 become 0 and above 255 become 255; both ends are valid."
QUOTE-B:   rgba: "The red component, clamped to 0..255." gray: "The level every channel takes, clamped to 0..255." hsla/withAlpha alpha: "clamped to 0..255: 0 fully transparent, 255 fully opaque."
VERDICT:   rgba's spelling wins; four pages use it and the Parameters column is the place a reader scans across pages.
SUGGESTED: rgb: "The red component, clamped to 0..255." (same for green and blue).

### 8. toPacked and fromPacked spell the same 24-bit mask two different ways
UNIT:      man-pkg:color
PAGES:     toPacked, fromPacked
CATEGORY:  divergence
QUOTE-A:   toPacked: "bits::band(color::toPacked(c), 16777215)" (desc and example), after introducing the result as "the 24-bit 0xRRGGBB form".
QUOTE-B:   fromPacked: "color::fromPacked(bits::bor(rgb24, 0xFF000000))" and example literal "color::fromPacked(0xFF3366CC)".
VERDICT:   fromPacked wins. A hex literal is the whole point of a 0xAARRGGBB page; probe `bits::band(color::toPacked(color::rgba(51,102,204,255)), 0xFFFFFF)` compiles and prints 3368652 (0x3366CC).
SUGGESTED: toPacked: "bits::band(color::toPacked(c), 0xFFFFFF)" in both the description and the example.

### 9. isDark restates the WCAG thresholds that contrastRatio owns, and drops one
UNIT:      man-pkg:color
PAGES:     isDark, contrastRatio
CATEGORY:  redundancy
QUOTE-A:   contrastRatio: "The WCAG thresholds worth remembering: 4.5 for body text, 3.0 for large text and for user-interface components."
QUOTE-B:   isDark: "the answer is color::contrastRatio against the WCAG thresholds (4.5 for body text, 3.0 for large text), not this."
VERDICT:   contrastRatio owns the thresholds; isDark's copy already differs (no UI-component clause) and will keep drifting. isLight correctly just links.
SUGGESTED: isDark: "the answer is color::contrastRatio against the WCAG thresholds it documents, not this."

No factual errors escaped iteration 2 among the claims I probed: fromHex rejects leading whitespace and fromName trims it exactly as each page says (probe output "hex rejected TRUE" / "#ff0000ff"), and gray's alpha is 255.
