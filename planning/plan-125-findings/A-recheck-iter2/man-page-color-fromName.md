Verification complete. All three page examples compiled and produced exactly what the page implies (`#663399`; `green #008000` / `lime #00ff00`; `#008080` / `unknown (TRUE)`). The edge-case probe confirmed every description claim: case-folding, Unicode whitespace trimming (tab, newline, NBSP), 148-entry table, seven grey pairs, aqua/cyan and fuchsia/magenta duplicates, no `transparent`, opaque alpha, `ErrNotFound` 77050004 on every miss. No banned vocabulary or compiler-model sentences. Two gaps by omission remain.

### 1. Parameter description does not say what an empty name does
UNIT:      man-page:color/fromName
CLAIM:     "The CSS colour name. Case-insensitive; surrounding whitespace is ignored."
VERDICT:   incomplete
EVIDENCE:  `.ai/man-content.md` §2 requires each parameter description to state what happens at empty. Probe `/tmp/plan-125-scratch/A-recheck-iter2/man-page-color-fromName/p_probe/src/main.mfb` printed `empty -> ERR 77050004 notfound=TRUE msg=[unknown colour name: ]` and `spaces only -> ERR 77050004 ...`, and `internal space -> ERR 77050004 ... rebecca purple`. The lowering `__color_fromName` in `src/codegen/builtins/color/func_from_name.rs` does `strings::lower(strings::trim(name))` then `hasKey`, so empty, whitespace-only, and interior-spaced names all miss. The Description says "Anything not in the table raises ErrNotFound", but the parameter row itself leaves the empty case unanswered.
SUGGESTED: "The CSS colour name. Case-insensitive; leading and trailing whitespace is ignored, but whitespace inside the name is not (`"rebecca purple"` is unknown). An empty or all-whitespace name raises ErrNotFound."

### 2. No cross-reference to color::fromHex for hex strings
UNIT:      man-page:color/fromName
CLAIM:     "Anything not in the table raises ErrNotFound (77050004) — there is no best-effort or nearest-colour behaviour. color::nameOf is the reverse lookup."
VERDICT:   incomplete
EVIDENCE:  §2 requires a cross-reference to "the sibling a developer would otherwise reach for by mistake". The page's own motivating case is "a value read straight out of a config file or a form field", where a developer is as likely to meet `"#ff0000"` as `"red"`. Probe printed `hex -> ERR 77050004 notfound=TRUE msg=[unknown colour name: #ff0000]`. `src/codegen/builtins/color/func_from_hex.rs` is the sibling that parses that form, and the rendered See also lists only `color::nameOf` and `color::rgba`.
SUGGESTED: Append to that paragraph: "A hex string such as `"#ff0000"` is not a name and raises ErrNotFound too; parse those with color::fromHex." (Naming `color::fromHex` in the Description also adds it to the derived See also list.)
