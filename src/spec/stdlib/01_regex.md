# Regex Engine

The `regex` package is a pure-MFBASIC regular-expression engine: a recursive-descent
parser builds an AST of `__regex_Node` values, and a continuation-passing backtracking
matcher walks that AST in leftmost-first (greedy-by-default) preference order. The
Unicode general-category table is a generated companion file; the rest of the engine is
hand-written MFBASIC. All matching is over Unicode scalar values.[[src/builtins/regex_package.mfb:__regex_Node]]

The package ships as two physical files that compile as one source unit: the engine
(`regex_package.mfb`) and the generated general-category table (`regex_unicode.mfb`,
pinned to Unicode 16.0.0). They must be intra-file because `__regex_genCat` is a
file-local `FUNC` and package visibility is not valid in an executable.[[src/builtins/regex.rs:source_file]]

## Public Surface

Four built-in calls are recognized and rewritten to internal entry points during the
front end. Their signatures and return types are fixed (resolved by exact arg-type
match); `find`/`findAll` take an optional `start` that is padded to `0` during IR
lowering.[[src/builtins/regex.rs:resolve_call]][[src/builtins/regex.rs:default_argument_padding]]

| Call | Internal | Returns | Args |
|------|----------|---------|------|
| `regex.match` | `__regex_match` | `Boolean` | `value, pattern` |
| `regex.find` | `__regex_find` | `Integer` | `value, pattern[, start=0]` |
| `regex.findAll` | `__regex_findAll` | `List OF Integer` | `value, pattern[, start=0]` |
| `regex.replace` | `__regex_replace` | `String` | `value, pattern, replacement` |

`find` returns the scalar index of the first match at or after `start`, or `-1`.
`findAll` returns the start index of every non-overlapping match. `replace` substitutes
every match. There is no separate flags argument: flags are set inline in the pattern
(see [Flags](#flags)). Per-call API detail is owned by `mfb man regex`.[[src/builtins/regex_package.mfb:__regex_find]]

Errors use `FAIL error(code, ...)`: `77050003` invalid pattern, `77050001` `start` index
out of range. There is no `ErrNotFound`; absence is reported as `-1` / empty / unchanged.[[src/builtins/regex_package.mfb:__regex_find]]

## Scalar Model

A subject string is decomposed into a `__regex_Ctx`: a parallel list of single-scalar
`String`s (`text`) and their code points (`cps`), plus the length `n`. Positions
throughout the engine are scalar offsets into these lists, **not** byte offsets, so all
returned indices are scalar indices.[[src/builtins/regex_package.mfb:__regex_makeCtx]]

Code points are derived two ways. `__regex_chr` UTF-8-encodes an `Integer` to a scalar
string, clamping out-of-range and surrogate values; `__regex_scalarToCp` recovers a code
point by binary search over `__regex_chr` (valid because UTF-8 byte order equals scalar
order). The pattern is decomposed into scalars the same way before parsing.[[src/builtins/regex_package.mfb:__regex_scalarToCp]]

## Pattern Grammar

Recursive descent: `parseAlt → parseConcat → (parseAtom | parseParen) → parseQuantSuffix`.
A full parse must consume the entire pattern or it is invalid.[[src/builtins/regex_package.mfb:__regex_compile]]

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
(`__regex_isCountedAt`); otherwise `{` is a literal.[[src/builtins/regex_package.mfb:__regex_parseConcat]] `{m}` is exact, `{m,}` is
`m..∞` (`hi = -1`), `{m,n}` requires `m ≤ n`. Counts are clamped at 7 digits.[[src/builtins/regex_package.mfb:__regex_parseCounted]]

### Concatenation / Alternation Folding

`parseConcat` returns the single child directly when there is exactly one part (no
`__regex_Concat` wrapper); likewise `parseAlt` returns the single branch when there is no
`|`. So trivial patterns produce a bare atom node.[[src/builtins/regex_package.mfb:__regex_parseAlt]]

### Groups

Capturing groups allocate the next slot (`g + 1`) and record a `__regex_Group` node;
named groups additionally register `name → slot` in a `Map`, rejecting duplicates.
`(?:...)` is non-capturing. Group `0` is the whole match. **Lookarounds
(`(?=`, `(?!`, `(?<=`, `(?<!`) and backreferences are not supported** and are parse
errors.[[src/builtins/regex_package.mfb:__regex_parseParen]][[src/builtins/regex_package.mfb:__regex_parseNamedGroup]]

## AST Node Set

`__regex_Node` is a `UNION` of eight node types.[[src/builtins/regex_package.mfb:__regex_Node]]

| Node | Fields | Meaning |
|------|--------|---------|
| `__regex_Lit` | `ch`, `fold` | single literal scalar; `fold` = case-insensitive |
| `__regex_Any` | `dotall` | `.`; matches `\n` only when `dotall` |
| `__regex_Class` | `neg`, `fold`, `items` | character class; `items: List OF __regex_ClassItem` |
| `__regex_Anchor` | `kind`, `ml` | zero-width assertion (kind 1..6, see below) |
| `__regex_Concat` | `parts` | sequence of nodes |
| `__regex_Alt` | `opts` | ordered alternatives |
| `__regex_Repeat` | `child`, `lo`, `hi`, `greedy` | quantifier; `hi = -1` is unbounded |
| `__regex_Group` | `child`, `slot` | capturing group writing into capture `slot` |

Class items are a separate `UNION __regex_ClassItem`: `__regex_Range` (`lo`,`hi`),
`__regex_Single` (`ch`), `__regex_Short` (`kind` 1..6 for `\d\D\w\W\s\S`), and
`__regex_Prop` (`name`, `neg`) for `\p{...}` / POSIX.[[src/builtins/regex_package.mfb:__regex_ClassItem]]

Anchor `kind` encoding: `1` = `^`, `2` = `$` (both honor `ml`); `3` = `\A`, `4` = `\z`
(absolute); `5` = `\b`, `6` = `\B` (word boundary).[[src/builtins/regex_package.mfb:__regex_anchorMatch]]

## CPS Backtracking Matcher

The matcher is continuation-passing. `__regex_matchNode(node, pos, caps, cont, ctx)`
attempts `node` at `pos`; on success it invokes the continuation `cont` rather than
returning, threading the new position and capture list forward. A continuation
(`__regex_Cont`, a `UNION` of four) encodes "what to match after this":[[src/builtins/regex_package.mfb:__regex_Cont]]

| Cont | Role |
|------|------|
| `__regex_ContDone` | terminal success; produce `__regex_Result[TRUE, pos, caps]` |
| `__regex_ContSeq` | walk `parts[idx..]` of a `__regex_Concat`, then `nxt` |
| `__regex_ContCap` | close capture `slot` (write end index `2*slot+1`), then `nxt` |
| `__regex_ContRep` | resume a `__regex_Repeat` after one iteration |

Consuming nodes (`Lit`, `Any`, `Class`) advance `pos` by one scalar and call the
continuation; anchors assert and call the continuation at the same `pos`. A `Group`
records the start index (`2*slot`) immediately, then matches its child under a
`ContCap` continuation that records the end index when the child succeeds.[[src/builtins/regex_package.mfb:__regex_matchNode]]

Backtracking is implemented by ordinary return values and sequential trial: every
alternative/iteration choice tries its preferred branch first and, on failure (an
`ok = FALSE` result), falls through to the next. There is no explicit backtrack stack;
the call stack and the continuation chain carry the state.[[src/builtins/regex_package.mfb:__regex_matchAlt]]

### Preference Ordering (leftmost-first, greedy by default)

- **Alternation**: `__regex_matchAlt` tries `opts` in source order, returning the first
  branch whose full continuation succeeds. This is leftmost-first (PCRE-style ordered
  choice), not leftmost-longest.[[src/builtins/regex_package.mfb:__regex_matchAlt]]
- **Greedy repeat**: when `greedy`, `__regex_matchRep` first tries to consume **one more**
  iteration (recursing through `ContRep`), and only if that whole path fails does it try
  the continuation at the current position — provided the minimum `lo` is already met.[[src/builtins/regex_package.mfb:__regex_matchRep]]
- **Lazy repeat**: when not greedy, the order inverts — try the continuation first
  (if `lo` is satisfied), then try one more iteration.[[src/builtins/regex_package.mfb:__regex_matchRep]]
- **Empty-iteration guard**: `ContRep` compares the post-iteration position to the
  iteration start; if the child matched empty, it stops iterating and proceeds to `nxt`,
  preventing infinite loops on e.g. `(a*)*`.[[src/builtins/regex_package.mfb:__regex_matchCont]]

### Search and Captures

`__regex_searchFrom` performs an unanchored search by trying `__regex_tryAt` at each
start position `from .. n` (so an empty match can occur at `n`). `tryAt` seeds the
capture list, records group-0 start, and matches the root under a `ContCap[0, ContDone]`
so group 0 is closed on success.[[src/builtins/regex_package.mfb:__regex_searchFrom]]

Captures are a flat `List OF Integer` of `2*(groups+1)` slots: for group `k`, slot
`2k` is the start scalar index and `2k+1` the end, both `-1` when unset. The whole-match
span is group 0.[[src/builtins/regex_package.mfb:__regex_initCaps]]

`findAll` and `replace` iterate non-overlapping matches. After a non-empty match they
resume at the match end; after an empty match they record it once and advance by one
scalar, tracking `lastMatch` to avoid emitting an empty match adjacent to a prior
non-empty one.[[src/builtins/regex_package.mfb:__regex_findAll]]

## Supported Syntax

### Anchors

`^` `$` (line-start/end), `\A` `\z` (absolute string start/end), `\b` `\B` (word
boundary / non-boundary). With multiline (`m`), `^`/`$` also match adjacent to a `\n`.
Word boundaries compare the word-ness of the scalars before and after `pos`.[[src/builtins/regex_package.mfb:__regex_wordBoundary]]

### Repeats / Quantifiers

`*` (`0..∞`), `+` (`1..∞`), `?` (`0..1`), and counted `{m}`, `{m,}`, `{m,n}`. A trailing
`?` toggles laziness (under `ungreedy`, the toggle is inverted).[[src/builtins/regex_package.mfb:__regex_parseQuantSuffix]]

### Escapes

Literal/control escapes: `\n \r \t \f \v \a \e \0`, `\xHH`, `\x{H..H}` (1–6 hex digits,
no surrogates), and any escaped ASCII punctuation as itself. Under verbose mode `\ ` is a
literal space. **Unknown letter escapes and backreferences are rejected.**[[src/builtins/regex_package.mfb:__regex_parseLiteralEscape]]

### Character Classes

`[...]`, negated `[^...]`. Items: literal scalars, ranges `a-z` (low ≤ high required;
escapes are non-rangeable), shorthands `\d \D \w \W \s \S`, `\p{...}`/`\P{...}`, and
POSIX `[:name:]` / `[:^name:]`. A class must be non-empty; `&&` set intersection is a
parse error. Under `i`, class membership also tries the lower- and upper-cased scalar.[[src/builtins/regex_package.mfb:__regex_parseClass]][[src/builtins/regex_package.mfb:__regex_classMatch]]

Shorthand semantics are Unicode-aware via general category: `\d` = `Nd`; `\w` =
letter/`Nl`/mark/`Nd`/`Pc`/ZWJ/ZWNJ; `\s` = `Z*` plus `\t..\r` and U+0085.[[src/builtins/regex_package.mfb:__regex_shorthandMatch]]

POSIX names map to properties (`alpha`→`Alphabetic`, `digit`→`Nd`, `upper`→`Lu`,
`punct`→`P`, `cntrl`→`Cc`, …). Note `alnum`, `word`, `xdigit`, `blank`, `graph`, and
`print` map to special tokens (`posixAlnum`, etc.) that `__regex_propTest` does not
implement, so those POSIX classes effectively never match a scalar.[[src/builtins/regex_package.mfb:__regex_posixProp]]

### Unicode Properties `\p{...}`

`\p{...}`/`\P{...}` (and single-letter `\pL`/`\PL`) resolve a name through
`__regex_canonProp`. Accepted forms:[[src/builtins/regex_package.mfb:__regex_canonProp]]

- Top-level categories `L M N P S Z C` (and long aliases `letter`, `mark`, `number`,
  `punctuation`, `symbol`, `separator`, `other`) — prefix-tested against the scalar's
  general category.
- Two-letter general-category names (`Lu`, `Ll`, `Nd`, `Mn`, `Zs`, …) — exact match.
- Binary properties `White_Space` (alias `whitespace`) and `Alphabetic` (alias `alpha`).
- Script names `Latin Greek Cyrillic Han Hiragana Katakana Hangul Arabic Hebrew Common`
  (matched by hard-coded code-point ranges in `__regex_scriptTest`, not the table).[[src/builtins/regex_package.mfb:__regex_scriptTest]]
- `key=value` form with `gc`/`general_category` or `sc`/`script` keys.

Unknown property names are parse errors. The general-category lookup `__regex_genCat`
maps each scalar to its two-letter category via contiguous ranges over `0..0x10FFFF`,
generated from Unicode 16.0.0.[[src/builtins/regex_unicode.mfb:__regex_genCat]]

## Flags

Flags live in `__regex_Flags` and are set inline only — there is no flags parameter.
`(?flags)` is a directive that mutates the flags for the rest of the enclosing
concatenation; `(?flags:...)` scopes flags to a sub-expression. A leading `-` clears the
following flags. Flags are baked into nodes at parse time (e.g. `Lit.fold = flags.ci`),
so they are static per node, not consulted at match time.[[src/builtins/regex_package.mfb:__regex_parseFlagSpec]]

| Letter | Field | Effect |
|--------|-------|--------|
| `i` | `ci` | case-insensitive (case-fold literals; widen class membership) |
| `m` | `ml` | multiline: `^`/`$` match at line boundaries |
| `s` | `dotall` | `.` matches `\n` |
| `U` | `ungreedy` | swap greedy/lazy defaults |
| `x` | `verbose` | ignore unescaped pattern whitespace and `#`-to-EOL comments |

In verbose mode `parseConcat` skips unescaped whitespace and `#` comments while building
the AST.[[src/builtins/regex_package.mfb:__regex_parseConcat]]

## Replacement Expansion

`replace` expands the replacement string per match via `__regex_expand`: `$$` is a literal
`$`; `$N` / `${N}` insert capture group `N`; `$name` / `${name}` insert a named group.
Unknown or unmatched references expand to empty; a dangling `$` is emitted literally.
References resolve against the capture spans, slicing the original `value` with
`strings::mid`.[[src/builtins/regex_package.mfb:__regex_expand]]

## See Also

- `mfb man regex` — per-function API reference.
- `./mfb spec unicode strings-model` — scalar/grapheme string model.
- `./mfb spec stdlib csv` — another pure-MFBASIC source package.
- `./mfb spec architecture frontend` — built-in package augmentation and call resolution.
- `./mfb spec architecture monomorphization` — how internal calls are mangled and lowered.
