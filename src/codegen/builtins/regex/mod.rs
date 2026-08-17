//! Package: json
//! Type: Pure MFBasic
//! Plan: plan-72-T

use crate::codegen::registry::{
    RecordProp, Registry, RegistryPackage, RegistryRecord, RegistryUnion, UnionVariant,
};
use crate::types::ParameterType;

mod func_find;
mod func_find_all;
mod func_match;
mod func_replace;

const INTRO: &str = r#"Match, search, and replace text with regular expressions"#;

const DESC: &str = r#"The `regex` package searches and rewrites text with a single portable
regular-expression dialect that is MFBASIC's own. Its syntax and semantics are
defined entirely by `mfb spec stdlib regex` and produce byte-for-byte identical
results on every target, never deferring to a host libc, locale, or OS regex
library. `regex` is a built-in package: `IMPORT regex` needs no manifest
dependency. For the full pattern language, run `mfb man regex language`.

The package defines no new types. `pattern` and `replacement` are ordinary
runtime `String` values, so they may be literals, built at run time, or read from
input; a pattern is compiled at the moment a function is called. An invalid
pattern fails the call with `ErrInvalidFormat` rather than being silently treated
as "no match". Because MFBASIC `String` literals process their own backslash
escapes, a backslash the regex needs is written `"\\"` in a source literal
(`"\\d"` is the pattern `\d`); a pattern read from a file or user input has no
such doubling.

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
match at or after `start`, or `-1` when there is none; `findAll` returns a
`List OF Integer` of the start index of every non-overlapping match; and
`replace` returns a new `String` with every non-overlapping match rewritten by a
replacement template. Every search is unanchored and leftmost: the reported match
is the one beginning at the smallest position where any match exists. `find` and
`findAll` take an optional `start` (default `0`) restricting only where a match
may begin — the absolute anchors `\A`, `\z`, and unflagged `^`/`$` are still
evaluated against the whole value. A zero-length match is valid; iteration
advances one scalar past an empty match so it always terminates.

No `regex` function fails on the absence of a match: `match` returns `FALSE`,
`find` returns `-1`, `findAll` returns an empty list, and `replace` returns
`value` unchanged. `ErrNotFound` is never raised by this package. None of the
functions mutate their arguments or have side effects."#;

pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("regex", INTRO, DESC);

    pkg.add_imports(vec!["collections", "strings", "encoding"]);

    pkg.add_record(RegistryRecord {
        name: "__regex_Flags",
        export: false,
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
        props: vec![RecordProp {
            name: "ch",
            ty: ParameterType::String,
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Short",
        export: false,
        props: vec![RecordProp {
            name: "kind",
            ty: ParameterType::Integer,
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Prop",
        export: false,
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
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Any",
        export: false,
        props: vec![RecordProp {
            name: "dotall",
            ty: ParameterType::Boolean,
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Class",
        export: false,
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
                ty: ParameterType::list_of(ParameterType::Named("__regex_ClassItem")),
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
        props: vec![RecordProp {
            name: "parts",
            ty: ParameterType::list_of(ParameterType::Named("__regex_Node")),
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Alt",
        export: false,
        props: vec![RecordProp {
            name: "opts",
            ty: ParameterType::list_of(ParameterType::Named("__regex_Node")),
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_Repeat",
        export: false,
        props: vec![
            RecordProp {
                name: "child",
                ty: ParameterType::Named("__regex_Node"),
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
        props: vec![
            RecordProp {
                name: "child",
                ty: ParameterType::Named("__regex_Node"),
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
        props: vec![RecordProp {
            name: "dummy",
            ty: ParameterType::Boolean,
            description: "",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_ContSeq",
        export: false,
        props: vec![
            RecordProp {
                name: "parts",
                ty: ParameterType::list_of(ParameterType::Named("__regex_Node")),
                description: "",
            },
            RecordProp {
                name: "idx",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::Named("__regex_Cont"),
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_ContCap",
        export: false,
        props: vec![
            RecordProp {
                name: "slot",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "nxt",
                ty: ParameterType::Named("__regex_Cont"),
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__regex_ContRep",
        export: false,
        props: vec![
            RecordProp {
                name: "rep",
                ty: ParameterType::Named("__regex_Repeat"),
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
                ty: ParameterType::Named("__regex_Cont"),
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

    pkg.add_record(RegistryRecord {
        name: "__regex_Result",
        export: false,
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
        props: vec![
            RecordProp {
                name: "text",
                ty: ParameterType::list_of(ParameterType::String),
                description: "",
            },
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
        props: vec![
            RecordProp {
                name: "root",
                ty: ParameterType::Named("__regex_Node"),
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
        props: vec![
            RecordProp {
                name: "node",
                ty: ParameterType::Named("__regex_Node"),
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
        props: vec![
            RecordProp {
                name: "isDir",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "flags",
                ty: ParameterType::Named("__regex_Flags"),
                description: "",
            },
            RecordProp {
                name: "node",
                ty: ParameterType::Named("__regex_Node"),
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
                ty: ParameterType::Named("__regex_ClassItem"),
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
        props: vec![
            RecordProp {
                name: "flags",
                ty: ParameterType::Named("__regex_Flags"),
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

    // The shared private `__regex_*` helpers the member bodies call: the engine
    // helpers plus the two generated Unicode tables shared from `src/codegen/unicode/`
    // (the general-category table `__regex_genCat` and the Script-property table
    // `__regex_scriptOf` / `__regex_scriptCanonName`).
    pkg.add_helper(crate::codegen::registry::RegistryHelper::always(
        "regex_package",
        include_str!("package.mfb"),
    ));
    pkg.add_helper(crate::codegen::registry::RegistryHelper::always(
        "regex_unicode_gencat",
        include_str!("../../unicode/unicode_gencat.mfb"),
    ));
    pkg.add_helper(crate::codegen::registry::RegistryHelper::always(
        "regex_unicode_script_of",
        include_str!("../../unicode/unicode_script_of.mfb"),
    ));

    func_find::register(&mut pkg);
    func_find_all::register(&mut pkg);
    func_match::register(&mut pkg);
    func_replace::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    #[test]
    fn regex_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("regex").expect("regex package");
        assert_eq!(pkg.functions().len(), 4);
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
        assert_eq!(registry::call_return_type("regex.match"), Some("Boolean"));
        assert_eq!(registry::call_return_type("regex.find"), Some("Integer"));
        assert_eq!(registry::call_return_type("regex.replace"), Some("String"));
        // match takes exactly 2 args; find/findAll's trailing `start` is optional.
        assert_eq!(registry().arity("regex.match"), Some((2, 2)));
        assert_eq!(registry().arity("regex.find"), Some((2, 3)));
        assert_eq!(registry().arity("regex.replace"), Some((3, 3)));
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
