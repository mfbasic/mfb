Probes done. Two findings: the range notation and a missing sharp edge.

### 1. `0..255` reads as end-exclusive in MFBASIC's own notation
UNIT:      man-page:color/rgb
CLAIM:     "The red component, clamped to 0..255." (all three parameter rows, and "each clamped to 0..255" in the Description)
VERDICT:   misleading
EVIDENCE:  The clamp is inclusive at both ends: `__color_clampByte` in `src/codegen/builtins/color/helper_clamp_byte.rs` returns `toByte(value)` for `value = 255`, and the probe `color::rgb(0, 255, 128)` printed `0 255 128 255`; `color::rgb(-1, 256, 9223372036854775807)` printed `0 255 255 255`. But the one place `mfb man` defines `..` on a range, `src/docs/man/debug/06_values.md:59`, says "`0..10` stops before 10", so a developer who has read that page reads `0..255` as excluding 255. The brief's test is explicit: the description must state inclusive or exclusive, and this one does neither. (Same wording appears in `func_rgba.rs`, `func_gray.rs`, `func_hsla.rs`, `func_with_alpha.rs`, `mod.rs`; those are other units, but the fix should be made consistently.)
SUGGESTED: Parameter rows: "The red component. Values below 0 become 0 and above 255 become 255; 0 and 255 are both valid." Description: "each clamped to the inclusive range 0 to 255".

### 2. A colour's own channels cannot be passed straight back in
UNIT:      man-page:color/rgb
CLAIM:     "The components are Integer rather than Byte precisely so that out-of-range arithmetic can reach the clamp instead of failing at the call site."
VERDICT:   incomplete
EVIDENCE:  The sentence invites the reader to compute components from an existing colour, but `color::Color`'s fields are `Byte` (`mfb man color types`) and a `Byte` is not accepted where an `Integer` parameter is expected. Probe:
```
LET v AS Byte = toByte(200)
LET c AS color::Color = color::rgb(v, v, v)
```
fails to build with `error[2-203-0043 TYPE_UNKNOWN_VALUE]: Initializer for binding `c` does not have a known type.` The working form is `color::rgb(toInt(c.red) + 250, toInt(c.green), toInt(c.blue))`, which built and printed `#ff141eff` (the clamp applied). The page never says that a channel read from a `Color` must go through `toInt` first, and the diagnostic the developer gets instead does not mention Byte or Integer at all, so the page is the only place they could learn it. (The "failing at the call site" half of the claim is verified: `toByte(300)` raises runtime error 7-705-0010 "Arithmetic overflow or numeric conversion outside the destination range.") The unhelpful `TYPE_UNKNOWN_VALUE` diagnostic for a Byte-to-Integer argument mismatch is a compiler defect worth filing by the main thread.
SUGGESTED: Add after that sentence: "A colour's own red, green, blue and alpha fields are Byte, so to feed one back in, convert it first: `color::rgb(toInt(c.red) + 40, toInt(c.green), toInt(c.blue))`."

Everything else on the page checked out against the binary. Both examples compiled and ran from `/tmp/plan-125-scratch/A-recheck-iter2/man-page-color-rgb/proj_ex1` and `proj_ex2`; the second printed `255 64` as the prose implies. `color::rgb(1, 2, 3)` and `color::rgba(1, 2, 3, 255)` compared equal field for field and printed `#010203ff`, confirming "rgba with the fourth argument supplied" and alpha fixed at 255. The Errors table is absent and the descriptor declares `errors: vec![]`, which matches a body that only clamps. No banned memory vocabulary and no implementation vocabulary appear on the rendered page.
