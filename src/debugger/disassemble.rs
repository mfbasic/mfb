//! `disassemble`/`dis`: the program's code, shown the way lldb shows it.
//!
//! A function's extent comes from wherever it is known: the executable's
//! symbol sizes (a `--debug` build), its import stubs, or a loaded library's
//! function starts. Without one, a listing runs to the first `ret` or 64
//! instructions. Code is read with every breakpoint's original instruction in
//! place, and decoded by the decoder for the host's architecture — AArch64, the
//! only one with a debugger backend, whose decoder knows what the compiler
//! emits and the instructions of dyld and the system libraries; a word it does
//! not know (SVE, SME, memory tagging) shows as `.word`.

use super::names::{classify_defined, Collisions};
use super::{FunctionRange, Inferior};
use crate::arch::aarch64::decode::{decode, Flow};

/// The most instructions a listing without a known extent shows.
pub(crate) const UNSIZED_LIMIT: usize = 64;
/// The longest function a listing reads.
const MAX_FUNCTION: u64 = 1 << 20;
/// Where a comment starts, counted from the instruction text.
const COMMENT_COLUMN: usize = 33;

/// What a `dis` asks for. A count is never 0: a positive count is that many
/// instructions from the starting point, a negative one that many before it
/// and then the starting point itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    /// The function holding the current thread's `pc`.
    Current,
    /// Instructions around the current thread's `pc`.
    Count(i64),
    /// Instructions around an address: through the end of its function when
    /// no count is given.
    At { address: u64, count: Option<i64> },
    /// Every function with this name: all of it, or a count from its start.
    Named { name: String, count: Option<i64> },
}

/// The size of an AArch64 instruction; every one starts on a multiple of it.
const INSTRUCTION: u64 = 4;

/// How a function is named in a listing: what the developer wrote for their
/// own functions, otherwise the symbol less the leading `_` C adds (as lldb
/// prints it), tagged when an MFBASIC and a native function share the name
/// (`main <mfb>`, `main <native>`).
pub(crate) fn display_name(symbol: &str, collisions: &Collisions) -> String {
    at_offset(symbol, 0, collisions)
}

/// [`display_name`] `offset` bytes into the function, as lldb writes it
/// (`countPrimes + 72`), the tag last (`main + 8 <mfb>`).
pub(crate) fn at_offset(symbol: &str, offset: u64, collisions: &Collisions) -> String {
    if let Some(import) = symbol.strip_prefix("symbol stub for: ") {
        return format!("symbol stub for: {}", at_offset(import, offset, collisions));
    }
    let (origin, written) = classify_defined(symbol);
    let name = written.unwrap_or_else(|| symbol.strip_prefix('_').unwrap_or(symbol).to_string());
    let tag = collisions.tag(&name, origin);
    let mut shown = name;
    if offset != 0 {
        shown = format!("{shown} + {offset}");
    }
    match tag {
        Some(tag) => format!("{shown} {tag}"),
        None => shown,
    }
}

/// `image`name:` — or `image`0x…:` for a function the image does not name.
fn header(range: &FunctionRange, collisions: &Collisions) -> String {
    match &range.name {
        Some(name) => format!("{}`{}:", range.image, display_name(name, collisions)),
        None => format!("{}`0x{:016x}:", range.image, range.start),
    }
}

/// The listing for `request`, with `pc` (the current thread's) marked `->`.
pub(crate) fn disassemble(
    inferior: &mut dyn Inferior,
    pc: u64,
    request: &Request,
) -> Result<Vec<String>, String> {
    match request {
        Request::Current => {
            let range = inferior.function_at(pc)?;
            let lines = match &range {
                Some(range) => listing(inferior, range, range.start, range.size, None, pc)?,
                None => listing(inferior, &unknown(pc), pc, None, None, pc)?,
            };
            Ok(lines)
        }
        Request::Count(count) => around(inferior, pc, Some(*count), pc),
        Request::At { address, count } => {
            if address % INSTRUCTION != 0 {
                return Err(format!(
                    "0x{address:016x} is not the start of an instruction: they start every \
                     {INSTRUCTION} bytes"
                ));
            }
            around(inferior, *address, *count, pc)
        }
        Request::Named { name, count } => {
            let ranges = resolve(inferior, name)?;
            let mut lines = Vec::new();
            for range in ranges {
                if !lines.is_empty() {
                    lines.push(String::new());
                }
                lines.extend(match count {
                    None => listing(inferior, &range, range.start, range.size, None, pc)?,
                    Some(count) => counted(inferior, &range, range.start, *count, pc)?,
                });
            }
            Ok(lines)
        }
    }
}

/// The listing around `anchor`, headed by the function holding it: `count`
/// instructions as `Request` describes, or with none the rest of that
/// function from `anchor`.
fn around(
    inferior: &mut dyn Inferior,
    anchor: u64,
    count: Option<i64>,
    pc: u64,
) -> Result<Vec<String>, String> {
    let range = inferior
        .function_at(anchor)?
        .filter(|range| range.start <= anchor)
        .unwrap_or_else(|| unknown(anchor));
    match count {
        Some(count) => counted(inferior, &range, anchor, count, pc),
        None => {
            let rest = range
                .size
                .map(|size| (range.start + size).saturating_sub(anchor));
            listing(inferior, &range, anchor, rest, None, pc)
        }
    }
}

/// `count` instructions from `anchor`, or before it and then `anchor`.
fn counted(
    inferior: &mut dyn Inferior,
    range: &FunctionRange,
    anchor: u64,
    count: i64,
    pc: u64,
) -> Result<Vec<String>, String> {
    if count > 0 {
        return listing(inferior, range, anchor, None, Some(count as u64), pc);
    }
    let back = count.unsigned_abs();
    let from = back
        .checked_mul(INSTRUCTION)
        .and_then(|bytes| anchor.checked_sub(bytes))
        .ok_or_else(|| format!("there are not {back} instructions before 0x{anchor:016x}"))?;
    listing(inferior, range, from, None, Some(back + 1), pc)
}

/// The address of every `ret` in `range`: through its size when known, else
/// the first within [`UNSIZED_LIMIT`] instructions.
pub(crate) fn returns(
    inferior: &mut dyn Inferior,
    range: &FunctionRange,
) -> Result<Vec<u64>, String> {
    let length = match range.size {
        Some(size) => size.min(MAX_FUNCTION) as usize,
        None => UNSIZED_LIMIT * 4,
    };
    let code = read_up_to(inferior, range.start, length)?;
    let mut found = Vec::new();
    for (index, word) in code.chunks_exact(4).enumerate() {
        let word = u32::from_le_bytes(word.try_into().expect("4 bytes"));
        let address = range.start + index as u64 * 4;
        if decode(word, address).is_some_and(|instruction| instruction.flow == Flow::Return) {
            found.push(address);
            if range.size.is_none() {
                break;
            }
        }
    }
    Ok(found)
}

/// A range for code no image names: just its address.
fn unknown(address: u64) -> FunctionRange {
    FunctionRange {
        image: String::new(),
        name: None,
        start: address,
        size: None,
    }
}

/// The functions `name` means: symbols spelled `name` (the executable's, its
/// imports', the libraries'), or else functions written `name`; one listing
/// per distinct start.
fn resolve(inferior: &mut dyn Inferior, name: &str) -> Result<Vec<FunctionRange>, String> {
    let functions = inferior.functions().unwrap_or_default();
    let mut starts = Vec::new();
    let mut unbound = false;
    for pass in [true, false] {
        for function in &functions {
            let matches = if pass {
                function.name == name
            } else if function.linked {
                super::names::linked_name(&function.name) == name
            } else {
                classify_defined(&function.name).1.as_deref() == Some(name)
            };
            if !matches {
                continue;
            }
            match function.address {
                Some(address) => starts.push(address),
                None => unbound = true,
            }
        }
        let symbol = if pass {
            name.to_string()
        } else {
            format!("_{name}")
        };
        let mut ranges = inferior
            .library_functions(&symbol)?
            .into_iter()
            .map(|range| range.start)
            .collect::<Vec<_>>();
        starts.append(&mut ranges);
        if !starts.is_empty() {
            break;
        }
    }
    starts.sort_unstable();
    starts.dedup();
    if starts.is_empty() {
        return Err(if unbound {
            format!("'{name}' is not loaded yet; run the program until it has loaded its libraries")
        } else {
            format!("no function is named '{name}'; type 'list func' for the functions")
        });
    }
    let mut ranges = Vec::new();
    for start in starts {
        ranges.push(
            inferior
                .function_at(start)?
                .filter(|range| range.start == start)
                .unwrap_or_else(|| unknown(start)),
        );
    }
    Ok(ranges)
}

/// One listing: the header, then an instruction per line from `from`, through
/// the function's `size` when known, else `count` instructions, else to the
/// first `ret` or [`UNSIZED_LIMIT`].
fn listing(
    inferior: &mut dyn Inferior,
    range: &FunctionRange,
    from: u64,
    size: Option<u64>,
    count: Option<u64>,
    pc: u64,
) -> Result<Vec<String>, String> {
    let limit = match (count, size) {
        (Some(count), _) => count.min(MAX_FUNCTION / 4),
        (None, Some(size)) => size.min(MAX_FUNCTION) / 4,
        (None, None) => UNSIZED_LIMIT as u64,
    };
    let code = read_up_to(inferior, from, limit as usize * 4)?;
    let collisions = Collisions::of(&inferior.functions().unwrap_or_default());
    let mut rows = Vec::new();
    for (index, word) in code.chunks_exact(4).enumerate() {
        let address = from + index as u64 * 4;
        let word = u32::from_le_bytes(word.try_into().expect("4 bytes"));
        let decoded = decode(word, address);
        let returns = decoded
            .as_ref()
            .is_some_and(|instruction| instruction.flow == Flow::Return);
        rows.push((address, decoded, word));
        if returns && count.is_none() && size.is_none() {
            break;
        }
    }
    let offset = |address: u64| {
        if range.image.is_empty() && range.name.is_none() {
            String::new()
        } else {
            // An instruction before the function's start (a listing counted
            // back past it) shows how far before.
            match address.checked_sub(range.start) {
                Some(after) => format!(" <+{after}>:"),
                None => format!(" <-{}>:", range.start - address),
            }
        }
    };
    let width = rows
        .iter()
        .map(|(address, _, _)| offset(*address).len())
        .max()
        .unwrap_or(0);
    // Each register an `adrp` set, and the page it holds, until overwritten.
    let mut pages: Vec<(u32, u64)> = Vec::new();
    let mut lines = vec![if range.image.is_empty() {
        format!("0x{:016x}:", range.start)
    } else {
        header(range, &collisions)
    }];
    for (address, decoded, word) in rows {
        let marker = if address == pc { "->  " } else { "    " };
        let text = match &decoded {
            Some(instruction) => instruction.text.clone(),
            None => format!(".word  {word:#010x}"),
        };
        let comment = match &decoded {
            Some(instruction) => {
                let paged = instruction.page_offset.and_then(|(register, offset)| {
                    pages
                        .iter()
                        .find(|(page_register, _)| *page_register == register)
                        .map(|(_, page)| page + offset)
                });
                if let Some((register, page)) = instruction.page {
                    pages.retain(|(other, _)| *other != register);
                    pages.push((register, page));
                }
                match paged {
                    Some(address) => named_address(inferior, address, &collisions),
                    None => comment(inferior, range, instruction, &collisions),
                }
            }
            None => None,
        };
        let mut line = format!(
            "{marker}0x{address:016x}{:<width$} {text}",
            if offset(address).is_empty() {
                ":".to_string()
            } else {
                offset(address)
            }
        );
        if let Some(comment) = comment {
            let pad = COMMENT_COLUMN.saturating_sub(text.len()).max(1);
            line = format!("{line}{:pad$}; {comment}", "");
        }
        lines.push(line.trim_end().to_string());
    }
    Ok(lines)
}

/// `length` bytes of code from `address`, or as many as can be read: a
/// listing without a known extent may run past the end of mapped memory.
fn read_up_to(inferior: &mut dyn Inferior, address: u64, length: usize) -> Result<Vec<u8>, String> {
    match inferior.read_code(address, length) {
        Ok(code) => Ok(code),
        Err(message) => {
            let mut code = Vec::new();
            while code.len() < length {
                match inferior.read_code(address + code.len() as u64, 4) {
                    Ok(word) => code.extend(word),
                    Err(_) if !code.is_empty() => break,
                    Err(_) => return Err(message),
                }
            }
            Ok(code)
        }
    }
}

/// The name of what an `adrp` + `add` pair addresses: a data symbol, else a
/// function, else nothing.
fn named_address(
    inferior: &mut dyn Inferior,
    address: u64,
    collisions: &Collisions,
) -> Option<String> {
    if let Some(symbol) = inferior.data_symbol_at(address).ok().flatten() {
        return Some(display_name(&symbol, collisions));
    }
    let function = inferior.function_at(address).ok().flatten()?;
    Some(at_offset(
        function.name.as_deref()?,
        address - function.start,
        collisions,
    ))
}

/// What lldb notes after an instruction: where a branch, call, `adr` or literal
/// load points (an offset in this function, else the function there), or the
/// value a `mov` of an immediate sets.
fn comment(
    inferior: &mut dyn Inferior,
    range: &FunctionRange,
    instruction: &crate::arch::aarch64::decode::Instruction,
    collisions: &Collisions,
) -> Option<String> {
    if let Some(value) = instruction.value {
        return Some(format!("={value}"));
    }
    let target = instruction.target?;
    let within = range
        .size
        .is_some_and(|size| target >= range.start && target < range.start + size);
    if within {
        return Some(format!("<+{}>", target - range.start));
    }
    match inferior.function_at(target).ok().flatten() {
        Some(function) => {
            let offset = target - function.start;
            Some(match function.name.as_deref() {
                Some(name) => at_offset(name, offset, collisions),
                None if offset == 0 => format!("0x{:016x}", function.start),
                None => format!("0x{:016x} + {offset}", function.start),
            })
        }
        None => None,
    }
}
