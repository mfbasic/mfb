### 1. Attribute-aware text operations are undiscoverable
UNIT:      man-pkg:astrings
PAGE:      package-wide
CATEGORY:  discoverability
CLAIM:     The overview lists only `astrings::` members and never tells readers that `strings::` operations accept `AttributedString` values or that `AttributedString & AttributedString` preserves styling.
VERDICT:   missing
EVIDENCE:  Rendered overview command `mfb man astrings` contains no `strings::` or concatenation reference. `src/docs/spec/stdlib/15_astrings.md` “Attribute-aware `strings::` overloads” specifies both surfaces. Probe built and ran with `mfb build /tmp/plan-125-scratch/B-phase2/man-pkg-astrings/coverage-probe`; `strings::mid` on styled `"hello"` followed by `&` printed `ell!` and `styled`, proving the transformed value retains its attribute.
SUGGESTED: Add an overview paragraph: “Many `strings::` operations accept an `AttributedString`: queries read its visible text, while supported transformations return an `AttributedString` and retain or adjust its attributes. `AttributedString & AttributedString` joins both text and attributes. See `mfb man strings` for the available operations.”

### 2. Markdown renderer summary hides color loss
UNIT:      man-pkg:astrings
PAGE:      astrings::toMarkdown
CATEGORY:  overview-mismatch
CLAIM:     “Render an AttributedString into a bespoke markdown-flavored format.”
VERDICT:   misleading
EVIDENCE:  The package overview promises foreground and background color as overlay styles, while `src/codegen/builtins/astrings/helper_md_state_at.rs:BODY` explicitly ignores `Foreground` and `Background` because the format cannot represent them. The probe command above printed `x` for a foreground-colored attributed string passed to `astrings::toMarkdown`, with no color marker.
SUGGESTED: Change the summary to: “Render flags, font, and font size from an AttributedString into a bespoke markdown-flavored format; foreground and background colors are omitted.”