//! plan-176-L: the debugger's value view code. Reading and rendering a value (`show`,
//! `Memory`) lives in `debugger/values/mod.rs`. This part needs nothing from its parent,
//! so it has no `use super::*;`.

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
    pub(super) fn width(self) -> u64 {
        match self {
            Scalar::Boolean | Scalar::Byte => 1,
            Scalar::Scalar => 4,
            Scalar::Integer | Scalar::Float | Scalar::Fixed | Scalar::Money => 8,
        }
    }

    pub(super) fn name(self) -> &'static str {
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

    pub(super) fn name(self) -> &'static str {
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
            None => number(word).and_then(|at| {
                at.checked_add(1).map(|end| Span {
                    start: at,
                    end: Some(end),
                })
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

pub(super) fn span(word: &str) -> Result<Span, String> {
    Span::parse(word)
        .ok_or_else(|| format!("'{word}' is not an index or a range (3, 0..10, 2..) or a type"))
}

/// A key as typed, less one pair of surrounding double quotes.
fn unquote(key: &str) -> &str {
    key.strip_prefix('"')
        .and_then(|key| key.strip_suffix('"'))
        .unwrap_or(key)
}
