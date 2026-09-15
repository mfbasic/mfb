//! bug-621: an in-place grow of a fixed-width collection reserves data capacity in
//! proportion to its element capacity, not on an independent step.
//!
//! Every in-place grow arm stepped `capacity` and `dataCapacity` separately, on
//! every grow — including the grows the element *count* triggered. For a
//! fixed-width element the count is always the binding limit, so the data step
//! drifted ahead of the data: it starts 8× the element capacity (32 vs 4) and
//! doubles for three steps longer (to 64 KiB vs 1024), freezing the ratio at ≈19
//! data bytes per slot whatever the element's width. A 16 MiB `List OF Byte` built
//! by `append` peaked at 981,975,040 bytes RSS, and a `List OF Integer` measured
//! identically to the byte.
//!
//! **Where each bound comes from.** A fixed-width list is entry-free (stride 0), so
//! a grow allocates `HEADER + newCapacity × width` and a fixed-width `Map` allocates
//! `HEADER + newCapacity × (ENTRY + BUCKET + perEntryData)`. `newCapacity` follows
//! `emit_geometric_step` (4, doubling below 1024, then ×1.5), raised to the batch's
//! count for a bulk append. [`generations`] replays that rule for the program's
//! appends, so the bound is the sum of every block the fixed rule allocates — the
//! freed predecessors included, since the arena counter and (on macOS) the peak RSS
//! count them — rounded up to the allocator's 16-byte granularity, plus the same
//! program's allocations at `n = 0`. Nothing is tuned: the pre-fix compiler
//! overshoots every bound by the ≈19/width factor (86,029,216 bytes against a
//! 4,540,083-byte rule for 1 MiB appended bytes).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// `COLLECTION_HEADER_SIZE`.
const HEADER: u64 = 40;
/// `COLLECTION_ENTRY_SIZE`: a `Map`'s lookup entry.
const MAP_ENTRY: u64 = 40;
/// A `Map`'s hash-bucket region: `capacity << 4`.
const MAP_BUCKET: u64 = 16;
/// `COLLECTION_GROW_LOOKUP_INIT` / `COLLECTION_GROW_LOOKUP_TAPER`.
const GROW_INIT: u64 = 4;
const GROW_TAPER: u64 = 1024;
/// The arena rounds a request up to a 16-byte multiple.
const ALLOC_GRANULE: u64 = 16;

/// `emit_geometric_step` over the element capacity.
fn step(capacity: u64) -> u64 {
    if capacity == 0 {
        GROW_INIT
    } else if capacity < GROW_TAPER {
        capacity * 2
    } else {
        capacity + capacity / 2
    }
}

/// The element capacity of every block the fixed-width grow rule allocates while
/// `ops` operations each add `per_op` elements. With data capacity tied to element
/// capacity the count is the only limit, so a grow fires exactly when the batch no
/// longer fits, and a bulk grow takes at least the batch's count.
fn generations(ops: u64, per_op: u64) -> Vec<u64> {
    let (mut capacity, mut count) = (0u64, 0u64);
    let mut out = Vec::new();
    for _ in 0..ops {
        let need = count + per_op;
        if need > capacity {
            capacity = step(capacity).max(need);
            out.push(capacity);
        }
        count = need;
    }
    out
}

fn round_up(bytes: u64) -> u64 {
    bytes.div_ceil(ALLOC_GRANULE) * ALLOC_GRANULE
}

/// Sum of every grown block, each `HEADER + capacity × slot_bytes`.
fn grown_bytes(ops: u64, per_op: u64, slot_bytes: u64) -> u64 {
    generations(ops, per_op)
        .into_iter()
        .map(|capacity| round_up(HEADER + capacity * slot_bytes))
        .sum()
}

/// Build `project` with `--debug` and return the executable (the host glibc one on
/// Linux), as `rt_debug_arena` does.
fn build_debug(name: &str, source: &str) -> PathBuf {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed to build:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let chosen = written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    PathBuf::from(chosen)
}

/// Run a `--debug` program, require it to print `expected`, and return the main
/// arena's `alloc_bytes`: every byte the program ever requested, freed or not.
fn alloc_bytes(name: &str, exe: &Path, expected: &str) -> u64 {
    let output = Command::new(exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{name} failed:\n{stdout}\n{stderr}"
    );
    assert_eq!(
        stdout.lines().next(),
        Some(expected),
        "{name}: wrong output"
    );
    stderr
        .lines()
        .find_map(|line| line.strip_prefix("arena.0.alloc_bytes "))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{name}: no arena.0.alloc_bytes in:\n{stderr}"))
}

#[derive(Clone, Copy)]
enum Arm {
    Append,
    Insert,
    Prepend,
    BulkAppend,
    InlineAppend,
    InlineBulkAppend,
}

impl Arm {
    fn name(self) -> &'static str {
        match self {
            Arm::Append => "append",
            Arm::Insert => "insert",
            Arm::Prepend => "prepend",
            Arm::BulkAppend => "bulk",
            Arm::InlineAppend => "inline",
            Arm::InlineBulkAppend => "inline_bulk",
        }
    }

    fn inline(self) -> bool {
        matches!(self, Arm::InlineAppend | Arm::InlineBulkAppend)
    }

    /// Elements one operation adds: a bulk arm appends an 8-element list.
    fn per_op(self) -> u64 {
        match self {
            Arm::BulkAppend | Arm::InlineBulkAppend => 8,
            _ => 1,
        }
    }

    /// The statement that reaches this arm's in-place lowering.
    fn statement(self) -> &'static str {
        match self {
            Arm::Append => "data = collections::append(data, b)",
            Arm::Insert => "data = collections::insert(data, len(data), b)",
            Arm::Prepend => "data = collections::prepend(data, b)",
            Arm::BulkAppend => "data = collections::append(data, chunk)",
            Arm::InlineAppend => "rec = WITH rec { items := collections::append(rec.items, b) }",
            Arm::InlineBulkAppend => {
                "rec = WITH rec { items := collections::append(rec.items, chunk) }"
            }
        }
    }
}

/// A program that runs `arm` `n` times over a `List OF element`.
fn list_program(arm: Arm, element: &str, n: u64) -> String {
    let (item, chunk) = match element {
        "Byte" => (
            "LET b AS Byte = toByte((i * 7 + 3) MOD 256)",
            "[toByte(1), toByte(2), toByte(3), toByte(4), toByte(5), toByte(6), toByte(7), toByte(8)]",
        ),
        "Integer" => ("LET b AS Integer = i * 7 + 3", "[1, 2, 3, 4, 5, 6, 7, 8]"),
        other => panic!("no item expression for {other}"),
    };
    let mut src = String::from("IMPORT io\nIMPORT collections\n");
    if arm.inline() {
        src.push_str(&format!(
            "TYPE Box\n  tag AS Integer\n  items AS List OF {element}\nEND TYPE\n"
        ));
    }
    src.push_str(&format!("SUB main()\n  LET n AS Integer = {n}\n"));
    if arm.inline() {
        src.push_str("  MUT rec AS Box = Box[1, []]\n");
    } else {
        src.push_str(&format!("  MUT data AS List OF {element} = []\n"));
    }
    if arm.per_op() > 1 {
        src.push_str(&format!("  LET chunk AS List OF {element} = {chunk}\n"));
    }
    src.push_str(&format!(
        "  MUT i AS Integer = 0\n  WHILE i < n\n    {item}\n    {}\n    i = i + 1\n  END WHILE\n",
        arm.statement()
    ));
    let list = if arm.inline() { "rec.items" } else { "data" };
    src.push_str(&format!("  io::print(toString(len({list})))\nEND SUB\n"));
    src
}

/// `alloc_bytes` of `arm` over `element` at `n` operations and at zero.
fn measure_list(arm: Arm, element: &str, n: u64) -> (u64, u64) {
    let tag = format!("growth_{}_{}", arm.name(), element.to_lowercase());
    let exe = build_debug(&tag, &list_program(arm, element, n));
    let grown = alloc_bytes(&tag, &exe, &(n * arm.per_op()).to_string());
    let base_tag = format!("{tag}_zero");
    let exe = build_debug(&base_tag, &list_program(arm, element, 0));
    let base = alloc_bytes(&base_tag, &exe, "0");
    (grown, base)
}

/// A record-field arm grows the whole record block, `fieldOffset + HEADER +
/// capacity × width`: `Box`'s two 8-byte field slots precede the inlined list.
const BOX_PREFIX: u64 = 2 * 8;

/// Assert `arm`'s `List OF Byte` and `List OF Integer` stay inside the fixed-width
/// rule; for a single-element arm, also that the byte list is the smaller.
fn assert_list_arm(arm: Arm, n: u64) {
    let (byte, byte_base) = measure_list(arm, "Byte", n);
    let (integer, integer_base) = measure_list(arm, "Integer", n);
    let prefix = if arm.inline() {
        BOX_PREFIX * generations(n, arm.per_op()).len() as u64
    } else {
        0
    };
    let byte_bound = byte_base + grown_bytes(n, arm.per_op(), 1) + prefix;
    let integer_bound = integer_base + grown_bytes(n, arm.per_op(), 8) + prefix;
    assert!(
        byte <= byte_bound,
        "{}: List OF Byte requested {byte} bytes; the fixed-width rule allows {byte_bound}",
        arm.name()
    );
    assert!(
        integer <= integer_bound,
        "{}: List OF Integer requested {integer} bytes; the fixed-width rule allows {integer_bound}",
        arm.name()
    );
    assert!(
        byte < integer,
        "{}: List OF Byte ({byte}) must reserve less than List OF Integer ({integer})",
        arm.name()
    );
}

#[test]
fn append_reserves_data_by_element_width() {
    assert_list_arm(Arm::Append, 1 << 20);
}

#[test]
fn insert_at_the_end_reserves_data_by_element_width() {
    assert_list_arm(Arm::Insert, 1 << 20);
}

#[test]
fn prepend_reserves_data_by_element_width() {
    // Each prepend shifts the whole list, so the count stays small.
    assert_list_arm(Arm::Prepend, 1 << 16);
}

#[test]
fn bulk_append_reserves_data_by_element_width() {
    assert_list_arm(Arm::BulkAppend, 1 << 20);
}

#[test]
fn record_field_append_reserves_data_by_element_width() {
    assert_list_arm(Arm::InlineAppend, 1 << 20);
}

#[test]
fn record_field_bulk_append_reserves_data_by_element_width() {
    assert_list_arm(Arm::InlineBulkAppend, 1 << 20);
}

#[test]
fn map_set_reserves_data_by_entry_width() {
    // `Map OF Integer TO Integer`: an entry's data is an 8-byte key then an 8-byte
    // value, both 8-aligned, so exactly 16 bytes per slot.
    let program = |n: u64| {
        format!(
            "IMPORT io\nIMPORT collections\nSUB main()\n  LET n AS Integer = {n}\n  \
             MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer {{}}\n  \
             MUT i AS Integer = 0\n  WHILE i < n\n    m = collections::set(m, i, i * 7 + 3)\n    \
             i = i + 1\n  END WHILE\n  io::print(toString(len(m)))\nEND SUB\n"
        )
    };
    let n = 1u64 << 20;
    let exe = build_debug("growth_map_set", &program(n));
    let grown = alloc_bytes("growth_map_set", &exe, &n.to_string());
    let exe = build_debug("growth_map_set_zero", &program(0));
    let base = alloc_bytes("growth_map_set_zero", &exe, "0");
    let bound = base + grown_bytes(n, 1, MAP_ENTRY + MAP_BUCKET + 16);
    assert!(
        grown <= bound,
        "Map OF Integer TO Integer requested {grown} bytes; the fixed-width rule allows {bound}"
    );
}

/// The reproduction as filed: 16 MiB appended one byte at a time, measured by peak
/// RSS. The bound assumes every freed generation stays resident (it does on macOS:
/// the report showed peak RSS equal to the sum of all generations), plus the program's
/// resident size at `n = 0` and one 64 KiB mapping granule per generation.
#[test]
fn a_16_mib_append_built_byte_list_peaks_near_its_data() {
    let n = 1u64 << 24;
    let program = |element: &str, n: u64| list_program(Arm::Append, element, n);
    let rss = |tag: &str, source: String, expected: String| {
        let project = common::temp_project(tag, &source);
        let exe = common::build_project(&project);
        let (status, stdout, rss) = common::run_bounded_with_rss(
            &exe,
            Duration::from_secs(120),
            "the append loop did not finish",
        );
        assert!(
            status.success(),
            "{tag}: program {}",
            common::exit_description(&status)
        );
        assert_eq!(stdout.lines().next(), Some(expected.as_str()), "{tag}");
        let _ = std::fs::remove_dir_all(&project);
        rss.unwrap_or_else(|| panic!("{tag}: no peak RSS"))
    };
    let byte = rss("growth_rss_byte", program("Byte", n), n.to_string());
    let integer = rss("growth_rss_integer", program("Integer", n), n.to_string());
    let base = rss("growth_rss_zero", program("Byte", 0), "0".to_string());
    let granules = generations(n, 1).len() as u64 * 64 * 1024;
    let byte_bound = base + grown_bytes(n, 1, 1) + granules;
    let integer_bound = base + grown_bytes(n, 1, 8) + granules;
    assert!(
        byte <= byte_bound,
        "16 MiB List OF Byte peaked at {byte} bytes RSS; the fixed-width rule allows {byte_bound}"
    );
    assert!(
        integer <= integer_bound,
        "16 MiB List OF Integer peaked at {integer} bytes RSS; the fixed-width rule allows {integer_bound}"
    );
    assert!(
        byte < integer,
        "16 MiB List OF Byte ({byte}) must peak below List OF Integer ({integer})"
    );
}
