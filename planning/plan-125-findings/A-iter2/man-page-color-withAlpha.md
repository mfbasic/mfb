### 1. Transparency round-trip does not restore an arbitrary colour
UNIT:      man-page:color/withAlpha
CLAIM:     “A colour at alpha 0 still remembers its hue — color::withAlpha(color::withAlpha(c, 0), 255) is c.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/color/func_with_alpha.rs:BODY` always replaces `alpha`. Probe `identity/src/main.mfb` printed:
`12 -> 255`
`not equal`
for `c = color::rgba(1, 2, 3, 12)`.
SUGGESTED: “A colour at alpha 0 retains its red, green and blue channels. Setting alpha back to 255 restores those channels, but the original alpha is not recovered.”

### 2. Negative and over-range alpha results are only implicit
UNIT:      man-page:color/withAlpha
CLAIM:     “The replacement alpha, clamped to 0..255: 0 fully transparent, 255 fully opaque.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_with_alpha.rs:BODY` calls `__color_clampByte(alpha)`. Probe `main.mfb` printed `0 255` for `color::withAlpha(brand, -1).alpha` and `color::withAlpha(brand, 256).alpha`.
SUGGESTED: “The replacement alpha: values below 0 become 0 (fully transparent), values above 255 become 255 (fully opaque), and values in between are kept.”