### 1. Empty input behavior is undocumented
UNIT:      man-page:astrings/toMarkdown  
CLAIM:     “The attributed string to render.”  
VERDICT:   incomplete  
EVIDENCE:  Probe at `/tmp/plan-125-scratch/D-phase2/man-page-astrings-toMarkdown/example/src/main.mfb`, built and run with the specified release binary, printed `empty=[]`; `tests/acceptance/src/astrings.mfb` (`TCASE "plain and empty text pass through"`) also expects `toMarkdown(fromString(""))` to return `""`.  
SUGGESTED: The attributed string to render. An empty attributed string returns an empty string.

### 2. “enum-declaration” explains an implementation rationale, not the output contract
UNIT:      man-page:astrings/toMarkdown  
CLAIM:     “Flags wrap each run as nested pairs in canonical (enum-declaration) order — **bold**, *italic*, __underline__, ~~strike~~, ^^overline^^ — opened in order and closed in reverse, so overlapping spans always produce valid nesting.”  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/builtins/astrings/func_to_markdown.rs:84-118` explicitly emits bold, italic, underline, strike, then overline, and reverses that sequence when closing. The probe printed `flags=**a*****b******__cd__******e*****f**`. “enum-declaration” is not needed for a developer to use or interpret the output.  
SUGGESTED: Flags wrap each run as nested pairs in this order: **bold**, *italic*, __underline__, ~~strike~~, then ^^overline^^. They close in reverse order, including where styled ranges overlap.