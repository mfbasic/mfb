//! plan-176-L: the debugger's value collection code. Reading and rendering a value (`show`,
//! `Memory`) lives in `debugger/values/mod.rs`.

use super::*;

/// Which collection a view expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    List,
    Set,
    Map,
}

impl Shape {
    pub(super) fn name(self) -> &'static str {
        match self {
            Shape::List => "List",
            Shape::Set => "Set",
            Shape::Map => "Map",
        }
    }
}

/// A collection header, checked.
#[derive(Debug, Clone)]
pub(super) struct Collection {
    pub(super) shape: Shape,
    pub(super) address: u64,
    pub(super) kind: usize,
    pub(super) key_type: usize,
    pub(super) value_type: usize,
    pub(super) count: u64,
    pub(super) capacity: u64,
    pub(super) data_length: u64,
    pub(super) data_capacity: u64,
    /// Removed entries still in place (a `Map`'s or `Set`'s tombstones).
    pub(super) dead: u64,
    /// Where the data region starts.
    pub(super) data: u64,
    /// A fixed-width list's element width; 0 for entry-addressed payloads.
    pub(super) width: u64,
    /// The block's whole size.
    pub(super) size: u64,
}

/// One lookup entry: where its key and value payloads are in the data region.
#[derive(Debug, Clone, Copy)]
pub(super) struct Entry {
    key_offset: u64,
    key_length: u64,
    value_offset: u64,
    value_length: u64,
}

/// Which payload of an entry an element listing shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    Key,
    Value,
}

impl Collection {
    /// The one-line summary of the header.
    pub(super) fn summary(&self) -> String {
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
    pub(super) fn entry(&self, memory: &mut Memory, index: u64) -> Result<(u8, Entry), String> {
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
    pub(super) fn live_entries(&self, memory: &mut Memory) -> Result<Vec<Entry>, String> {
        let mut live = Vec::new();
        for index in 0..self.count {
            let (flags, entry) = self.entry(memory, index)?;
            if usize::from(flags) == COLLECTION_ENTRY_FLAG_USED {
                live.push(entry);
            }
        }
        Ok(live)
    }

    pub(super) fn key_at(&self, entry: &Entry) -> (u64, u64) {
        (self.data + entry.key_offset, entry.key_length)
    }

    fn value_at(&self, entry: &Entry) -> (u64, u64) {
        (self.data + entry.value_offset, entry.value_length)
    }
}

/// The table of elements `span` picks, each as its index, address, size and
/// value: a list's values, a set's elements, a map's keys or values.
pub(super) fn elements(
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
pub(super) fn entry_lines(
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
