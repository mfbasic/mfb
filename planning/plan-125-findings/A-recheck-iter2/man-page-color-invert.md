### 1. "replaces" reads as in-place mutation; `base` is in fact left unchanged
UNIT:      man-page:color/invert
CLAIM:     "invert replaces each of red, green and blue with 255 - that channel and returns base's alpha unchanged."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/color/func_invert.rs:BODY` builds and RETURNs a fresh `Color[...]`; it never assigns into `base`. Probe `/tmp/plan-125-scratch/A-recheck-iter2/man-page-color-invert/probe.mfb` (built with `mfb build -q p_probe`, run `p_probe/build/p_probe.out`): after `LET n = color::invert(c)` the line `show("orig after invert", c)` printed `10 200 30 a=128`, i.e. `c` is untouched. The page's only signal that a new value comes back is the verb "Return" in the intro; the Description's lead verb "replaces" is the one a reader skims, and §2 of `.ai/man-content.md` lists mutation-vs-new-value as a sharp edge that must be stated explicitly. Every other sentence on the page checked true (see below).
SUGGESTED: "invert returns a new colour whose red, green and blue are each 255 - the matching channel of base, with base's alpha carried through unchanged; base itself is not changed."

Everything else verified against the binary at HEAD, for the audit record:

- **Both examples compile and run.** Example 1 printed `245 55 225 alpha 128`, which is exactly 255-10, 255-200, 255-30 with alpha 128 preserved. Example 2 printed `10 200 30`, confirming double inversion is the identity.
- **"Black inverts to white, red to cyan"**: probe printed `black->: 255 255 255`, `red->: 0 255 255`. True.
- **Alpha not inverted**: probe with alpha 0, 77, 128 and 255 all came back unchanged. True.
- **Mid-grey inverse is another mid-grey and not a readable contrast**: 128 inverts to 127 and the ratio printed by `color::contrastRatio` was `1.01`. True.
- **Errors table**: descriptor has `errors: vec![]`, and the rendered page has no Errors section. Consistent. The body is pure Byte arithmetic on 0..255, so no raise path exists.
- **See also**: `mfb man color contrastRatio` renders.
- **Scope and vocabulary**: no compiler internals and no banned memory words on the page. The only ownership-flavoured phrasing is the "replaces" wording flagged above.
