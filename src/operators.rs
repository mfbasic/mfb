//! The compiler's operator vocabulary.
//!
//! [`BinaryOp`] and [`UnaryOp`] are the *only* representation of a language
//! operator anywhere after `src/ast/expr.rs` mints one. Every stage — HIR, IR,
//! NIR, the optimizer, codegen — carries the enum and `match`es it
//! exhaustively, so rustc, not a runtime `format!("unknown operator …")`, is
//! what proves the set is covered.
//!
//! Unlike [`crate::types::ParameterType`], this vocabulary is **closed**: MFB
//! has no operator overloading, no user-declared operator, and no syntax that
//! turns an identifier into one. The parser mints an operator only from a fixed
//! [`TokenKind`]/[`Keyword`], so there is nothing to intern, nothing to
//! recurse through, and no user-extensible leaf. A `u8`-sized `Copy` enum is
//! the whole representation.
//!
//! Two enums rather than one, because arity is a real distinction: a single
//! `Operator` would re-admit `Binary { op: Not }`, which is exactly the illegal
//! state this module exists to make unrepresentable.
//!
//! ## The spellings are a wire format
//!
//! [`BinaryOp::name`]/[`UnaryOp::name`] are not cosmetic. They are rendered
//! verbatim into three committed sinks — the `.ast` JSON
//! (`src/ast/serialize.rs`), the `.ir` JSON (`src/ir/json.rs`), and the
//! length-prefixed operator string in the IR binary package format
//! (`src/ir/binary.rs`) — and pinned by the `tests/**` golden corpus. Changing
//! a returned string is a format break, not a rename.

use crate::lexer::{Keyword, TokenKind};

/// A binary (two-operand) language operator.
///
/// The 17 spellings the parser can mint, from `src/ast/expr.rs`'s precedence
/// ladder: `parse_or` (`OR`, `XOR`), `parse_and` (`AND`), `parse_comparison`
/// (`=`, `<>`, `<`, `<=`, `>`, `>=`), `parse_concat` (`&`), `parse_addition`
/// (`+`, `-`), `parse_multiplication` (`*`, `/`, `MOD`, `DIV`) and
/// `parse_power` (`^`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum BinaryOp {
    /// `OR` — logical or bitwise disjunction.
    Or,
    /// `XOR` — logical or bitwise exclusive disjunction.
    Xor,
    /// `AND` — logical or bitwise conjunction.
    And,
    /// `=` — equality.
    Equal,
    /// `<>` — inequality.
    NotEqual,
    /// `<` — ordered less-than.
    Less,
    /// `<=` — ordered less-than-or-equal.
    LessEqual,
    /// `>` — ordered greater-than.
    Greater,
    /// `>=` — ordered greater-than-or-equal.
    GreaterEqual,
    /// `&` — string concatenation.
    Concat,
    /// `+` — addition.
    Add,
    /// `-` — subtraction.
    Subtract,
    /// `*` — multiplication.
    Multiply,
    /// `/` — division.
    Divide,
    /// `MOD` — remainder.
    Mod,
    /// `DIV` — integer (truncating) division.
    IntDiv,
    /// `^` — exponentiation.
    Power,
}

/// A unary (one-operand) language operator.
///
/// `NOT` is minted by `parse_not` and by `src/ast/link_items.rs`'s `ERROR_ON`
/// De Morgan negation; `-` by `parse_unary`. `SIZEOF` is LINK-only: it appears
/// solely in a `CONST … = SIZEOF <CSTRUCT>` pin and folds to an integer during
/// LINK lowering, before any serialization sink sees it.
///
/// There is deliberately no `Plus`: no parser path mints a unary `+`, so the
/// missing variant is what deletes the dead arms that used to handle one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum UnaryOp {
    /// `NOT` — logical or bitwise negation.
    Not,
    /// `-` — arithmetic negation.
    Negate,
    /// `SIZEOF` — the byte size of a `CSTRUCT`, folded at LINK lowering.
    SizeOf,
}

impl BinaryOp {
    /// The operator's source spelling.
    ///
    /// This is the render used at all three serialization sinks; see the module
    /// docs — the strings are a committed format, not a display convenience.
    pub(crate) fn name(self) -> &'static str {
        match self {
            BinaryOp::Or => "OR",
            BinaryOp::Xor => "XOR",
            BinaryOp::And => "AND",
            BinaryOp::Equal => "=",
            BinaryOp::NotEqual => "<>",
            BinaryOp::Less => "<",
            BinaryOp::LessEqual => "<=",
            BinaryOp::Greater => ">",
            BinaryOp::GreaterEqual => ">=",
            BinaryOp::Concat => "&",
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::Mod => "MOD",
            BinaryOp::IntDiv => "DIV",
            BinaryOp::Power => "^",
        }
    }

    /// Recover an operator from its spelling.
    ///
    /// This exists for **decode boundaries only** — the IR binary package
    /// format (`src/ir/binary.rs`), which carries the operator as a
    /// length-prefixed string and must reject anything outside the set rather
    /// than silently mis-lower it (bug-403). No compiler stage should reach for
    /// `parse` to make a decision: the stages carry the enum.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "OR" => BinaryOp::Or,
            "XOR" => BinaryOp::Xor,
            "AND" => BinaryOp::And,
            "=" => BinaryOp::Equal,
            "<>" => BinaryOp::NotEqual,
            "<" => BinaryOp::Less,
            "<=" => BinaryOp::LessEqual,
            ">" => BinaryOp::Greater,
            ">=" => BinaryOp::GreaterEqual,
            "&" => BinaryOp::Concat,
            "+" => BinaryOp::Add,
            "-" => BinaryOp::Subtract,
            "*" => BinaryOp::Multiply,
            "/" => BinaryOp::Divide,
            "MOD" => BinaryOp::Mod,
            "DIV" => BinaryOp::IntDiv,
            "^" => BinaryOp::Power,
            _ => return None,
        })
    }

    /// The operator a token spells, when it spells one.
    ///
    /// The parser's precedence ladder has already decided *which* tokens are
    /// admissible at each rung; this maps the accepted token onto its operator
    /// so no rung has to name a spelling.
    pub(crate) fn from_token(kind: &TokenKind) -> Option<Self> {
        Some(match kind {
            TokenKind::Keyword(Keyword::Or) => BinaryOp::Or,
            TokenKind::Keyword(Keyword::Xor) => BinaryOp::Xor,
            TokenKind::Keyword(Keyword::And) => BinaryOp::And,
            TokenKind::Equal => BinaryOp::Equal,
            TokenKind::NotEqual => BinaryOp::NotEqual,
            TokenKind::Less => BinaryOp::Less,
            TokenKind::LessEqual => BinaryOp::LessEqual,
            TokenKind::Greater => BinaryOp::Greater,
            TokenKind::GreaterEqual => BinaryOp::GreaterEqual,
            TokenKind::Ampersand => BinaryOp::Concat,
            TokenKind::Plus => BinaryOp::Add,
            TokenKind::Minus => BinaryOp::Subtract,
            TokenKind::Star => BinaryOp::Multiply,
            TokenKind::Slash => BinaryOp::Divide,
            TokenKind::Keyword(Keyword::Mod) => BinaryOp::Mod,
            TokenKind::Keyword(Keyword::Div) => BinaryOp::IntDiv,
            TokenKind::Caret => BinaryOp::Power,
            _ => return None,
        })
    }

    /// Whether this is one of the six comparisons `= <> < > <= >=`.
    ///
    /// The LINK wire format admits exactly this subset in an
    /// `IrLinkExpr::Compare` (`src/ir/link.rs`), and several codegen paths
    /// branch on "is a comparison" before dispatching on which one.
    pub(crate) fn is_comparison(self) -> bool {
        matches!(
            self,
            BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
        )
    }
}

impl UnaryOp {
    /// The operator's source spelling. See [`BinaryOp::name`].
    pub(crate) fn name(self) -> &'static str {
        match self {
            UnaryOp::Not => "NOT",
            UnaryOp::Negate => "-",
            UnaryOp::SizeOf => "SIZEOF",
        }
    }

    /// Recover an operator from its spelling. See [`BinaryOp::parse`] — this is
    /// for decode boundaries only.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "NOT" => UnaryOp::Not,
            "-" => UnaryOp::Negate,
            "SIZEOF" => UnaryOp::SizeOf,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant, written out. Deliberately not derived from an iterator or
    /// a helper: an enumeration that can silently skip a variant is not a
    /// coverage proof. Adding a variant without adding it here leaves
    /// `every_variant_is_listed` red.
    const ALL_BINARY: &[BinaryOp] = &[
        BinaryOp::Or,
        BinaryOp::Xor,
        BinaryOp::And,
        BinaryOp::Equal,
        BinaryOp::NotEqual,
        BinaryOp::Less,
        BinaryOp::LessEqual,
        BinaryOp::Greater,
        BinaryOp::GreaterEqual,
        BinaryOp::Concat,
        BinaryOp::Add,
        BinaryOp::Subtract,
        BinaryOp::Multiply,
        BinaryOp::Divide,
        BinaryOp::Mod,
        BinaryOp::IntDiv,
        BinaryOp::Power,
    ];

    const ALL_UNARY: &[UnaryOp] = &[UnaryOp::Not, UnaryOp::Negate, UnaryOp::SizeOf];

    /// `ALL_BINARY`/`ALL_UNARY` are hand-written, so something has to prove they
    /// are complete. An exhaustive `match` does: adding a variant makes this fn
    /// fail to compile until the constant is extended too.
    #[test]
    fn every_variant_is_listed() {
        fn binary_is_listed(op: BinaryOp) -> bool {
            match op {
                BinaryOp::Or
                | BinaryOp::Xor
                | BinaryOp::And
                | BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::Concat
                | BinaryOp::Add
                | BinaryOp::Subtract
                | BinaryOp::Multiply
                | BinaryOp::Divide
                | BinaryOp::Mod
                | BinaryOp::IntDiv
                | BinaryOp::Power => true,
            }
        }
        fn unary_is_listed(op: UnaryOp) -> bool {
            match op {
                UnaryOp::Not | UnaryOp::Negate | UnaryOp::SizeOf => true,
            }
        }
        assert_eq!(ALL_BINARY.len(), 17, "ALL_BINARY lost or gained a variant");
        assert_eq!(ALL_UNARY.len(), 3, "ALL_UNARY lost or gained a variant");
        assert!(ALL_BINARY.iter().copied().all(binary_is_listed));
        assert!(ALL_UNARY.iter().copied().all(unary_is_listed));
    }

    #[test]
    fn round_trip() {
        for op in ALL_BINARY {
            assert_eq!(
                BinaryOp::parse(op.name()),
                Some(*op),
                "BinaryOp::{op:?} does not round-trip through {:?}",
                op.name()
            );
        }
        for op in ALL_UNARY {
            assert_eq!(
                UnaryOp::parse(op.name()),
                Some(*op),
                "UnaryOp::{op:?} does not round-trip through {:?}",
                op.name()
            );
        }
    }

    /// Two variants sharing a spelling would make `parse` lossy in a way
    /// `round_trip` alone cannot see (it would still return *a* variant).
    #[test]
    fn spellings_are_distinct() {
        let mut binary: Vec<&str> = ALL_BINARY.iter().map(|op| op.name()).collect();
        binary.sort_unstable();
        let count = binary.len();
        binary.dedup();
        assert_eq!(
            binary.len(),
            count,
            "two BinaryOp variants share a spelling"
        );

        let mut unary: Vec<&str> = ALL_UNARY.iter().map(|op| op.name()).collect();
        unary.sort_unstable();
        let count = unary.len();
        unary.dedup();
        assert_eq!(unary.len(), count, "two UnaryOp variants share a spelling");
    }

    #[test]
    fn parse_rejects_non_operators() {
        for text in [
            "GARBAGE", "", "and", "or", "==", "!=", "%", "mod", " +", "+ ",
        ] {
            assert_eq!(BinaryOp::parse(text), None, "BinaryOp::parse({text:?})");
        }
        for text in ["GARBAGE", "", "not", "+", "sizeof", "--"] {
            assert_eq!(UnaryOp::parse(text), None, "UnaryOp::parse({text:?})");
        }
    }

    /// No parser path mints a unary `+`, and `UnaryOp` has no variant for one.
    /// Pinned here because two stages used to carry dead arms handling it.
    #[test]
    fn there_is_no_unary_plus() {
        assert_eq!(UnaryOp::parse("+"), None);
        assert!(ALL_UNARY.iter().all(|op| op.name() != "+"));
    }

    #[test]
    fn from_token_agrees_with_name() {
        use crate::lexer::{Keyword, TokenKind};
        let cases: &[(TokenKind, BinaryOp)] = &[
            (TokenKind::Keyword(Keyword::Or), BinaryOp::Or),
            (TokenKind::Keyword(Keyword::Xor), BinaryOp::Xor),
            (TokenKind::Keyword(Keyword::And), BinaryOp::And),
            (TokenKind::Equal, BinaryOp::Equal),
            (TokenKind::NotEqual, BinaryOp::NotEqual),
            (TokenKind::Less, BinaryOp::Less),
            (TokenKind::LessEqual, BinaryOp::LessEqual),
            (TokenKind::Greater, BinaryOp::Greater),
            (TokenKind::GreaterEqual, BinaryOp::GreaterEqual),
            (TokenKind::Ampersand, BinaryOp::Concat),
            (TokenKind::Plus, BinaryOp::Add),
            (TokenKind::Minus, BinaryOp::Subtract),
            (TokenKind::Star, BinaryOp::Multiply),
            (TokenKind::Slash, BinaryOp::Divide),
            (TokenKind::Keyword(Keyword::Mod), BinaryOp::Mod),
            (TokenKind::Keyword(Keyword::Div), BinaryOp::IntDiv),
            (TokenKind::Caret, BinaryOp::Power),
        ];
        assert_eq!(cases.len(), ALL_BINARY.len(), "a variant has no token case");
        for (kind, expected) in cases {
            assert_eq!(BinaryOp::from_token(kind), Some(*expected), "{kind:?}");
        }
        // A token that spells no operator.
        assert_eq!(BinaryOp::from_token(&TokenKind::Comma), None);
        assert_eq!(BinaryOp::from_token(&TokenKind::ColonEqual), None);
        assert_eq!(BinaryOp::from_token(&TokenKind::PipeGreater), None);
    }

    #[test]
    fn is_comparison_is_exactly_the_link_wire_set() {
        let comparisons: Vec<&str> = ALL_BINARY
            .iter()
            .filter(|op| op.is_comparison())
            .map(|op| op.name())
            .collect();
        assert_eq!(comparisons, vec!["=", "<>", "<", "<=", ">", ">="]);
    }

    /// The spellings are a committed format: 793 `.ir` and 793 `.ast` goldens
    /// carry them verbatim. This walks the corpus and asserts every operator
    /// string in it parses to a variant whose `name()` is byte-equal to the
    /// input — so a one-byte drift in `name()` fails here, before any carrier
    /// depends on it, instead of churning 1586 goldens.
    #[test]
    fn golden_corpus_spellings_all_round_trip() {
        use std::path::Path;

        fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("ir") | Some("ast")
                ) {
                    out.push(path);
                }
            }
        }

        // The corpus lives beside the crate root, not the process cwd.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let mut files = Vec::new();
        walk(&root, &mut files);
        assert!(
            files.len() > 500,
            "expected the golden corpus, found {} file(s) under {}",
            files.len(),
            root.display()
        );

        // `"op"` (in `.ir`) is an overloaded key: it also tags a statement kind
        // (`bind`, `if`, `while`, `forEach`, …). So the extraction anchors on
        // the node instead of the key — every binary/unary node is rendered
        // `{ "kind": "binary", "type": …, "op": … }` (`src/ir/json.rs`) or
        // `{ "kind": "binary", "operator": …, … }` (`src/ast/serialize.rs`),
        // with the operator always preceding the child operands. Taking the
        // first operator-bearing key after the kind marker is therefore exact,
        // not heuristic.
        let mut seen: Vec<String> = Vec::new();
        let mut checked = 0usize;
        for file in &files {
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            for marker in ["\"kind\": \"binary\"", "\"kind\": \"unary\""] {
                let mut rest = text.as_str();
                while let Some(at) = rest.find(marker) {
                    rest = &rest[at + marker.len()..];
                    // Bounded window: the operator key sits a few dozen bytes
                    // after the kind marker (only `"type"` may intervene), and
                    // an unbounded `find` for a key a `.ir` file never contains
                    // rescans the whole file per node — quadratic over 1586
                    // corpus files.
                    let mut bound = rest.len().min(256);
                    while !rest.is_char_boundary(bound) {
                        bound += 1;
                    }
                    let window = &rest[..bound];
                    let key_at = ["\"op\": \"", "\"operator\": \""]
                        .iter()
                        .filter_map(|key| window.find(key).map(|at| (at, key.len())))
                        .min();
                    let Some((at, key_len)) = key_at else {
                        panic!(
                            "{}: a {marker} node carries no operator key",
                            file.display()
                        );
                    };
                    let value = &rest[at + key_len..];
                    let Some(end) = value.find('"') else {
                        panic!("{}: unterminated operator spelling", file.display());
                    };
                    let spelling = &value[..end];
                    let parsed = if marker.ends_with("binary\"") {
                        BinaryOp::parse(spelling).map(|op| op.name())
                    } else {
                        UnaryOp::parse(spelling).map(|op| op.name())
                    };
                    let Some(rendered) = parsed else {
                        panic!(
                            "{}: {marker} spelling {spelling:?} is in the golden corpus but \
                             parses to no operator variant",
                            file.display()
                        );
                    };
                    assert_eq!(
                        rendered,
                        spelling,
                        "{}: name() renders {rendered:?} for corpus spelling {spelling:?}",
                        file.display()
                    );
                    if !seen.iter().any(|s| s == spelling) {
                        seen.push(spelling.to_string());
                    }
                    checked += 1;
                }
            }
        }
        assert!(
            checked > 1000,
            "only {checked} operator spelling(s) found in {} corpus file(s) — the \
             extraction stopped matching the golden format",
            files.len()
        );
        seen.sort_unstable();
        // A record of what the corpus actually exercises, so a variant that no
        // golden covers is visible rather than assumed.
        eprintln!("corpus operator spellings ({}): {seen:?}", seen.len());
    }
}
