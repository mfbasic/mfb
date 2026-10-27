### 1. Overview never mentions the reviver form of json::parse
UNIT:      man-pkg:json
PAGE:      package-wide (overview)
CATEGORY:  overview-mismatch
CLAIM:     "json::parse turns JSON text — a UTF-8 String, or an open fs::File read from its current position to its end — holding one complete JSON document into a json::Json value"
VERDICT:   missing
EVIDENCE:  `mfb man json --all` > scratch all.txt; `grep -n reviver all.txt` shows the word only inside the PARSE page (overloads 3 and 4, parameter row, six paragraphs of description, two examples). The overview describes parse as a one-argument conversion and never says a transform hook exists. A developer scanning the overview for "can I convert values while reading" would conclude no and write a second traversal.
SUGGESTED: Add one clause to the overview's parse sentence: "...into a json::Json value, optionally passing a reviver that is called for every parsed value, innermost first, so a single pass can convert or normalize members (see json::parse)."

### 2. getOr's description names its third parameter `defaultValue`; the declaration and table name it `default`
UNIT:      man-pkg:json
PAGE:      json::getOr
CATEGORY:  consistency
CLAIM:     "returns defaultValue instead of failing whenever traversal cannot continue" ... "path may also be passed by the name key, and defaultValue under the names default or fallback."
VERDICT:   inconsistent
EVIDENCE:  `src/codegen/builtins/json/func_get_or.rs` declares `name: "default", aliases: &["defaultValue", "fallback"]`; the rendered Declaration is `json::getOr(value, path, default AS json::Json)` and the Parameters row reads `default | defaultValue, fallback`. The description uses `defaultValue` seven times as if it were the canonical name and then says it can be passed "under the names default or fallback", which inverts the table. The page's own last example uses `default :=`. Probe /tmp/plan-125-scratch/B-recheck/man-pkg-json/proj_names confirms all three spellings are accepted (`defaultValue :=` printed 9, `fallback :=` printed 8), so this is purely a naming inconsistency, not a behavior bug. The sibling page json::get and the overview both say "default value" in prose.
SUGGESTED: Use `default` throughout the description (as the declaration does), and end with "default may also be passed under the names defaultValue or fallback."

### 3. No page shows how to build an object or unwrap a scalar
UNIT:      man-pkg:json
PAGE:      package-wide (overview, stringify, types)
CATEGORY:  coverage
CLAIM:     Overview intro: "Parse, build, serialize, and read JSON values as a json::Json tree"
VERDICT:   missing
EVIDENCE:  `grep -n "JsonObj\[" all.txt` returns nothing: no example constructs a json::JsonObj. The only building example is stringify's JsonArr one. Constructing an object needs the typed map literal `json::JsonObj[Map OF String TO json::Json { "x" := json::JsonNum[1.0] }]` (verified in proj_names, prints `{"x":1}`); a JavaScript-shaped `{"x": ...}` literal fails to parse (first probe attempt, MFB_PARSE_UNEXPECTED_STATEMENT). Likewise the only place a developer sees how to get a String or Float *out* of a json::Json is incidental, inside the parse reviver example (`CASE json::JsonNum(n)` ... `n.value`). json::get and json::getOr return json::Json, so "read" never reaches a plain value on any page. The types page lists the field names (value, items, fields) but shows no MATCH.
SUGGESTED: Add to the overview, after the union paragraph, a short worked snippet: build `json::JsonObj[Map OF String TO json::Json { "name" := json::JsonStr["Ada"] }]`, and unwrap with `MATCH json::get(doc, ["name"]) CASE json::JsonStr(s) ... s.value`. Add the JsonObj build as a stringify example beside the existing JsonArr one.

### 4. Nesting limit is documented for parse only; stringify's behaviour on a deeper hand-built tree is unstated
UNIT:      man-pkg:json
PAGE:      json::stringify (and overview)
CATEGORY:  coverage
CLAIM:     parse page: "a document nested beyond a fixed structural depth (256 levels of arrays and objects combined) is rejected with errorCode::ErrDepthExceeded rather than being allowed to exhaust the native stack" — nothing on stringify or the overview says whether the writer has a limit.
VERDICT:   missing
EVIDENCE:  `grep -n "depth\|Depth" src/codegen/builtins/json/func_stringify.rs src/codegen/builtins/json/helper_stringify_indent.rs` shows no use of `__JSON_DEPTH_LIMIT`; the helper comment in helper_depth_limit.rs says the cap is for parse only. Probe proj_deep (code-built nested JsonArr): 300 levels and 5000 levels compact stringify fine (601 and 10001 bytes); 5000 levels indented, and 50000 levels compact, trap with 77010002 ErrStackOverflow. So a tree a program can legally build (parse would never produce one that deep) serializes until the thread stack runs out, then fails with a language-wide error the page does not mention. The declared-errors test deliberately excludes ErrStackOverflow from every member's table (`src/codegen/builtins/tests/general_declared_errors.rs`, comment at the `symbol == root` check), so the Errors table is not wrong; the prose is silent.
SUGGESTED: One sentence on the stringify page, next to the recursion remark: "There is no fixed depth cap on output; a tree nested deeper than json::parse's 256-level limit can only be built in code, and serializing one that exceeds the thread's stack raises ErrStackOverflow like any deep recursion."

Checked and found consistent: the overview's claim that a non-finite JsonNum cannot be built (probe proj_nan: `json::JsonNum[z / z]` traps 77050013 ErrFloatNaN at construction, never reaching stringify); the "written in MFBASIC source, no manifest dependency" claim (helpers under `src/codegen/builtins/json/helper_*.rs` are MFBASIC bodies; `IMPORT json` compiled in a fresh `mfb init` project with no manifest edit); File-parse semantics and error set agree between overview, parse table, and parse description; get/getOr index-spelling rules match each other and the overview; the function table covers every public member (`sciParts` is `internal_only: true` and correctly absent).
