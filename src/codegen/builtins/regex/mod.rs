//! Package: regex
//! Type: Pure MFBasic
//! Plan: plan-72-T

use crate::codegen::registry::{
    RecordProp, Registry, RegistryPackage, RegistryRecord, RegistryUnion, UnionVariant,
};
use crate::types::ParameterType;

mod func_count;
mod func_find;
mod func_find_all;
mod func_find_all_matches;
mod func_find_match;
mod func_gen_cat;
mod func_match;
mod func_replace;
mod func_script_of;
mod func_split;

mod helper_all_digits;
mod helper_anchor_match;
mod helper_ascii_class_bitset;
mod helper_canon_prop;
mod helper_cat_is_letter;
mod helper_cat_is_mark;
mod helper_char_eq;
mod helper_chr;
mod helper_class_match;
mod helper_class_match_one;
mod helper_compile;
mod helper_expand;
mod helper_fail;
mod helper_init_caps;
mod helper_is_ascii_punct;
mod helper_is_counted_at;
mod helper_is_digit;
mod helper_is_gc_name;
mod helper_is_name_cont;
mod helper_is_name_start;
mod helper_is_pat_space;
mod helper_is_script_name;
mod helper_is_simple_node;
mod helper_is_space_cp;
mod helper_is_word;
mod helper_is_word_cp;
mod helper_lookup_name;
mod helper_lookup_num;
mod helper_lookup_ref;
mod helper_make_class;
mod helper_make_ctx;
mod helper_make_match;
mod helper_match_results;
mod helper_no_match;
mod helper_parse_alt;
mod helper_parse_atom;
mod helper_parse_class;
mod helper_parse_class_endpoint;
mod helper_parse_concat;
mod helper_parse_counted;
mod helper_parse_depth_limit;
mod helper_parse_escape_atom;
mod helper_parse_flag_spec;
mod helper_parse_hex_escape;
mod helper_parse_int_clamp;
mod helper_parse_literal_escape;
mod helper_parse_name;
mod helper_parse_named_group;
mod helper_parse_paren;
mod helper_parse_posix;
mod helper_parse_prop;
mod helper_parse_quant_suffix;
mod helper_posix_prop;
mod helper_prop_match_item;
mod helper_prop_test;
mod helper_required_first_cp;
mod helper_run;
mod helper_scalar_to_cp;
mod helper_script_canon;
mod helper_script_test;
mod helper_search_from;
mod helper_set_cap;
mod helper_short_kind;
mod helper_shorthand_match;
mod helper_simple_match_at;
mod helper_step_budget;
mod helper_steps;
mod helper_to_scalars;
mod helper_try_at;
mod helper_word_boundary;

const INTRO: &str = r#"Match, search, and replace text with regular expressions"#;

const DESC: &str = r#"The `regex` package searches and rewrites text with a single portable
regular-expression dialect that is MFBASIC's own. Its syntax and semantics are
defined entirely by `mfb spec stdlib regex` and produce byte-for-byte identical
results on every target, never deferring to a host libc, locale, or OS regex
library. `regex` is a built-in package: `IMPORT regex` needs no manifest
dependency. For the full pattern language, run `mfb man regex language`.

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
at the smallest position where any match exists. `find`, `findAll`, `findMatch`
and `findAllMatches` take an optional `start` (default `0`) restricting only
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

- The **measurements and decompositions** — `len`, `byteLen`, `displayWidth`,
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

// ---------------------------------------------------------------------------
// The `AttributedString` Tier-A seam (bug-534).
//
// `regex::` QUERY members can take an `astrings::AttributedString` (astrings' type,
// which STAYS hardcoded/always-in-scope — astrings has not migrated) at the text
// position. This is the same genuine non-registry behavior `strings` carries
// (the registry matcher speaks only its own type vocabulary), so it lives here as
// a co-located rewrite that `ir::lower` and `builtins::resolve_call_return_type_typed`
// consult, NOT as a registry matcher entry.
//
// `regex` has Tier-A only. There is no Tier-B: an attribute-preserving
// `regex::replace` would have to remap attribute spans across a pattern rewrite,
// where a match can span attribute boundaries and the replacement has no
// corresponding extent. That is a design problem of its own, deliberately out of
// scope, and a `regex::replace(AttributedString, …)` call stays a type error
// rather than silently dropping the attributes.
// ---------------------------------------------------------------------------

/// Argument-validated return type of a `regex::` call. A Tier-A query answers on
/// the visible text of an `AttributedString`, so its result type is exactly the
/// `String` overload's (substitute `String` for the leading `AttributedString` and
/// reuse the registry resolution — `ir::lower` rewrites the argument to
/// `toString(a)`). Every other call defers to the generic `registry::resolve_call`.
/// `strict` carries through the bug-443 strict(validation)/lenient(inference) split.
pub(crate) fn resolve_return_type(
    name: &str,
    arg_types: &[ParameterType],
    strict: bool,
) -> Option<ParameterType> {
    if is_tier_a_query(name)
        && arg_types
            .first()
            .is_some_and(|a| a.is_named("AttributedString"))
    {
        let mut substituted = arg_types.to_vec();
        substituted[0] = ParameterType::String;
        return crate::codegen::registry::resolve_call_typed(name, &substituted, strict);
    }
    crate::codegen::registry::resolve_call_typed(name, arg_types, strict)
}

/// The Tier-A `regex::` query members: they *interrogate* the text (returning a
/// verdict, a position, a count, a match record, or a decomposition into a
/// collection) rather than re-expressing it, so an `AttributedString` argument is
/// answered on its visible text and the result type matches the `String` overload.
/// `ir::lower` wraps the leading argument in `toString(a)` for these. Keyed on the
/// qualified dot name.
///
/// `regex.replace` is deliberately absent — see the module comment above.
pub(crate) fn is_tier_a_query(name: &str) -> bool {
    matches!(
        name,
        "regex.count"
            | "regex.find"
            | "regex.findAll"
            | "regex.findAllMatches"
            | "regex.findMatch"
            | "regex.match"
            | "regex.split"
    )
}

pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("regex", INTRO, DESC);

    pkg.add_imports(vec!["collections", "strings", "encoding"]);

    pkg.add_record(RegistryRecord {
        name: "__regex_Flags",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "ci",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "ml",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "dotall",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "ungreedy",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "verbose",
                ty: ParameterType::Boolean,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Range",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "lo",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "hi",
                ty: ParameterType::String,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Single",
        export: false,
        description: "",
        props: vec![RecordProp {
            name: "ch",
            ty: ParameterType::String,
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Short",
        export: false,
        description: "",
        props: vec![RecordProp {
            name: "kind",
            ty: ParameterType::Integer,
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Prop",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "name",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "neg",
                ty: ParameterType::Boolean,
                description: "",
            },
        ],
    });

    pkg.add_union(RegistryUnion {
        name: "__regex_ClassItem",
        export: false,
        variants: vec![
            UnionVariant {
                name: "__regex_Range",
                description: "",
            },
            UnionVariant {
                name: "__regex_Single",
                description: "",
            },
            UnionVariant {
                name: "__regex_Short",
                description: "",
            },
            UnionVariant {
                name: "__regex_Prop",
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Lit",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "ch",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "fold",
                ty: ParameterType::Boolean,
                description: "",
            },
            // bug-510: the literal's code point, computed once at parse so the matcher
            // compares scalars (the context no longer carries the subject as Strings).
            RecordProp {
                name: "cp",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Any",
        export: false,
        description: "",
        props: vec![RecordProp {
            name: "dotall",
            ty: ParameterType::Boolean,
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Class",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "neg",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "fold",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "items",
                ty: ParameterType::list_of(ParameterType::named("__regex_ClassItem")),
                description: "",
            },
            RecordProp {
                name: "ascii",
                ty: ParameterType::list_of(ParameterType::Boolean),
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Anchor",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "kind",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "ml",
                ty: ParameterType::Boolean,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Concat",
        export: false,
        description: "",
        props: vec![RecordProp {
            name: "parts",
            ty: ParameterType::list_of(ParameterType::named("__regex_Node")),
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Alt",
        export: false,
        description: "",
        props: vec![RecordProp {
            name: "opts",
            ty: ParameterType::list_of(ParameterType::named("__regex_Node")),
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Repeat",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "child",
                ty: ParameterType::named("__regex_Node"),
                description: "",
            },
            RecordProp {
                name: "lo",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "hi",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "greedy",
                ty: ParameterType::Boolean,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Group",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "child",
                ty: ParameterType::named("__regex_Node"),
                description: "",
            },
            RecordProp {
                name: "slot",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    pkg.add_union(RegistryUnion {
        name: "__regex_Node",
        export: false,
        variants: vec![
            UnionVariant {
                name: "__regex_Lit",
                description: "",
            },
            UnionVariant {
                name: "__regex_Any",
                description: "",
            },
            UnionVariant {
                name: "__regex_Class",
                description: "",
            },
            UnionVariant {
                name: "__regex_Anchor",
                description: "",
            },
            UnionVariant {
                name: "__regex_Concat",
                description: "",
            },
            UnionVariant {
                name: "__regex_Alt",
                description: "",
            },
            UnionVariant {
                name: "__regex_Repeat",
                description: "",
            },
            UnionVariant {
                name: "__regex_Group",
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_ContDone",
        export: false,
        description: "",
        props: vec![RecordProp {
            name: "dummy",
            ty: ParameterType::Boolean,
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_ContSeq",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "parts",
                ty: ParameterType::list_of(ParameterType::named("__regex_Node")),
                description: "",
            },
            RecordProp {
                name: "idx",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::named("__regex_Cont"),
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_ContCap",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "slot",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::named("__regex_Cont"),
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_ContRep",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "rep",
                ty: ParameterType::named("__regex_Repeat"),
                description: "",
            },
            RecordProp {
                name: "count",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "startPos",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::named("__regex_Cont"),
                description: "",
            },
        ],
    });

    pkg.add_union(RegistryUnion {
        name: "__regex_Cont",
        export: false,
        variants: vec![
            UnionVariant {
                name: "__regex_ContDone",
                description: "",
            },
            UnionVariant {
                name: "__regex_ContSeq",
                description: "",
            },
            UnionVariant {
                name: "__regex_ContCap",
                description: "",
            },
            UnionVariant {
                name: "__regex_ContRep",
                description: "",
            },
        ],
    });

    // bug-510: the matcher's backtrack stack. A choice point is a record whose `nxt`
    // is the choice below it -- a linked list, never a growable `List OF`, because a
    // `collections::get` of a recursive-type element aliases the list's storage and a
    // growing `append` frees it (bug-538). `__regex_run` documents the field roles.
    pkg.add_record(RegistryRecord {
        name: "__regex_NoChoice",
        export: false,
        description: "",
        props: vec![RecordProp {
            name: "none",
            ty: ParameterType::Boolean,
            description: "",
        }],
    });
    pkg.add_record(RegistryRecord {
        name: "__regex_Choice",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "kind",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "alt",
                ty: ParameterType::named("__regex_Node"),
                description: "",
            },
            RecordProp {
                name: "rep",
                ty: ParameterType::named("__regex_Repeat"),
                description: "",
            },
            RecordProp {
                name: "cont",
                ty: ParameterType::named("__regex_Cont"),
                description: "",
            },
            RecordProp {
                name: "pos",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "caps",
                ty: ParameterType::list_of(ParameterType::Integer),
                description: "",
            },
            RecordProp {
                name: "i",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "count",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "p",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::named("__regex_Choices"),
                description: "",
            },
        ],
    });
    pkg.add_union(RegistryUnion {
        name: "__regex_Choices",
        export: false,
        variants: vec![
            UnionVariant {
                name: "__regex_NoChoice",
                description: "",
            },
            UnionVariant {
                name: "__regex_Choice",
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Result",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "ok",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "pos",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "caps",
                ty: ParameterType::list_of(ParameterType::Integer),
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Ctx",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "cps",
                ty: ParameterType::list_of(ParameterType::Integer),
                description: "",
            },
            RecordProp {
                name: "n",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Program",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "root",
                ty: ParameterType::named("__regex_Node"),
                description: "",
            },
            RecordProp {
                name: "groups",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "names",
                ty: ParameterType::map_of(ParameterType::String, ParameterType::Integer),
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Parse",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "node",
                ty: ParameterType::named("__regex_Node"),
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "groups",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "names",
                ty: ParameterType::map_of(ParameterType::String, ParameterType::Integer),
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Paren",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "isDir",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "flags",
                ty: ParameterType::named("__regex_Flags"),
                description: "",
            },
            RecordProp {
                name: "node",
                ty: ParameterType::named("__regex_Node"),
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "groups",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "names",
                ty: ParameterType::map_of(ParameterType::String, ParameterType::Integer),
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Count",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "lo",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "hi",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_LitScalar",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "ch",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_PropParse",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "name",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "neg",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Endpoint",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "kind",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "ch",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "item",
                ty: ParameterType::named("__regex_ClassItem"),
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_FlagSpec",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "flags",
                ty: ParameterType::named("__regex_Flags"),
                description: "",
            },
            RecordProp {
                name: "any",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "term",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Name",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "name",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    // bug-532: the package's two PUBLIC value types, and the only exported names
    // here — everything above is a `__regex_`-prefixed engine internal. `Group` is
    // declared first because `MatchInfo` holds a list of them.
    pkg.add_record(RegistryRecord {
        name: "Group",
        export: true,
        description: "What one capturing group of a regular expression matched: where it matched, and the text it covered.",
        props: vec![
            RecordProp {
                name: "start",
                ty: ParameterType::Integer,
                description: "The zero-based scalar index of the group's first scalar, or -1 when the group took no part in the match.",
            },
            RecordProp {
                name: "endIndex",
                ty: ParameterType::Integer,
                description: "The zero-based scalar index one past the group's last scalar, so endIndex - start is the group's length in scalars. -1 when the group took no part in the match.",
            },
            RecordProp {
                name: "text",
                ty: ParameterType::String,
                description: "The text the group matched, which is empty both for a zero-length capture and for a group that took no part in the match.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "MatchInfo",
        export: true,
        description: "One regular-expression match: where it matched, the text it covered, and its capturing groups.",
        props: vec![
            RecordProp {
                name: "start",
                ty: ParameterType::Integer,
                description: "The zero-based scalar index of the match's first scalar, or -1 when there was no match.",
            },
            RecordProp {
                name: "endIndex",
                ty: ParameterType::Integer,
                description: "The zero-based scalar index one past the match's last scalar, so endIndex - start is the match's length in scalars. -1 when there was no match.",
            },
            RecordProp {
                name: "text",
                ty: ParameterType::String,
                description: "The text the match covered, the same text $0 inserts in a replacement template. Empty for a zero-length match and when there was no match.",
            },
            RecordProp {
                name: "groups",
                ty: ParameterType::list_of(ParameterType::named("Group")),
                description: "One Group per capturing group, indexed by group number; groups[0] restates the whole match, so the length is one more than the number of capturing groups in the pattern. Empty when there was no match.",
            },
            RecordProp {
                name: "names",
                ty: ParameterType::map_of(ParameterType::String, ParameterType::Integer),
                description: "Each named group's name mapped to its group number, for indexing groups by name instead of by counting parentheses. Empty when the pattern names no groups and when there was no match.",
            },
        ],
    });

    // The shared private `__regex_*` helpers the member bodies call. Each lives in
    // its own `helper_*.rs` and registers via `add_helper`; order preserved from the
    // old `package.mfb` blob so the compiled `.ncode` stays byte-identical.
    helper_chr::register(&mut pkg);
    helper_scalar_to_cp::register(&mut pkg);
    helper_to_scalars::register(&mut pkg);
    helper_make_ctx::register(&mut pkg);
    helper_cat_is_letter::register(&mut pkg);
    helper_cat_is_mark::register(&mut pkg);
    helper_is_space_cp::register(&mut pkg);
    helper_is_word_cp::register(&mut pkg);
    helper_is_word::register(&mut pkg);
    helper_shorthand_match::register(&mut pkg);
    helper_is_gc_name::register(&mut pkg);
    helper_script_canon::register(&mut pkg);
    helper_script_test::register(&mut pkg);
    helper_is_script_name::register(&mut pkg);
    helper_prop_test::register(&mut pkg);
    helper_canon_prop::register(&mut pkg);
    helper_class_match_one::register(&mut pkg);
    helper_prop_match_item::register(&mut pkg);
    helper_class_match::register(&mut pkg);
    helper_word_boundary::register(&mut pkg);
    helper_anchor_match::register(&mut pkg);
    helper_fail::register(&mut pkg);
    helper_set_cap::register(&mut pkg);
    helper_char_eq::register(&mut pkg);
    helper_run::register(&mut pkg);
    helper_steps::register(&mut pkg);
    helper_step_budget::register(&mut pkg);
    helper_parse_depth_limit::register(&mut pkg);
    helper_is_simple_node::register(&mut pkg);
    helper_simple_match_at::register(&mut pkg);
    helper_init_caps::register(&mut pkg);
    helper_try_at::register(&mut pkg);
    helper_search_from::register(&mut pkg);
    helper_is_digit::register(&mut pkg);
    helper_is_name_start::register(&mut pkg);
    helper_is_name_cont::register(&mut pkg);
    helper_is_ascii_punct::register(&mut pkg);
    helper_is_pat_space::register(&mut pkg);
    helper_parse_int_clamp::register(&mut pkg);
    helper_parse_hex_escape::register(&mut pkg);
    helper_parse_literal_escape::register(&mut pkg);
    helper_parse_prop::register(&mut pkg);
    helper_short_kind::register(&mut pkg);
    helper_parse_posix::register(&mut pkg);
    helper_posix_prop::register(&mut pkg);
    helper_parse_class_endpoint::register(&mut pkg);
    helper_parse_class::register(&mut pkg);
    helper_is_counted_at::register(&mut pkg);
    helper_parse_counted::register(&mut pkg);
    helper_parse_flag_spec::register(&mut pkg);
    helper_parse_name::register(&mut pkg);
    helper_parse_named_group::register(&mut pkg);
    helper_parse_paren::register(&mut pkg);
    helper_parse_escape_atom::register(&mut pkg);
    helper_parse_atom::register(&mut pkg);
    helper_parse_quant_suffix::register(&mut pkg);
    helper_parse_concat::register(&mut pkg);
    helper_parse_alt::register(&mut pkg);
    helper_compile::register(&mut pkg);
    helper_all_digits::register(&mut pkg);
    helper_lookup_num::register(&mut pkg);
    helper_lookup_name::register(&mut pkg);
    helper_lookup_ref::register(&mut pkg);
    helper_expand::register(&mut pkg);
    helper_match_results::register(&mut pkg);
    helper_ascii_class_bitset::register(&mut pkg);
    helper_make_class::register(&mut pkg);
    helper_required_first_cp::register(&mut pkg);
    helper_no_match::register(&mut pkg);
    helper_make_match::register(&mut pkg);

    // plan-118-B: the general-category and Script *scalar* tables are no longer
    // generated MFBASIC -- `regex::genCat` / `regex::scriptOf` look them up in
    // rodata instead of compiling 5,807 IF arms per program. What is left of the
    // generated file is `__regex_scriptCanonName`, which maps a lowercased script
    // NAME to its canonical spelling (171 arms, not a per-scalar lookup).
    func_gen_cat::register(&mut pkg);
    func_script_of::register(&mut pkg);
    pkg.add_helper(crate::codegen::registry::RegistryHelper::always(
        "regex_unicode_script_canon_name",
        include_str!("../../string/unicode/unicode_script_names.mfb"),
    ));

    func_find::register(&mut pkg);
    func_find_all::register(&mut pkg);
    func_match::register(&mut pkg);
    func_replace::register(&mut pkg);
    func_find_match::register(&mut pkg);
    func_find_all_matches::register(&mut pkg);
    func_count::register(&mut pkg);
    func_split::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use super::{is_tier_a_query, resolve_return_type};
    use crate::codegen::registry::{self, registry};

    /// plan-118-B split the member list in two, so this asserts the split by
    /// NAME rather than by a bare count: eight PUBLIC members (bug-532 added the
    /// two span-returning ones, bug-534 `count` and `split`), and two
    /// `internal_only` Unicode lookups the companion resolves through but user
    /// source must never reach. A bare count could not tell a new public member
    /// (a language change) from a new internal one (an implementation detail),
    /// and accidentally publishing `genCat` is exactly the mistake worth
    /// catching.
    #[test]
    fn regex_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("regex").expect("regex package");
        let mut public: Vec<&str> = pkg
            .functions()
            .iter()
            .filter(|function| !function.internal_only)
            .map(|function| function.name)
            .collect();
        public.sort_unstable();
        assert_eq!(
            public,
            [
                // bug-534: `count` and `split` complete the `strings` mirror.
                "count",
                "find",
                "findAll",
                "findAllMatches",
                "findMatch",
                "match",
                "replace",
                "split"
            ]
        );
        let mut internal: Vec<&str> = pkg
            .functions()
            .iter()
            .filter(|function| function.internal_only)
            .map(|function| function.name)
            .collect();
        internal.sort_unstable();
        assert_eq!(internal, ["genCat", "scriptOf"]);
        assert_eq!(pkg.functions().len(), 10);
    }

    /// bug-534: the Tier-A list and the public member list are two lists that
    /// must not drift. Every public member EXCEPT `replace` is a query and takes
    /// an `AttributedString` at `value`; `replace` is the one rewriter and is
    /// deliberately excluded, because remapping attribute spans across a pattern
    /// rewrite has no answer. A member added without a Tier-A row would silently
    /// lack the overload, and a member wrongly added to it would type-check a
    /// call the lowering cannot serve — this fails either way.
    #[test]
    fn every_public_query_member_is_tier_a_and_replace_is_not() {
        let pkg = registry().resolve_package("regex").expect("regex package");
        for function in pkg.functions() {
            if function.internal_only {
                continue;
            }
            let qualified = format!("regex.{}", function.name);
            let expected = function.name != "replace";
            assert_eq!(
                is_tier_a_query(&qualified),
                expected,
                "regex::{} tier-A membership",
                function.name
            );
        }
        // The list holds no name the package does not export.
        for name in [
            "regex.count",
            "regex.find",
            "regex.findAll",
            "regex.findAllMatches",
            "regex.findMatch",
            "regex.match",
            "regex.split",
        ] {
            assert!(
                registry().is_member(name),
                "tier-A list names {name}, which the registry does not export"
            );
        }
    }

    /// An `AttributedString` at `value` resolves a Tier-A query to exactly the
    /// `String` overload's return type, and leaves `replace` unresolvable.
    #[test]
    fn attributed_string_resolves_the_string_overloads_return() {
        use crate::types::ParameterType;
        let attributed = ParameterType::named("AttributedString");
        let cases: &[(&str, &str)] = &[
            ("regex.match", "Boolean"),
            ("regex.find", "Integer"),
            ("regex.count", "Integer"),
            ("regex.findAll", "List OF Integer"),
            ("regex.split", "List OF String"),
        ];
        for (name, expected) in cases {
            let args = vec![attributed.clone(), ParameterType::String];
            assert_eq!(
                resolve_return_type(name, &args, true)
                    .map(|t| t.name().into_owned())
                    .as_deref(),
                Some(*expected),
                "{name} through the AttributedString overload"
            );
        }
        assert_eq!(
            resolve_return_type(
                "regex.replace",
                &[attributed, ParameterType::String, ParameterType::String],
                true
            ),
            None,
            "regex::replace must reject an AttributedString rather than drop its attributes"
        );
    }

    #[test]
    fn generic_dispatch_reaches_regex() {
        assert!(registry().is_member("regex.match"));
        assert!(!registry().is_member("regex.nope"));
        assert_eq!(
            registry::rewrite_target("regex.find", &[]),
            Some("__regex_find")
        );
        assert_eq!(
            registry::rewrite_target("regex.findAll", &[]),
            Some("__regex_findAll")
        );
        assert_eq!(
            registry::call_return_type_typed("regex.match")
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("Boolean")
        );
        assert_eq!(
            registry::call_return_type_typed("regex.find")
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("Integer")
        );
        assert_eq!(
            registry::call_return_type_typed("regex.replace")
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("String")
        );
        // bug-532: the span-returning members return the package's own record
        // types, qualified, and mirror `find`/`findAll`'s optional `start`.
        assert_eq!(
            registry::rewrite_target("regex.findMatch", &[]),
            Some("__regex_findMatch")
        );
        assert_eq!(
            registry::rewrite_target("regex.findAllMatches", &[]),
            Some("__regex_findAllMatches")
        );
        assert_eq!(
            registry::call_return_type_typed("regex.findMatch")
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("regex.MatchInfo")
        );
        assert_eq!(
            registry::call_return_type_typed("regex.findAllMatches")
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("List OF regex.MatchInfo")
        );
        // match takes exactly 2 args; find/findAll's trailing `start` is optional.
        assert_eq!(registry().arity("regex.match"), Some((2, 2)));
        assert_eq!(registry().arity("regex.find"), Some((2, 3)));
        assert_eq!(registry().arity("regex.replace"), Some((3, 3)));
        assert_eq!(registry().arity("regex.findMatch"), Some((2, 3)));
        assert_eq!(registry().arity("regex.findAllMatches"), Some((2, 3)));
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry()
            .resolve_package("regex")
            .expect("regex")
            .get_mfb();
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-regex>"),
            "builtins/regex.mfb",
            &source,
        )
        .expect("reassembled regex source parses");
    }
}
