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
mod tests;
