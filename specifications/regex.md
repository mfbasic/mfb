# MFBASIC Regex Dialect

Last updated: 2026-06-24

This document is the **normative, self-contained definition** of the regular
expression dialect used by the `regex` package
(`specifications/standard_package.md` §6). It defines the supported syntax and
matching semantics directly — with prose, grammar, and worked examples — and is
the single source of truth for regex behavior across every target and every
build path (native executable and Binary Representation package).

This dialect is **modeled on the Rust `regex` crate** (see the informal lineage
note in §16), but its behavior is fixed by *this document*, not by any external
engine, library, or host `regcomp()`. Where this spec and any other engine
disagree, **this spec wins**. No feature is defined "by reference" to Rust,
PCRE, POSIX, or a platform library; an external engine may be used only as an
implementation detail or development aid that must reproduce what is written
here.

See also: `specifications/standard_package.md` (§3.1 scalar-index model, §6 the
`regex` functions) and `specifications/error_codes.md` (canonical error codes).

---

## 1. The Functions

The `regex` package exports these functions
(`standard_package.md` §6):

```basic
FUNC match(value AS String, pattern AS String) AS Boolean
FUNC find(value AS String, pattern AS String, start AS Integer = 0) AS Integer
FUNC findAll(value AS String, pattern AS String, start AS Integer = 0) AS List OF Integer
FUNC replace(value AS String, pattern AS String, replacement AS String) AS String
```

- `match` returns `TRUE` when `pattern` matches anywhere in `value`.
- `find` returns the zero-based Unicode scalar index where the first match at or
  after `start` begins.
- `findAll` returns the zero-based Unicode scalar start index of *every*
  non-overlapping match at or after `start`, left to right.
- `replace` returns `value` with every non-overlapping match replaced by the
  expansion of `replacement`.

`pattern` and `replacement` are ordinary runtime `String` values; they are not
required to be compile-time constants. Their exact behavior is defined in §11
(matching), §12 (the functions), and §13 (replacement).

---

## 2. Matching Domain (Unicode Scalars)

Matching operates over **sequences of Unicode scalar values** (Unicode code
points excluding surrogates), consistent with `standard_package.md` §3.1.

- Strings are stored as UTF-8, but every user-visible position and index in this
  dialect is a **zero-based Unicode scalar index**, never a byte offset and
  never a grapheme-cluster index.
- A string of `n` scalars has `n + 1` positions, `0 .. n` inclusive. Position
  `0` is before the first scalar; position `n` is after the last scalar.
- Pattern elements consume whole scalars. A pattern can never match a partial
  scalar or a partial UTF-8 sequence.
- All length, range, and index arithmetic in this document is in scalars.

The dialect is **Unicode-aware by default**: `.`, the class shorthands
(`\d`, `\w`, `\s`), case-insensitive matching, and Unicode property classes all
use the Unicode database.

### 2.1 Pinned Unicode Version

All Unicode-dependent behavior (general categories, scripts, properties, case
folding, the shorthand class definitions in §8) is resolved against a **single
Unicode version pinned by the implementation**. That version:

- is a recorded build constant,
- **must be identical across every target** (`macos-aarch64`,
  `linux-aarch64` glibc and musl, and any future target), and
- must be identical between the native code path and the Binary Representation
  package path.

The pinned version is the *only* source of Unicode data; the host libc/OS
Unicode tables are never consulted. (v1 targets a Unicode 15-series database;
the normative requirement is "one pinned version, identical everywhere," not a
specific number.)

---

## 3. Grammar

A pattern is a `Regex` over the scalar alphabet. The following EBNF is
normative for *syntax*; semantics are defined in the sections that follow.
Literal terminals are written in `'quotes'`. `SCALAR` is any Unicode scalar.

```ebnf
Regex        = Alternation ;
Alternation  = Concat , { '|' , Concat } ;
Concat       = { Quantified } ;
Quantified   = Atom , [ Quantifier ] ;
Quantifier   = ( '*' | '+' | '?' | Counted ) , [ '?' ] ;   (* trailing '?' = lazy *)
Counted      = '{' , Int , '}'
             | '{' , Int , ',' , '}'
             | '{' , Int , ',' , Int , '}' ;
Atom         = Group
             | Class
             | Anchor
             | Dot
             | Escape
             | Literal ;
Group        = '(' , GroupHead , Regex , ')' ;
GroupHead    = ''                                  (* capturing, auto-numbered *)
             | '?:'                                 (* non-capturing *)
             | '?<' , Name , '>'                    (* named capturing *)
             | '?P<' , Name , '>'                   (* named capturing, alt syntax *)
             | FlagSpec , ':'                       (* non-capturing, scoped flags *)
             | FlagSpec ;                            (* flag-set directive, no body *)
FlagSpec     = '?' , { Flag } , [ '-' , { Flag } ] ;
Flag         = 'i' | 'm' | 's' | 'U' | 'x' ;
Name         = NameStart , { NameCont } ;
NameStart    = 'A'..'Z' | 'a'..'z' | '_' ;
NameCont     = NameStart | '0'..'9' ;
Anchor       = '^' | '$' | '\A' | '\z' | '\b' | '\B' ;
Dot          = '.' ;
Class        = '[' , [ '^' ] , ClassItems , ']' ;
Int          = Digit , { Digit } ;
Digit        = '0'..'9' ;
Literal      = SCALAR ;                              (* any scalar not a metachar *)
```

The metacharacters that are not ordinary literals outside a class are:
`\ . * + ? ( ) [ ] { } ^ $ |`. To match one literally, escape it with `\`
(§7) or, where unambiguous, place it in a class. Inside a class the
metacharacter set is different (§9).

A `{` that does not begin a well-formed `Counted` quantifier is treated as a
literal `{` (and likewise a stray `}` is a literal `}`). All other syntactic
violations are invalid patterns (§14).

---

## 4. Match Selection (Leftmost-First / Preference Order)

This dialect uses **leftmost-first** match semantics (also called
"preference-order" matching). For a given pattern and input there is at most one
*reported* match for any search:

1. **Leftmost.** The reported match is the one whose start position is the
   smallest position at which *any* match exists. A match that starts earlier
   always wins over one that starts later, even if the later one is longer.

2. **Preference order at that position.** Among the matches that start at the
   chosen position, exactly one is selected by following the pattern's
   preference order:
   - **Concatenation** is matched left to right.
   - **Alternation** `A|B|C` prefers the earliest alternative that allows the
     overall match to succeed (`A` over `B` over `C`).
   - A **greedy** quantifier prefers to match as many repetitions as possible
     while still allowing the overall match to succeed.
   - A **lazy** quantifier (suffixed `?`, §6) prefers to match as few
     repetitions as possible while still allowing the overall match to succeed.

This is the single match a backtracking engine with these preferences would find
first. Because the dialect has no backreferences and no look-around (§15), this
selection is also realizable by a linear-time automaton; the rule above defines
*which* match is reported, independent of the implementation strategy.

`find` reports the **start** scalar index of this match (§12.2). `match` only
reports whether such a match exists (§12.1). The match's end position governs
`replace` (§12.3, §13).

---

## 5. Greedy Quantifiers

`*`, `+`, `?`, and the counted forms are **greedy by default**: they match as
many repetitions as the preference order in §4 allows.

| Form     | Meaning                                              |
|----------|------------------------------------------------------|
| `X*`     | zero or more `X`, greedy                              |
| `X+`     | one or more `X`, greedy                               |
| `X?`     | zero or one `X`, greedy (prefers one)                |
| `X{m}`   | exactly `m` repetitions                              |
| `X{m,}`  | `m` or more repetitions, greedy                      |
| `X{m,n}` | between `m` and `n` repetitions (inclusive), greedy  |

Counted bounds: `m` and `n` are non-negative decimal integers. `X{m,n}` requires
`m <= n`, else the pattern is invalid (§14). `X{0}` is permitted and matches the
empty string (it makes `X` contribute nothing). A quantifier applies to the
single preceding atom (a literal, a class, a group, an escape, the dot, etc.).

A quantifier with no preceding atom (e.g. a leading `*`, or `a|*`) is invalid.
Stacking quantifiers directly (e.g. `a**`, `a+?*`) is invalid; use a group:
`(?:a+)*`. (`a+?` is *not* stacking — the trailing `?` is the lazy marker, §6.)

---

## 6. Lazy Quantifiers

A quantifier immediately followed by `?` is **lazy**: it matches as *few*
repetitions as the preference order in §4 allows.

| Form      | Meaning                                |
|-----------|----------------------------------------|
| `X*?`     | zero or more `X`, lazy                 |
| `X+?`     | one or more `X`, lazy                  |
| `X??`     | zero or one `X`, lazy (prefers zero)   |
| `X{m,}?`  | `m` or more, lazy                      |
| `X{m,n}?` | between `m` and `n`, lazy              |

Laziness changes only the preference order, never *which positions* can match —
it cannot make a match start earlier or later than the leftmost rule in §4
requires. The `U` flag (§10) swaps the default: under `(?U)`, bare quantifiers
are lazy and the `?` suffix makes them greedy.

---

## 7. Escapes (Closed Set)

Outside a character class, a backslash introduces exactly one of the escapes
below. **Any backslash sequence not listed here is an invalid pattern** (§14).
There is no "unknown escape means the literal letter" fallback for ASCII letters
and digits.

### 7.1 Class-shorthand escapes

`\d \D \w \W \s \S` — defined in §8.

### 7.2 Anchor escapes

`\A \z \b \B` — defined in §11.4.

### 7.3 Literal control escapes

| Escape | Scalar    | Name            |
|--------|-----------|-----------------|
| `\n`   | U+000A    | line feed       |
| `\r`   | U+000D    | carriage return |
| `\t`   | U+0009    | tab             |
| `\f`   | U+000C    | form feed       |
| `\v`   | U+000B    | vertical tab    |
| `\a`   | U+0007    | bell            |
| `\e`   | U+001B    | escape          |
| `\0`   | U+0000    | null            |

### 7.4 Numeric scalar escapes

| Escape      | Meaning                                                       |
|-------------|--------------------------------------------------------------|
| `\xHH`      | scalar with the 2-hex-digit value `HH`                       |
| `\x{H...H}` | scalar with the 1–6-hex-digit value in braces                |

The resulting value must be a valid Unicode scalar (`0 .. 0x10FFFF`, excluding
the surrogate range `0xD800 .. 0xDFFF`); otherwise the pattern is invalid. Hex
digits are `0-9 A-F a-f`. `\xHH` requires exactly two hex digits. (There is no
`\uHHHH` form; use `\x{...}`.)

### 7.5 Unicode property escapes

| Escape       | Meaning                                                       |
|--------------|---------------------------------------------------------------|
| `\p{Prop}`   | any scalar with Unicode property/category/script `Prop`       |
| `\P{Prop}`   | any scalar **without** `Prop`                                  |
| `\pX`        | single-letter general category `X` (e.g. `\pL`)               |
| `\PX`        | negation of `\pX`                                              |

`Prop` is resolved against the pinned Unicode version (§2.1) and may be a general
category (short or long name, e.g. `L`, `Lu`, `Letter`), a script name
(e.g. `Greek`, `Han`), or the `Name=Value` form (e.g. `\p{Script=Greek}`,
`\p{gc=Nd}`). An unrecognized property name is an invalid pattern (§14).

### 7.6 Escaped punctuation

A backslash followed by an **ASCII punctuation** scalar (any ASCII scalar that is
not a letter, not a digit, and not whitespace — e.g. `\.`, `\*`, `\(`, `\\`,
`\|`, `\{`, `\/`) matches that punctuation scalar literally. This is how
metacharacters are written as literals.

A backslash followed by an ASCII letter or digit not covered by §7.1–§7.5, or by
ASCII whitespace, is invalid (except inside `x` mode, where `\ ` denotes a
literal space — see §10).

---

## 8. Class Shorthands (Pinned Unicode Meaning)

The shorthands are **Unicode-aware** and resolved against the pinned Unicode
version (§2.1). Their meaning is fixed here and does not vary by target or host
locale.

| Shorthand | Matches                                                            |
|-----------|---------------------------------------------------------------------|
| `\d`      | any scalar in general category **Nd** (Decimal_Number)              |
| `\D`      | any scalar **not** matched by `\d`                                  |
| `\w`      | any scalar with property **Alphabetic**, or in category **M** (Mark), or **Nd** (Decimal_Number), or **Pc** (Connector_Punctuation), or with property **Join_Control** |
| `\W`      | any scalar **not** matched by `\w`                                  |
| `\s`      | any scalar with property **White_Space**                           |
| `\S`      | any scalar **not** matched by `\s`                                  |

The negated forms are exact set complements over the whole scalar range
(`0 .. 0x10FFFF`, excluding surrogates). These definitions hold both standalone
(`\d`) and inside a class (`[\d.]`).

---

## 9. Character Classes

A class `[...]` matches a single scalar drawn from a defined set.

- `[abc]` matches `a`, `b`, or `c`.
- `[a-z]` matches any scalar whose value is in the inclusive range `a .. z`. The
  low end must be `<= ` the high end, else the pattern is invalid.
- `[^...]` is a **negated** class: it matches any single scalar **not** in the
  set. Negation is over the whole scalar range.
- Shorthands and property escapes are allowed inside a class:
  `[\d\s]`, `[\p{L}_]`, `[^\w]`.
- POSIX bracket classes are allowed inside a class and name the set indicated:
  `[:alpha:]`, `[:digit:]`, `[:alnum:]`, `[:space:]`, `[:upper:]`, `[:lower:]`,
  `[:xdigit:]`, `[:punct:]`, `[:blank:]`, `[:cntrl:]`, `[:graph:]`, `[:print:]`,
  `[:word:]`, and their negated `[:^name:]` forms. They are written only inside a
  class, e.g. `[[:alpha:]_]`. Their member sets are resolved against the pinned
  Unicode version.

### 9.1 Literals and escapes inside a class

Inside `[...]` the active metacharacters are `]`, `\`, `^` (only as the first
character, to negate), and `-` (only between two endpoints, to form a range).
Everything else, including `.`, `*`, `(`, `|`, `$`, is an ordinary literal.

To include the special ones literally:

| Want      | Write                                          |
|-----------|------------------------------------------------|
| `]`       | `\]`, or place it first: `[]...]` is **not** allowed — use `\]` |
| `^`       | `\^`, or place it anywhere but first           |
| `-`       | `\-`, or place it first or last: `[-a]`, `[a-]` |
| `\`       | `\\`                                            |

The control, numeric, and shorthand escapes of §7.1, §7.3, §7.4, §7.5 are valid
inside a class and mean the same scalar(s) there. The anchor escapes
(`\A \z \b \B`, §7.2) and the dot `.` are **not** anchors/wildcards inside a
class: `\b` inside a class is invalid (it is not a backspace literal in this
dialect), and `.` is a literal period.

### 9.2 No nested set operations in v1

Class **set operations** (intersection `&&`, difference, symmetric difference,
nested `[...]` inside a class) are **not** supported in v1 (§15). Inside a class,
`&` is a literal `&`.

An empty class `[]` and an empty negated class `[^]` are invalid (§14).

---

## 10. Flags

Flags adjust matching within their scope. They are set with a flag directive or a
scoped flag group (§3 `GroupHead`).

| Flag | Effect                                                                  |
|------|-------------------------------------------------------------------------|
| `i`  | case-insensitive matching using **Unicode simple case folding** (pinned version) |
| `m`  | multiline: `^` and `$` also match at line boundaries (§11.4)             |
| `s`  | dot-all: `.` also matches `\n` (§11.3)                                   |
| `U`  | swap greediness: bare quantifiers become lazy, `?`-suffixed become greedy (§6) |
| `x`  | verbose mode (below)                                                     |

### 10.1 Setting flags

- `(?flags)` — a directive with no body. It turns the listed flags **on** for the
  remainder of the innermost enclosing group (or the whole pattern at top level).
  `(?-flags)` turns them off; `(?on-off)` does both, e.g. `(?i-s)`.
- `(?flags:R)` — a non-capturing group whose flag changes apply **only** to `R`.
  Outside the group the previous flag state resumes. `(?on-off:R)` is allowed,
  e.g. `(?i-x:R)`.

Flags nest lexically; an inner group sees the enclosing flag state unless it
overrides it. An empty flag list, an unknown flag letter, or a `-` with no
following flags is invalid.

### 10.2 Verbose mode (`x`)

When `x` is in effect:

- Unescaped Unicode whitespace in the pattern is **ignored** (it does not match
  anything). To match a literal space, write `\ ` (escaped space, valid only in
  `x` mode), `[ ]`, or `\x20`.
- An unescaped `#` begins a comment that runs to the next line feed (`\n`) or the
  end of the pattern.
- Whitespace and `#` are **not** ignored inside a character class `[...]`; there
  they are literals.

---

## 11. Anchors, Dot, Boundaries

### 11.1 Literal scalars

A non-metacharacter scalar matches itself (subject to the `i` flag). Under `i`,
two scalars match when they are equal after Unicode simple case folding (pinned
version).

### 11.2 Position semantics and `start`

Anchors and boundaries are evaluated against the **full input** `value`, not
against a substring. The `start` argument of `find` only restricts where a match
may *begin*; it does not redefine position 0 or the input's boundaries. In
particular, with `start > 0`, `^` and `\A` still refer to absolute position 0,
and `\b`/`\B` consider the actual scalar immediately before `start`.

### 11.3 Dot

`.` matches any single scalar **except** the line feed `\n` (U+000A). Under the
`s` flag, `.` matches **any** single scalar, including `\n`. The dot never
matches "no scalar"; it always consumes exactly one scalar.

### 11.4 Anchors and boundaries

| Token | Matches at position `p` when …                                        |
|-------|------------------------------------------------------------------------|
| `\A`  | `p == 0` (absolute start of input), regardless of flags                |
| `\z`  | `p == len` (absolute end of input), regardless of flags                |
| `^`   | `p == 0`; **and**, under `m`, also when the scalar at `p-1` is `\n`     |
| `$`   | `p == len`; **and**, under `m`, also when the scalar at `p` is `\n`     |
| `\b`  | exactly one of the scalars at `p-1` and `p` is a `\w` scalar (§8); the out-of-range side (before position 0 / after position `len`) counts as non-`\w` |
| `\B`  | the negation of `\b`                                                    |

Without `m`, `$` matches **only** at the absolute end of input — it does **not**
match before a trailing `\n`. (Use `\n?\z` or the `m` flag if line-end behavior
is wanted.) All anchors and boundaries are **zero-width**: they consume no
scalar.

---

## 12. Function Semantics

### 12.1 `match(value, pattern)`

Compiles `pattern` (invalid → `ErrInvalidFormat`, §14) and returns `TRUE` iff a
match exists anywhere in `value` — i.e. iff the leftmost-match search over the
whole input (§4) finds any match (including a zero-length match). Otherwise
returns `FALSE`. `match` never fails with `ErrNotFound`; absence is `FALSE`.

### 12.2 `find(value, pattern, start = 0)`

1. Compile `pattern`; invalid → `ErrInvalidFormat` (§14).
2. Validate `start` as a scalar index: it must satisfy `0 <= start <= len(value)`
   (where `len` is the scalar length; `start == len` is permitted and can match a
   zero-length or end-anchored pattern). Otherwise fail with
   `ErrIndexOutOfRange` (`77050001`), matching the out-of-range behavior of
   `strings::find`.
3. Find the leftmost match (§4) whose start position is `>= start`. Anchors are
   still evaluated against the full input (§11.2).
4. On success, return the match's **start** scalar index. On no match at or after
   `start`, fail with `ErrNotFound` (`77050004`).

A zero-length match is a valid result: `find` returns its position.

### 12.3 `replace(value, pattern, replacement)`

Compiles `pattern` (invalid → `ErrInvalidFormat`) and returns `value` with every
non-overlapping match replaced by the expansion of `replacement` (§13), using the
global iteration rule in §12.4. `replacement` is parsed by the rules in §13; a
`replacement` string is always well-formed (see §13.4), so `replace` does not
fail on `replacement` content. `replace` does not fail with `ErrNotFound`; if
there are no matches it returns `value` unchanged.

### 12.4 Global, non-overlapping iteration (with zero-length rule)

Matches are produced left to right, non-overlapping, by the following
deterministic iterator. Positions are scalar indexes; `len == len(value)`.

```
last_end   = 0         # where the next search starts
last_match = NONE      # end position of the previously yielded match
yield_next():
    if last_end > len: return NONE
    m = leftmost match (§4) at or after position last_end
    if m == NONE: return NONE
    if m.start == m.end:                 # zero-length match
        last_end = m.end + 1             # advance one scalar to guarantee progress
        if m.end == last_match:          # sits exactly at the previous match's end
            return yield_next()          # skip it; do not yield an adjacent empty match
    else:
        last_end = m.end
    last_match = m.end
    return m
```

`replace` consumes this iterator with an output cursor:

```
out    = ""
cursor = 0
for each m yielded by the iterator:
    out += value[cursor .. m.start]      # copy the text before the match
    out += expand(replacement, m)        # §13
    cursor = m.end
out += value[cursor .. len]              # copy the trailing text
return out
```

Because matches are non-overlapping and ordered, `m.start >= cursor` always
holds. Any empty match skipped by the iterator's `last_match` guard is not
emitted; its scalar is copied as ordinary trailing/intervening text. This
guarantees termination, determinism, and that an empty match is never reported
immediately adjacent to the end of the previous match.

### 12.5 `findAll(value, pattern, start = 0)`

1. Compile `pattern`; invalid → `ErrInvalidFormat` (§14).
2. Validate `start` exactly as `find` does (§12.2 step 2): `0 <= start <= len(value)`,
   else `ErrIndexOutOfRange` (`77050001`).
3. Run the §12.4 iterator with `last_end` initialized to `start` (rather than `0`),
   and collect the **start** scalar index of each yielded match, left to right.
   Anchors are still evaluated against the full input (§11.2).
4. Return the collected indexes as a `List OF Integer`.

`findAll` is total over the match set: if there is no match at or after `start` it
returns an **empty list** — it does **not** fail with `ErrNotFound`. The list it
returns is exactly the sequence of `m.start` values for the matches `replace`
would act on when `start == 0`; in particular the zero-length advance and the
`last_match` guard (§12.4) apply identically, so `findAll(value, pattern)` reports
the same positions and count `replace` would replace. For example
`findAll("abc", "")` is `[0, 1, 2, 3]` (one empty match before each scalar and at
the end), matching the `replace("abc", "", "-")` → `"-a-b-c-"` example (§17.4).

---

## 13. Replacement Mini-Language

`replacement` is literal text interleaved with **capture references**. It is
defined here on its own terms.

### 13.1 References

| Form         | Expands to                                                       |
|--------------|------------------------------------------------------------------|
| `$$`         | a literal `$`                                                    |
| `$N`         | capture group number `N` (longest run of decimal digits after `$`) |
| `${N}`       | capture group number `N` (explicitly delimited)                  |
| `$name`      | named group `name` (longest run of `[A-Za-z0-9_]` after `$`)     |
| `${name}`    | named group `name` (explicitly delimited)                        |

- Group `0` is the **whole match**. Groups `1, 2, …` are the capturing groups in
  order of their opening parenthesis (§3, named or unnamed; non-capturing
  `(?:…)` groups are not numbered).
- A reference whose group **did not participate** in this match (e.g. an
  alternative that wasn't taken, or an unrepeated optional group) expands to the
  **empty string**.
- A reference to a group number greater than the pattern's group count, or to a
  name the pattern does not define, expands to the **empty string** (it is not an
  error). This keeps `replace` total over any `replacement`.

### 13.2 Disambiguating unbraced references

An unbraced `$N` / `$name` greedily consumes the longest valid run. Use the
braced form to terminate a reference before adjacent literal text:

- `${1}0` → group 1 followed by a literal `0` (whereas `$10` → group 10).
- `${name}_x` → group `name` followed by literal `_x` (whereas `$name_x` → group
  `name_x`).

### 13.3 Literal `$`

`$$` is the only way to emit a literal `$` adjacent to a reference. A `$` that is
**not** followed by `$`, `{`, a decimal digit, or a name-start scalar
(`[A-Za-z_]`) is itself a literal `$` followed by the next scalar (e.g. `"$ "`
yields `"$ "`, and `"price: $5"` — where `5` is a digit — references group 5,
which is typically absent and so expands to empty; write `"$$5"` for a literal
`$5`).

### 13.4 No invalid replacement

Every `replacement` string is well-formed: unknown/out-of-range references
expand to empty (§13.1) and any `$` that cannot start a reference is literal
(§13.3). Therefore `replace` never fails because of `replacement` content; only
an invalid `pattern` fails (`ErrInvalidFormat`).

---

## 14. Errors and Rejected Constructs

`regex` functions follow `standard_package.md` and `error_codes.md`:

| Condition                                   | Failure                                  |
|---------------------------------------------|------------------------------------------|
| invalid `pattern` (any function)            | `ErrInvalidFormat` (`77050003`)          |
| `find` no match at or after `start`         | `ErrNotFound` (`77050004`)               |
| `find`/`findAll` `start` outside `0 .. len(value)` | `ErrIndexOutOfRange` (`77050001`)  |

`findAll` never fails with `ErrNotFound`: when there is no match at or after
`start` it returns an empty list (§12.5). Only `find`'s single-result contract
fails on absence.

A malformed pattern must **not** be mapped to `FALSE`, `-1`, an unchanged source
string, or any host/errno-style error.

A pattern is **invalid** (`ErrInvalidFormat`) exactly when it violates the
grammar (§3) or one of these closed rules:

- unbalanced or unterminated `(`, `)`, `[`, or `]`;
- a quantifier (`*`, `+`, `?`, `{m,n}`) with no preceding atom, or two
  quantifiers stacked on one atom (other than the lazy `?` suffix) — §5;
- a counted quantifier `{m,n}` with `m > n`;
- a class range whose low endpoint exceeds its high endpoint;
- an empty class `[]` or empty negated class `[^]`;
- a backslash escape not in the closed set of §7 (including an unknown ASCII
  letter/digit escape, and `\b` used inside a class);
- a `\x{…}`/`\xHH` value that is not a valid Unicode scalar, or `\xHH` without
  exactly two hex digits;
- an unknown `\p{…}`/`\P{…}` property name;
- an empty, malformed, or unknown flag specification (e.g. `(?)`, `(?q)`,
  `(?-)`);
- a malformed group head (e.g. `(?<>…)`, a duplicate group name, an unterminated
  `(?<name…`);
- a v1 non-goal construct (§15): a backreference (`\1`, `\k<name>`), look-around
  (lookahead `(?=…)`, `(?!…)`; lookbehind `(?<=…)`, `(?<!…)`), or a class set
  operation (`[a&&b]` intersection form).

Lookbehind specifically: the group heads `(?<=` and `(?<!` are **rejected** with
`ErrInvalidFormat`. They are not a special case in the parser — they fall out of
the named-group rule. A `(?<` group head must be `(?<` followed by a `Name`
(`[A-Za-z_][A-Za-z0-9_]*`, §3) and then `>`. The lookbehind markers `=` and `!`
are not valid name-start scalars, so `(?<=…)` and `(?<!…)` are simply malformed
group heads. Lookahead `(?=…)` and `(?!…)` are rejected the same way: `?=` and
`?!` are not valid group heads (§3 `GroupHead`).

`start` being out of range is the only *runtime argument* error
(`ErrIndexOutOfRange`); it is independent of pattern validity, but pattern
compilation failure (`ErrInvalidFormat`) takes precedence if both apply.

---

## 15. Non-Goals for v1

The following are **out of scope** and are rejected as invalid patterns (§14),
not silently accepted:

- **Backreferences** (`\1`, `\k<name>`).
- **Look-around** — both forms are out of scope and rejected (§14):
  - **lookahead**: `(?=…)` (positive), `(?!…)` (negative);
  - **lookbehind**: `(?<=…)` (positive), `(?<!…)` (negative).

  None of these are parsed as zero-width assertions; they are invalid group
  heads. Note the deliberate non-conflict with **named groups**: `(?<name>…)` is
  a valid named capturing group (§3, §13), whereas `(?<=…)` / `(?<!…)` are
  lookbehind and are invalid — they differ only in whether `<` is followed by a
  `Name` or by the assertion markers `=` / `!`. There is no ambiguity, because
  `=` and `!` cannot start a group name.
- **Class set operations** (intersection/difference/symmetric-difference,
  nested classes).
- **Locale- or target-sensitive behavior** of any kind. All Unicode behavior is
  pinned (§2.1).
- Any construct whose behavior cannot be pinned identically across targets.

These may be considered in a later version only if they remain compatible with a
single cross-target contract.

---

## 16. Determinism Guarantee and Lineage

**Determinism.** For the same `(pattern, value)` — and, for `replace`, the same
`replacement` — every `regex` function produces **byte-for-byte identical**
observable results on every target (`macos-aarch64`, `linux-aarch64` glibc and
musl, and any future target) and on both the native and Binary Representation
package code paths. There is no permitted source of variation: no host libc, no
host locale, no host Unicode tables, no nondeterministic match selection. The
pinned Unicode version (§2.1) is the only Unicode authority and is identical
everywhere.

**Informal lineage (non-normative).** This dialect is intentionally close to the
Rust `regex` crate: Unicode-by-default semantics, leftmost-first matching, the
greedy/lazy model, the `\d \w \s` definitions, the `(?flags:…)` groups, and the
`$name`/`${name}`/`$$` replacement syntax all follow that style. This note is
historical context only. Nothing in this document is defined by what Rust (or
PCRE, POSIX, or any host `regcomp()`) does; §1–§15 are the sole authority, and a
disagreement with any external engine is resolved in favor of this document.

---

## 17. Worked Examples

Each row is a normative example and should have a corresponding test under
`tests/func_regex_*`. Indexes are zero-based scalar indexes.

### 17.1 `match`

| value         | pattern        | result  | why                                            |
|---------------|----------------|---------|------------------------------------------------|
| `"hello"`     | `"ell"`        | `TRUE`  | substring match                                |
| `"hello"`     | `"^h"`         | `TRUE`  | `^` at position 0                              |
| `"hello"`     | `"^e"`         | `FALSE` | `e` is not at position 0                        |
| `"abc123"`    | `"\d+"`        | `TRUE`  | one or more digits present                     |
| `"abc"`       | `"\d"`         | `FALSE` | no digit                                       |
| `"café"`      | `"caf\x{E9}"`  | `TRUE`  | `\x{E9}` is `é` (no `\uHHHH` form; use `\x{…}`) |
| `"Hello"`     | `"(?i)hello"`  | `TRUE`  | case-insensitive                                |
| `""`          | `"a*"`         | `TRUE`  | `a*` matches the empty string                  |
| `"x"`         | `"^$"`         | `FALSE` | input is non-empty                             |
| `""`          | `"^$"`         | `TRUE`  | empty input, start == end                      |

### 17.2 `find`

| value         | pattern    | start | result                | why                               |
|---------------|------------|-------|-----------------------|-----------------------------------|
| `"hello"`     | `"l"`      | `0`   | `2`                   | first `l` at index 2              |
| `"hello"`     | `"l"`      | `3`   | `3`                   | first `l` at or after 3           |
| `"hello"`     | `"l"`      | `4`   | `ErrNotFound`         | no `l` at or after 4              |
| `"a1b2c3"`    | `"\d"`     | `0`   | `1`                   | first digit                       |
| `"aaa"`       | `"a+"`     | `0`   | `0`                   | leftmost; greedy matches `aaa`, start is 0 |
| `"abc"`       | `"x"`      | `0`   | `ErrNotFound`         | no match                          |
| `"abc"`       | `""`       | `0`   | `0`                   | empty pattern matches at 0        |
| `"abc"`       | `"^b"`     | `1`   | `ErrNotFound`         | `^` is absolute pos 0, not `start`|
| `"hello"`     | `"l"`      | `6`   | `ErrIndexOutOfRange`  | `start > len` (len is 5)          |
| `"hello"`     | `"o$"`     | `0`   | `4`                   | `$` at end of input               |

### 17.3 `findAll`

| value         | pattern    | start | result          | why                                            |
|---------------|------------|-------|-----------------|------------------------------------------------|
| `"hello"`     | `"l"`      | `0`   | `[2, 3]`        | both `l`s                                       |
| `"a1b2c3"`    | `"\d"`     | `0`   | `[1, 3, 5]`     | every digit position                           |
| `"abc"`       | `"x"`      | `0`   | `[]`            | no match → empty list (not `ErrNotFound`)      |
| `"aaa"`       | `"a+"`     | `0`   | `[0]`           | one greedy, non-overlapping match              |
| `"abc"`       | `""`       | `0`   | `[0, 1, 2, 3]`  | empty match before each scalar and at end      |
| `"x1y2"`      | `"\d"`     | `2`   | `[3]`           | only matches at or after `start`               |
| `"hello"`     | `"l"`      | `6`   | `ErrIndexOutOfRange` | `start > len` (len is 5)                   |

### 17.4 `replace`

| value           | pattern     | replacement | result            | why                                  |
|-----------------|-------------|-------------|-------------------|--------------------------------------|
| `"a-b-c"`       | `"-"`       | `":"`       | `"a:b:c"`         | all matches replaced                 |
| `"hello"`       | `"l"`       | `"L"`       | `"heLLo"`         | both `l`s                            |
| `"2024-06-24"`  | `"(\d+)-(\d+)-(\d+)"` | `"$3/$2/$1"` | `"24/06/2024"` | capture references                   |
| `"foo"`         | `"(?<x>o)"` | `"[${x}]"`  | `"f[o][o]"`       | named group reference                |
| `"abc"`         | `""`        | `"-"`       | `"-a-b-c-"`       | empty match before each scalar and at end |
| `"a1b2"`        | `"\d"`      | `""`        | `"ab"`            | delete all digits                    |
| `"price"`       | `"x"`       | `"y"`       | `"price"`         | no match → unchanged                 |
| `"5"`           | `"5"`       | `"$$"`      | `"$"`             | `$$` is a literal `$`                |
| `"ab"`          | `"a"`       | `"${1}z"`   | `"zb"`            | group 1 absent → empty, then `z`     |
| `"cat"`         | `"a"`       | `"4"`       | `"c4t"`           | literal digit replacement            |

### 17.5 Zero-length advance detail

For `replace("abc", "x*", "-")`:

- at pos 0: `x*` matches empty → emit `-`, copy `a`, advance to 1
- at pos 1: empty → emit `-`, copy `b`, advance to 2
- at pos 2: empty → emit `-`, copy `c`, advance to 3
- at pos 3 (== len): empty → emit `-`, end

Result: `"-a-b-c-"`. The `last_match` guard prevents a second empty match at a
position already consumed, so the count is exactly `len + 1`.
