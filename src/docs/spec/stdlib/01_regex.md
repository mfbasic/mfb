# Regex Engine

The `regex` package is a pure-MFBASIC regular-expression engine: a recursive-descent
parser builds an AST of `__regex_Node` values, and a continuation-passing backtracking
matcher walks that AST in leftmost-first (greedy-by-default) preference order. The
engine is hand-written MFBASIC; the Unicode general-category and Script tables are
pinned generated data the compiler reads natively. All matching is over Unicode scalar
values.[[src/codegen/builtins/regex/mod.rs:__regex_Node]]

The scalar-keyed Unicode properties are **data, not code**: `regex::genCat` and
`regex::scriptOf` are internal native members that binary-search a read-only run table
emitted with the program (`unicode_gencat_ranges.txt`, `unicode_script_ranges.txt`, both
pinned to Unicode 16.0.0), so a category or script query costs `O(log runs)` and adds
nothing to the compiled program beyond the table. One generated companion file remains
intra-file with the engine — `unicode_script_names.mfb`, the canonical spelling of each
script name, which is looked up once per pattern compile rather than per
scalar.[[src/codegen/builtins/regex/mod.rs:source_file]]

## Public Surface

Four built-in calls are recognized and rewritten to internal entry points during the
front end. Their signatures and return types are fixed (resolved by exact arg-type
match); `find`/`findAll` take an optional `start` that is padded to `0` during IR
lowering.[[src/codegen/builtins/regex/mod.rs:resolve_call]][[src/codegen/builtins/regex/mod.rs:default_argument_padding]]

| Call | Internal | Returns | Args |
|------|----------|---------|------|
| `regex.match` | `__regex_match` | `Boolean` | `value, pattern` |
| `regex.find` | `__regex_find` | `Integer` | `value, pattern[, start=0]` |
| `regex.findAll` | `__regex_findAll` | `List OF Integer` | `value, pattern[, start=0]` |
| `regex.replace` | `__regex_replace` | `String` | `value, pattern, replacement` |

`find` returns the scalar index of the first match at or after `start`, or `-1`.
`findAll` returns the start index of every non-overlapping match. `replace` substitutes
every match. There is no separate flags argument: flags are set inline in the pattern
(see [Flags](#flags)). Per-call API detail is owned by `mfb man regex`.[[src/codegen/builtins/regex/func_find.rs:__regex_find]]

Errors use `FAIL error(code, ...)`: `77050003` invalid pattern, `77050001` `start` index
out of range. There is no `ErrNotFound`; absence is reported as `-1` / empty / unchanged.[[src/codegen/builtins/regex/func_find.rs:__regex_find]]

## Scalar Model

A subject string is decomposed into a `__regex_Ctx`: a parallel list of single-scalar
`String`s (`text`) and their code points (`cps`), plus the length `n`. Positions
throughout the engine are scalar offsets into these lists, **not** byte offsets, so all
returned indices are scalar indices.[[src/codegen/builtins/regex/helper_make_ctx.rs:__regex_makeCtx]]

Code points are derived two ways. `__regex_chr` UTF-8-encodes an `Integer` to a scalar
string, clamping out-of-range and surrogate values; `__regex_scalarToCp` recovers a code
point via the shared UTF-32 encoder (`encoding::utf32Encode`), returning the first
scalar's code point. The pattern is decomposed into scalars the same way before parsing.[[src/codegen/builtins/regex/helper_scalar_to_cp.rs:__regex_scalarToCp]]

## Pattern Grammar

Recursive descent: `parseAlt → parseConcat → (parseAtom | parseParen) → parseQuantSuffix`.
A full parse must consume the entire pattern or it is invalid.[[src/codegen/builtins/regex/helper_compile.rs:__regex_compile]]

```
alt      := concat ("|" concat)*
concat   := (item)*                  ; stops at "|" or ")"
item     := atom quant?  |  group quant?  |  directive
atom     := "."  |  "^"  |  "$"  |  class  |  escape  |  literal
quant    := ("*" | "+" | "?" | "{" m "}" | "{" m "," "}" | "{" m "," n "}") "?"?
group    := "(" alt ")"                       ; capturing
          | "(?:" alt ")"                      ; non-capturing
          | "(?<name>" alt ")" | "(?P<name>" alt ")"   ; named capture
          | "(?" flags ")"                     ; inline flag directive
          | "(?" flags ":" alt ")"             ; scoped flags
class    := "[" "^"? class-item+ "]"
escape   := "\" (literal-escape | shorthand | property | anchor-escape)
```

A bare `*`, `+`, `?`, or a counted `{m,n}` with nothing to quantify is an error.
Counted braces are only treated as a quantifier when they form a valid count
(`__regex_isCountedAt`); otherwise `{` is a literal.[[src/codegen/builtins/regex/helper_parse_concat.rs:__regex_parseConcat]] `{m}` is exact, `{m,}` is
`m..∞` (`hi = -1`), `{m,n}` requires `m ≤ n`. Counts are clamped at 7 digits.[[src/codegen/builtins/regex/helper_parse_counted.rs:__regex_parseCounted]]

### Concatenation / Alternation Folding

`parseConcat` returns the single child directly when there is exactly one part (no
`__regex_Concat` wrapper); likewise `parseAlt` returns the single branch when there is no
`|`. So trivial patterns produce a bare atom node.[[src/codegen/builtins/regex/helper_parse_alt.rs:__regex_parseAlt]]

### Groups

Capturing groups allocate the next slot (`g + 1`) and record a `__regex_Group` node;
named groups additionally register `name → slot` in a `Map`, rejecting duplicates.
`(?:...)` is non-capturing. Group `0` is the whole match. **Lookarounds
(`(?=`, `(?!`, `(?<=`, `(?<!`) and backreferences are not supported** and are parse
errors.[[src/codegen/builtins/regex/helper_parse_paren.rs:__regex_parseParen]][[src/codegen/builtins/regex/helper_parse_named_group.rs:__regex_parseNamedGroup]]

## AST Node Set

`__regex_Node` is a `UNION` of eight node types.[[src/codegen/builtins/regex/mod.rs:__regex_Node]]

| Node | Fields | Meaning |
|------|--------|---------|
| `__regex_Lit` | `ch`, `fold`, `cp` | single literal scalar; `fold` = case-insensitive |
| `__regex_Any` | `dotall` | `.`; matches `\n` only when `dotall` |
| `__regex_Class` | `neg`, `fold`, `items` | character class; `items: List OF __regex_ClassItem` |
| `__regex_Anchor` | `kind`, `ml` | zero-width assertion (kind 1..6, see below) |
| `__regex_Concat` | `parts` | sequence of nodes |
| `__regex_Alt` | `opts` | ordered alternatives |
| `__regex_Repeat` | `child`, `lo`, `hi`, `greedy` | quantifier; `hi = -1` is unbounded |
| `__regex_Group` | `child`, `slot` | capturing group writing into capture `slot` |

Class items are a separate `UNION __regex_ClassItem`: `__regex_Range` (`lo`,`hi`),
`__regex_Single` (`ch`), `__regex_Short` (`kind` 1..6 for `\d\D\w\W\s\S`), and
`__regex_Prop` (`name`, `neg`) for `\p{...}` / POSIX.[[src/codegen/builtins/regex/mod.rs:__regex_ClassItem]]

Anchor `kind` encoding: `1` = `^`, `2` = `$` (both honor `ml`); `3` = `\A`, `4` = `\z`
(absolute); `5` = `\b`, `6` = `\B` (word boundary).[[src/codegen/builtins/regex/helper_anchor_match.rs:__regex_anchorMatch]]

## Explicit-Stack Backtracking Matcher

The matcher is `__regex_run(root, start, caps, ctx)`: one loop over a *task* — either
"match `node` at `pos`, then run `cont`" or "run `cont` at `pos`" — with an explicit
backtrack stack of pending choice points. Before bug-510 it was continuation-passing
recursion, one native frame per node visit, continuation step and repeat iteration; a
group repetition cost about ten frames, so a depth guard needed to keep the process from
overflowing its stack fired at sixty repetitions and `^(ab)*$` failed on a 200-character
input. The recursion is gone and the guard with it. A continuation (`__regex_Cont`, a
`UNION` of four) still encodes "what to match after this":[[src/codegen/builtins/regex/mod.rs:__regex_Cont]]

| Cont | Role |
|------|------|
| `__regex_ContDone` | terminal success; produce `__regex_Result[TRUE, pos, caps]` |
| `__regex_ContSeq` | walk `parts[idx..]` of a `__regex_Concat`, then `nxt` |
| `__regex_ContCap` | close capture `slot` (write end index `2*slot+1`), then `nxt` |
| `__regex_ContRep` | resume a `__regex_Repeat` after one iteration |

Consuming nodes (`Lit`, `Any`, `Class`) advance `pos` by one scalar and hand the task to
the continuation; anchors assert and hand it on at the same `pos`. A `Group` records the
start index (`2*slot`) immediately, then matches its child under a `ContCap` continuation
that records the end index when the child succeeds.[[src/codegen/builtins/regex/helper_run.rs:__regex_run]]

Backtracking is an explicit stack. Every point where the recursive engine "tried the
preferred branch first and fell through on failure" now pushes the *other* branch as a
choice point and runs the preferred one; a failure pops the most recent choice point and
resumes it. Four kinds exist: the next alternative of an `Alt`, "stop repeating and run
the continuation" (greedy), "one more iteration" (lazy), and "give back one scalar" (a
greedy repeat over a one-scalar child). A choice point is a `__regex_Choice` record —
kind, the `Alt` node or `Repeat` record it resumes, the continuation, position and
capture list to restore, three per-kind counters — whose `nxt` is the choice below it:
a linked list built the way the continuations are, never a growable `List OF`, because
`collections::get` of a recursive-type element aliases the list's storage and a growing
`append` frees it (bug-538). The number of pending choice points is capped by
`__REGEX_PENDING_LIMIT` (500 000), the matcher's memory bound.[[src/codegen/builtins/regex/mod.rs:__regex_Choice]]

### Preference Ordering (leftmost-first, greedy by default)

The order is exactly the recursive engine's, because the stack is LIFO and the preferred
branch always runs first:

- **Alternation**: `opts` are tried in source order; the first branch whose full
  continuation succeeds wins. This is leftmost-first (PCRE-style ordered choice), not
  leftmost-longest.[[src/codegen/builtins/regex/helper_run.rs:__regex_run]]
- **Greedy repeat**: when `greedy`, the engine first tries to consume **one more**
  iteration (through `ContRep`), having pushed "stop here and run the continuation" as
  the alternative — provided the minimum `lo` is already met. A greedy repeat over a
  one-scalar child consumes as far as it can in a loop and gives back one scalar at a
  time, longest first.[[src/codegen/builtins/regex/helper_run.rs:__regex_run]]
- **Lazy repeat**: when not greedy, the order inverts — the continuation runs first (if
  `lo` is satisfied) with "one more iteration" pushed as the alternative.[[src/codegen/builtins/regex/helper_run.rs:__regex_run]]
- **Empty-iteration guard**: `ContRep` compares the post-iteration position to the
  iteration start; if the child matched empty, it stops iterating and proceeds to `nxt`,
  preventing infinite loops on e.g. `(a*)*`.[[src/codegen/builtins/regex/helper_run.rs:__regex_run]]

### Cost Budgets

Every node visit is one step. A search may spend at most `__REGEX_STEP_BUDGET`
(2 000 000) steps, and — since bug-510 — a whole public call (`match`, `find`,
`findAll`, `replace`) may spend at most that plus one hundred steps per scalar of
subject; `__regex_makeCtx` arms the call-wide budget, so `findAll`/`replace` cannot spend
a fresh search budget on every match. Exceeding either, or the pending-choice cap,
raises `ErrInvalidFormat` ("pattern too complex for this
input").[[src/codegen/builtins/regex/helper_make_ctx.rs:__regex_makeCtx]]

### Search and Captures

`__regex_searchFrom` performs an unanchored search by trying `__regex_tryAt` at each
start position `from .. n` (so an empty match can occur at `n`). `tryAt` seeds the
capture list, records group-0 start, and matches the root under a `ContCap[0, ContDone]`
so group 0 is closed on success.[[src/codegen/builtins/regex/helper_search_from.rs:__regex_searchFrom]]

Captures are a flat `List OF Integer` of `2*(groups+1)` slots: for group `k`, slot
`2k` is the start scalar index and `2k+1` the end, both `-1` when unset. The whole-match
span is group 0.[[src/codegen/builtins/regex/helper_init_caps.rs:__regex_initCaps]]

`findAll` and `replace` iterate non-overlapping matches. After a non-empty match they
resume at the match end; after an empty match they record it once and advance by one
scalar, tracking `lastMatch` to avoid emitting an empty match adjacent to a prior
non-empty one.[[src/codegen/builtins/regex/func_find_all.rs:__regex_findAll]]

## Supported Syntax

### Anchors

`^` `$` (line-start/end), `\A` `\z` (absolute string start/end), `\b` `\B` (word
boundary / non-boundary). With multiline (`m`), `^`/`$` also match adjacent to a `\n`.
Word boundaries compare the word-ness of the scalars before and after `pos`.[[src/codegen/builtins/regex/helper_word_boundary.rs:__regex_wordBoundary]]

### Repeats / Quantifiers

`*` (`0..∞`), `+` (`1..∞`), `?` (`0..1`), and counted `{m}`, `{m,}`, `{m,n}`. A trailing
`?` toggles laziness (under `ungreedy`, the toggle is inverted).[[src/codegen/builtins/regex/helper_parse_quant_suffix.rs:__regex_parseQuantSuffix]]

### Escapes

Literal/control escapes: `\n \r \t \f \v \a \e \0`, `\xHH`, `\x{H..H}` (1–6 hex digits,
no surrogates), and any escaped ASCII punctuation as itself. Under verbose mode `\ ` is a
literal space. **Unknown letter escapes and backreferences are rejected.**[[src/codegen/builtins/regex/helper_parse_literal_escape.rs:__regex_parseLiteralEscape]]

### Character Classes

`[...]`, negated `[^...]`. Items: literal scalars, ranges `a-z` (low ≤ high required;
escapes are non-rangeable), shorthands `\d \D \w \W \s \S`, `\p{...}`/`\P{...}`, and
POSIX `[:name:]` / `[:^name:]`. A class must be non-empty; `&&` set intersection is a
parse error. Under `i`, class membership also tries the lower- and upper-cased scalar.[[src/codegen/builtins/regex/helper_parse_class.rs:__regex_parseClass]][[src/codegen/builtins/regex/helper_class_match.rs:__regex_classMatch]]

Shorthand semantics are Unicode-aware via general category: `\d` = `Nd`; `\w` =
letter/`Nl`/mark/`Nd`/`Pc`/ZWJ/ZWNJ; `\s` = `Z*` plus `\t..\r` and U+0085.[[src/codegen/builtins/regex/helper_shorthand_match.rs:__regex_shorthandMatch]]

POSIX names map to properties (`alpha`→`Alphabetic`, `digit`→`Nd`, `upper`→`Lu`,
`punct`→`P`, `cntrl`→`Cc`, …). Note `alnum`, `word`, `xdigit`, `blank`, `graph`, and
`print` map to special tokens (`posixAlnum`, etc.) that `__regex_propTest` does not
implement, so those POSIX classes effectively never match a scalar.[[src/codegen/builtins/regex/helper_posix_prop.rs:__regex_posixProp]]

### Unicode Properties `\p{...}`

`\p{...}`/`\P{...}` (and single-letter `\pL`/`\PL`) resolve a name through
`__regex_canonProp`. Accepted forms:[[src/codegen/builtins/regex/helper_canon_prop.rs:__regex_canonProp]]

- Top-level categories `L M N P S Z C` (and long aliases `letter`, `mark`, `number`,
  `punctuation`, `symbol`, `separator`, `other`) — prefix-tested against the scalar's
  general category.
- Two-letter general-category names (`Lu`, `Ll`, `Nd`, `Mn`, `Zs`, …) — exact match.
- Binary properties `White_Space` (alias `whitespace`) and `Alphabetic` (alias `alpha`).
- Any Unicode Script name (all 170 of Unicode 16.0.0 — `Latin`, `Greek`, `Han`,
  `Armenian`, `Thai`, `Devanagari`, …), matched against the scalar's Script
  property. `__regex_scriptTest(name, cp)` returns `regex::scriptOf(cp) = name`.[[src/codegen/builtins/regex/helper_script_test.rs:__regex_scriptTest]]
- `key=value` form with `gc`/`general_category` or `sc`/`script` keys.

Unknown property names are parse errors. The general-category lookup `regex::genCat`
maps each scalar to its two-letter category through contiguous runs over `0..0x10FFFF`,
generated from Unicode 16.0.0.[[src/codegen/string/unicode/unicode_gencat_ranges.txt]] The
Script lookup `regex::scriptOf` is the analogous run table, generated from the vendored
UCD `Scripts.txt` (Unicode 16.0.0).[[src/codegen/string/unicode/unicode_script_ranges.txt]]
Both are read from the emitted program's read-only data by a binary search over the
runs.[[src/codegen/string/unicode_props.rs:emit_unicode_range_lookup]]

## Flags

Flags live in `__regex_Flags` and are set inline only — there is no flags parameter.
`(?flags)` is a directive that mutates the flags for the rest of the enclosing
concatenation; `(?flags:...)` scopes flags to a sub-expression. A leading `-` clears the
following flags. Flags are baked into nodes at parse time (e.g. `Lit.fold = flags.ci`),
so they are static per node, not consulted at match time.[[src/codegen/builtins/regex/helper_parse_flag_spec.rs:__regex_parseFlagSpec]]

| Letter | Field | Effect |
|--------|-------|--------|
| `i` | `ci` | case-insensitive (case-fold literals; widen class membership) |
| `m` | `ml` | multiline: `^`/`$` match at line boundaries |
| `s` | `dotall` | `.` matches `\n` |
| `U` | `ungreedy` | swap greedy/lazy defaults |
| `x` | `verbose` | ignore unescaped pattern whitespace and `#`-to-EOL comments |

In verbose mode `parseConcat` skips unescaped whitespace and `#` comments while building
the AST.[[src/codegen/builtins/regex/helper_parse_concat.rs:__regex_parseConcat]]

## Replacement Expansion

`replace` expands the replacement string per match via `__regex_expand`: `$$` is a literal
`$`; `$N` / `${N}` insert capture group `N`; `$name` / `${name}` insert a named group.
Unknown or unmatched references expand to empty; a dangling `$` is emitted literally.
References resolve against the capture spans, slicing the original `value` with
`strings::mid`.[[src/codegen/builtins/regex/helper_expand.rs:__regex_expand]]

## See Also

- `mfb man regex` — per-function API reference.
- `./mfb spec unicode strings-model` — scalar/grapheme string model.
- `./mfb spec stdlib csv` — another pure-MFBASIC source package.
- `./mfb spec architecture frontend` — built-in package augmentation and call resolution.
- `./mfb spec architecture monomorphization` — how internal calls are mangled and lowered.
