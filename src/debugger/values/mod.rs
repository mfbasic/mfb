//! `show type`: MFBASIC values in the arena, read as the type the developer
//! says they are.
//!
//! Arena memory carries no type: a `String` is a length and bytes, a record its
//! fields (`./mfb spec memory heap-values`). These views read it the way the
//! compiler lays each type out, and refuse whatever cannot be one: memory
//! outside every arena region (`show mem` reads any memory), a collection
//! header whose kind, type codes or sizes are impossible, text that is not
//! UTF-8. Collections name their element types with the runtime type codes
//! (`./mfb spec memory collections`), so an element is decoded without being
//! told its type, except for the codes several types share: a nested
//! `List`/`Map` (20/21, inlined or a pointer) and every other type (22).

/// Reads `length` bytes of the program's memory at an address.
pub(crate) type Read<'a> = dyn FnMut(u64, usize) -> Result<Vec<u8>, String> + 'a;
use crate::codegen::error::constants::error_constants::{
    COLLECTION_ENTRY_FLAG_REMOVED, COLLECTION_ENTRY_FLAG_USED, COLLECTION_ENTRY_OFFSET_FLAGS,
    COLLECTION_ENTRY_OFFSET_KEY_LENGTH, COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
    COLLECTION_ENTRY_OFFSET_VALUE_LENGTH, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    COLLECTION_ENTRY_SIZE, COLLECTION_HEADER_SIZE, COLLECTION_KIND_LIST,
    COLLECTION_KIND_LIST_FIXED, COLLECTION_KIND_MAP, COLLECTION_KIND_SET,
    COLLECTION_OFFSET_CAPACITY, COLLECTION_OFFSET_COUNT, COLLECTION_OFFSET_DATA_CAPACITY,
    COLLECTION_OFFSET_DATA_LENGTH, COLLECTION_OFFSET_KEY_TYPE, COLLECTION_OFFSET_KIND,
    COLLECTION_OFFSET_MAP_STATE, COLLECTION_OFFSET_VALUE_TYPE, COLLECTION_TYPE_BOOLEAN,
    COLLECTION_TYPE_BYTE, COLLECTION_TYPE_FIXED, COLLECTION_TYPE_FLOAT, COLLECTION_TYPE_INTEGER,
    COLLECTION_TYPE_LIST, COLLECTION_TYPE_MAP, COLLECTION_TYPE_MONEY, COLLECTION_TYPE_NONE,
    COLLECTION_TYPE_NOTHING, COLLECTION_TYPE_OBJECT, COLLECTION_TYPE_SCALAR,
    COLLECTION_TYPE_STRING, RESOURCE_CLOSED_BIT, RESOURCE_MOVED_BIT, RESOURCE_OFFSET_CLOSED,
    RESOURCE_OFFSET_HANDLE, RESOURCE_OFFSET_OWNER, RESOURCE_OFFSET_STATE, RESOURCE_OFFSET_TAG,
    RESOURCE_RECORD_SIZE_BYTES,
};
use crate::numeric::{fixed_default_to_string_text, MONEY_SCALE};
// From a part, `super` is `debugger::values`; `collection.rs` spells `super::prompt`, a
// sibling module of `values` in `debugger`.
use super::prompt;

mod collection;
mod view;
use collection::*;
pub(crate) use view::*;

/// The most text a `String` view reads and shows.
pub(crate) const MAX_TEXT: u64 = 64 << 10;
/// The handed-out part of one region of an arena's slice (plan-183-C): values live
/// in `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region {
    pub(crate) arena: usize,
    pub(crate) area: usize,
    pub(crate) start: u64,
    pub(crate) end: u64,
}

impl Region {
    /// Region number `area` of `arena`, `size` bytes from `address`.
    pub(crate) fn of_area(arena: usize, area: usize, address: u64, size: u64) -> Region {
        Region {
            arena,
            area,
            start: address,
            end: address.saturating_add(size),
        }
    }
}

/// The lines `view` shows for the value at `address`, which must lie in one
/// of `regions`.
/// A `String` literal the executable carries (a `_mfb_str_*` data symbol): laid
/// out as an arena `String` is — its length, its bytes, a NUL — but in the
/// file's data, outside every arena region. Shown as `show type string` shows a
/// `String`, naming the literal.
pub(crate) fn show_literal(
    read: &mut Read,
    address: u64,
    symbol: &str,
) -> Result<Vec<String>, String> {
    let length = u64::from_le_bytes(
        read(address, 8)?
            .try_into()
            .map_err(|_| format!("cannot read the String literal at 0x{address:016x}"))?,
    );
    let literal = Region {
        arena: usize::MAX,
        area: usize::MAX,
        start: address,
        end: address.saturating_add(9).saturating_add(length),
    };
    let mut lines = show(read, &[literal], address, &View::String)?;
    let name = super::names::plain_name(symbol);
    if let Some(line) = lines.first_mut() {
        line.push_str(&format!(" (the literal {name})"));
    }
    Ok(lines)
}

pub(crate) fn show(
    read: &mut Read,
    regions: &[Region],
    address: u64,
    view: &View,
) -> Result<Vec<String>, String> {
    let mut memory = Memory { read, regions };
    match view {
        View::Scalar(scalar) => {
            let bytes = memory.read(address, scalar.width(), scalar.name())?;
            Ok(vec![format!(
                "Value: {}",
                scalar_text(*scalar, &bytes, address)?
            )])
        }
        View::String => {
            let length = u64::from_le_bytes(
                memory
                    .read(address, 8, "String")?
                    .try_into()
                    .expect("8 bytes"),
            );
            let shown = length.min(MAX_TEXT);
            memory
                .check(address, 8 + length.saturating_add(1), "String")
                .map_err(|_| {
                    format!(
                    "not a String at 0x{address:016x}: its length, {length}, runs past the end \
                     of its arena region"
                )
                })?;
            let bytes = memory.read(address + 8, shown, "String")?;
            let text = utf8(&bytes, shown < length, address, "String")?;
            let mut line = format!("Length: {length} Value: \"{}\"", quoted(text));
            if shown < length {
                line.push_str(&format!(" (the first {shown} bytes)"));
            }
            Ok(vec![line])
        }
        View::List { element, span } => {
            let collection = memory.collection(address, Shape::List)?;
            let mut lines = vec![collection.summary()];
            if let Some(span) = span {
                lines = elements(&mut memory, &collection, *element, *span, Part::Value)?;
            } else if element.is_some() {
                return Err("give an index or a range after the element type".to_string());
            }
            Ok(lines)
        }
        View::Set { element, span } => {
            let collection = memory.collection(address, Shape::Set)?;
            let mut lines = vec![collection.summary()];
            if let Some(span) = span {
                lines = elements(&mut memory, &collection, *element, *span, Part::Key)?;
            } else if element.is_some() {
                return Err("give an index or a range after the element type".to_string());
            }
            Ok(lines)
        }
        View::Map(map) => {
            let collection = memory.collection(address, Shape::Map)?;
            match map {
                MapView::Summary => Ok(vec![collection.summary()]),
                MapView::Keys(kind) => {
                    let all = Span {
                        start: 0,
                        end: None,
                    };
                    elements(&mut memory, &collection, *kind, all, Part::Key)
                }
                MapView::Values(kind) => {
                    let all = Span {
                        start: 0,
                        end: None,
                    };
                    elements(&mut memory, &collection, *kind, all, Part::Value)
                }
                MapView::Entry(index) => {
                    let entries = collection.live_entries(&mut memory)?;
                    let entry = entries.get(*index as usize).ok_or_else(|| {
                        format!(
                            "no entry {index}: the map has {} (numbered from 0)",
                            entries.len()
                        )
                    })?;
                    entry_lines(&mut memory, &collection, *index, entry)
                }
                MapView::Get(key) => {
                    let wanted = key_bytes(collection.key_type, key)?;
                    let entries = collection.live_entries(&mut memory)?;
                    for (index, entry) in entries.iter().enumerate() {
                        let (at, length) = collection.key_at(entry);
                        if length == wanted.len() as u64
                            && memory.read(at, length, "Map")? == wanted
                        {
                            return entry_lines(&mut memory, &collection, index as u64, entry);
                        }
                    }
                    Err(format!("the map has no key {key}"))
                }
            }
        }
        View::Resource => Ok(vec![resource_summary(&mut memory, address)?]),
    }
}

/// Process memory, read only inside the arena's blocks.
struct Memory<'a, 'b> {
    read: &'a mut Read<'b>,
    regions: &'a [Region],
}

impl Memory<'_, '_> {
    /// Check that `length` bytes at `address` lie in one arena region, or say
    /// they cannot be the `what` asked for.
    fn check(&self, address: u64, length: u64, what: &str) -> Result<Region, String> {
        let end = address.checked_add(length);
        self.regions
            .iter()
            .find(|region| region.start <= address && end.is_some_and(|end| end <= region.end))
            .copied()
            .ok_or_else(|| {
                if self
                    .regions
                    .iter()
                    .any(|region| region.start <= address && address < region.end)
                {
                    format!(
                        "not {} at 0x{address:016x}: its {length} bytes run past the end of \
                         their arena region",
                        a(what)
                    )
                } else {
                    format!(
                        "0x{address:016x} is not in an arena region, so it holds no {what}; \
                         'show mem' reads any memory"
                    )
                }
            })
    }

    fn read(&mut self, address: u64, length: u64, what: &str) -> Result<Vec<u8>, String> {
        self.check(address, length, what)?;
        if length == 0 {
            return Ok(Vec::new());
        }
        (self.read)(address, length as usize)
    }

    fn u64_at(&mut self, address: u64, what: &str) -> Result<u64, String> {
        let bytes = self.read(address, 8, what)?;
        Ok(u64::from_le_bytes(bytes.try_into().expect("8 bytes")))
    }

    /// The collection header at `address`, checked as a `shape`.
    fn collection(&mut self, address: u64, shape: Shape) -> Result<Collection, String> {
        let what = shape.name();
        let header = self.read(address, COLLECTION_HEADER_SIZE as u64, what)?;
        let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().expect("8 bytes"));
        let kind = usize::from(header[COLLECTION_OFFSET_KIND]);
        let key_type = usize::from(header[COLLECTION_OFFSET_KEY_TYPE]);
        let value_type = usize::from(header[COLLECTION_OFFSET_VALUE_TYPE]);
        let count = word(COLLECTION_OFFSET_COUNT);
        let capacity = word(COLLECTION_OFFSET_CAPACITY);
        let data_length = word(COLLECTION_OFFSET_DATA_LENGTH);
        let data_capacity = word(COLLECTION_OFFSET_DATA_CAPACITY);
        let state = u32::from_le_bytes(
            header[COLLECTION_OFFSET_MAP_STATE..COLLECTION_OFFSET_MAP_STATE + 4]
                .try_into()
                .expect("4 bytes"),
        );
        let not = |why: String| format!("not a {what} at 0x{address:016x}: {why}");
        let expected = match shape {
            Shape::List => kind == COLLECTION_KIND_LIST || kind == COLLECTION_KIND_LIST_FIXED,
            Shape::Set => kind == COLLECTION_KIND_SET,
            Shape::Map => kind == COLLECTION_KIND_MAP,
        };
        if !expected {
            return Err(not(match kind_name(kind) {
                Some(name) => format!("its kind byte is {kind}, {name}"),
                None => format!("its kind byte is {kind}, which no collection has"),
            }));
        }
        let known = |code: usize| code_name(code).is_some() && code != COLLECTION_TYPE_NONE;
        match shape {
            Shape::List if key_type != COLLECTION_TYPE_NONE => {
                return Err(not(format!("a list's key type is 0, not {key_type}")))
            }
            Shape::List if !known(value_type) => {
                return Err(not(format!("{value_type} is not an element type code")))
            }
            Shape::Set if !known(key_type) || value_type != COLLECTION_TYPE_BOOLEAN => {
                return Err(not(format!(
                    "a set's element type code is {key_type} and its value type {value_type}"
                )))
            }
            Shape::Map if !known(key_type) || !known(value_type) => {
                return Err(not(format!(
                    "{key_type} and {value_type} are not key and value type codes"
                )))
            }
            _ => {}
        }
        if count > capacity {
            return Err(not(format!(
                "its count, {count}, is over its capacity, {capacity}"
            )));
        }
        if data_length > data_capacity {
            return Err(not(format!(
                "its data length, {data_length}, is over its data capacity, {data_capacity}"
            )));
        }
        let fixed = kind == COLLECTION_KIND_LIST_FIXED;
        let width = if fixed {
            payload_width(value_type)
                .ok_or_else(|| not(format!("a fixed-width list of type code {value_type}")))?
        } else {
            0
        };
        let entries = if fixed {
            0
        } else {
            capacity.saturating_mul(COLLECTION_ENTRY_SIZE as u64)
        };
        let data = address + COLLECTION_HEADER_SIZE as u64 + entries;
        // A map's or set's hash index follows the data region; it is not part
        // of what is checked or shown.
        let size = (COLLECTION_HEADER_SIZE as u64)
            .saturating_add(entries)
            .saturating_add(data_capacity);
        if fixed && count.saturating_mul(width) > data_capacity {
            return Err(not(format!(
                "{count} elements of {width} bytes do not fit its {data_capacity} data bytes"
            )));
        }
        self.check(address, size, what).map_err(|_| {
            not(format!(
                "its {size} bytes run past the end of its arena region"
            ))
        })?;
        Ok(Collection {
            shape,
            address,
            kind,
            key_type,
            value_type,
            count,
            capacity,
            data_length,
            data_capacity,
            dead: u64::from(state >> 8),
            data,
            width,
            size,
        })
    }
}

/// A payload in a collection's data region, decoded as its type code says
/// (or as `kind`, for the codes several types share).
fn payload_text(
    memory: &mut Memory,
    code: usize,
    kind: Option<Kind>,
    address: u64,
    size: u64,
) -> Result<String, String> {
    let from_code = match code {
        COLLECTION_TYPE_NOTHING => return Ok("Nothing".to_string()),
        COLLECTION_TYPE_STRING => Some(Kind::String),
        COLLECTION_TYPE_LIST => Some(Kind::List),
        COLLECTION_TYPE_MAP => Some(Kind::Map),
        COLLECTION_TYPE_OBJECT => None,
        code => code_scalar(code).map(Kind::Scalar),
    };
    let kind = match (from_code, kind) {
        (Some(known), Some(asked)) if known != asked => {
            return Err(format!(
                "the elements are {}, not {}",
                known.name(),
                asked.name()
            ))
        }
        (Some(known), _) => known,
        (None, Some(asked @ (Kind::Set | Kind::Resource))) => asked,
        (None, Some(asked)) => {
            return Err(format!(
                "type code 22 holds a Set, a resource, a record, an enum or a function, not \
                 {}",
                asked.name()
            ))
        }
        (None, None) => {
            return Ok("(type code 22: name it, as set or resource, to read it)".to_string())
        }
    };
    match kind {
        Kind::Scalar(scalar) => {
            if size != scalar.width() {
                return Err(format!(
                    "{} payload is {} bytes, not {size}",
                    a(scalar.name()),
                    scalar.width()
                ));
            }
            let bytes = memory.read(address, size, scalar.name())?;
            scalar_text(scalar, &bytes, address)
        }
        Kind::String => {
            let shown = size.min(MAX_TEXT);
            let bytes = memory.read(address, shown, "String")?;
            let text = utf8(&bytes, shown < size, address, "String")?;
            Ok(format!("\"{}\"", quoted(text)))
        }
        Kind::List | Kind::Set | Kind::Map => {
            let shape = match kind {
                Kind::List => Shape::List,
                Kind::Set => Shape::Set,
                _ => Shape::Map,
            };
            // A non-flat nested collection is an 8-byte pointer; a flat one is
            // its whole block, at least a header long, inline.
            let (block, how) = if size == 8 {
                let pointer = memory.u64_at(address, shape.name())?;
                (pointer, format!(" (stored at 0x{pointer:016x})"))
            } else {
                (address, " (inline)".to_string())
            };
            Ok(format!(
                "{}{how}",
                memory.collection(block, shape)?.summary()
            ))
        }
        Kind::Resource => {
            if size != 8 {
                return Err(format!(
                    "a resource payload is an 8-byte pointer, not {size} bytes"
                ));
            }
            let pointer = memory.u64_at(address, "resource")?;
            resource_summary(memory, pointer)
        }
    }
}

/// `0x<hex> (<value>)` for a scalar's bytes, little-endian.
fn scalar_text(scalar: Scalar, bytes: &[u8], address: u64) -> Result<String, String> {
    let not = |why: String| format!("not {} at 0x{address:016x}: {why}", a(scalar.name()));
    Ok(match scalar {
        Scalar::Boolean => match bytes[0] {
            0 => "0x00 (FALSE)".to_string(),
            1 => "0x01 (TRUE)".to_string(),
            byte => return Err(not(format!("its byte is 0x{byte:02x}, not 0 or 1"))),
        },
        Scalar::Byte => format!("0x{:02x} ({})", bytes[0], bytes[0]),
        Scalar::Scalar => {
            let code = u32::from_le_bytes(bytes.try_into().expect("4 bytes"));
            let character = char::from_u32(code)
                .ok_or_else(|| not(format!("0x{code:x} is not a Unicode codepoint")))?;
            format!("0x{code:08x} (U+{code:04X} '{}')", character.escape_debug())
        }
        Scalar::Integer | Scalar::Float | Scalar::Fixed | Scalar::Money => {
            let raw = u64::from_le_bytes(bytes.try_into().expect("8 bytes"));
            let value = match scalar {
                Scalar::Integer => (raw as i64).to_string(),
                Scalar::Float => format!("{:?}", f64::from_bits(raw)),
                // As `toString` prints it, rounded to two places.
                Scalar::Fixed => fixed_default_to_string_text(raw as i64),
                _ => money_text(raw as i64),
            };
            format!("0x{raw:016x} ({value})")
        }
    })
}

/// A `Money` raw value with all 5 decimal places: `raw / 100000`.
fn money_text(raw: i64) -> String {
    let scale = MONEY_SCALE.unsigned_abs();
    let magnitude = raw.unsigned_abs();
    let sign = if raw < 0 { "-" } else { "" };
    format!("{sign}{}.{:05}", magnitude / scale, magnitude % scale)
}

/// The bytes of `key` as a map stores a key of type `code`.
fn key_bytes(code: usize, key: &str) -> Result<Vec<u8>, String> {
    let bad = |what: &str| format!("'{key}' is not {what}");
    Ok(match code {
        COLLECTION_TYPE_STRING => key.as_bytes().to_vec(),
        COLLECTION_TYPE_INTEGER => parse_integer(key)
            .ok_or_else(|| bad("an Integer"))?
            .to_le_bytes()
            .to_vec(),
        COLLECTION_TYPE_FLOAT => key
            .parse::<f64>()
            .map_err(|_| bad("a Float"))?
            .to_bits()
            .to_le_bytes()
            .to_vec(),
        COLLECTION_TYPE_FIXED => crate::numeric::fixed_raw_from_decimal(key)
            .map_err(|_| bad("a Fixed"))?
            .to_le_bytes()
            .to_vec(),
        COLLECTION_TYPE_MONEY => crate::numeric::money_raw_from_decimal(key)
            .map_err(|_| bad("a Money amount"))?
            .to_le_bytes()
            .to_vec(),
        COLLECTION_TYPE_BOOLEAN => match key.to_ascii_lowercase().as_str() {
            "true" => vec![1],
            "false" => vec![0],
            _ => return Err(bad("TRUE or FALSE")),
        },
        COLLECTION_TYPE_BYTE => vec![parse_integer(key)
            .and_then(|value| u8::try_from(value).ok())
            .ok_or_else(|| bad("a Byte"))?],
        COLLECTION_TYPE_SCALAR => {
            let mut characters = key.chars();
            let code = match (characters.next(), characters.next()) {
                (Some(character), None) => u32::from(character),
                _ => key
                    .strip_prefix("U+")
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .filter(|code| char::from_u32(*code).is_some())
                    .ok_or_else(|| bad("one character or U+<hex>"))?,
            };
            code.to_le_bytes().to_vec()
        }
        COLLECTION_TYPE_NOTHING => vec![0],
        code => {
            return Err(format!(
                "{} key cannot be looked up; list the keys with 'keys'",
                a(&code_label(code))
            ))
        }
    })
}

/// A decimal integer, or hex after `0x`, either signed.
fn parse_integer(text: &str) -> Option<i64> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text),
    };
    let magnitude = match digits.strip_prefix("0x") {
        Some(hex) => i128::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<i128>().ok()?,
    };
    i64::try_from(if negative { -magnitude } else { magnitude }).ok()
}

/// The resource record at `address`, on one line: its kind, closed and moved
/// flags, handle, `STATE`, owner, and its kind's own fields.
fn resource_summary(memory: &mut Memory, address: u64) -> Result<String, String> {
    let record = memory.read(address, RESOURCE_RECORD_SIZE_BYTES as u64, "resource")?;
    let word = |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().expect("8 bytes"));
    let tag = word(RESOURCE_OFFSET_TAG);
    let (name, handle) = resource_kind(tag).ok_or_else(|| {
        format!("not a resource at 0x{address:016x}: its tag, {tag}, is no resource kind")
    })?;
    let flags = word(RESOURCE_OFFSET_CLOSED);
    let flag = |bit: u64| if flags & (1 << bit) != 0 { "Y" } else { "N" };
    let owner = word(RESOURCE_OFFSET_OWNER);
    let owner = if owner == 0 {
        "none (not bound to a scope yet)".to_string()
    } else {
        // A scope token is `((sp + token_slot) << 12) - depth`.
        let slot = (owner >> 12) + 1;
        let depth = (slot << 12) - owner;
        format!("0x{owner:x} (scope depth {depth} of the frame whose token slot is 0x{slot:x})")
    };
    let shown = |value: u64, number: bool| {
        if number {
            value.to_string()
        } else {
            format!("0x{value:x}")
        }
    };
    let mut line = format!(
        "Type: {name} Closed: {} Moved: {} {}: {} State: 0x{:016x} Scope: {owner}",
        flag(RESOURCE_CLOSED_BIT),
        flag(RESOURCE_MOVED_BIT),
        handle.0,
        shown(word(RESOURCE_OFFSET_HANDLE), handle.1),
        word(RESOURCE_OFFSET_STATE),
    );
    for (field, at, number) in resource_tail(tag) {
        line.push_str(&format!(" {field}: {}", shown(word(*at), *number)));
    }
    Ok(line)
}

/// A resource tag's kind, what its handle word holds, and whether that is a
/// number (shown in decimal) rather than an address
/// (`./mfb spec memory heap-values`, Resource Record).
type Handle = (&'static str, bool);

fn resource_kind(tag: u64) -> Option<(&'static str, Handle)> {
    const FD: Handle = ("Handle (fd)", true);
    const WORD: Handle = ("Handle", false);
    Some(match tag {
        1 => ("File", FD),
        2 => ("Socket", FD),
        3 => ("UdpSocket", FD),
        4 => ("Listener", FD),
        5 => ("tls::Socket (OpenSSL)", FD),
        6 => (
            "tls::Socket (Network.framework)",
            ("Handle (connection)", false),
        ),
        7 => ("tls::Socket (SChannel)", FD),
        8 => ("tls::Listener", FD),
        9 => ("audio stream", ("Handle (kind)", true)),
        10 => ("Process", WORD),
        11 => ("Image", WORD),
        12 => ("Font", WORD),
        13 => ("Thread", WORD),
        14 => ("ThreadWorker", WORD),
        255 => ("native LINK resource", ("Handle (native pointer)", false)),
        _ => return None,
    })
}

/// The fields past the header that a resource kind's record holds: name,
/// offset, and whether the word is a number rather than an address.
fn resource_tail(tag: u64) -> &'static [(&'static str, usize, bool)] {
    match tag {
        1 => &[
            ("Write Buffer", 40, false),
            ("Write Filled", 48, true),
            ("Write Enabled", 56, true),
            ("Read Buffer", 64, false),
            ("Read Position", 72, true),
            ("Read Filled", 80, true),
            ("At EOF", 88, true),
        ],
        5 => &[("SSL_CTX", 40, false), ("SSL", 48, false)],
        6 => &[
            ("Connection Context", 40, false),
            ("Dispatch Queue", 48, false),
        ],
        7 => &[("SSPI Block", 48, false)],
        8 => &[("SSL_CTX", 40, false)],
        9 => &[
            ("Sample Rate", 40, true),
            ("Channels", 48, true),
            ("Bytes Per Frame", 56, true),
            ("Buffer Frames", 64, true),
            ("Audio State", 72, false),
        ],
        _ => &[],
    }
}

/// The text of a `String`'s bytes, or why they cannot be one; `cut` when the
/// bytes stop at the [`MAX_TEXT`] limit, which may split a character.
fn utf8<'a>(bytes: &'a [u8], cut: bool, address: u64, what: &str) -> Result<&'a str, String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(error) if error.error_len().is_none() && cut => {
            Ok(std::str::from_utf8(&bytes[..error.valid_up_to()]).expect("valid up to here"))
        }
        Err(error) => Err(format!(
            "not a {what} at 0x{address:016x}: byte {} of its value, 0x{:02x}, is not UTF-8",
            error.valid_up_to(),
            bytes[error.valid_up_to()]
        )),
    }
}

/// Text as it reads between double quotes: quotes, backslashes and control
/// characters escaped the way Rust escapes them, so nothing in it can act on
/// the terminal.
pub(crate) fn quoted(text: &str) -> String {
    text.chars().flat_map(char::escape_debug).collect()
}

fn code_scalar(code: usize) -> Option<Scalar> {
    Some(match code {
        COLLECTION_TYPE_BOOLEAN => Scalar::Boolean,
        COLLECTION_TYPE_INTEGER => Scalar::Integer,
        COLLECTION_TYPE_FLOAT => Scalar::Float,
        COLLECTION_TYPE_FIXED => Scalar::Fixed,
        COLLECTION_TYPE_BYTE => Scalar::Byte,
        COLLECTION_TYPE_MONEY => Scalar::Money,
        COLLECTION_TYPE_SCALAR => Scalar::Scalar,
        _ => return None,
    })
}

/// `what` with its indefinite article: "an Integer", "a List".
fn a(what: &str) -> String {
    let vowel = what
        .chars()
        .next()
        .is_some_and(|first| "AEIOUaeiou".contains(first));
    format!("{} {what}", if vowel { "an" } else { "a" })
}

/// The type a runtime type code names, when it names one.
fn code_name(code: usize) -> Option<&'static str> {
    Some(match code {
        COLLECTION_TYPE_NONE => "none",
        COLLECTION_TYPE_NOTHING => "Nothing",
        COLLECTION_TYPE_STRING => "String",
        COLLECTION_TYPE_LIST => "List",
        COLLECTION_TYPE_MAP => "Map",
        COLLECTION_TYPE_OBJECT => "object",
        code => code_scalar(code)?.name(),
    })
}

/// How a summary names a type code; code 22 lists what it may be.
fn code_label(code: usize) -> String {
    match code {
        COLLECTION_TYPE_OBJECT => "object (Set, record, enum, resource or function)".to_string(),
        code => code_name(code).unwrap_or("?").to_string(),
    }
}

/// The collection a kind byte names.
fn kind_name(kind: usize) -> Option<&'static str> {
    Some(match kind {
        COLLECTION_KIND_LIST | COLLECTION_KIND_LIST_FIXED => "a List",
        COLLECTION_KIND_MAP => "a Map",
        COLLECTION_KIND_SET => "a Set",
        _ => return None,
    })
}

/// The width of a fixed-width list's payload of type `code`.
fn payload_width(code: usize) -> Option<u64> {
    match code {
        COLLECTION_TYPE_NOTHING => Some(1),
        code => code_scalar(code).map(Scalar::width),
    }
}

#[cfg(test)]
mod tests;
