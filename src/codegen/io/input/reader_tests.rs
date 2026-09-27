//! The readers consume through the input buffer (bug-699).
//!
//! Every reader must take the bytes a look-ahead left in the buffer BEFORE
//! anything new, or mixing readers would lose or reorder input. The region is
//! filled by hand here — exactly the state a look-ahead that peeked and did not
//! consume leaves behind — and the emitted reader helpers are run in the
//! `.ncode` interpreter with the stdin source modelled behind them.

use super::ncode_emu::{Externals, Machine};
use super::{INPUT_STATE_BUF_OFFSET, INPUT_STATE_LEN_OFFSET};
use std::collections::VecDeque;
use std::sync::OnceLock;

const PROGRAM: &str = "\
IMPORT io

FUNC main AS Integer
  LET l AS String = io::readLine()
  LET raw AS Byte = io::readRaw()
  LET s AS Scalar = io::readScalar()
  LET g AS String = io::readGrapheme()
  LET k AS io::Key = io::readKey()
  io::print(l & toString(raw) & toString(s) & g & k.grapheme)
  RETURN 0
END FUNC
";

/// The macos-aarch64 dump with the buffer reserved, and where the region is.
fn dump() -> &'static (serde_json::Value, usize) {
    static DUMP: OnceLock<(serde_json::Value, usize)> = OnceLock::new();
    DUMP.get_or_init(|| {
        let (plan, offset) = crate::testutil::code_for_src_with_input_buffer(
            PROGRAM,
            crate::testutil::CodeTarget::MacosAarch64,
            crate::target::NativeBuildMode::Console,
        );
        (
            serde_json::from_str(&plan.to_json()).expect("the ncode dump is JSON"),
            offset,
        )
    })
}

/// The process a reader runs in: stdin is a pipe (so no terminal mode is
/// touched) whose remaining bytes are `source`; the stdin log reader hands them
/// out one per call; the arena allocator is a bump allocator.
struct Process {
    source: VecDeque<u8>,
    source_reads: usize,
    errno: u64,
    /// The monotonic clock, in nanoseconds. A `poll` that finds nothing waits
    /// out its whole timeout, so the clock moves by it.
    now: u64,
    /// What `isatty(0)` answers: a terminal (the keyboard delay applies) or a
    /// pipe.
    tty: bool,
    /// How many `tcsetattr` calls the reader made.
    set_attrs: usize,
}

impl Externals for Process {
    fn intercepts(&self, name: &str) -> bool {
        matches!(
            name,
            "_mfb_rt_stdin_next_byte"
                | "_mfb_arena_alloc"
                | "_mfb_arena_free"
                | "_mfb_rt_io_stdout_drain"
        )
    }

    fn call(&mut self, m: &mut Machine, name: &str) -> Option<u64> {
        match name {
            // x1 = destination; 1 = a byte, 0 = end of input.
            "_mfb_rt_stdin_next_byte" => {
                self.source_reads += 1;
                match self.source.pop_front() {
                    Some(byte) => {
                        m.write_uint(m.arg(1), 1, u64::from(byte));
                        Some(1)
                    }
                    None => Some(0),
                }
            }
            // x0 = size in, OK tag out, the block in x1.
            "_mfb_arena_alloc" => {
                let block = m.alloc(m.arg(0) as usize);
                m.set_reg("x1", block);
                Some(0)
            }
            "_mfb_arena_free" | "_mfb_rt_io_stdout_drain" => Some(0),
            "_isatty" => Some(u64::from(self.tty)),
            "_tcgetattr" => Some(0),
            "_tcsetattr" => {
                self.set_attrs += 1;
                Some(0)
            }
            // x1 = the timespec.
            "_clock_gettime" => {
                m.write_uint(m.arg(1), 8, self.now / 1_000_000_000);
                m.write_uint(m.arg(1) + 8, 8, self.now % 1_000_000_000);
                Some(0)
            }
            // x2 = the timeout in milliseconds.
            "_poll" => {
                if self.source.is_empty() {
                    self.now += m.arg(2) * 1_000_000;
                    Some(0)
                } else {
                    Some(1)
                }
            }
            "_pthread_mutex_lock" | "_pthread_mutex_unlock" => Some(0),
            "___error" => {
                if self.errno == 0 {
                    self.errno = m.alloc(8);
                }
                Some(self.errno)
            }
            _ => None,
        }
    }
}

struct Run {
    machine: Machine<'static>,
    process: Process,
    region: u64,
}

impl Run {
    /// A thread whose input buffer holds `buffered` and whose stdin has `source`
    /// still to deliver.
    fn new(buffered: &[u8], source: &[u8]) -> Run {
        let (ncode, offset) = dump();
        let mut machine = Machine::new(ncode);
        let arena = machine.alloc(1 << 16);
        machine.set_reg("x19", arena);
        let region = arena + *offset as u64;
        machine.write_uint(
            region + INPUT_STATE_LEN_OFFSET as u64,
            8,
            buffered.len() as u64,
        );
        machine.write_bytes(region + INPUT_STATE_BUF_OFFSET as u64, buffered);
        Run {
            machine,
            process: Process {
                source: source.iter().copied().collect(),
                source_reads: 0,
                errno: 0,
                now: 1_000_000_000,
                tty: false,
                set_attrs: 0,
            },
            region,
        }
    }

    /// Call a reader; its `(tag, value)` result.
    fn call(&mut self, symbol: &str, args: &[u64]) -> (u64, u64) {
        let tag = self.machine.call(symbol, args, &mut self.process);
        (tag, self.machine.reg("x1"))
    }

    fn string(&self, block: u64) -> Vec<u8> {
        let len = self.machine.read_u64(block) as usize;
        self.machine.read_bytes(block + 8, len)
    }

    fn buffered(&self) -> Vec<u8> {
        let len = self
            .machine
            .read_u64(self.region + INPUT_STATE_LEN_OFFSET as u64) as usize;
        self.machine
            .read_bytes(self.region + INPUT_STATE_BUF_OFFSET as u64, len)
    }
}

const READ_LINE: &str = "_mfb_rt_io_io_readLine";

#[test]
fn read_line_joins_buffered_bytes_and_the_rest_of_the_line() {
    let mut run = Run::new(b"ab", b"c\nnext");
    let (tag, block) = run.call(READ_LINE, &[UNBOUNDED]);
    assert_eq!(tag, 0, "readLine succeeds");
    assert_eq!(run.string(block), b"abc");
    assert_eq!(
        run.process.source.iter().copied().collect::<Vec<u8>>(),
        b"next"
    );
}

// --- The buffered readers (bug-699 Phase 4) -------------------------------------

const READ_RAW: &str = "_mfb_rt_io_io_readRaw";
const READ_SCALAR: &str = "_mfb_rt_io_io_readScalar";
const READ_GRAPHEME: &str = "_mfb_rt_io_io_readGrapheme";
const READ_KEY: &str = "_mfb_rt_io_io_readKey";
/// The omitted-`timeoutMs` sentinel (i64::MIN).
const UNBOUNDED: u64 = 1 << 63;

fn error_code(name: &str) -> u64 {
    crate::codegen::registry::runtime_error_emission(name)
        .unwrap_or_else(|| panic!("{name} is an errorCode"))
        .0
        .parse()
        .expect("an error code is an integer")
}

impl Run {
    /// The `io::Key` fields at `record`: (key ordinal, grapheme, event ordinal,
    /// modifier bits shift|alt<<1|ctrl<<2|meta<<3).
    fn key(&self, record: u64) -> (u64, Vec<u8>, u64, u64) {
        let field = |i: u64| self.machine.read_u64(record + 8 * i);
        let grapheme = self.string(record + field(1));
        let mods = field(3) | field(5) << 1 | field(4) << 2 | field(6) << 3;
        (field(0), grapheme, field(2), mods)
    }
}

#[test]
fn read_raw_is_byte_exact_through_the_buffer_and_the_source() {
    let mut run = Run::new(b"\x1b[", b"D\xff");
    let mut got = Vec::new();
    for _ in 0..4 {
        let (tag, value) = run.call(READ_RAW, &[UNBOUNDED]);
        assert_eq!(tag, 0, "readRaw succeeds");
        got.push(value as u8);
    }
    assert_eq!(got, b"\x1b[D\xff");
}

#[test]
fn read_key_decodes_a_sequence_split_across_the_buffer_and_the_source() {
    let mut run = Run::new(b"\x1b[1;", b"2Dz");
    let (tag, record) = run.call(READ_KEY, &[UNBOUNDED]);
    assert_eq!(tag, 0, "readKey succeeds");
    assert_eq!(
        run.key(record),
        (3, Vec::new(), 3, 1),
        "Shift+Left, NotSupported"
    );
    assert_eq!(
        run.process.source.iter().copied().collect::<Vec<u8>>(),
        b"z"
    );
}

#[test]
fn read_scalar_skips_keys_without_text_and_releases() {
    let mut run = Run::new(b"", b"\x1b[A\x1b[97;1:3uq");
    let (tag, value) = run.call(READ_SCALAR, &[UNBOUNDED]);
    assert_eq!((tag, value), (0, u64::from(b'q')));
}

#[test]
fn read_scalar_gives_the_rest_of_a_multi_scalar_key_back() {
    // A kitty text event carrying `e` + U+0301: two scalars, two reads.
    let mut run = Run::new(b"", b"\x1b[0;;101:769u");
    assert_eq!(run.call(READ_SCALAR, &[UNBOUNDED]), (0, 101));
    assert_eq!(run.call(READ_SCALAR, &[UNBOUNDED]), (0, 0x301));
}

#[test]
fn read_grapheme_joins_a_cluster_from_a_pipe() {
    let mut run = Run::new(b"", "e\u{301}x".as_bytes());
    let (tag, block) = run.call(READ_GRAPHEME, &[UNBOUNDED]);
    assert_eq!(tag, 0);
    assert_eq!(run.string(block), "e\u{301}".as_bytes());
}

#[test]
fn a_timeout_leaves_a_half_sequence_in_the_buffer() {
    // A pipe: no delay decides a prefix, only more input or its end. `0` is one
    // immediate attempt, and nothing more is ready.
    let mut run = Run::new(b"", b"\x1b[1;");
    let (tag, code) = run.call(READ_KEY, &[0]);
    assert_ne!(tag, 0, "readKey(0) with a half sequence times out");
    assert_eq!(code, error_code("ErrTimeout"));
    assert_eq!(run.buffered(), b"\x1b[1;", "nothing is lost");
    // The rest arrives: the next read completes it.
    run.process.source.extend(b"5D");
    let (tag, record) = run.call(READ_KEY, &[UNBOUNDED]);
    assert_eq!(tag, 0);
    assert_eq!(run.key(record).0, 3, "Ctrl+Left");
}

#[test]
fn a_lone_escape_from_a_keyboard_is_decided_after_the_delay() {
    // A terminal: the lone ESC waits at most the delay for its sequence. A poll
    // with `0` sees it wait; once 25 ms have passed it is the Escape key.
    let mut run = Run::new(b"", b"\x1b");
    run.process.tty = true;
    let (tag, code) = run.call(READ_KEY, &[0]);
    assert_eq!((tag != 0, code), (true, error_code("ErrTimeout")));
    run.process.now += 30_000_000;
    let (tag, record) = run.call(READ_KEY, &[0]);
    assert_eq!(tag, 0, "the delay has passed");
    assert_eq!(run.key(record), (14, b"\x1b".to_vec(), 3, 0), "Escape");
}

#[test]
fn a_keyboard_escape_sequence_is_not_split_by_the_delay() {
    let mut run = Run::new(b"", b"\x1b[D");
    run.process.tty = true;
    let (tag, record) = run.call(READ_KEY, &[UNBOUNDED]);
    assert_eq!(tag, 0);
    assert_eq!(run.key(record).0, 3, "Left, not Escape then `[D`");
}

#[test]
fn a_timed_read_line_gives_a_half_line_back() {
    let mut run = Run::new(b"", b"ab");
    let (tag, code) = run.call(READ_LINE, &[0]);
    assert_eq!((tag != 0, code), (true, error_code("ErrTimeout")));
    assert_eq!(run.buffered(), b"ab", "the half line is kept");
    run.process.source.extend(b"c\n");
    let (tag, block) = run.call(READ_LINE, &[UNBOUNDED]);
    assert_eq!(tag, 0);
    assert_eq!(run.string(block), b"abc");
}

#[test]
fn a_negative_timeout_is_an_invalid_argument() {
    for reader in [READ_RAW, READ_SCALAR, READ_GRAPHEME, READ_KEY, READ_LINE] {
        let mut run = Run::new(b"", b"x");
        // The frame the reader gets is whatever the stack last held — here, the
        // word `1` everywhere, which is what an earlier reader that configured
        // the terminal leaves in its restore flag — so a slot read before it is
        // written shows up (the flag did: `readKey(-1)` "restored" a terminal
        // it never configured and raised `ErrInputFailed`).
        let sp = run.machine.reg("sp");
        for word in 1..=8192u64 {
            run.machine.write_uint(sp - 8 * word, 8, 1);
        }
        let (tag, code) = run.call(reader, &[(-5i64) as u64]);
        assert_eq!(
            (tag != 0, code),
            (true, error_code("ErrInvalidArgument")),
            "{reader}"
        );
        assert_eq!(run.process.source_reads, 0, "{reader} reads nothing");
        assert_eq!(
            run.process.set_attrs, 0,
            "{reader} leaves the terminal alone"
        );
    }
}
