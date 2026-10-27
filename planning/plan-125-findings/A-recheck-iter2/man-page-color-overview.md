### 1. "rejected" does not say when or how the rejection happens
UNIT:      man-page:color/overview
CLAIM:     "A color::Color[...] literal does not clamp: its fields are Byte, so an out-of-range value is rejected rather than pulled into range."
VERDICT:   incomplete
EVIDENCE:  Three probes under /tmp/plan-125-scratch/A-recheck-iter2/man-page-color-overview, built with the P-125 release `mfb build`:
           - p2: `color::Color[300, 0, 0, 255]` → build fails, `error[2-203-0013 TYPE_BYTE_LITERAL_OVERFLOW]: Integer literal 300 is outside the Byte range 0..255.`
           - p3: `LET v AS Integer = 300` then `color::Color[v, 0, 0, 255]` → build fails, `error[2-203-0023 TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH]: Argument 1 for color.Color has type Integer, expected Byte for field red.`
           - p4: `color::Color[toByte(v), 0, 0, 255]` with v = 300 → builds, then the program stops at run time with `Error: 7-705-0010 Arithmetic overflow or numeric conversion outside the destination range.` (rc=255).
           "Rejected" is true in all three cases but the page does not tell the reader that the literal and the Integer-typed argument are both rejected at build time, while a computed value they convert themselves with toByte becomes a run-time error. That is exactly the clamp-vs-raise sharp edge the paragraph exists to draw, and the raise half is left vague.
SUGGESTED: "A color::Color[...] literal does not clamp: its fields are Byte. An out-of-range literal, or an Integer passed where a Byte field is expected, is a build error; a value you convert yourself with toByte raises at run time if it is outside 0..255. Use color::rgb or color::rgba when a component is computed."

### 2. The three Errors rows are never tied to the calls that raise them
UNIT:      man-page:color/overview
CLAIM:     "Text forms use color::fromHex and color::toHex/color::toHexAlpha."
VERDICT:   incomplete
EVIDENCE:  Rendered page: the Errors table lists ErrOutOfMemory, ErrInvalidFormat and ErrNotFound, but no sentence of the description names a condition for any of them. `grep -n 'errors: vec!\[' src/codegen/builtins/color/func_*.rs` shows exactly four raising members: func_from_hex.rs (ErrInvalidFormat, ErrOutOfMemory), func_from_name.rs and func_name_of.rs (ErrNotFound, ErrOutOfMemory), func_to_hex.rs / func_to_hex_alpha.rs (ErrOutOfMemory). Every other member has `errors: vec![]`. The description introduces fromHex and fromName by name and says nothing about either raising, so a reader of this page sees three error codes with no idea which of the 28 functions can produce them. §2 of `.ai/man-content.md` requires "every raisable error and the condition that raises it, consistent with the auto-derived Errors table".
SUGGESTED: Append to the text-forms paragraph: "color::fromHex raises ErrInvalidFormat for anything that is not a well-formed hex colour, and color::fromName and color::nameOf raise ErrNotFound for a name or colour that is not in the CSS table; nothing else in the package raises, and the constructors never do."

### 3. "every other package speaks" overstates the type's reach
UNIT:      man-page:color/overview
CLAIM:     "The color package defines color::Color, the colour value every other package speaks."
VERDICT:   misleading
EVIDENCE:  `grep -rl 'color::Color\|COLOR_TYPE' src/codegen/builtins | grep -v '/color/' | cut -d/ -f4 | sort | uniq -c` → astrings 3, canvas 16, money 1, term 6, vector 1 (plus the tests tree). `ls src/codegen/builtins` lists 28 user-facing packages; five of them take or return a colour, and `strings`, `fs`, `http`, `json` and the rest have no colour API at all. The intended meaning (there is one colour type, and any package that deals in colour uses it) is right; "every other package" is not.
SUGGESTED: "The color package defines color::Color, the one colour type in the language: every package that takes or returns a colour (canvas, term, astrings, …) uses it."

Everything else on the page checked out against the binary. One probe program (p1 in the scratch directory) printed: the literal `color::Color[10, 20, 30, 40]` renders `#0a141e28` and `WITH lit { alpha := 255 }` yields `#0a141eff` while the original is unchanged; `rgba(300, -20, 128, 255) = rgba(255, 0, 128, 255)` is TRUE and prints `#ff0080ff`; `toPacked(color::red)` is 4294901760 and `toPacked(rgba(1,2,3,4))` is 67174915, both matching 0xAARRGGBB; `toString` of a translucent colour is `#ff008007` where `toHex` gives `#ff0080`; `color::green` is `#008000` and equals `fromName("green")`; `fromName("lime")` is `#00ff00ff`; the `color::gray` constant is `#808080ff` and `color::gray(77)` is `#4d4d4dff`; `color::Color[0,0,0,0]` has alpha 0; and all sixteen constants compare equal to `fromName` of their own name (`all16=TRUE`). The project's generated project.json declared no dependencies, confirming `IMPORT color` needs no manifest entry. No banned memory vocabulary appears in the authored prose; the only hit, "Allocation failed.", is the derived ErrOutOfMemory message covered by carve-out 2.
