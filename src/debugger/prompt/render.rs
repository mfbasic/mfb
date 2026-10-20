//! plan-176-L: the `mfb dbg` prompt's render code. The command loop `run` and the help
//! text live in `debugger/prompt/mod.rs`.

use super::*;

/// What separates a listing's columns.
const COLUMN_SEPARATOR: &str = " | ";

/// Lay out a listing: a line of `headers`, then one line per row (a cell for
/// each header), the columns separated by ` | ` and each but the last
/// left-aligned to its widest entry, header included. A listing with no rows
/// has no lines, not even the header.
pub(crate) fn table(headers: &[&str], rows: Vec<Vec<String>>) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut widths = headers
        .iter()
        .map(|header| header.chars().count())
        .collect::<Vec<_>>();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let last = headers.len() - 1;
    std::iter::once(headers.iter().map(|header| header.to_string()).collect())
        .chain(rows)
        .map(|row: Vec<String>| {
            row.iter()
                .zip(&widths)
                .enumerate()
                .map(|(at, (cell, width))| {
                    if at == last {
                        cell.clone()
                    } else {
                        format!("{cell:<width$}")
                    }
                })
                .collect::<Vec<_>>()
                .join(COLUMN_SEPARATOR)
        })
        .collect()
}

/// The `list func` lines: the program's own functions, then the runtime's, then
/// linked ones, each group in address order (a linked function not yet bound
/// last), symbols that tie in symbol-table order, and a name repeated at the
/// same address once.
pub(crate) fn function_lines(symbols: &[FunctionSymbol], filter: &FunctionFilter) -> Vec<String> {
    let collisions = Collisions::of(symbols);
    let mut rows = symbols
        .iter()
        .filter_map(|symbol| {
            let (origin, written) = classify(symbol);
            if !filter.admits(origin, &symbol.name, written.as_deref()) {
                return None;
            }
            let written = written.map(|name| collisions.tagged(name, origin));
            Some((origin, symbol.address, &symbol.name, written))
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|(origin, address, _, _)| (*origin, address.is_none(), *address));
    // The same function named again at the same address (an import's repeated
    // records) is shown once; one name at two addresses is shown twice.
    let mut seen = std::collections::HashSet::new();
    rows.retain(|(_, address, name, _)| seen.insert((*address, name.to_string())));
    let mut origins = rows.iter().map(|(origin, ..)| *origin).collect::<Vec<_>>();
    origins.dedup();
    let mut lines = table(
        &["Type", "Address", "Symbol", "Written name"],
        rows.into_iter()
            .map(|(origin, address, name, written)| {
                vec![
                    origin.code().to_string(),
                    address.map_or_else(|| "-".to_string(), |at| format!("0x{at:016x}")),
                    name.clone(),
                    written.unwrap_or_else(|| "-".to_string()),
                ]
            })
            .collect(),
    );
    // A legend for the types the listing shows.
    if !origins.is_empty() {
        lines.push(String::new());
        lines.extend(
            origins
                .into_iter()
                .map(|origin| format!("{}  {}", origin.code(), origin.legend())),
        );
    }
    lines
}

/// The `list reg` lines: one register per line, the value in full-width hex and
/// then in decimal — signed for a 64-bit register, as MFBASIC's `Integer` is.
pub(crate) fn register_lines(registers: &[Register]) -> Vec<String> {
    table(
        &["Register", "Hex", "Decimal"],
        registers
            .iter()
            .map(|register| {
                let (hex, decimal) = register_value(register);
                vec![register.name.clone(), hex, decimal]
            })
            .collect(),
    )
}

/// The lines of a bare `list reg`: the registers that have a role in compiled
/// code, as [`register_lines`] shows them, with the role in a fourth column.
pub(crate) fn common_register_lines(registers: &[Register]) -> Vec<String> {
    table(
        &["Register", "Hex", "Decimal", "Role"],
        registers
            .iter()
            .filter_map(|register| {
                let role = register.role?;
                let (hex, decimal) = register_value(register);
                Some(vec![register.name.clone(), hex, decimal, role.to_string()])
            })
            .collect(),
    )
}

/// A register's value as `list reg` shows it: in full-width hex, and in
/// decimal in parentheses.
pub(super) fn register_value(register: &Register) -> (String, String) {
    match register.value {
        RegisterValue::Bits64(value) => (format!("0x{value:016x}"), format!("({})", value as i64)),
        RegisterValue::Bits128(value) => (format!("0x{value:032x}"), format!("({value})")),
    }
}

/// The `list thread` lines: `<id> <name> <pc> <task>`. A thread's name is the
/// one it gave itself, else the function it is in, else its system id.
pub(crate) fn thread_lines(
    threads: &[ThreadFacts],
    roles: &[Role],
    functions: &[FunctionSymbol],
) -> Vec<String> {
    table(
        &["ID", "Name", "PC", "Task"],
        threads
            .iter()
            .zip(roles)
            .enumerate()
            .map(|(id, (thread, role))| {
                let name = thread
                    .name
                    .clone()
                    .or_else(|| runtime::locate(functions, thread.pc))
                    .unwrap_or_else(|| format!("thread-{:#x}", thread.id));
                vec![
                    id.to_string(),
                    name,
                    format!("0x{:016x}", thread.pc),
                    role.label().to_string(),
                ]
            })
            .collect(),
    )
}

/// The `list arena` lines: `<id> <thread id> <address> <size in KB>`, the size
/// rounded up to whole kilobytes and `-` for an arena no thread allocates from.
pub(crate) fn arena_lines(arenas: &[Arena]) -> Vec<String> {
    table(
        &["ID", "Thread", "Address", "Size (KB)"],
        arenas
            .iter()
            .map(|arena| {
                vec![
                    arena.id.to_string(),
                    arena
                        .thread
                        .map_or_else(|| "-".to_string(), |id| id.to_string()),
                    format!("0x{:016x}", arena.address),
                    arena.mapped.div_ceil(1024).to_string(),
                ]
            })
            .collect(),
    )
}

/// The width of the `show arena` name column: the longest field name.
const FIELD_WIDTH: usize = 17;

/// The `Arena State:` lines: each field, an array as its count of non-empty
/// entries followed by each one, every value at its own width in hex.
pub(crate) fn state_lines(state: &[u8]) -> Vec<String> {
    let value = |at: usize, width: usize| -> String {
        match width {
            16 => format!(
                "0x{:032x}",
                u128::from_le_bytes(state[at..at + 16].try_into().expect("16 bytes"))
            ),
            _ => format!(
                "0x{:016x}",
                u64::from_le_bytes(state[at..at + 8].try_into().expect("8 bytes"))
            ),
        }
    };
    let mut lines = vec!["Arena State:".to_string()];
    for field in ARENA_STATE_FIELDS {
        if field.count == 1 {
            lines.push(format!(
                "  {:<FIELD_WIDTH$} {}",
                field.name,
                value(field.offset, field.width)
            ));
            continue;
        }
        let entries = (0..field.count)
            .map(|index| (index, field.offset + index * field.width))
            .filter(|(_, at)| state[*at..*at + field.width].iter().any(|byte| *byte != 0))
            .collect::<Vec<_>>();
        lines.push(format!(
            "  {:<FIELD_WIDTH$} {} non-empty",
            format!("{}[{}]", field.name, field.count),
            entries.len()
        ));
        for (index, at) in entries {
            lines.push(format!(
                "    {:<width$} {}",
                format!("[{index}]"),
                value(at, field.width),
                width = FIELD_WIDTH - 2
            ));
        }
    }
    lines
}

/// The `Arena Block (#):` sections, oldest block first.
pub(crate) fn block_lines(blocks: &[Block]) -> Vec<String> {
    let mut lines = Vec::new();
    for (number, block) in blocks.iter().enumerate() {
        lines.push(format!("Arena Block ({number}):"));
        for (name, value) in [
            ("prevBlock", block.prev),
            ("blockSize", block.size),
            ("usableCapacity", block.usable),
            ("bumpOffset", block.bump),
        ] {
            lines.push(format!("  {name:<FIELD_WIDTH$} 0x{value:016x}"));
        }
    }
    lines
}

/// Hex-dump lines of 16 bytes: the offset in the block (hex digits in groups
/// of four), the bytes, and the printable ones.
pub(crate) fn dump_lines(offset: u64, bytes: &[u8]) -> Vec<String> {
    hex_dump(offset, bytes, |at| {
        let digits = format!("{at:08x}");
        let grouped = digits
            .as_bytes()
            .rchunks(4)
            .rev()
            .map(|group| std::str::from_utf8(group).expect("hex digits"))
            .collect::<Vec<_>>()
            .join("_");
        format!("0x{grouped}")
    })
}

/// Hex-dump lines of memory, each led by its full address.
pub(crate) fn memory_lines(address: u64, bytes: &[u8]) -> Vec<String> {
    hex_dump(address, bytes, |at| format!("0x{at:016x}"))
}

/// Lines of 16 bytes from `start`, each led by `label` of its first byte's
/// position, then the bytes in hex and the printable ones.
fn hex_dump(start: u64, bytes: &[u8], label: impl Fn(u64) -> String) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(row, chunk)| {
            let label = label(start + row as u64 * 16);
            let hex = (0..16)
                .map(|index| {
                    chunk
                        .get(index)
                        .map_or_else(|| "  ".to_string(), |byte| format!("{byte:02x}"))
                })
                .collect::<Vec<_>>()
                .join(" ");
            let text = chunk
                .iter()
                .map(|byte| {
                    if (0x20..0x7f).contains(byte) {
                        char::from(*byte)
                    } else {
                        '.'
                    }
                })
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" ");
            format!("{label} {hex} | {text}")
        })
        .collect()
}
