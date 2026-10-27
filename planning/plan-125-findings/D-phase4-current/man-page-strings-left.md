### 1. `count` parameter description omits the zero and negative outcomes
UNIT:      man-page:strings/left
CLAIM:     "The number of leading Unicode scalar values to take. Must be 0 or greater; values at or above the scalar length of value yield the whole string."
VERDICT:   incomplete
EVIDENCE:  `.ai/man-content.md` §2 requires each parameter description to state "what happens at zero/empty/negative". The table row states the rule ("Must be 0 or greater") but not the consequence of breaking it, nor the zero case; both are only in the Description body three paragraphs down. Probes in /tmp/plan-125-scratch/D-phase4-current/man-page-strings-left: p2 (`strings::left("abc", -1)`) exits 255 with `Error: 7-705-0002 / Argument value is not valid for the requested operation.` (ErrInvalidArgument, 77050002 per `mfb man errorCode`); ex1 (`strings::left("hi", 0)`) prints `[]`. The `value` row, by contrast, does state its empty case.
SUGGESTED: "The number of leading Unicode scalar values to take. `0` returns the empty string; a negative value raises `ErrInvalidArgument`; a value at or above the scalar length of `value` returns the whole string."

UNVERIFIED: "`ErrOutOfMemory` means the program ran out of memory." The lowering (`gen_left_right.rs:lower_strings_left_right` → `emit_materialize_string_from_bytes`) does build a fresh string, so the error is reachable in principle, but I did not construct an out-of-memory condition in the sandbox. Everything else on the page checked out.

Audit trail for the sentences that passed, all run with the release binary from a scratch project under /tmp/plan-125-scratch/D-phase4-current/man-page-strings-left (`mfb build -q proj && ./proj/build/proj.out`):
- ex1 printed `hel`, `hi`, `[]`; ex2 printed `a😀`. Both examples compile and match the page.
- p1: `left("", 3)` and `left("", 0)` → `[]`; `left("héllo", 2)` → `hé`; `left("abc", 3)` and `left("abc", 9223372036854775807)` → `abc` (clamp, no padding); after `t = left(s, 2)`, `s & " " & t` → `hello he` (value not mutated).
- p7 with decomposed `e` + U+0301 + `x` (byteLen 4, graphemesCount 2): `left(d, 1)` → `e` with byteLen 1, so a cluster is cut in half exactly as the page warns; `left(d, 2)` → `é` intact.
- p3: `strings::mid("abc", 1, 10)` exits 255 with `7-705-0001` (ErrIndexOutOfRange, 77050001), confirming the stated contrast with `mid`. `src/codegen/builtins/strings/func_mid.rs` declares `errors: vec!["ErrIndexOutOfRange"]`.
- p4/p6/p5 (AttributedString overload via `src/codegen/builtins/astrings/helper_left.rs:__astrings_left`): bold span 0..4 on "hello world", `left(styled, 3)` → `**hel**`, `left(styled, 7)` → `**hello** w`, `left(plain, 50)` → `hello world`, `left(plain, -1)` → ErrInvalidArgument. Spans are remapped and the error path matches the String overload, as claimed.
- Cross-references `mfb man strings graphemes`, `mfb man strings mid`, and `mfb man astrings types` all render.
- Vocabulary sweep of the rendered page: no banned word appears; "new String" / "not mutated" use only permitted terms, and no sentence teaches an ownership model. The only "Allocation failed." is the derived ErrOutOfMemory message row (carve-out 2).
