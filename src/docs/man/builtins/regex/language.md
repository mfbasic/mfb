# language

How to write patterns for the regex package.

## Synopsis

```
Metacharacters    \ . * + ? ( ) [ ] { } ^ $ |
Classes           [abc]  [a-z]  [^abc]  [\d_]  [[:alpha:]]
Shorthands        \d \D   \w \W   \s \S
Anchors           ^ $   \A \z   \b \B
Quantifiers       X*  X+  X?  X{m}  X{m,}  X{m,n}   (+ trailing ? for lazy)
Groups            (R)  (?:R)  (?<name>R)
Alternation       a|b|c
Flags             (?i) (?m) (?s) (?U) (?x)   scoped: (?i:R)
```

## Package

regex

## Imports

```
IMPORT regex
```

`regex` is a built-in package, so `IMPORT regex` needs no manifest dependency.

## Description

This page is a developer's guide to writing patterns for `regex::match`,
`regex::find`, `regex::findAll`, and `regex::replace`. It covers the pattern
syntax itself. For what each function returns, how start positions work, and the
errors they raise, see the individual function pages.

A pattern is an ordinary `String`. It is compiled at the moment a regex function
is called, so patterns can be literals, built at run time, or read from input.
An invalid pattern fails the call with `ErrInvalidFormat`; it is never silently
treated as "no match".

### Patterns in String literals

The single most common mistake. MFBASIC `String` literals process their own
backslash escapes before the regex engine ever sees the text, so a backslash you
want the regex to receive must be written as two backslashes in the literal:

```
regex::find(value, "\\d")        the pattern is \d (a digit)
regex::match(value, "\\.")       the pattern is \. (a literal dot)
regex::match(value, "a\\\\b")    the pattern is a\b (a literal backslash)
```

As a rule: every backslash the regex needs becomes `"\\"` in the source literal.
A pattern read from a file or built from user input has no such doubling — the
doubling is purely a property of writing the backslash in a `String` literal.

### Literals and metacharacters

Any scalar that is not a metacharacter matches itself: the pattern `abc` matches
the text `"abc"`. The metacharacters outside a character class are:

```
\ . * + ? ( ) [ ] { } ^ $ |
```

To match one of these literally, escape it with a backslash (`\.` `\*` `\(` `\|`
`\\`), or, where allowed, place it inside a character class. A backslash before
ASCII punctuation always means that punctuation literally.

Two exceptions match literally without an escape: a `{` that does not begin a
well-formed counted quantifier is a literal `{` (so `a{b` matches the text
`"a{b"`), and likewise a stray `}` is a literal `}`. Every other syntax violation
is an invalid pattern rather than a literal.

### Character classes

A class `[...]` matches one scalar drawn from a set:

```
[abc]        one of a, b, or c
[a-z]        any scalar in the inclusive range a through z
[^aeiou]     any one scalar NOT listed (negated class)
[a-zA-Z0-9_] a typical identifier-character set
[\d.]        a digit or a literal dot
[\p{L}_]     any letter or underscore
[[:alpha:]]  a POSIX named set (also [:digit:], [:space:], [:^alpha:], ...)
```

Inside a class, the only active metacharacters are `]` (close), `\` (escape),
`^` (negation, only as the first character), and `-` (a range, only between two
endpoints). Everything else — including `.` `*` `(` `|` `$` — is an ordinary
literal there. To include the special ones literally write `\]` `\^` `\\`, or
place `-` first or last (`[-a]`, `[a-]`). A literal `]` must be written `\]` — a
leading-`]` form like `[]...]` is not accepted. An empty class `[]` or `[^]` is
invalid, as is a range whose low endpoint exceeds its high endpoint (`[z-a]`).

A few characters that are special outside a class lose that meaning inside one:
`.` is a literal period, and there are no set operations, so `&` is a literal `&`
(no `[a&&b]` intersection). The control, hex, shorthand, and property escapes
still work inside a class, but the anchor escapes do not — `\b` inside a class is
invalid (it is not a backspace in this dialect), not a word boundary. POSIX named
sets like `[:alpha:]` are written only inside a class, e.g. `[[:alpha:]_]`.

### Shorthand classes

These name common sets and may be used on their own or inside a class. Their
meaning is Unicode-aware and identical on every target:

```
\d   a decimal digit            \D   anything but a digit
\w   a word scalar (letter,     \W   anything but a word scalar
     mark, digit, connector
     punctuation, or joiner)
\s   whitespace                 \S   anything but whitespace
```

### The dot

`.` matches any single scalar except newline (`U+000A`). Under the `s` flag it
matches any scalar including newline. The dot always consumes exactly one scalar;
it never matches "nothing".

### Anchors and boundaries

These are zero-width: they match a position, not a scalar, and consume nothing.

```
^    start of input; also after each newline under the m flag
$    end of input; also before each newline under the m flag
\A   the absolute start of input, regardless of flags
\z   the absolute end of input, regardless of flags
\b   a word boundary (between a \w scalar and a non-\w scalar or an edge)
\B   a position that is NOT a word boundary
```

Without the `m` flag, `^` and `$` refer to the very start and end of the whole
input. In particular, `$` then matches only at the absolute end — it does not
match just before a trailing newline. Write `\n?\z`, or use the `m` flag, if you
want to allow a final newline. `\b` treats the position before the first scalar
and after the last as non-word, so `\bword\b` matches a whole word at either edge
of the input.

### Quantifiers

A quantifier repeats the single atom before it (a literal, class, group, escape,
or the dot):

```
X*       zero or more X        X{m}     exactly m
X+       one or more X         X{m,}    m or more
X?       zero or one X         X{m,n}   between m and n (m <= n)
```

Quantifiers are greedy by default: they match as many repetitions as possible
while still letting the overall pattern match. Add a trailing `?` to make a
quantifier lazy — it then matches as few as possible:

```
X*?  X+?  X??  X{m,}?  X{m,n}?
```

Greedy `a.*b` over `"axbxb"` spans to the last `b`; lazy `a.*?b` stops at the
first. A quantifier with no atom before it (a leading `*`) is invalid, and you
cannot stack two quantifiers on one atom (`a**`) — wrap it in a group instead,
`(?:a+)*`. (`a+?` is not stacking; the trailing `?` is the lazy marker.) A
counted quantifier must have `m <= n`: `a{3,1}` is invalid. `a{0}` is allowed and
matches the empty string. A quantifier binds only the single atom immediately
before it, so `ab*` repeats just `b`, not `ab` — group it, `(ab)*`, to repeat the
pair.

### Groups

Parentheses group a subexpression so a quantifier or alternation can apply to it
as a whole, and capture the matched text for use in replacement:

```
(R)          capturing group, numbered 1, 2, ... by opening parenthesis
(?:R)        group without capturing (no number assigned)
(?<name>R)   named capturing group (also (?P<name>R))
```

Group 0 is always the whole match. Captured groups are referenced from a
replacement template with `$N`, `${N}`, `$name`, or `${name}`; see the replace
page. A group name is `[A-Za-z_][A-Za-z0-9_]*`; a missing or malformed head
(`(?<>R)`) and a duplicate group name are both invalid.

### Alternation

`a|b|c` matches whichever alternative lets the overall pattern succeed, tried
left to right. Alternation has the lowest precedence, so `ab|cd` means
`(ab)|(cd)`; group it to limit its reach, `gr(a|e)y`.

### Flags

Flags adjust matching for part or all of a pattern:

```
i   case-insensitive (Unicode simple case folding)
m   multiline: ^ and $ also match at line boundaries
s   dot-all: . also matches newline
U   swap greedy and lazy (bare quantifiers become lazy)
x   verbose: unescaped whitespace is ignored and # starts a comment
```

Set them with a directive `(?flags)`, which applies to the rest of the enclosing
group, or scope them to one group with `(?flags:R)`. Turn flags off after a dash:

```
(?i)abc            case-insensitive from here on
(?i:abc)def        only abc is case-insensitive
(?i-s:R)           i on, s off, just for R
```

Verbose mode (`x`) lets you space out and comment a complex pattern; write a
literal space as `\ `, `[ ]`, or `\x20`, since unescaped spaces are then ignored.
Whitespace and `#` are not ignored inside a character class, where they remain
literals. An empty flag list (`(?)`), an unknown flag letter (`(?q)`), or a dash
with no flags after it (`(?-)`) is invalid.

### Unicode escapes and properties

Beyond the shorthands and anchors:

```
\n \r \t \f \v \a \e \0     control scalars by name
\xHH                        a scalar by two hex digits (\x41 is "A")
\x{H..H}                    a scalar by 1-6 hex digits (\x{1F600})
\p{Prop} / \P{Prop}         with / without a Unicode property
\pX / \PX                   single-letter general category (\pL is a letter)
```

`Prop` may be a general category (`L`, `Lu`, `Letter`), a script (`Greek`,
`Han`), or the `Name=Value` form (`\p{Script=Greek}`). There is no `\uHHHH` form;
use `\x{...}`. A backslash before any ASCII letter or digit that is not one of
the escapes above is invalid — there is no "unknown escape is the literal letter"
fallback.

`\xHH` requires exactly two hex digits, and the value of any `\x` escape must be
a valid Unicode scalar (`0` through `0x10FFFF`, excluding the surrogate range
`0xD800`–`0xDFFF`); an out-of-range or surrogate value is an invalid pattern. An
unrecognized `\p{...}` property name is likewise invalid.

### Unicode positions

Matching works over Unicode scalar values, so every index a regex function
accepts or returns is a zero-based scalar index, never a byte offset and never a
grapheme-cluster index — consistent with `len` and the `strings` package.

### Not supported

These are rejected as invalid patterns, not parsed as assertions, so do not reach
for them: backreferences (`\1`, `\k<name>`) and look-around (lookahead `(?=...)`,
`(?!...)`; lookbehind `(?<=...)`, `(?<!...)`). Note that `(?<name>...)` is a valid
named group; it differs from the lookbehind heads `(?<=` and `(?<!` only in
whether `<` is followed by a name or by `=` / `!`. Class set operations
(intersection, nested classes) are also unsupported.

## Examples

Match, locate, and rewrite with patterns (note the doubled backslashes):

```
IMPORT regex

SUB main()
  LET hasDigit AS Boolean = regex::match("abc123", "\\d")
  LET at AS Integer = regex::find("a1b2c3", "\\d")
  LET all AS List OF Integer = regex::findAll("a1b2c3", "\\d")
  LET ymd AS String = regex::replace("2024-06-24", "(\\d+)-(\\d+)-(\\d+)", "$3/$2/$1")
END SUB
```

Anchors, classes, and a case-insensitive flag:

```
IMPORT regex

SUB main()
  LET isHex AS Boolean = regex::match("1a2f", "^[0-9a-f]+$")
  LET isHello AS Boolean = regex::match("HELLO", "(?i)^hello$")
END SUB
```

Greedy versus lazy:

```
IMPORT regex

SUB main()
  LET greedy AS Integer = regex::find("<a><b>", "<.*>")    ' matches the whole span
  LET lazy AS Integer = regex::find("<a><b>", "<.*?>")     ' matches just <a>
END SUB
```

## See also

- `mfb man regex match`
- `mfb man regex find`
- `mfb man regex findAll`
- `mfb man regex replace`
