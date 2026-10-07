//! `show type`: MFBASIC values in the arena, read as the type the developer
//! says they are.
//!
//! Arena memory carries no type: a `String` is a length and bytes, a record its
//! fields (`./mfb spec memory heap-values`). These views read it the way the
//! compiler lays each type out, and refuse whatever cannot be one: memory
//! outside every arena block (`show mem` reads any memory), a collection
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

/// The most text a `String` view reads and shows.
pub(crate) const MAX_TEXT: u64 = 64 << 10;
/// The bytes before an arena block's usable space (`ARENA_BLOCK_HEADER_SIZE`).
const BLOCK_HEADER: u64 = 32;

/// The usable space of one arena block: values live in `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region {
    pub(crate) arena: usize,
    pub(crate) block: usize,
    pub(crate) start: u64,
    pub(crate) end: u64,
}

impl Region {
    /// The usable space of the block at `address`, `size` bytes long.
    pub(crate) fn of_block(arena: usize, block: usize, address: u64, size: u64) -> Region {
        Region {
            arena,
            block,
            start: address + BLOCK_HEADER,
            end: address.saturating_add(size),
        }
    }
}

/// A scalar type, as stored in 1, 4 or 8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scalar {
    Integer,
    Float,
    Fixed,
    Money,
    Boolean,
    Byte,
    Scalar,
}

impl Scalar {
    fn width(self) -> u64 {
        match self {
            Scalar::Boolean | Scalar::Byte => 1,
            Scalar::Scalar => 4,
            Scalar::Integer | Scalar::Float | Scalar::Fixed | Scalar::Money => 8,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Scalar::Integer => "Integer",
            Scalar::Float => "Float",
            Scalar::Fixed => "Fixed",
            Scalar::Money => "Money",
            Scalar::Boolean => "Boolean",
            Scalar::Byte => "Byte",
            Scalar::Scalar => "Scalar",
        }
    }
}

/// A type a view or an element can be read as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Scalar(Scalar),
    String,
    List,
    Set,
    Map,
    Resource,
}

impl Kind {
    /// The type a `show type` word names, in its short or MFBASIC spelling.
    pub(crate) fn parse(word: &str) -> Option<Kind> {
        Some(match word.to_ascii_lowercase().as_str() {
            "int" | "integer" => Kind::Scalar(Scalar::Integer),
            "float" => Kind::Scalar(Scalar::Float),
            "fixed" => Kind::Scalar(Scalar::Fixed),
            "money" => Kind::Scalar(Scalar::Money),
            "bool" | "boolean" => Kind::Scalar(Scalar::Boolean),
            "byte" => Kind::Scalar(Scalar::Byte),
            "scalar" => Kind::Scalar(Scalar::Scalar),
            "string" => Kind::String,
            "list" => Kind::List,
            "set" => Kind::Set,
            "map" => Kind::Map,
            "resource" => Kind::Resource,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Scalar(scalar) => scalar.name(),
            Kind::String => "String",
            Kind::List => "List",
            Kind::Set => "Set",
            Kind::Map => "Map",
            Kind::Resource => "resource",
        }
    }
}

/// Which elements a list or set view shows: `start..end`, `end` `None` for
/// through the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) start: u64,
    pub(crate) end: Option<u64>,
}

impl Span {
    /// `3`, `0..10` (10 excluded) or `2..` (through the last).
    pub(crate) fn parse(word: &str) -> Option<Span> {
        let number = |text: &str| text.parse::<u64>().ok();
        match word.split_once("..") {
            None => number(word).map(|at| Span {
                start: at,
                end: Some(at + 1),
            }),
            Some((start, "")) => Some(Span {
                start: number(start)?,
                end: None,
            }),
            Some((start, end)) => {
                let (start, end) = (number(start)?, number(end)?);
                (start < end).then_some(Span {
                    start,
                    end: Some(end),
                })
            }
        }
    }
}

/// What part of a map a view shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MapView {
    Summary,
    Keys(Option<Kind>),
    Values(Option<Kind>),
    Entry(u64),
    Get(String),
}

/// A `show type` view of the value at an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum View {
    Scalar(Scalar),
    String,
    List {
        element: Option<Kind>,
        span: Option<Span>,
    },
    Set {
        element: Option<Kind>,
        span: Option<Span>,
    },
    Map(MapView),
    Resource,
}

/// What a bare `show type` prints.
pub(crate) const TYPE_USAGE: &str = "\
Usage: show type <type> <where> [...]

Reads the value at <where> (an address, or a register holding one, as for
show mem) as <type>. The value must be in one of the program's arena blocks,
in a debug build; show mem reads any memory.

  int, float, fixed, money, bool, byte, scalar
                One value: Value: <hex> (<value>)
  string        Length: <bytes> Value: \"<text>\"
  list [<type>] [<index> | <from>..<to> | <from>..]
                The list's header; with an index or a range, its elements.
                <type> is needed only for elements a list cannot name:
                set or resource.
  set [<type>] [<index> | <from>..<to> | <from>..]
                The same, for a set
  map           The map's header
  map <where> keys [<type>] | values [<type>] | <index> | get <key>
                Its keys, its values, one entry, or the entry for a key
  resource      Its kind, closed and moved flags, handle, STATE, owner, and
                its kind's own fields";

/// Parse the words after `show type <type> <where>` (`rest`) into a view;
/// `line` is the whole command, which `get` takes its key from verbatim.
pub(crate) fn parse_view(type_word: &str, rest: &[&str], line: &str) -> Result<View, String> {
    let kind = Kind::parse(type_word).ok_or_else(|| {
        format!(
            "cannot show type '{type_word}'; the types are: {}",
            type_names()
        )
    })?;
    let too_many = || format!("too many arguments to 'show type {type_word}'");
    match kind {
        Kind::Scalar(scalar) if rest.is_empty() => Ok(View::Scalar(scalar)),
        Kind::String if rest.is_empty() => Ok(View::String),
        Kind::Resource if rest.is_empty() => Ok(View::Resource),
        Kind::Scalar(_) | Kind::String | Kind::Resource => Err(too_many()),
        Kind::List | Kind::Set => {
            let (element, span) = match rest {
                [] => (None, None),
                [word] => match Kind::parse(word) {
                    Some(kind) => (Some(kind), None),
                    None => (None, Some(span(word)?)),
                },
                [kind, word] => (Some(element_kind(kind)?), Some(span(word)?)),
                _ => return Err(too_many()),
            };
            Ok(if kind == Kind::List {
                View::List { element, span }
            } else {
                View::Set { element, span }
            })
        }
        Kind::Map => match rest {
            [] => Ok(View::Map(MapView::Summary)),
            ["keys"] => Ok(View::Map(MapView::Keys(None))),
            ["keys", kind] => Ok(View::Map(MapView::Keys(Some(element_kind(kind)?)))),
            ["values"] => Ok(View::Map(MapView::Values(None))),
            ["values", kind] => Ok(View::Map(MapView::Values(Some(element_kind(kind)?)))),
            ["get", ..] => {
                let key = line
                    .split_once(" get ")
                    .map(|(_, key)| key.trim())
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| {
                        "'get' needs a key: show type map <where> get <key>".to_string()
                    })?;
                Ok(View::Map(MapView::Get(unquote(key).to_string())))
            }
            [word] => match word.parse::<u64>() {
                Ok(index) => Ok(View::Map(MapView::Entry(index))),
                Err(_) => Err(format!(
                    "'{word}' is not an index, 'keys', 'values' or 'get <key>'"
                )),
            },
            _ => Err(too_many()),
        },
    }
}

fn type_names() -> &'static str {
    "int, float, fixed, money, bool, byte, scalar, string, list, set, map, resource"
}

fn element_kind(word: &str) -> Result<Kind, String> {
    Kind::parse(word)
        .ok_or_else(|| format!("'{word}' is not a type; the types are: {}", type_names()))
}

fn span(word: &str) -> Result<Span, String> {
    Span::parse(word)
        .ok_or_else(|| format!("'{word}' is not an index or a range (3, 0..10, 2..) or a type"))
}

/// A key as typed, less one pair of surrounding double quotes.
fn unquote(key: &str) -> &str {
    key.strip_prefix('"')
        .and_then(|key| key.strip_suffix('"'))
        .unwrap_or(key)
}

/// The lines `view` shows for the value at `address`, which must lie in one
/// of `regions`.
/// A `String` literal the executable carries (a `_mfb_str_*` data symbol): laid
/// out as an arena `String` is — its length, its bytes, a NUL — but in the
/// file's data, outside every arena block. Shown as `show type string` shows a
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
        block: usize::MAX,
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
                     of its arena block"
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
    /// Check that `length` bytes at `address` lie in one arena block, or say
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
                         their arena block",
                        a(what)
                    )
                } else {
                    format!(
                        "0x{address:016x} is not in an arena block, so it holds no {what}; \
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
                "its {size} bytes run past the end of its arena block"
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

/// Which collection a view expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    List,
    Set,
    Map,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Shape::List => "List",
            Shape::Set => "Set",
            Shape::Map => "Map",
        }
    }
}

/// A collection header, checked.
#[derive(Debug, Clone)]
struct Collection {
    shape: Shape,
    address: u64,
    kind: usize,
    key_type: usize,
    value_type: usize,
    count: u64,
    capacity: u64,
    data_length: u64,
    data_capacity: u64,
    /// Removed entries still in place (a `Map`'s or `Set`'s tombstones).
    dead: u64,
    /// Where the data region starts.
    data: u64,
    /// A fixed-width list's element width; 0 for entry-addressed payloads.
    width: u64,
    /// The block's whole size.
    size: u64,
}

/// One lookup entry: where its key and value payloads are in the data region.
#[derive(Debug, Clone, Copy)]
struct Entry {
    key_offset: u64,
    key_length: u64,
    value_offset: u64,
    value_length: u64,
}

/// Which payload of an entry an element listing shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Key,
    Value,
}

impl Collection {
    /// The one-line summary of the header.
    fn summary(&self) -> String {
        let block = format!(
            "Block: 0x{:016x}-0x{:016x} ({} bytes)",
            self.address,
            self.address + self.size,
            self.size
        );
        let data = format!("Data: {}/{} bytes", self.data_length, self.data_capacity);
        match self.shape {
            Shape::List => format!(
                "List Type: {} Length: {} Capacity: {} {data} {block}",
                code_label(self.value_type),
                self.count,
                self.capacity
            ),
            Shape::Set => format!(
                "Set Type: {} Length: {} Removed: {} Capacity: {} {data} {block}",
                code_label(self.key_type),
                self.count.saturating_sub(self.dead),
                self.dead,
                self.capacity
            ),
            Shape::Map => format!(
                "Map Keys: {} Key Type: {} Value Type: {} Removed: {} Capacity: {} {data} \
                 {block}",
                self.count.saturating_sub(self.dead),
                code_label(self.key_type),
                code_label(self.value_type),
                self.dead,
                self.capacity
            ),
        }
    }

    /// Entry `index` (of `count`), checked against the data region.
    fn entry(&self, memory: &mut Memory, index: u64) -> Result<(u8, Entry), String> {
        let what = self.shape.name();
        if self.kind == COLLECTION_KIND_LIST_FIXED {
            return Ok((
                COLLECTION_ENTRY_FLAG_USED as u8,
                Entry {
                    key_offset: 0,
                    key_length: 0,
                    value_offset: index * self.width,
                    value_length: self.width,
                },
            ));
        }
        let at =
            self.address + COLLECTION_HEADER_SIZE as u64 + index * COLLECTION_ENTRY_SIZE as u64;
        let bytes = memory.read(at, COLLECTION_ENTRY_SIZE as u64, what)?;
        let word =
            |offset: usize| u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("8"));
        let flags = bytes[COLLECTION_ENTRY_OFFSET_FLAGS];
        let entry = Entry {
            key_offset: word(COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            key_length: word(COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            value_offset: word(COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            value_length: word(COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        };
        let removable = self.shape != Shape::List;
        if usize::from(flags) != COLLECTION_ENTRY_FLAG_USED
            && !(removable && usize::from(flags) == COLLECTION_ENTRY_FLAG_REMOVED)
        {
            return Err(format!(
                "not a {what} at 0x{:016x}: entry {index} has flags {flags}",
                self.address
            ));
        }
        let inside = |offset: u64, length: u64| {
            offset
                .checked_add(length)
                .is_some_and(|end| end <= self.data_length)
        };
        if !inside(entry.value_offset, entry.value_length)
            || (removable && !inside(entry.key_offset, entry.key_length))
        {
            return Err(format!(
                "not a {what} at 0x{:016x}: entry {index} points past its {} data bytes",
                self.address, self.data_length
            ));
        }
        Ok((flags, entry))
    }

    /// The entries in use, in order, skipping removed ones.
    fn live_entries(&self, memory: &mut Memory) -> Result<Vec<Entry>, String> {
        let mut live = Vec::new();
        for index in 0..self.count {
            let (flags, entry) = self.entry(memory, index)?;
            if usize::from(flags) == COLLECTION_ENTRY_FLAG_USED {
                live.push(entry);
            }
        }
        Ok(live)
    }

    fn key_at(&self, entry: &Entry) -> (u64, u64) {
        (self.data + entry.key_offset, entry.key_length)
    }

    fn value_at(&self, entry: &Entry) -> (u64, u64) {
        (self.data + entry.value_offset, entry.value_length)
    }
}

/// The table of elements `span` picks, each as its index, address, size and
/// value: a list's values, a set's elements, a map's keys or values.
fn elements(
    memory: &mut Memory,
    collection: &Collection,
    kind: Option<Kind>,
    span: Span,
    part: Part,
) -> Result<Vec<String>, String> {
    let entries = collection.live_entries(memory)?;
    let length = entries.len() as u64;
    let end = span.end.unwrap_or(length);
    let everything = span.start == 0 && span.end.is_none();
    if !everything && (span.start >= length || end > length) {
        return Err(format!(
            "no element {}: the {} has {length} (numbered from 0)",
            if span.start >= length {
                span.start
            } else {
                end - 1
            },
            collection.shape.name()
        ));
    }
    let code = match part {
        Part::Key => collection.key_type,
        Part::Value => collection.value_type,
    };
    let mut rows = Vec::new();
    for (index, entry) in entries
        .iter()
        .enumerate()
        .take(end as usize)
        .skip(span.start as usize)
    {
        let (at, size) = match part {
            Part::Key => collection.key_at(entry),
            Part::Value => collection.value_at(entry),
        };
        rows.push(vec![
            index.to_string(),
            format!("0x{at:016x}"),
            size.to_string(),
            payload_text(memory, code, kind, at, size)?,
        ]);
    }
    let heading = match (collection.shape, part) {
        (Shape::Map, Part::Key) => "Key",
        (Shape::Set, _) => "Element",
        _ => "Value",
    };
    Ok(super::prompt::table(
        &["Index", "Address", "Bytes", heading],
        rows,
    ))
}

/// A map entry's key and value, each with where it is.
fn entry_lines(
    memory: &mut Memory,
    collection: &Collection,
    index: u64,
    entry: &Entry,
) -> Result<Vec<String>, String> {
    let (key_at, key_size) = collection.key_at(entry);
    let (value_at, value_size) = collection.value_at(entry);
    Ok(vec![
        format!("Index: {index}"),
        format!(
            "Key: {} at 0x{key_at:016x} ({key_size} bytes)",
            payload_text(memory, collection.key_type, None, key_at, key_size)?
        ),
        format!(
            "Value: {} at 0x{value_at:016x} ({value_size} bytes)",
            payload_text(memory, collection.value_type, None, value_at, value_size)?
        ),
    ])
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
mod tests {
    use super::*;

    /// Where the fake arena block's usable space starts.
    const BASE: u64 = 0x1_0000;

    /// Fake memory: `bytes` at `BASE`, all of it one arena block's usable
    /// space, with a block header's worth of readable bytes before it.
    struct Fake {
        bytes: Vec<u8>,
    }

    impl Fake {
        fn new(bytes: Vec<u8>) -> Fake {
            Fake { bytes }
        }

        fn region(&self) -> Region {
            Region::of_block(0, 0, BASE - 32, 32 + self.bytes.len() as u64)
        }

        fn show(&self, address: u64, view: &View) -> Result<Vec<String>, String> {
            let bytes = self.bytes.clone();
            let mut read = |at: u64, length: usize| {
                at.checked_sub(BASE)
                    .and_then(|from| bytes.get(from as usize..from as usize + length))
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| format!("unmapped {at:#x}"))
            };
            show(&mut read, &[self.region()], address, view)
        }
    }

    fn words(values: &[u64]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    fn string_record(text: &[u8]) -> Vec<u8> {
        let mut bytes = (text.len() as u64).to_le_bytes().to_vec();
        bytes.extend(text);
        bytes.push(0);
        bytes
    }

    /// A collection block: header, `entries` (flags, key offset, key length,
    /// value offset, value length), then `data` padded to `data_capacity`.
    fn collection(
        kind: u8,
        key_type: u8,
        value_type: u8,
        dead: u32,
        count: u64,
        capacity: u64,
        entries: &[(u8, u64, u64, u64, u64)],
        data: &[u8],
        data_capacity: u64,
    ) -> Vec<u8> {
        let mut bytes = vec![kind, key_type, value_type, 1];
        bytes.extend((dead << 8).to_le_bytes());
        bytes.extend(words(&[count, capacity, data.len() as u64, data_capacity]));
        for (flags, key_offset, key_length, value_offset, value_length) in entries {
            bytes.push(*flags);
            bytes.extend([0; 7]);
            bytes.extend(words(&[
                *key_offset,
                *key_length,
                *value_offset,
                *value_length,
            ]));
        }
        if kind != 2 {
            bytes.resize(40 + capacity as usize * 40, 0);
        }
        bytes.extend(data);
        bytes.resize(bytes.len() + data_capacity as usize - data.len(), 0);
        bytes
    }

    fn all() -> Option<Span> {
        Some(Span {
            start: 0,
            end: None,
        })
    }

    #[test]
    fn scalars_show_their_bits_and_their_value() {
        let fake = Fake::new(
            [
                words(&[(-20_i64) as u64, 3.5_f64.to_bits(), 0x1_8000_0000, 125_000]),
                vec![1, 0x41, 2, 0],
                0xe9_u32.to_le_bytes().to_vec(),
            ]
            .concat(),
        );
        let value = |offset: u64, scalar: Scalar| {
            fake.show(BASE + offset, &View::Scalar(scalar))
                .map(|lines| lines.join("\n"))
        };
        assert_eq!(
            value(0, Scalar::Integer),
            Ok("Value: 0xffffffffffffffec (-20)".to_string())
        );
        assert_eq!(
            value(8, Scalar::Float),
            Ok("Value: 0x400c000000000000 (3.5)".to_string())
        );
        assert_eq!(
            value(16, Scalar::Fixed),
            Ok("Value: 0x0000000180000000 (1.50)".to_string())
        );
        assert_eq!(
            value(24, Scalar::Money),
            Ok("Value: 0x000000000001e848 (1.25000)".to_string())
        );
        assert_eq!(
            value(32, Scalar::Boolean),
            Ok("Value: 0x01 (TRUE)".to_string())
        );
        assert_eq!(value(33, Scalar::Byte), Ok("Value: 0x41 (65)".to_string()));
        assert_eq!(
            value(34, Scalar::Boolean),
            Err(format!(
                "not a Boolean at 0x{:016x}: its byte is 0x02, not 0 or 1",
                BASE + 34
            ))
        );
        assert_eq!(
            value(36, Scalar::Scalar),
            Ok("Value: 0x000000e9 (U+00E9 'é')".to_string())
        );
    }

    #[test]
    fn money_shows_all_five_places_and_its_sign() {
        assert_eq!(money_text(-300_000), "-3.00000");
        assert_eq!(money_text(1), "0.00001");
        assert_eq!(money_text(-1), "-0.00001");
    }

    #[test]
    fn a_string_literal_reads_as_a_string_outside_the_arena() {
        let bytes = string_record(b"function handler ");
        let mut read = |at: u64, length: usize| {
            at.checked_sub(BASE)
                .and_then(|from| bytes.get(from as usize..from as usize + length))
                .map(<[u8]>::to_vec)
                .ok_or_else(|| format!("unmapped {at:#x}"))
        };
        assert_eq!(
            show_literal(&mut read, BASE, "_mfb_str_2"),
            Ok(vec![
                "Length: 17 Value: \"function handler \" (the literal mfb_str_2)".to_string()
            ])
        );
    }

    #[test]
    fn memory_outside_the_arena_or_past_a_block_is_refused() {
        let fake = Fake::new(words(&[1]));
        assert_eq!(
            fake.show(0x10, &View::Scalar(Scalar::Integer)),
            Err(
                "0x0000000000000010 is not in an arena block, so it holds no Integer; \
                 'show mem' reads any memory"
                    .to_string()
            )
        );
        assert_eq!(
            fake.show(BASE + 4, &View::Scalar(Scalar::Integer)),
            Err(format!(
                "not an Integer at 0x{:016x}: its 8 bytes run past the end of their arena \
                 block",
                BASE + 4
            ))
        );
    }

    #[test]
    fn a_string_is_its_length_and_its_text_escaped() {
        let fake = Fake::new(string_record("a\"\\\n\u{1b}é".as_bytes()));
        assert_eq!(
            fake.show(BASE, &View::String),
            Ok(vec!["Length: 7 Value: \"a\\\"\\\\\\n\\u{1b}é\"".to_string()])
        );
        let fake = Fake::new(string_record(b"ab\xffd"));
        assert_eq!(
            fake.show(BASE, &View::String),
            Err(format!(
                "not a String at 0x{BASE:016x}: byte 2 of its value, 0xff, is not UTF-8"
            ))
        );
        let mut long = string_record(b"abc");
        long[0] = 200;
        let fake = Fake::new(long);
        assert_eq!(
            fake.show(BASE, &View::String),
            Err(format!(
                "not a String at 0x{BASE:016x}: its length, 200, runs past the end of its \
                 arena block"
            ))
        );
    }

    #[test]
    fn a_fixed_width_list_shows_its_header_and_its_elements() {
        let fake = Fake::new(collection(
            2,
            0,
            3,
            0,
            3,
            4,
            &[],
            &words(&[10, (-20_i64) as u64, 30]),
            32,
        ));
        let block = format!("0x{BASE:016x}-0x{:016x} (72 bytes)", BASE + 72);
        assert_eq!(
            fake.show(
                BASE,
                &View::List {
                    element: None,
                    span: None
                }
            ),
            Ok(vec![format!(
                "List Type: Integer Length: 3 Capacity: 4 Data: 24/32 bytes Block: {block}"
            )])
        );
        assert_eq!(
            fake.show(
                BASE,
                &View::List {
                    element: None,
                    span: Span::parse("1..")
                }
            ),
            Ok(vec![
                "Index | Address            | Bytes | Value".to_string(),
                format!(
                    "1     | 0x{:016x} | 8     | 0xffffffffffffffec (-20)",
                    BASE + 48
                ),
                format!(
                    "2     | 0x{:016x} | 8     | 0x000000000000001e (30)",
                    BASE + 56
                ),
            ])
        );
        assert_eq!(
            fake.show(
                BASE,
                &View::List {
                    element: None,
                    span: Span::parse("3")
                }
            ),
            Err("no element 3: the List has 3 (numbered from 0)".to_string())
        );
        assert_eq!(
            fake.show(
                BASE,
                &View::List {
                    element: Some(Kind::String),
                    span: Span::parse("0")
                }
            ),
            Err("the elements are Integer, not String".to_string())
        );
    }

    #[test]
    fn a_list_of_strings_reads_each_from_its_entry() {
        let fake = Fake::new(collection(
            0,
            0,
            6,
            0,
            2,
            2,
            &[(1, 0, 0, 0, 2), (1, 0, 0, 2, 3)],
            b"hibye",
            5,
        ));
        let data = BASE + 40 + 80;
        assert_eq!(
            fake.show(
                BASE,
                &View::List {
                    element: None,
                    span: all()
                }
            ),
            Ok(vec![
                "Index | Address            | Bytes | Value".to_string(),
                format!("0     | 0x{data:016x} | 2     | \"hi\""),
                format!("1     | 0x{:016x} | 3     | \"bye\"", data + 2),
            ])
        );
    }

    #[test]
    fn a_header_that_cannot_be_the_collection_asked_for_is_refused() {
        let map = Fake::new(collection(1, 6, 3, 0, 0, 0, &[], &[], 0));
        let list = View::List {
            element: None,
            span: None,
        };
        assert_eq!(
            map.show(BASE, &list),
            Err(format!(
                "not a List at 0x{BASE:016x}: its kind byte is 1, a Map"
            ))
        );
        let over = Fake::new(collection(2, 0, 3, 0, 5, 4, &[], &words(&[0; 4]), 32));
        assert_eq!(
            over.show(BASE, &list),
            Err(format!(
                "not a List at 0x{BASE:016x}: its count, 5, is over its capacity, 4"
            ))
        );
        let big = Fake::new(collection(2, 0, 3, 0, 1, 1, &[], &words(&[7]), 4096));
        let mut bytes = big.bytes.clone();
        bytes.truncate(64);
        assert_eq!(
            Fake::new(bytes).show(BASE, &list),
            Err(format!(
                "not a List at 0x{BASE:016x}: its 4136 bytes run past the end of its arena \
                 block"
            ))
        );
    }

    #[test]
    fn a_set_skips_removed_elements_and_counts_them() {
        let fake = Fake::new(collection(
            3,
            6,
            2,
            1,
            3,
            4,
            &[(1, 0, 3, 3, 1), (2, 4, 4, 8, 1), (1, 9, 5, 14, 1)],
            b"red\x01gone\x01green\x01",
            16,
        ));
        let data = BASE + 40 + 160;
        assert_eq!(
            fake.show(
                BASE,
                &View::Set {
                    element: None,
                    span: None
                }
            )
            .map(|lines| lines[0]
                .split(" Data:")
                .next()
                .unwrap_or_default()
                .to_string()),
            Ok("Set Type: String Length: 2 Removed: 1 Capacity: 4".to_string())
        );
        assert_eq!(
            fake.show(
                BASE,
                &View::Set {
                    element: None,
                    span: all()
                }
            ),
            Ok(vec![
                "Index | Address            | Bytes | Element".to_string(),
                format!("0     | 0x{data:016x} | 3     | \"red\""),
                format!("1     | 0x{:016x} | 5     | \"green\"", data + 9),
            ])
        );
    }

    #[test]
    fn a_map_shows_its_keys_its_values_an_entry_and_a_lookup() {
        let mut data = b"Ada\0\0\0\0\0".to_vec();
        data.extend(words(&[36]));
        data.extend(b"Grace\0\0\0");
        data.extend(words(&[85]));
        let fake = Fake::new(collection(
            1,
            6,
            3,
            0,
            2,
            2,
            &[(1, 0, 3, 8, 8), (1, 16, 5, 24, 8)],
            &data,
            32,
        ));
        let base = BASE + 40 + 80;
        let map = |view: MapView| fake.show(BASE, &View::Map(view));
        assert_eq!(
            map(MapView::Keys(None)),
            Ok(vec![
                "Index | Address            | Bytes | Key".to_string(),
                format!("0     | 0x{base:016x} | 3     | \"Ada\""),
                format!("1     | 0x{:016x} | 5     | \"Grace\"", base + 16),
            ])
        );
        assert_eq!(
            map(MapView::Values(None)),
            Ok(vec![
                "Index | Address            | Bytes | Value".to_string(),
                format!(
                    "0     | 0x{:016x} | 8     | 0x0000000000000024 (36)",
                    base + 8
                ),
                format!(
                    "1     | 0x{:016x} | 8     | 0x0000000000000055 (85)",
                    base + 24
                ),
            ])
        );
        let grace = Ok(vec![
            "Index: 1".to_string(),
            format!("Key: \"Grace\" at 0x{:016x} (5 bytes)", base + 16),
            format!(
                "Value: 0x0000000000000055 (85) at 0x{:016x} (8 bytes)",
                base + 24
            ),
        ]);
        assert_eq!(map(MapView::Entry(1)), grace);
        assert_eq!(map(MapView::Get("Grace".to_string())), grace);
        assert_eq!(
            map(MapView::Get("Nobody".to_string())),
            Err("the map has no key Nobody".to_string())
        );
        assert_eq!(
            map(MapView::Entry(2)),
            Err("no entry 2: the map has 2 (numbered from 0)".to_string())
        );
    }

    #[test]
    fn keys_are_encoded_as_their_type_stores_them() {
        assert_eq!(
            key_bytes(COLLECTION_TYPE_INTEGER, "-2"),
            Ok(words(&[(-2_i64) as u64]))
        );
        assert_eq!(key_bytes(COLLECTION_TYPE_INTEGER, "0x10"), Ok(words(&[16])));
        assert_eq!(key_bytes(COLLECTION_TYPE_BOOLEAN, "TRUE"), Ok(vec![1]));
        assert_eq!(key_bytes(COLLECTION_TYPE_BYTE, "255"), Ok(vec![255]));
        assert_eq!(
            key_bytes(COLLECTION_TYPE_SCALAR, "U+E9"),
            Ok(0xe9_u32.to_le_bytes().to_vec())
        );
        assert_eq!(
            key_bytes(COLLECTION_TYPE_BYTE, "256"),
            Err("'256' is not a Byte".to_string())
        );
        assert_eq!(
            key_bytes(COLLECTION_TYPE_OBJECT, "x"),
            Err(
                "an object (Set, record, enum, resource or function) key cannot be looked up; \
                 list the keys with 'keys'"
                    .to_string()
            )
        );
    }

    #[test]
    fn a_resource_shows_its_kind_flags_owner_and_fields() {
        let owner = ((0x16f0_0048_u64) << 12) - 2;
        let mut record = words(&[1, 3, 3, 0x2000, owner, 0x3000, 12, 1, 0, 0, 0, 1]);
        record.resize(112, 0);
        let fake = Fake::new(record.clone());
        assert_eq!(
            fake.show(BASE, &View::Resource),
            Ok(vec![format!(
                "Type: File Closed: Y Moved: Y Handle (fd): 3 State: 0x0000000000002000 Scope: \
                 0x{owner:x} (scope depth 2 of the frame whose token slot is 0x16f00048) Write \
                 Buffer: 0x3000 Write Filled: 12 Write Enabled: 1 Read Buffer: 0x0 Read \
                 Position: 0 Read Filled: 0 At EOF: 1"
            )])
        );
        record[0] = 99;
        assert_eq!(
            Fake::new(record).show(BASE, &View::Resource),
            Err(format!(
                "not a resource at 0x{BASE:016x}: its tag, 99, is no resource kind"
            ))
        );
    }

    #[test]
    fn views_parse_their_types_indexes_and_ranges() {
        assert_eq!(Kind::parse("Integer"), Some(Kind::Scalar(Scalar::Integer)));
        assert_eq!(Kind::parse("bool"), Some(Kind::Scalar(Scalar::Boolean)));
        assert_eq!(Kind::parse("record"), None);
        assert_eq!(
            Span::parse("4"),
            Some(Span {
                start: 4,
                end: Some(5)
            })
        );
        assert_eq!(
            Span::parse("2.."),
            Some(Span {
                start: 2,
                end: None
            })
        );
        assert_eq!(Span::parse("5..5"), None);
        assert_eq!(
            parse_view("list", &["resource", "0..2"], ""),
            Ok(View::List {
                element: Some(Kind::Resource),
                span: Span::parse("0..2")
            })
        );
        assert_eq!(
            parse_view("map", &["get", "\"a"], "show type map x1 get \"a b\""),
            Ok(View::Map(MapView::Get("a b".to_string())))
        );
    }
}
