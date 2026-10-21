//! `disassemble`/`dis`: the program's code, shown the way lldb shows it.
//!
//! A function's extent comes from wherever it is known: the executable's
//! symbol sizes (a `--debug` build), its import stubs, or a loaded library's
//! function starts. Without one, a listing runs to the first `ret` or 64
//! instructions. Code is read with every breakpoint's original instruction in
//! place, and decoded by the decoder of the debuggee's processor
//! ([`Arch::decode`]), which knows what the compiler emits and the instructions
//! of the system libraries; an instruction it does not know (on AArch64: SVE,
//! SME, memory tagging) shows as `.word` (4 bytes) or `.short` (2).

use super::arch::{Arch, Decoded};
use super::names::{classify, classify_defined, Collisions};
use super::{FunctionRange, FunctionSymbol, Inferior};

/// The most instructions a listing without a known extent shows.
pub(crate) const UNSIZED_LIMIT: usize = 64;
/// The longest function a listing reads.
const MAX_FUNCTION: u64 = 1 << 20;
/// Where a comment starts, counted from the instruction text.
const COMMENT_COLUMN: usize = 33;

/// The instructions a counted `dis` shows around its starting point: `length`
/// of them, from `back` instructions before it. `length` is never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    pub(crate) back: u64,
    pub(crate) length: u64,
}

impl Window {
    /// One count, never 0: a positive count is that many instructions from the
    /// starting point, a negative one that many before it and then the
    /// starting point itself.
    pub(crate) fn count(count: i64) -> Window {
        if count > 0 {
            Window {
                back: 0,
                length: count as u64,
            }
        } else {
            Window {
                back: count.unsigned_abs(),
                length: count.unsigned_abs() + 1,
            }
        }
    }
}

/// What a `dis` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    /// The function holding the current thread's `pc`.
    Current,
    /// Instructions around the current thread's `pc`.
    Count(Window),
    /// Instructions around an address: through the end of its function when
    /// no count is given.
    At { address: u64, count: Option<Window> },
    /// Every function with this name: all of it, or a count from its start.
    Named { name: String, count: Option<Window> },
}

/// How a function is named in a listing: what the developer wrote for it,
/// otherwise its symbol (a system library's as C names it), tagged when an
/// MFBASIC and a native function share the name (`exit <mfb>`,
/// `exit <native>`).
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
    let name = written.unwrap_or_else(|| super::names::plain_name(symbol));
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

/// `image`name:` — or `image`0x…:` for a function the image does not name,
/// or `0x…:` for code no image holds.
fn header(range: &FunctionRange, collisions: &Collisions) -> String {
    match &range.name {
        _ if range.image.is_empty() => format!("0x{:016x}:", range.start),
        Some(name) => format!("{}`{}:", range.image, display_name(name, collisions)),
        None => format!("{}`0x{:016x}:", range.image, range.start),
    }
}

/// Whether `address` is in `range`: at or after its start, and before its end
/// when its extent is known.
fn holds(range: &FunctionRange, address: u64) -> bool {
    range.start <= address && range.size.is_none_or(|size| address - range.start < size)
}

/// The listing for `request`, with `pc` (the current thread's) marked `->`.
pub(crate) fn disassemble(
    inferior: &mut dyn Inferior,
    pc: u64,
    request: &Request,
) -> Result<Vec<String>, String> {
    let alignment = inferior.arch().alignment;
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
            if address % alignment != 0 {
                return Err(format!(
                    "0x{address:016x} is not the start of an instruction: they start every \
                     {alignment} bytes"
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

/// The listing around `anchor`, headed by the function holding it: the
/// instructions `count` describes, or with none the rest of that function from
/// `anchor`.
fn around(
    inferior: &mut dyn Inferior,
    anchor: u64,
    count: Option<Window>,
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

/// The instructions `window` picks around `anchor`.
fn counted(
    inferior: &mut dyn Inferior,
    range: &FunctionRange,
    anchor: u64,
    window: Window,
    pc: u64,
) -> Result<Vec<String>, String> {
    let back = window.back;
    let arch = inferior.arch();
    let from = if arch.alignment == arch.longest as u64 {
        // Every instruction is the same size: count back by bytes.
        back.checked_mul(arch.alignment)
            .and_then(|bytes| anchor.checked_sub(bytes))
    } else {
        start_before(inferior, range, anchor, back)?
    }
    .ok_or_else(|| format!("there are not {back} instructions before 0x{anchor:016x}"))?;
    listing(inferior, range, from, None, Some(window.length), pc)
}

/// The start of the instruction `back` instructions before `anchor`, for code
/// whose instructions differ in size: found by decoding forward from the start
/// of `range` (the function holding `anchor`), and of the functions before it
/// while that is not far enough back. `None` when no function before it holds
/// code to count through.
fn start_before(
    inferior: &mut dyn Inferior,
    range: &FunctionRange,
    anchor: u64,
    back: u64,
) -> Result<Option<u64>, String> {
    let arch = inferior.arch();
    // Instruction starts before `anchor`, in address order.
    let mut starts = Vec::<u64>::new();
    let (mut start, mut end) = (range.start.min(anchor), anchor);
    loop {
        if start < end {
            let code = inferior.read_code(start, (end - start) as usize)?;
            let found = instructions(arch, &code, start)
                .map(|(address, _)| address)
                .collect::<Vec<_>>();
            starts.splice(0..0, found);
        }
        if let Some(index) = starts.len().checked_sub(back as usize) {
            return Ok(Some(starts[index]));
        }
        let Some(previous) = start
            .checked_sub(1)
            .map(|before| inferior.function_at(before))
            .transpose()?
            .flatten()
            .filter(|previous| previous.start < start && start - previous.start <= MAX_FUNCTION)
        else {
            return Ok(None);
        };
        (start, end) = (previous.start, start);
    }
}

/// The address of every `ret` in `range`: through its size when known, else
/// the first within [`UNSIZED_LIMIT`] instructions.
pub(crate) fn returns(
    inferior: &mut dyn Inferior,
    range: &FunctionRange,
) -> Result<Vec<u64>, String> {
    let arch = inferior.arch();
    let (length, limit) = match range.size {
        Some(size) => (size.min(MAX_FUNCTION) as usize, usize::MAX),
        None => (UNSIZED_LIMIT * arch.longest, UNSIZED_LIMIT),
    };
    let code = read_up_to(inferior, range.start, length)?;
    let mut found = Vec::new();
    for (address, bytes) in instructions(arch, &code, range.start).take(limit) {
        if (arch.decode)(bytes, address).is_some_and(|instruction| instruction.returns) {
            found.push(address);
            if range.size.is_none() {
                break;
            }
        }
    }
    Ok(found)
}

/// Each instruction in `code`, which starts at `from`, with its address: as
/// many as fit whole.
pub(crate) fn instructions<'a>(
    arch: &'a Arch,
    code: &'a [u8],
    from: u64,
) -> impl Iterator<Item = (u64, &'a [u8])> + 'a {
    let mut offset = 0;
    std::iter::from_fn(move || {
        let rest = code.get(offset..)?;
        let length = (arch.length)(rest);
        let bytes = rest.get(..length)?;
        let address = from + offset as u64;
        offset += length;
        Some((address, bytes))
    })
}

/// How a listing shows an instruction the decoder does not know: the 4 bytes
/// as a `.word`, 2 as a `.short`.
fn undecoded(bytes: &[u8]) -> String {
    match *bytes {
        [a, b] => format!(".short {:#06x}", u16::from_le_bytes([a, b])),
        [a, b, c, d] => format!(".word  {:#010x}", u32::from_le_bytes([a, b, c, d])),
        _ => format!(
            ".byte  {}",
            bytes
                .iter()
                .map(|byte| format!("{byte:#04x}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
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
/// imports', the libraries'), or else the one function written `name` — an
/// error when several functions are written so; one listing per distinct
/// start.
pub(crate) fn resolve(
    inferior: &mut dyn Inferior,
    name: &str,
) -> Result<Vec<FunctionRange>, String> {
    let functions = inferior.functions().unwrap_or_default();
    let mut starts = Vec::new();
    let mut unbound = false;
    let mut take = |matches: &[&FunctionSymbol], starts: &mut Vec<u64>| {
        for function in matches {
            match function.address {
                Some(address) => starts.push(address),
                None => unbound = true,
            }
        }
    };
    let spelled = functions
        .iter()
        .filter(|function| function.name == name)
        .collect::<Vec<_>>();
    take(&spelled, &mut starts);
    starts.extend(
        inferior
            .library_functions(name)?
            .into_iter()
            .map(|range| range.start),
    );
    if starts.is_empty() {
        let written = functions
            .iter()
            .filter(|function| classify(function).1.as_deref() == Some(name))
            .collect::<Vec<_>>();
        // A library's function is written as C names it; where the format
        // spells that as the symbol, the pass above found it.
        let symbol = format!("{}{name}", super::names::C_PREFIX);
        let library = if super::names::C_PREFIX.is_empty() {
            Vec::new()
        } else {
            inferior.library_functions(&symbol)?
        };
        let symbols = written
            .iter()
            .map(|function| function.name.as_str())
            .chain((!library.is_empty()).then_some(symbol.as_str()));
        if let Some(error) = super::names::ambiguity(name, symbols) {
            return Err(error);
        }
        take(&written, &mut starts);
        starts.extend(library.into_iter().map(|range| range.start));
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
/// first `ret` or [`UNSIZED_LIMIT`]. A counted listing that runs out of
/// `range` (before its start, or past its end) heads each other function's
/// instructions with that function, as lldb does; instructions no function
/// holds stay under `range`.
fn listing(
    inferior: &mut dyn Inferior,
    range: &FunctionRange,
    from: u64,
    size: Option<u64>,
    count: Option<u64>,
    pc: u64,
) -> Result<Vec<String>, String> {
    let arch = inferior.arch();
    let longest = arch.longest as u64;
    // How many bytes to read, and the most instructions to show from them.
    let (bytes, limit) = match (count, size) {
        (Some(count), _) => {
            let count = count.min(MAX_FUNCTION / longest);
            (count * longest, count)
        }
        (None, Some(size)) => (size.min(MAX_FUNCTION), u64::MAX),
        (None, None) => (UNSIZED_LIMIT as u64 * longest, UNSIZED_LIMIT as u64),
    };
    let code = read_up_to(inferior, from, bytes as usize)?;
    let collisions = Collisions::of(&inferior.functions().unwrap_or_default());
    // The functions the listing shows code of, in order; each row holds the
    // index of its own.
    let mut owners = vec![range.clone()];
    let mut rows = Vec::new();
    for (address, bytes) in instructions(arch, &code, from).take(limit as usize) {
        let decoded = (arch.decode)(bytes, address);
        let returns = decoded
            .as_ref()
            .is_some_and(|instruction| instruction.returns);
        let last = owners.len() - 1;
        let owner = if holds(&owners[last], address) {
            last
        } else if holds(range, address) {
            owners.push(range.clone());
            last + 1
        } else {
            match inferior
                .function_at(address)?
                .filter(|other| holds(other, address))
            {
                Some(other) if other.start == owners[last].start => last,
                Some(other) => {
                    owners.push(other);
                    last + 1
                }
                None if owners[last].start == range.start => last,
                None => {
                    owners.push(range.clone());
                    last + 1
                }
            }
        };
        rows.push((address, owner, decoded, bytes));
        if returns && count.is_none() && size.is_none() {
            break;
        }
    }
    // The first row's function heads the listing, even one before `range`.
    if rows.first().is_some_and(|(_, owner, ..)| *owner != 0) {
        owners.remove(0);
        for (_, owner, ..) in &mut rows {
            *owner -= 1;
        }
    }
    let offset = |owner: &FunctionRange, address: u64| {
        if owner.image.is_empty() && owner.name.is_none() {
            String::new()
        } else {
            // An instruction before the function's start (a listing counted
            // back past it into code no function holds) shows how far before.
            match address.checked_sub(owner.start) {
                Some(after) => format!(" <+{after}>:"),
                None => format!(" <-{}>:", owner.start - address),
            }
        }
    };
    let width = rows
        .iter()
        .map(|(address, owner, ..)| offset(&owners[*owner], *address).len())
        .max()
        .unwrap_or(0);
    // Each register an `adrp` set, and the page it holds, until overwritten
    // or the listing moves into another function.
    let mut pages: Vec<(u32, u64)> = Vec::new();
    let mut lines = vec![header(&owners[0], &collisions)];
    let mut shown = 0;
    for (address, owner, decoded, bytes) in rows {
        if owner != shown {
            lines.push(header(&owners[owner], &collisions));
            pages.clear();
            shown = owner;
        }
        let owner = &owners[owner];
        let marker = if address == pc { "->  " } else { "    " };
        let text = match &decoded {
            Some(instruction) => instruction.text.clone(),
            None => undecoded(bytes),
        };
        let comment = match &decoded {
            Some(instruction) => {
                let paged = instruction.page_offset.and_then(|(register, offset)| {
                    pages
                        .iter()
                        .find(|(page_register, _)| *page_register == register)
                        .map(|(_, page)| page.wrapping_add_signed(offset))
                });
                if let Some((register, page)) = instruction.page {
                    pages.retain(|(other, _)| *other != register);
                    pages.push((register, page));
                }
                match paged {
                    Some(address) => named_address(inferior, address, &collisions),
                    None => comment(inferior, owner, instruction, &collisions),
                }
            }
            None => None,
        };
        let mut line = format!(
            "{marker}0x{address:016x}{:<width$} {text}",
            if offset(owner, address).is_empty() {
                ":".to_string()
            } else {
                offset(owner, address)
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
            let step = inferior.arch().alignment as usize;
            let mut code = Vec::new();
            while code.len() < length {
                match inferior.read_code(address + code.len() as u64, step) {
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
    instruction: &Decoded,
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
