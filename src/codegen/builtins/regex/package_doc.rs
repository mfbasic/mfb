//! plan-176-T: the `regex` package's man prose — `INTRO` and `DESC`, read by `register` in
//! `builtins/regex/mod.rs`.

pub(super) const INTRO: &str = r#"Match, search, and replace text with regular expressions"#;

pub(super) const DESC: &str = r#"The `regex` package searches and rewrites text with a single portable
regular-expression dialect that is MFBASIC's own. Its syntax and semantics are
defined entirely by `mfb spec stdlib regex` and produce byte-for-byte identical
results on every target, never deferring to a host libc, locale, or OS regex
library. `regex` is a built-in package: `IMPORT regex` needs no manifest
dependency. For the full pattern language, run `mfb spec stdlib regex`.

The package defines two value types, `regex::MatchInfo` and `regex::Group`, which are
how a match reports what it covered; run `mfb man regex types` for their fields.
(The type is spelled `MatchInfo` because `MATCH` is an MFBASIC keyword and cannot
be a type name; the function is still called `regex::match`.)
Everything else is ordinary text: `pattern` and `replacement` are runtime `String`
values, so they may be literals, built at run time, or read from input; a pattern
is compiled at the moment a function is called. An invalid pattern fails the call
with `ErrInvalidFormat` rather than being silently treated as "no match".

**Backslashes in a pattern need doubling, and `\x{...}` is the trap.** A pattern
is written as an ordinary MFBASIC string, and MFBASIC's own string escapes are
applied first. `\x{...}` is **not** one of them: MFBASIC's Unicode escape is
`\u{...}`, so an unrecognised `\x` simply loses its backslash. That turns

```
LET pattern AS String = "\x{41}"
```

into the five-character string `x{41}` — which the regex engine reads as the
quantifier "41 letter `x`s", not as the letter `A`. Nothing warns you; the
pattern is valid, it just means something else.

Double the backslash to send a real one through:

```
LET pattern AS String = "\\x{41}"   ' six characters: \x{41}
IF regex::match("A", pattern) THEN     ' TRUE
```

The same doubling applies to every pattern backslash — `\\d`, `\\w`, `\\s`,
`\\b`, `\\p{Lu}`, `\\A`, `\\z`. The rule is simply: one backslash in the
*pattern* is two in the *source*. A pattern read from a file or from user
input is not a source literal and needs no doubling at all.

Matching operates over Unicode scalar values. Every position and index a regex
function accepts or reports is a zero-based Unicode scalar index — never a byte
offset and never a grapheme-cluster index — consistent with `len` and the
`strings` package. A string of `n` scalars has positions `0` through `n`;
position `n` is after the last scalar, so a `start` argument may equal
`len(value)`. All Unicode-dependent behavior (the `\d`/`\w`/`\s` shorthands,
`\p{...}` properties, and `(?i)` case folding) resolves against a single pinned
Unicode version, identical across every target.

The functions differ only in what they report. `match` returns a `Boolean` for
whether the pattern matches anywhere; `find` returns the start index of the first
match at or after `start`, and raises `ErrNotFound` when there is none; `findAll`
returns a
`List OF Integer` of the start index of every non-overlapping match; `count`
returns how many non-overlapping matches there are; `findMatch`
returns a `regex::MatchInfo` for the first match — its span, its text, and its
capture groups; `findAllMatches` returns a `List OF regex::MatchInfo`, one for every
non-overlapping match; `split` returns the `List OF String` of text between the
matches; and `replace` returns a new `String` with every
non-overlapping match rewritten by a replacement template.

The reporting pairs agree exactly. `findMatch` finds the match `find` locates, so
wherever a match exists `findMatch(value, pattern, start).start` is
`find(value, pattern, start)` — where none exists they part company, because
`find` raises and `findMatch` reports a no-match `MatchInfo`. `findAllMatches`
finds the matches `findAll` locates, in the same order and the same number — the index-only members are the cheaper call when the text is not
wanted, never a different search. Reach for `findMatch` and `findAllMatches`
whenever the matched text is needed: a pattern's match length is an output, not
something the caller knows in advance, so a start index alone cannot be sliced.

Every search is unanchored and leftmost: the reported match is the one beginning
at the smallest position where any match exists. `count`, `find`, `findAll`,
`findMatch` and `findAllMatches` take an optional `start` (default `0`) restricting only
where a match may begin — the absolute anchors `\A`, `\z`, and unflagged
`^`/`$` are still evaluated against the whole value. A zero-length match is
valid; iteration advances one scalar past an empty match so it always
terminates.

Absence of a match is reported per member, and the split is decided by whether
the return type has a value that can mean "no match". `match` returns `FALSE`,
`findAll` and `findAllMatches` return empty lists, `findMatch` returns a
`MatchInfo` whose `start` is `-1`, and `replace` returns `value` unchanged — none
of those fails, because a `Boolean`, an empty list, a no-match record and an
unrewritten string each *are* the answer. `find` is the exception: it returns an
index, and every `Integer` is a position some search could legitimately report,
so there is no value left over to mean "absent". It raises `ErrNotFound`
(`77050004`), the same contract `strings::find` and the `collections` find-family
use, and `regex::match` is the guard for callers who treat absence as ordinary.
None of the functions mutate their arguments or have side effects.

## The empty pattern

An empty `pattern` is not a pattern that fails to match — it matches, zero-width,
at every position. `regex` reads that the same way `strings` reads an empty
needle, and the rule is stated once on `mfb man strings` ("The empty needle"):

- a member that **answers a question** about a match reports it — `match` returns
  `TRUE`, `find` returns `start`, `findAll` and `findAllMatches` return one
  zero-length match per position, and `findMatch` reports the one at `start`;
- a member that **counts or rewrites every match** refuses it with
  `ErrInvalidArgument` (`77050002`) — `count`, `split` and `replace`, because
  "every position" is neither a useful count, nor a division of the text, nor a
  rewrite that leaves anything of the input.

That guard is on the empty pattern **string** and on nothing else. A pattern such
as `a*`, `x?`, `(?:)` or `\b` that merely *matches* zero-width text is an ordinary
pattern: it still matches at every position, and `count`, `split` and `replace`
still act on every one of those matches. One consequence worth stating: `count`
is `len(findAll(...))` for every pattern except the empty one, where `findAll`
answers and `count` refuses — the same split `strings::find` and `strings::count`
already have, for the same reason.

## What `strings` has that `regex` does not

`regex` is the pattern mirror of `strings`, and the mirror is now complete for the
operations a *pattern* can express: `match`↔`contains`, `find`↔`find`,
`count`↔`count`, `split`↔`split`, `replace`↔`replace`. The `strings` members with
no pattern equivalent have none because a pattern would add nothing to them, not
because they were overlooked:

- The **measurements and decompositions** — `len`, `byteLen`,
  `graphemes`, `graphemesCount`, `graphemeAt`, `toBytes`, `toScalars`,
  `fromScalars` — ask about the text itself, not about a pattern in it.
- The **position-anchored members** — `startsWith`, `endsWith`, `startsWithAny`,
  `endsWithAny`, `stripPrefix`, `stripSuffix`, `left`, `right`, `mid` — are
  written as patterns instead: `^p` and `p$` anchor a `regex::match`, and a span
  from `regex::findMatch` is what `strings::mid` slices.
- The **rewriters that do not search** — `upper`, `lower`, `caseFold`,
  `normalizeNfc`, `trim`, `trimStart`, `trimEnd`, `trimChars`, `padLeft`,
  `padRight`, `repeat`, `join` — transform the whole value or assemble a new one,
  so there is nothing for a pattern to select. `regex::replace` is the searching
  rewriter, and it is present.

## Attributed text

Every `regex` **query** member — `match`, `find`, `findAll`, `findMatch`,
`findAllMatches`, `count` and `split` — also accepts an
`astrings::AttributedString` at the `value` position: the query runs on its
visible text and returns exactly what the `String` overload returns (same value,
type, and errors). That is the rule `strings` uses, unchanged.

`regex::replace` deliberately has no such overload, and a call passing an
`AttributedString` to it is a build error rather than a silent loss of the
attributes. A pattern rewrite has no answer for what the attributes should become:
a match can begin inside one attribute span and end inside another, and the
replacement text has no extent in the original to inherit from. Rewriting attributed
text with a pattern is a design of its own, not a missing line here."#;
