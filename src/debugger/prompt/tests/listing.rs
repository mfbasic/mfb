//! `dis` listings over real AArch64 code: comments on branches, calls,
//! `adrp`/`add` pairs and immediates, headers for stubs, unnamed functions and
//! tagged names, and listings that run before a function, past readable
//! memory, or to the first `ret`. Encodings checked with `clang -c` and
//! `objdump -d`.

use std::collections::HashMap;
use std::ffi::OsString;

use super::*;
use crate::debugger::disassemble::{disassemble, resolve, returns, Request, Window};
use crate::debugger::names::C_PREFIX;
use crate::debugger::{Ending, FunctionRange, Step};

const RET: u32 = 0xd65f_03c0;
const NOP: u32 = 0xd503_201f;

/// `work` at 0x1000: `adrp x0, 0x5000` / `add x0, x0, #0x10`, `mov x1, #5`,
/// `bl helper` (0x2000), `b <+24>`, an SVE word the decoder does not know,
/// `bl 0x3004` (in an unnamed library function), `ret`.
const WORK: [u32; 8] = [
    0x9000_0020,
    0x9100_4000,
    0xd280_00a1,
    0x9400_03fd,
    0x1400_0002,
    0x04e1_0000,
    0x9400_07fb,
    RET,
];

/// One function: its range and its code.
struct Code {
    range: FunctionRange,
    words: Vec<u32>,
}

#[derive(Default)]
struct Image {
    code: Vec<Code>,
    symbols: Vec<FunctionSymbol>,
    data: HashMap<u64, String>,
    /// Code no function holds, by address.
    loose: Vec<(u64, Vec<u32>)>,
    /// Reads of more than one instruction fail, as across an unmapped page.
    word_at_a_time: bool,
}

impl Image {
    fn function(
        &mut self,
        image: &str,
        name: Option<&str>,
        start: u64,
        sized: bool,
        words: &[u32],
    ) {
        let size = sized.then_some(words.len() as u64 * 4);
        if let Some(name) = name {
            self.symbols.push(FunctionSymbol {
                name: name.to_string(),
                linked: false,
                loaded: false,
                address: Some(start),
                size,
            });
        }
        self.code.push(Code {
            range: FunctionRange {
                image: image.to_string(),
                name: name.map(str::to_string),
                start,
                size,
            },
            words: words.to_vec(),
        });
    }

    fn word(&self, address: u64) -> Option<u32> {
        let found = |start: u64, words: &[u32]| {
            address
                .checked_sub(start)
                .and_then(|offset| words.get((offset / 4) as usize).copied())
        };
        self.code
            .iter()
            .find_map(|code| found(code.range.start, &code.words))
            .or_else(|| {
                self.loose
                    .iter()
                    .find_map(|(start, words)| found(*start, words))
            })
    }

    /// The usual program: `work`, `helper` and an unnamed library function.
    fn usual() -> Image {
        let mut program = Image::default();
        program.function("prog", Some("_mfb_fn_work"), 0x1000, true, &WORK);
        program.function("prog", Some("_mfb_fn_helper"), 0x2000, true, &[NOP, RET]);
        program.function("libfoo.dylib", None, 0x3000, false, &[NOP, NOP, RET]);
        program.data.insert(0x5010, "_mfb_str_1".to_string());
        program
    }

    fn dis(&mut self, pc: u64, request: Request) -> Result<Vec<String>, String> {
        disassemble(self, pc, &request)
    }
}

fn named(name: &str) -> Request {
    Request::Named {
        name: name.to_string(),
        count: None,
    }
}

impl Inferior for Image {
    fn pid(&self) -> u32 {
        42
    }
    fn arch(&self) -> &'static crate::debugger::arch::Arch {
        &crate::debugger::arm64::AARCH64
    }
    fn end(&mut self) -> Result<Ending, String> {
        Ok(Ending::Killed)
    }
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        Ok(self.symbols.clone())
    }
    fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn read_bytes(&mut self, address: u64, _: usize) -> Result<Vec<u8>, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn read_code(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if self.word_at_a_time && length > 4 {
            return Err(format!("unmapped {address:#x}"));
        }
        (0..length as u64 / 4)
            .map(|index| {
                self.word(address + index * 4)
                    .map(u32::to_le_bytes)
                    .ok_or_else(|| format!("unmapped {:#x}", address + index * 4))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|words| words.concat())
    }
    fn function_at(&mut self, address: u64) -> Result<Option<FunctionRange>, String> {
        Ok(self
            .code
            .iter()
            .find(|code| {
                let end = code.range.start + code.words.len() as u64 * 4;
                code.range.start <= address && address < end
            })
            .map(|code| code.range.clone()))
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn data_symbol_at(&mut self, address: u64) -> Result<Option<String>, String> {
        Ok(self.data.get(&address).cloned())
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        Ok(Vec::new())
    }
    fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
        Ok(Vec::new())
    }
    fn set_registers(&mut self, _: usize, _: &[(String, u64)]) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn step(&mut self, _: &[usize], _: u64) -> Result<Vec<Step>, String> {
        Err("read-only".to_string())
    }
    fn restart(&mut self, _: &[(OsString, OsString)]) -> Result<u32, String> {
        Err("read-only".to_string())
    }
    fn entry(&mut self) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn insert_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn remove_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn insert_watchpoint(&mut self, _: u64, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn remove_watchpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn run(&mut self) -> Result<RunStop, String> {
        Err("read-only".to_string())
    }
}

#[test]
fn a_listing_comments_what_each_instruction_points_at() {
    let mut program = Image::usual();
    assert_eq!(
        program.dis(0x1008, named("work")).unwrap(),
        [
            "prog`work:",
            "    0x0000000000001000 <+0>:  adrp   x0, 4",
            "    0x0000000000001004 <+4>:  add    x0, x0, #0x10             ; _mfb_str_1",
            "->  0x0000000000001008 <+8>:  mov    x1, #0x5                  ; =5",
            "    0x000000000000100c <+12>: bl     0x2000                    ; helper",
            "    0x0000000000001010 <+16>: b      0x1018                    ; <+24>",
            "    0x0000000000001014 <+20>: .word  0x04e10000",
            "    0x0000000000001018 <+24>: bl     0x3004                    ; 0x0000000000003000 + 4",
            "    0x000000000000101c <+28>: ret",
        ]
    );
}

#[test]
fn an_address_pair_into_a_function_and_calls_out_of_every_function_are_named() {
    let mut image = Image::usual();
    // At 0x8000: `adrp x0, 0x2000` (six pages back) and `add x0, x0, #4`,
    // which address 4 bytes into `helper`; `bl 0x3000` (an unnamed
    // function's start); `bl 0x10000` (no function there); `ret`.
    image.function(
        "prog",
        Some("_mfb_fn_far"),
        0x8000,
        true,
        &[0xd0ff_ffc0, 0x9100_1000, 0x97ff_ebfe, 0x9400_1ffd, RET],
    );
    let lines = image.dis(0, named("far")).unwrap();
    assert_eq!(
        lines[1..5],
        [
            "    0x0000000000008000 <+0>:  adrp   x0, -6",
            "    0x0000000000008004 <+4>:  add    x0, x0, #0x4              ; helper + 4",
            "    0x0000000000008008 <+8>:  bl     0x3000                    ; 0x0000000000003000",
            "    0x000000000000800c <+12>: bl     0x10000",
        ]
    );
}

#[test]
fn headers_name_stubs_unnamed_functions_and_tag_shared_names() {
    let mut image = Image::default();
    image.function(
        "prog",
        Some("symbol stub for: _malloc"),
        0x4000,
        true,
        &[NOP],
    );
    image.function("libfoo.dylib", None, 0x3000, true, &[NOP]);
    let exit_c = format!("{C_PREFIX}exit");
    image.function("prog", Some("_mfb_fn_exit"), 0x1000, true, &[RET]);
    image.function("libc", Some(&exit_c), 0x6000, true, &[RET]);
    let first = |image: &mut Image, address| {
        let request = Request::At {
            address,
            count: Some(Window::count(1)),
        };
        image.dis(0, request).unwrap().remove(0)
    };
    assert_eq!(first(&mut image, 0x4000), "prog`symbol stub for: malloc:");
    assert_eq!(
        first(&mut image, 0x3000),
        "libfoo.dylib`0x0000000000003000:"
    );
    assert_eq!(first(&mut image, 0x1000), "prog`exit <mfb>:");
    assert_eq!(first(&mut image, 0x6000), "libc`exit <native>:");
}

#[test]
fn code_no_function_holds_lists_to_its_first_ret() {
    let mut image = Image::usual();
    image.loose.push((0x7000, vec![NOP, NOP, RET, NOP]));
    let listing = [
        "0x0000000000007000:",
        "    0x0000000000007000: nop",
        "->  0x0000000000007004: nop",
        "    0x0000000000007008: ret",
    ];
    let current = image.dis(0x7004, Request::Current).unwrap();
    assert_eq!(current[0], "0x0000000000007004:");
    let at = Request::At {
        address: 0x7000,
        count: None,
    };
    assert_eq!(image.dis(0x7004, at).unwrap(), listing);
    let misaligned = Request::At {
        address: 0x7002,
        count: None,
    };
    assert_eq!(
        image.dis(0, misaligned),
        Err(
            "0x0000000000007002 is not the start of an instruction: they start every 4 bytes"
                .to_string()
        )
    );
}

#[test]
fn a_listing_from_inside_a_function_runs_to_its_end() {
    let mut image = Image::usual();
    let at = Request::At {
        address: 0x1014,
        count: None,
    };
    let lines = image.dis(0, at).unwrap();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_eq!(lines[0], "prog`work:");
    assert!(lines[3].ends_with("ret"), "{lines:?}");
}

#[test]
fn a_name_for_two_functions_lists_both_and_a_count_limits_each() {
    let mut image = Image::default();
    image.function("prog", Some("_mfb_fn_twice"), 0x1000, true, &[NOP, RET]);
    image.function("prog", Some("_mfb_fn_twice"), 0x2000, true, &[NOP, RET]);
    let request = Request::Named {
        name: "_mfb_fn_twice".to_string(),
        count: Some(Window::count(1)),
    };
    assert_eq!(
        image.dis(0, request).unwrap(),
        [
            "prog`twice:",
            "    0x0000000000001000 <+0>: nop",
            "",
            "prog`twice:",
            "    0x0000000000002000 <+0>: nop",
        ]
    );
}

#[test]
fn a_name_that_is_unbound_or_ambiguous_is_refused() {
    let mut image = Image::default();
    let malloc = format!("{C_PREFIX}malloc");
    image.symbols.push(FunctionSymbol {
        name: malloc.clone(),
        linked: true,
        loaded: false,
        address: None,
        size: None,
    });
    assert_eq!(
        resolve(&mut image, &malloc),
        Err(format!(
            "'{malloc}' is not loaded yet; run the program until it has loaded its libraries"
        ))
    );
    image.function("prog", Some("_mfb_fn_show_24Integer"), 0x1000, true, &[RET]);
    image.function("prog", Some("_mfb_fn_show_24String"), 0x2000, true, &[RET]);
    assert_eq!(
        resolve(&mut image, "show"),
        Err(
            "'show' is ambiguous: it names _mfb_fn_show_24Integer, _mfb_fn_show_24String; \
             name one by its symbol"
                .to_string()
        )
    );
}

#[test]
fn an_unsized_function_returns_at_its_first_ret() {
    let mut image = Image::usual();
    let range = image.function_at(0x3000).unwrap().unwrap();
    assert_eq!(returns(&mut image, &range), Ok(vec![0x3008]));
    // A sized one at every ret.
    image.function("prog", Some("_mfb_fn_two"), 0x8000, true, &[RET, NOP, RET]);
    let range = image.function_at(0x8000).unwrap().unwrap();
    assert_eq!(returns(&mut image, &range), Ok(vec![0x8000, 0x8008]));
}

#[test]
fn a_count_back_past_a_function_s_start_shows_how_far_before() {
    let mut image = Image::usual();
    // Code no function holds just before `helper`.
    image.loose.push((0x1ff8, vec![NOP, NOP]));
    let request = Request::At {
        address: 0x2000,
        count: Some(Window::count(-2)),
    };
    assert_eq!(
        image.dis(0, request).unwrap(),
        [
            "prog`helper:",
            "    0x0000000000001ff8 <-8>: nop",
            "    0x0000000000001ffc <-4>: nop",
            "    0x0000000000002000 <+0>: nop",
        ]
    );
}

#[test]
fn a_listing_reads_what_it_can_a_word_at_a_time() {
    let mut image = Image::default();
    image.loose.push((0x7000, vec![NOP, NOP]));
    image.word_at_a_time = true;
    // No extent and no ret: up to 64 instructions, of which two are mapped.
    let at = |address| Request::At {
        address,
        count: None,
    };
    assert_eq!(
        image.dis(0, at(0x7000)).unwrap(),
        [
            "0x0000000000007000:",
            "    0x0000000000007000: nop",
            "    0x0000000000007004: nop",
        ]
    );
    // Nothing readable at all is the first read's error.
    assert_eq!(image.dis(0, at(0x9000)), Err("unmapped 0x9000".to_string()));
}
