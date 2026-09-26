//! The plain readers consume through the input buffer (bug-699 Phase 2).
//!
//! A program with a look-ahead reader (Phase 4) reserves the buffer, and then
//! `io::readByte`, `io::readChar`, `io::readLine` and `io::pollInput` must take
//! the bytes a look-ahead left there BEFORE anything new, or mixing readers
//! would lose or reorder input. Those readers do not exist yet, so the region is
//! reserved here with the test-only switch and filled by hand — exactly the
//! state a look-ahead that peeked and did not consume leaves behind — and the
//! emitted reader helpers are run in the `.ncode` interpreter with the stdin
//! source modelled behind them.
//!
//! Without the switch nothing here is emitted at all; that half is the
//! artifact gate's byte-identity check, not a test in this file.

use super::ncode_emu::{Externals, Machine};
use super::{INPUT_STATE_BUF_OFFSET, INPUT_STATE_LEN_OFFSET};
use std::collections::VecDeque;
use std::sync::OnceLock;

const PROGRAM: &str = "\
IMPORT io

FUNC main AS Integer
  LET b AS Byte = io::readByte()
  LET c AS String = io::readChar()
  LET l AS String = io::readLine()
  LET r AS Boolean = io::pollInput(0)
  io::print(toString(b) & c & l & toString(r))
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
            "_isatty" => Some(0),
            "_poll" => Some(u64::from(!self.source.is_empty())),
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

const READ_BYTE: &str = "_mfb_rt_io_io_readByte";
const READ_CHAR: &str = "_mfb_rt_io_io_readChar";
const READ_LINE: &str = "_mfb_rt_io_io_readLine";
const POLL_INPUT: &str = "_mfb_rt_io_io_pollInput";

#[test]
fn read_byte_takes_buffered_bytes_first_then_the_source() {
    let mut run = Run::new(b"\x1b[D", b"x");
    let mut got = Vec::new();
    for _ in 0..4 {
        let (tag, value) = run.call(READ_BYTE, &[]);
        assert_eq!(tag, 0, "readByte succeeds");
        got.push(value as u8);
    }
    assert_eq!(got, b"\x1b[Dx");
    assert_eq!(
        run.process.source_reads, 1,
        "the source is read only once the buffer is empty"
    );
    assert!(run.buffered().is_empty());
}

#[test]
fn read_byte_with_an_empty_buffer_reads_the_source_as_before() {
    // The control: the buffer front must fall through when there is nothing in it.
    let mut run = Run::new(b"", b"q");
    let (tag, value) = run.call(READ_BYTE, &[]);
    assert_eq!((tag, value), (0, u64::from(b'q')));
    assert_eq!(run.process.source_reads, 1);
}

#[test]
fn read_char_completes_a_character_across_the_buffer_and_the_source() {
    // A look-ahead left the lead byte of `é`; its continuation is still in the
    // source. One readChar must join them.
    let mut run = Run::new(b"\xc3", b"\xa9z");
    let (tag, block) = run.call(READ_CHAR, &[]);
    assert_eq!(tag, 0, "readChar succeeds");
    assert_eq!(run.string(block), "é".as_bytes());
    let (_, block) = run.call(READ_CHAR, &[]);
    assert_eq!(run.string(block), b"z");
}

#[test]
fn read_char_takes_a_whole_buffered_character_without_the_source() {
    let mut run = Run::new("€a".as_bytes(), b"");
    let (tag, block) = run.call(READ_CHAR, &[]);
    assert_eq!(tag, 0);
    assert_eq!(run.string(block), "€".as_bytes());
    assert_eq!(run.buffered(), b"a");
    assert_eq!(run.process.source_reads, 0);
}

#[test]
fn read_line_joins_buffered_bytes_and_the_rest_of_the_line() {
    let mut run = Run::new(b"ab", b"c\nnext");
    let (tag, block) = run.call(READ_LINE, &[]);
    assert_eq!(tag, 0, "readLine succeeds");
    assert_eq!(run.string(block), b"abc");
    assert_eq!(
        run.process.source.iter().copied().collect::<Vec<u8>>(),
        b"next"
    );
}

#[test]
fn poll_input_reports_buffered_input_ready() {
    // Nothing in the source: only the buffered byte can make this TRUE.
    let mut run = Run::new(b"k", b"");
    let (tag, ready) = run.call(POLL_INPUT, &[0]);
    assert_eq!((tag, ready), (0, 1), "a buffered byte is input ready now");
    assert_eq!(run.buffered(), b"k", "pollInput reads nothing");
}

#[test]
fn poll_input_with_an_empty_buffer_still_asks_the_source() {
    let mut run = Run::new(b"", b"");
    assert_eq!(
        run.call(POLL_INPUT, &[0]),
        (0, 0),
        "nothing buffered, nothing pending"
    );
    let mut run = Run::new(b"", b"s");
    assert_eq!(run.call(POLL_INPUT, &[0]), (0, 1), "pending source input");
}
