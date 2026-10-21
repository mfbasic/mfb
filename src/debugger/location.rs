//! The places and values the prompt names, and the tests `breakpointif` makes.
//!
//! A [`Location`] is a term with an optional offset: a number (`0x…`), a name
//! — a register of the thread, else a function, which stands for its start —
//! or `[<location>]`, the 8 bytes in memory there. Its value is the register's
//! value, the number, the function's address or the bytes read, plus the
//! offset: `sp+0x18` is a number, `[sp+0x18]` what is stored there, and
//! `[sp]+0x18` what is stored at `sp`, plus 0x18. `show mem`, `wp` and `dis`
//! read memory at a location's value; a [`Condition`] compares it.

use super::{Inferior, RegisterValue};

/// What a [`Location`] starts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Term {
    /// A number, written `0x…` (perhaps with an offset, already added in).
    Number(u64),
    /// A register of the thread when it has one so named, else a function.
    Name(String),
    /// `[<location>]`: the 8 bytes, little-endian, at the location's value.
    Memory(Box<Location>),
}

/// A term plus an offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Location {
    pub(crate) term: Term,
    pub(crate) offset: i64,
}

/// Why a location has no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fault {
    /// It names something there is not: a register or function no one has, a
    /// vector register, a name that is several functions.
    Unknown(String),
    /// Its memory cannot be read, or an offset goes outside the address space.
    Unreadable(String),
}

impl Fault {
    pub(crate) fn message(self) -> String {
        match self {
            Fault::Unknown(message) | Fault::Unreadable(message) => message,
        }
    }
}

/// A location's value, and whether a register went into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Value {
    pub(crate) value: u64,
    pub(crate) register: bool,
}

/// Whether `byte` can be part of a register's or a function's name: a symbol
/// (`_mfb_fn_main`) or a written name (`datetime::#normInstant`, `f$own1`).
fn name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'#' | b'$' | b'.')
}

fn not_a_location(word: &str) -> String {
    format!("'{word}' is not an address, a register or a function")
}

/// `+<n>` or `-<n>` (`n` decimal or `0x` hex) as a signed offset.
fn offset(word: &str, text: &str) -> Result<i64, String> {
    let amount = super::prompt::number(&text[1..])?;
    let amount = i64::try_from(amount).map_err(|_| format!("'{word}' is too far"))?;
    Ok(if text.starts_with('-') {
        -amount
    } else {
        amount
    })
}

impl Location {
    /// `0x…`, `<name>` or `[<location>]`, each perhaps followed by `+<n>` or
    /// `-<n>` (`n` decimal or `0x` hex), where `<name>` is a register or a
    /// function. A number's offset is added in now.
    pub(crate) fn parse(word: &str) -> Result<Location, String> {
        if let Some(inside) = word.strip_prefix('[') {
            let mut depth = 1usize;
            let close = inside.char_indices().find_map(|(at, character)| {
                match character {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(at)
            });
            let Some(close) = close else {
                return Err(format!("'{word}' opens a '[' it does not close"));
            };
            let (inner, rest) = (&inside[..close], &inside[close + 1..]);
            if inner.is_empty() {
                return Err(not_a_location(word));
            }
            let offset = match rest.chars().next() {
                None => 0,
                Some('+' | '-') => offset(word, rest)?,
                Some(_) => return Err(not_a_location(word)),
            };
            return Ok(Location {
                term: Term::Memory(Box::new(Location::parse(inner)?)),
                offset,
            });
        }
        if let Some(address) = super::prompt::address(word) {
            return address.map(|address| Location {
                term: Term::Number(address),
                offset: 0,
            });
        }
        let (name, offset) = match word.find(['+', '-']) {
            Some(at) => (&word[..at], offset(word, &word[at..])?),
            None => (word, 0),
        };
        if name.is_empty() || !name.bytes().all(name_byte) {
            return Err(not_a_location(word));
        }
        Ok(Location {
            term: Term::Name(name.to_string()),
            offset,
        })
    }

    /// A number, with no offset.
    pub(crate) fn number(value: u64) -> Location {
        Location {
            term: Term::Number(value),
            offset: 0,
        }
    }

    /// A name, with an offset.
    #[cfg(test)]
    pub(crate) fn named(name: &str, offset: i64) -> Location {
        Location {
            term: Term::Name(name.to_string()),
            offset,
        }
    }

    /// The location as a message writes it: a number in full-width hex, an
    /// offset in hex (`[sp+0x18]`, `[x0]-0x8`, `0x0000000012348593`).
    pub(crate) fn shown(&self) -> String {
        let base = match &self.term {
            Term::Number(value) => format!("0x{value:016x}"),
            Term::Name(name) => name.clone(),
            Term::Memory(inner) => format!("[{}]", inner.shown()),
        };
        match self.offset {
            0 => base,
            offset if offset < 0 => format!("{base}-{:#x}", offset.unsigned_abs()),
            offset => format!("{base}+{offset:#x}"),
        }
    }

    /// Whether working out the value reads memory (`[…]` anywhere in it).
    pub(crate) fn reads_memory(&self) -> bool {
        match &self.term {
            Term::Memory(_) => true,
            Term::Number(_) | Term::Name(_) => false,
        }
    }

    /// The location's value for thread `thread`.
    pub(crate) fn value(&self, inferior: &mut dyn Inferior, thread: usize) -> Result<Value, Fault> {
        let base = match &self.term {
            Term::Number(value) => Value {
                value: *value,
                register: false,
            },
            Term::Name(name) => name_value(inferior, thread, name)?,
            Term::Memory(inner) => {
                let at = inner.value(inferior, thread)?;
                let bytes = inferior
                    .read_bytes(at.value, 8)
                    .map_err(Fault::Unreadable)?;
                let word = bytes
                    .get(..8)
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                    .ok_or_else(|| {
                        Fault::Unreadable(format!("cannot read 8 bytes at 0x{:016x}", at.value))
                    })?;
                Value {
                    value: u64::from_le_bytes(word),
                    register: at.register,
                }
            }
        };
        let value = base.value.checked_add_signed(self.offset).ok_or_else(|| {
            Fault::Unreadable(format!("'{}' is outside the address space", self.shown()))
        })?;
        Ok(Value { value, ..base })
    }
}

/// A name's value: thread `thread`'s register so named (64 bits only), else
/// the start of the one function the name names.
fn name_value(inferior: &mut dyn Inferior, thread: usize, name: &str) -> Result<Value, Fault> {
    let registers = if (inferior.arch().is_register)(name) {
        inferior.registers(thread).map_err(Fault::Unreadable)?
    } else {
        inferior.registers(thread).unwrap_or_default()
    };
    if let Some(register) = registers.iter().find(|register| register.name == name) {
        let RegisterValue::Bits64(value) = register.value else {
            return Err(Fault::Unknown(format!(
                "'{name}' is a vector register, not an address"
            )));
        };
        return Ok(Value {
            value,
            register: true,
        });
    }
    let start = super::prompt::function_start(inferior, name).map_err(|error| {
        Fault::Unknown(if error.starts_with("no function is named") {
            format!(
                "no register or function is named '{name}'; type 'list reg' or 'list func' for \
                 them"
            )
        } else {
            error
        })
    })?;
    Ok(Value {
        value: start,
        register: false,
    })
}

/// A value as `breakpointif` and `jump` take one: a whole number, perhaps
/// negative, in decimal or `0x` hex (`5`, `-1`, `0x10`, `-0x10`), as the 64
/// bits it is.
pub(crate) fn signed(word: &str) -> Result<u64, String> {
    let bad = || format!("'{word}' is not a value: give a number, such as 5, -1 or 0x10");
    let (negative, digits) = match word.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, word),
    };
    let magnitude = super::prompt::number(digits).map_err(|_| bad())?;
    if !negative {
        return Ok(magnitude);
    }
    if magnitude > 1 << 63 {
        return Err(format!(
            "'{word}' is below the smallest value, -0x8000000000000000"
        ));
    }
    Ok(0u64.wrapping_sub(magnitude))
}

/// How a [`Condition`] compares its location's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Test {
    /// `eq`: the 64 bits are the same.
    Equal(u64),
    /// `ne`: they are not.
    NotEqual(u64),
    /// `gt`: greater, read as signed 64-bit numbers.
    Greater(u64),
    /// `lt`: less, likewise.
    Less(u64),
    /// `in`: the value cannot be worked out — a memory read in it fails.
    Indeterminate,
}

/// A `breakpointif` test: the location, worked out for the thread that
/// reached the breakpoint, and how its value is tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Condition {
    pub(crate) location: Location,
    pub(crate) test: Test,
}

impl Condition {
    /// `<where> eq|ne|gt|lt <value>`, or `<where> in` (`value` `None`).
    pub(crate) fn parse(location: &str, test: &str, value: Option<&str>) -> Result<Self, String> {
        let location = Location::parse(location)?;
        let test = match (test, value) {
            ("in", None) => Test::Indeterminate,
            ("eq", Some(value)) => Test::Equal(signed(value)?),
            ("ne", Some(value)) => Test::NotEqual(signed(value)?),
            ("gt", Some(value)) => Test::Greater(signed(value)?),
            ("lt", Some(value)) => Test::Less(signed(value)?),
            ("eq" | "ne" | "gt" | "lt", None) => {
                return Err(format!(
                    "'{test}' needs a value to test against: <where> {test} <value>"
                ))
            }
            ("in", Some(_)) => {
                return Err(
                    "'in' takes no value: <where> in at <address|function[+offset]>".to_string(),
                )
            }
            _ => return Err(format!("'{test}' is not a test: give eq, ne, gt, lt or in")),
        };
        if test == Test::Indeterminate && !location.reads_memory() {
            return Err(format!(
                "'{}' can always be worked out: 'in' needs a memory read ([…])",
                location.shown()
            ));
        }
        Ok(Condition { location, test })
    }

    /// The test as `list breakpoint` and its messages write it:
    /// `x0 eq 0x0000000000000005 (5)`, `[x0] in`.
    pub(crate) fn shown(&self) -> String {
        let location = self.location.shown();
        let shown = |value: u64| super::breakpoints::value_shown(value, 8);
        match self.test {
            Test::Equal(value) => format!("{location} eq {}", shown(value)),
            Test::NotEqual(value) => format!("{location} ne {}", shown(value)),
            Test::Greater(value) => format!("{location} gt {}", shown(value)),
            Test::Less(value) => format!("{location} lt {}", shown(value)),
            Test::Indeterminate => format!("{location} in"),
        }
    }

    /// Refuse a test whose location names nothing thread `thread` has: a
    /// register or function no one has, a vector register. A memory read that
    /// fails now is no fault, since it is worked out again at each stop.
    pub(crate) fn check(&self, inferior: &mut dyn Inferior, thread: usize) -> Result<(), String> {
        match self.location.value(inferior, thread) {
            Err(Fault::Unknown(message)) => Err(message),
            Ok(_) | Err(Fault::Unreadable(_)) => Ok(()),
        }
    }

    /// Whether the test holds for thread `thread` now. A value that cannot be
    /// worked out holds only `in`.
    pub(crate) fn holds(&self, inferior: &mut dyn Inferior, thread: usize) -> bool {
        let Ok(value) = self.location.value(inferior, thread) else {
            return self.test == Test::Indeterminate;
        };
        let value = value.value;
        match self.test {
            Test::Equal(wanted) => value == wanted,
            Test::NotEqual(wanted) => value != wanted,
            Test::Greater(wanted) => (value as i64) > wanted as i64,
            Test::Less(wanted) => (value as i64) < (wanted as i64),
            Test::Indeterminate => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(inner: Location, offset: i64) -> Location {
        Location {
            term: Term::Memory(Box::new(inner)),
            offset,
        }
    }

    #[test]
    fn a_location_is_a_number_a_name_or_memory_each_with_an_offset() {
        assert_eq!(Location::parse("sp"), Ok(Location::named("sp", 0)));
        assert_eq!(Location::parse("sp+0x18"), Ok(Location::named("sp", 0x18)));
        assert_eq!(Location::parse("0x10+8"), Ok(Location::number(0x18)));
        assert_eq!(
            Location::parse("[sp]"),
            Ok(memory(Location::named("sp", 0), 0))
        );
        assert_eq!(
            Location::parse("[sp+0x18]"),
            Ok(memory(Location::named("sp", 0x18), 0))
        );
        assert_eq!(
            Location::parse("[sp]+0x18"),
            Ok(memory(Location::named("sp", 0), 0x18))
        );
        assert_eq!(
            Location::parse("[[sp+0x10]+8]-4"),
            Ok(memory(memory(Location::named("sp", 0x10), 8), -4))
        );
        assert_eq!(
            Location::parse("[0x1000]"),
            Ok(memory(Location::number(0x1000), 0))
        );
        for (word, message) in [
            ("[sp", "'[sp' opens a '[' it does not close"),
            ("[[sp]", "'[[sp]' opens a '[' it does not close"),
            ("[]", "'[]' is not an address, a register or a function"),
            (
                "[sp]x",
                "'[sp]x' is not an address, a register or a function",
            ),
            ("[sp]+zz", "'zz' is not a number"),
            ("[@]", "'@' is not an address, a register or a function"),
            ("sp]", "'sp]' is not an address, a register or a function"),
        ] {
            assert_eq!(Location::parse(word), Err(message.to_string()), "{word}");
        }
    }

    #[test]
    fn a_location_is_shown_with_full_width_numbers_and_hex_offsets() {
        for (word, shown) in [
            ("sp+24", "sp+0x18"),
            ("x19-16", "x19-0x10"),
            ("[sp+24]", "[sp+0x18]"),
            ("[sp]+0x18", "[sp]+0x18"),
            ("[0x12348593]", "[0x0000000012348593]"),
            ("[[x0]-8]", "[[x0]-0x8]"),
        ] {
            assert_eq!(Location::parse(word).unwrap().shown(), shown, "{word}");
        }
    }

    #[test]
    fn a_value_is_a_whole_number_in_decimal_or_hex_perhaps_negative() {
        assert_eq!(signed("5"), Ok(5));
        assert_eq!(signed("0x10"), Ok(0x10));
        assert_eq!(signed("-1"), Ok(u64::MAX));
        assert_eq!(signed("-0x10"), Ok(0u64.wrapping_sub(0x10)));
        assert_eq!(signed("0xffffffffffffffff"), Ok(u64::MAX));
        assert_eq!(signed("-0x8000000000000000"), Ok(1 << 63));
        assert_eq!(
            signed("-0x8000000000000001"),
            Err(
                "'-0x8000000000000001' is below the smallest value, -0x8000000000000000"
                    .to_string()
            )
        );
        assert_eq!(
            signed("five"),
            Err("'five' is not a value: give a number, such as 5, -1 or 0x10".to_string())
        );
    }

    #[test]
    fn a_condition_takes_a_test_and_a_value_or_in_with_a_memory_read() {
        assert_eq!(
            Condition::parse("x0", "eq", Some("5")),
            Ok(Condition {
                location: Location::named("x0", 0),
                test: Test::Equal(5)
            })
        );
        assert_eq!(
            Condition::parse("x0", "gt", Some("-1")).unwrap().shown(),
            "x0 gt 0xffffffffffffffff (-1)"
        );
        assert_eq!(
            Condition::parse("[x0+8]", "in", None).unwrap().shown(),
            "[x0+0x8] in"
        );
        for (words, message) in [
            (
                ("x0", "is", Some("5")),
                "'is' is not a test: give eq, ne, gt, lt or in",
            ),
            (
                ("x0", "eq", None),
                "'eq' needs a value to test against: <where> eq <value>",
            ),
            (
                ("[x0]", "in", Some("5")),
                "'in' takes no value: <where> in at <address|function[+offset]>",
            ),
            (
                ("x0+8", "in", None),
                "'x0+0x8' can always be worked out: 'in' needs a memory read ([…])",
            ),
        ] {
            assert_eq!(
                Condition::parse(words.0, words.1, words.2),
                Err(message.to_string()),
                "{words:?}"
            );
        }
    }
}
