//! `disassemble`/`dis`: the program's code, shown the way lldb shows it.
//!
//! A function's extent comes from wherever it is known: the executable's
//! symbol sizes (a `--debug` build), its import stubs, or a loaded library's
//! function starts. Without one, a listing runs to the first `ret` or 64
//! instructions. Code is read with every breakpoint's original instruction in
//! place, and decoded by the decoder for the host's architecture — AArch64, the
//! only one with a debugger backend, whose decoder knows exactly what the
//! compiler emits; anything else shows as `.word`.

use super::names::classify_defined;
use super::{FunctionRange, Inferior};
use crate::arch::aarch64::decode::{decode, Flow};

/// The most instructions a listing without a known extent shows.
pub(crate) const UNSIZED_LIMIT: usize = 64;
/// The longest function a listing reads.
const MAX_FUNCTION: u64 = 1 << 20;
/// Where a comment starts, counted from the instruction text.
const COMMENT_COLUMN: usize = 33;

/// What a `dis` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    /// The function holding the current thread's `pc`.
    Current,
    /// This many instructions from the current thread's `pc`.
    Count(u64),
    /// Every function with this name.
    Named(String),
}

/// How a function is named in a listing: what the developer wrote for their
/// own functions, otherwise the symbol less the leading `_` C adds (as lldb
/// prints it).
pub(crate) fn display_name(symbol: &str) -> String {
    if let Some(import) = symbol.strip_prefix("symbol stub for: ") {
        return format!("symbol stub for: {}", display_name(import));
    }
    if let (_, Some(written)) = classify_defined(symbol) {
        return written;
    }
    symbol.strip_prefix('_').unwrap_or(symbol).to_string()
}

/// `image`name:` — or `image`0x…:` for a function the image does not name.
fn header(range: &FunctionRange) -> String {
    match &range.name {
        Some(name) => format!("{}`{}:", range.image, display_name(name)),
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
        Request::Count(count) => {
            let range = inferior.function_at(pc)?.unwrap_or_else(|| unknown(pc));
            listing(inferior, &range, pc, None, Some(*count), pc)
        }
        Request::Named(name) => {
            let ranges = resolve(inferior, name)?;
            let mut lines = Vec::new();
            for range in ranges {
                if !lines.is_empty() {
                    lines.push(String::new());
                }
                lines.extend(listing(
                    inferior,
                    &range,
                    range.start,
                    range.size,
                    None,
                    pc,
                )?);
            }
            Ok(lines)
        }
    }
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
            format!(" <+{}>:", address.wrapping_sub(range.start))
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
        header(range)
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
                    Some(address) => named_address(inferior, address),
                    None => comment(inferior, range, instruction),
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
fn named_address(inferior: &mut dyn Inferior, address: u64) -> Option<String> {
    if let Some(symbol) = inferior.data_symbol_at(address).ok().flatten() {
        return Some(display_name(&symbol));
    }
    let function = inferior.function_at(address).ok().flatten()?;
    let name = display_name(function.name.as_deref()?);
    Some(if address == function.start {
        name
    } else {
        format!("{name} + {}", address - function.start)
    })
}

/// What lldb notes after an instruction: where a branch or call goes (an offset
/// in this function, else the function there), or a wide immediate's value.
fn comment(
    inferior: &mut dyn Inferior,
    range: &FunctionRange,
    instruction: &crate::arch::aarch64::decode::Instruction,
) -> Option<String> {
    if let Some(value) = instruction.value {
        return Some(format!("={value}"));
    }
    let target = instruction.target?;
    if instruction.flow != Flow::Branch && instruction.flow != Flow::Call {
        return None;
    }
    let within = range
        .size
        .is_some_and(|size| target >= range.start && target < range.start + size);
    if within {
        return Some(format!("<+{}>", target - range.start));
    }
    match inferior.function_at(target).ok().flatten() {
        Some(function) => {
            let name = function
                .name
                .as_deref()
                .map(display_name)
                .unwrap_or_else(|| format!("0x{:016x}", function.start));
            Some(if target == function.start {
                name
            } else {
                format!("{name} + {}", target - function.start)
            })
        }
        None => None,
    }
}
