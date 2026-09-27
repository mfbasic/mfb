### 1. Attribute addition is described as mutation

UNIT:      man-page:astrings/fromString  
CLAIM:     “The constructed value has no attributes until `astrings::addAttribute` (and the other mutation members) records some.”  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/astrings/func_add_attribute.rs:BODY` returns `astrings::writeSpans(a, spans)`, a new `AttributedString`. Probe `/tmp/plan-125-scratch/D-phase2/man-page-astrings-fromString/example/src/main.mfb` printed `before` for `astrings::toMarkdown(a)` and `**before**` for the separately bound `styled`; the original `a` remained unstyled.  
SUGGESTED: The returned value has no attributes. `astrings::addAttribute` returns a new `AttributedString` with the requested attribute recorded.