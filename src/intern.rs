//! A process-wide string interner returning a `Copy` [`Symbol`].
//!
//! [`ParameterType`](crate::types::ParameterType)'s nominal/variable leaves used to
//! hold a `Box::leak`ed `&'static str`, leaking one string per distinct type
//! *spelling* every time [`parse`](crate::types::ParameterType::parse) hit its
//! fallback arm. That was tolerable while `ParameterType` only lived at the
//! low-frequency registry boundary, but plan-102 puts a `ParameterType` on every
//! HIR/IR node — at which point the per-spelling leak becomes unbounded.
//!
//! This interner replaces that leak with a bounded, deduplicated table: the same
//! set of distinct strings, interned *once* instead of leaked once per occurrence.
//! Equal strings map to equal [`Symbol`]s, so `Eq`/`Ord`/`Hash` on a `Symbol` are
//! integer operations, and a `ParameterType` leaf is `Copy`.
//!
//! The table is append-only and lives for the process lifetime, so a `Symbol` is
//! stable once minted and [`Symbol::resolve`] hands back a `&'static str` (the
//! interned copy is itself leaked to `'static`, so it outlives any lock).

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Mutex, OnceLock};

/// A `Copy` handle to an interned string. Interning equal strings yields equal
/// `Symbol`s, so equality/order/hash are integer operations. Backed by
/// [`NonZeroU32`] so `Option<Symbol>` niche-packs to four bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Symbol(NonZeroU32);

/// The interner's backing store: a dedup map from the interned `&'static str` to its
/// `Symbol`, and an index → string table for resolution. Append-only.
struct Interner {
    map: HashMap<&'static str, Symbol>,
    strings: Vec<&'static str>,
}

fn interner() -> &'static Mutex<Interner> {
    static INTERNER: OnceLock<Mutex<Interner>> = OnceLock::new();
    INTERNER.get_or_init(|| {
        Mutex::new(Interner {
            map: HashMap::new(),
            strings: Vec::new(),
        })
    })
}

impl Symbol {
    /// Intern `s`, returning its `Symbol`. Interning the same string again returns
    /// the same `Symbol` without allocating; a new string is leaked to `'static`
    /// once and recorded.
    pub(crate) fn intern(s: &str) -> Symbol {
        let mut interner = interner().lock().expect("intern table poisoned");
        if let Some(&sym) = interner.map.get(s) {
            return sym;
        }
        // Leak the backing storage once, so `resolve` can hand out a `&'static str`
        // that outlives the lock. This is the whole table's only allocation per
        // distinct string — deduplicated, not per-occurrence.
        let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
        // `strings.len()` is the 0-based index; the `Symbol` stores index + 1 so it
        // is non-zero. `u32` is ample: the table holds distinct type spellings.
        let id =
            NonZeroU32::new(interner.strings.len() as u32 + 1).expect("interner overflowed u32");
        let sym = Symbol(id);
        interner.strings.push(leaked);
        interner.map.insert(leaked, sym);
        sym
    }

    /// Resolve back to the interned string. The stored value is `&'static`, so it is
    /// returned by copy and outlives the lock.
    pub(crate) fn resolve(self) -> &'static str {
        let interner = interner().lock().expect("intern table poisoned");
        interner.strings[(self.0.get() - 1) as usize]
    }
}

impl std::fmt::Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show the resolved text, so `ParameterType`'s derived `Debug` stays legible
        // (`Named("CsvReader")` rather than `Named(Symbol(42))`).
        write!(f, "{:?}", self.resolve())
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.resolve())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_string_interns_equal() {
        let a = Symbol::intern("CsvReader");
        let b = Symbol::intern("CsvReader");
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_strings_intern_distinct() {
        let a = Symbol::intern("List OF Integer");
        let b = Symbol::intern("Map OF String TO Integer");
        assert_ne!(a, b);
    }

    #[test]
    fn resolve_round_trips() {
        // Composite/nominal names must survive the round-trip exactly — these are the
        // shapes `ParameterType::Named`/`Var` carry.
        for s in [
            "T",
            "K",
            "V",
            "CsvReader",
            "fs.File",
            "File STATE Cursor",
            "some.deeply.qualified.Name",
            "",
        ] {
            assert_eq!(Symbol::intern(s).resolve(), s);
        }
    }

    #[test]
    fn resolve_is_stable_across_reintern() {
        let first = Symbol::intern("stable.name");
        // Interning many other strings must not shift the earlier symbol's mapping.
        for i in 0..64 {
            Symbol::intern(&format!("filler{i}"));
        }
        assert_eq!(first.resolve(), "stable.name");
        assert_eq!(Symbol::intern("stable.name"), first);
    }

    #[test]
    fn debug_shows_resolved_text() {
        let sym = Symbol::intern("Widget");
        assert_eq!(format!("{sym:?}"), "\"Widget\"");
    }
}
