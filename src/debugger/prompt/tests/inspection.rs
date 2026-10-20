//! plan-176-K: `debugger::prompt::tests` — the inspection tests. The fake `Recorder` inferior and\n//! the shared `session`/`drive` helpers live in `prompt/tests/mod.rs`.

use super::*;

#[test]
fn show_mem_parses_an_address_or_a_register_and_offset() {
    assert_eq!(
        parse("show mem 0x1000"),
        Ok(Command::ShowMemory {
            location: Location::number(0x1000),
            size: MEMORY_SIZE
        })
    );
    assert_eq!(
        parse("show mem sp+0x18 8"),
        Ok(Command::ShowMemory {
            location: Location::named("sp", 0x18),
            size: 8
        })
    );
    assert_eq!(
        parse("show mem x19-16"),
        Ok(Command::ShowMemory {
            location: Location::named("x19", -16),
            size: MEMORY_SIZE
        })
    );
    for (line, message) in [
        (
            "show mem",
            "'show mem' needs an address or a register: show mem <where> [<size>]",
        ),
        ("show mem sp+zz", "'zz' is not a number"),
        (
            "show mem @x",
            "'@x' is not an address, a register or a function",
        ),
        ("show mem sp 8 9", "too many arguments to 'show mem'"),
        ("finish now", "'finish' takes no arguments"),
        ("rs again", "'rs' takes no arguments"),
    ] {
        assert_eq!(parse(line), Err(message.to_string()), "{line}");
    }
}

/// A place `bt` or a stop shows (`main+0x110`) can be typed back: `show`
/// and `dis` read a name that is no register as a function, and
/// `breakpoint` takes one with an offset.
#[test]
fn a_function_and_an_offset_name_a_place() {
    assert_eq!(
        parse("show mem _mfb_fn_f$own4+8"),
        Ok(Command::ShowMemory {
            location: Location::named("_mfb_fn_f$own4", 8),
            size: MEMORY_SIZE
        })
    );
    assert_eq!(
        parse("dis datetime::#normInstant+0x10 2"),
        Ok(Command::Disassemble(Disassembly::At(
            Location::named("datetime::#normInstant", 16),
            Some(Window::count(2))
        )))
    );
    assert_eq!(
        parse("bp main+272"),
        Ok(Command::SetBreakpoint {
            target: Target::Offset {
                name: "main".to_string(),
                offset: 272,
            },
            condition: None,
            commands: Vec::new()
        })
    );
    // `main` is the program's (`_mfb_fn_main` at 0x1010), so `main+0x10`
    // is 0x1020; a register keeps its name; nothing else is a place.
    let (_, err, _) = session(
        "show mem main+0x10\nshow mem x0+8\nshow mem nothing+4\nq\n",
        Ending::Killed,
        false,
    );
    assert_eq!(
        err,
        "error: unmapped 0x1020\n\
             error: unmapped 0x32\n\
             error: no register or function is named 'nothing'; type 'list reg' or 'list func' \
             for them\n"
    );
}

#[test]
fn breakerror_takes_codes_names_any_or_clear() {
    assert_eq!(parse("be"), Ok(Command::ErrorWatch));
    assert_eq!(parse("breakerror clear"), Ok(Command::ClearErrors));
    assert_eq!(
        parse("be any"),
        Ok(Command::BreakError(ErrorCodes::Any, Vec::new()))
    );
    assert_eq!(
        parse("be 77050010 errorCode::ErrNotFound ErrOutOfMemory"),
        Ok(Command::BreakError(
            ErrorCodes::Codes(vec![77_050_010, 77_050_004, 77_010_001]),
            Vec::new()
        ))
    );
    assert_eq!(
        parse("be ErrNothing"),
        Err(
            "'ErrNothing' is not an error code: give its number (77050010) or its \
                 errorCode name (ErrOverflow)"
                .to_string()
        )
    );
    // The prompt's fake names no error park, as a build without --debug.
    let (out, err, _) = session("be\nbe any\nbe clear\nq\n", Ending::Killed, false);
    assert!(out.contains("No errors are being watched for."), "{out}");
    assert_eq!(
        err,
        "error: 'breakerror' stops in the runtime's error function, which only a debug \
             build names; rebuild the program with: mfb build --debug -O0 <project>\n\
             error: no errors are being watched for\n"
    );
}

#[test]
fn show_error_takes_a_thread_and_needs_a_debug_build() {
    assert_eq!(parse("show error"), Ok(Command::ShowError(None)));
    assert_eq!(parse("show error 2"), Ok(Command::ShowError(Some(2))));
    assert_eq!(
        parse("show error x"),
        Err("'x' is not a thread id; type 'list thread' for the threads".to_string())
    );
    assert_eq!(
        parse("show error 1 2"),
        Err("too many arguments to 'show error'".to_string())
    );
    let (_, err, _) = session("show error\nshow error 9\nq\n", Ending::Killed, false);
    assert_eq!(
        err,
        "error: 'show error' reads the thread's arena, which needs a debug build; rebuild \
             the program with: mfb build --debug -O0 <project>\n\
             error: no thread 9: the program has 3 threads; type 'list thread' for them\n"
    );
}

#[test]
fn show_type_needs_a_debug_build_s_arenas() {
    let (_, err, _) = session("show type string x0\nq\n", Ending::Killed, false);
    assert_eq!(
        err,
        "error: 'show type' reads values in the arena, which needs a debug build; rebuild \
             the program with: mfb build --debug -O0 <project>\n"
    );
}

#[test]
fn show_type_alone_shows_its_usage() {
    let (out, err, _) = session("show type\nq\n", Ending::Killed, false);
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}{}\n{PROMPT}Killed process 42.\n",
            values::TYPE_USAGE
        )
    );
}

#[test]
fn show_type_parses_a_type_a_location_and_a_view() {
    assert_eq!(
        parse("show type string x1"),
        Ok(Command::ShowType {
            location: Location::named("x1", 0),
            view: View::String
        })
    );
    assert_eq!(
        parse("show type list 0x1000 resource 2..5"),
        Ok(Command::ShowType {
            location: Location::number(0x1000),
            view: View::List {
                element: Some(values::Kind::Resource),
                span: Some(values::Span {
                    start: 2,
                    end: Some(5)
                })
            }
        })
    );
    assert_eq!(
        parse("show type map x3 get Grace Hopper"),
        Ok(Command::ShowType {
            location: Location::named("x3", 0),
            view: View::Map(values::MapView::Get("Grace Hopper".to_string()))
        })
    );
    for (line, message) in [
        (
            "show type string",
            "'show type string' needs an address or a register: show type string <where>",
        ),
        (
            "show type string x1 8",
            "too many arguments to 'show type string'",
        ),
        (
            "show type record x1",
            "cannot show type 'record'; the types are: int, float, fixed, money, bool, \
                 byte, scalar, string, list, set, map, resource",
        ),
        (
            "show type list x1 3..1",
            "'3..1' is not an index or a range (3, 0..10, 2..) or a type",
        ),
        (
            "show type map x1 nope",
            "'nope' is not an index, 'keys', 'values' or 'get <key>'",
        ),
    ] {
        assert_eq!(parse(line), Err(message.to_string()), "{line}");
    }
}

#[test]
fn memory_lines_lead_with_full_addresses() {
    assert_eq!(
        memory_lines(0x16f0_0010, b"MFB!"),
        ["0x0000000016f00010 4d 46 42 21                                     | M F B !"]
    );
}

#[test]
fn restart_needs_a_launched_program() {
    let (_, err, _) = session("restart\nq\n", Ending::Killed, false);
    assert_eq!(
        err,
        "error: restart needs a program the debugger launched\n"
    );
}

#[test]
fn list_arena_without_a_registry_asks_for_a_debug_build() {
    let (_, err, _) = session("list arena\nq\n", Ending::Killed, false);
    assert_eq!(
        err,
        "error: listing arenas needs a debug build; rebuild the program with: \
             mfb build --debug -O0 <project>\n"
    );
}

#[test]
fn arena_lines_round_up_to_kilobytes_and_mark_unowned_arenas() {
    let arena = |id, thread, address, mapped| Arena {
        id,
        thread,
        address,
        mapped,
    };
    assert_eq!(
        arena_lines(&[
            arena(0, Some(0), 0x16f0_0000, 4096),
            arena(12, Some(3), 0x1_0000, 4097),
            arena(13, None, 0x2_0000, 0),
        ]),
        [
            "ID | Thread | Address            | Size (KB)",
            "0  | 0      | 0x0000000016f00000 | 4",
            "12 | 3      | 0x0000000000010000 | 5",
            "13 | -      | 0x0000000000020000 | 0",
        ]
    );
    assert!(arena_lines(&[]).is_empty());
}

#[test]
fn backtrace_parses_its_forms_in_both_spellings() {
    for name in ["backtrace", "bt"] {
        assert_eq!(
            parse(name),
            Ok(Command::Backtrace(Backtrace::Thread {
                count: None,
                thread: None
            }))
        );
        assert_eq!(
            parse(&format!("{name} all")),
            Ok(Command::Backtrace(Backtrace::All))
        );
        assert_eq!(
            parse(&format!("{name} 5")),
            Ok(Command::Backtrace(Backtrace::Thread {
                count: Some(5),
                thread: None
            }))
        );
        assert_eq!(
            parse(&format!("{name} 5 2")),
            Ok(Command::Backtrace(Backtrace::Thread {
                count: Some(5),
                thread: Some(2)
            }))
        );
    }
    for (line, message) in [
        (
            "bt 0",
            "'0' is not a number of frames (1 or more), or 'all'",
        ),
        (
            "bt -1",
            "'-1' is not a number of frames (1 or more), or 'all'",
        ),
        (
            "bt x",
            "'x' is not a number of frames (1 or more), or 'all'",
        ),
        (
            "bt 1 x",
            "'x' is not a thread id; type 'list thread' for the threads",
        ),
        ("bt 1 2 3", "'bt' takes at most two arguments"),
    ] {
        assert_eq!(parse(line), Err(message.to_string()), "{line}");
    }
}

#[test]
fn an_address_may_carry_an_offset_either_way() {
    assert_eq!(address("0x10"), Some(Ok(0x10)));
    assert_eq!(address("0x10+8"), Some(Ok(0x18)));
    assert_eq!(address("0X10-0x8"), Some(Ok(0x8)));
    assert_eq!(
        address("0x10-0x20"),
        Some(Err("'0x10-0x20' is below address 0".to_string()))
    );
    assert_eq!(
        address("0xffffffffffffffff+1"),
        Some(Err(
            "'0xffffffffffffffff+1' is past the last address".to_string()
        ))
    );
    for word in ["0xzz", "0x10+zz", "0x10-", "0x+4"] {
        assert_eq!(
            address(word),
            Some(Err(format!("'{word}' is not an address"))),
            "{word}"
        );
    }
    assert_eq!(address("16"), None);
    assert_eq!(Location::parse("0x1000-16"), Ok(Location::number(0xff0)));
    assert_eq!(Target::parse("0x1000+4"), Ok(Target::Address(0x1004)));
}

#[test]
fn dis_takes_a_count_a_place_or_a_function_and_counts_back_when_negative() {
    let register = |name: &str, offset| Location::named(name, offset);
    for (line, expected) in [
        ("dis", Disassembly::Current),
        ("dis 5", Disassembly::Count(Window::count(5))),
        ("dis -3", Disassembly::Count(Window::count(-3))),
        (
            "dis pc-8 4",
            Disassembly::At(register("pc", -8), Some(Window::count(4))),
        ),
        ("dis x1", Disassembly::At(register("x1", 0), None)),
        (
            "dis lr -2",
            Disassembly::At(register("lr", 0), Some(Window::count(-2))),
        ),
        (
            "dis 0x100+4 -1",
            Disassembly::At(Location::number(0x104), Some(Window::count(-1))),
        ),
        (
            "dis isPrime",
            Disassembly::Named("isPrime".to_string(), None),
        ),
        (
            "dis isPrime -2",
            Disassembly::Named("isPrime".to_string(), Some(Window::count(-2))),
        ),
        ("dis x31", Disassembly::Named("x31".to_string(), None)),
        (
            "dis -6 8",
            Disassembly::Count(Window { back: 6, length: 8 }),
        ),
        (
            "dis isPrime -2 3",
            Disassembly::Named("isPrime".to_string(), Some(Window { back: 2, length: 3 })),
        ),
    ] {
        assert_eq!(parse(line), Ok(Command::Disassemble(expected)), "{line}");
    }
    for (line, message) in [
        (
            "dis 0",
            "cannot show 0 instructions; give a count other than 0 (negative counts back)",
        ),
        (
            "dis pc x",
            "cannot show x instructions; give a count other than 0 (negative counts back)",
        ),
        (
            "dis pc 1 2",
            "'1' does not count back; with two counts the first is negative, as in dis -6 8",
        ),
        (
            "dis -6 -2",
            "cannot show -2 instructions; the count after a negative one must be positive",
        ),
        ("dis pc -6 8 1", "'dis' takes at most three arguments"),
        ("dis 0xzz", "'0xzz' is not an address"),
    ] {
        assert_eq!(parse(line), Err(message.to_string()), "{line}");
    }
}

/// A counted `dis` that runs out of the current function heads each other
/// function's instructions with that function; code no function holds
/// stays under the starting point.
#[test]
fn dis_heads_each_function_s_instructions_with_that_function() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.code = true;
    inferior.pc = 0x1010;
    let (out, err) = drive(&mut inferior, "dis -3 5\ndis 0x1030 -3\nq\n", false);
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
prog`next_seed:
    0x0000000000001004 <+0>: nop
prog`_mfb_rt_print:
    0x0000000000001008 <+0>: nop
    0x000000000000100c <+4>: nop
prog`main:
->  0x0000000000001010 <+0>: nop
    0x0000000000001014 <+4>: nop
{PROMPT}\
prog`datetime::#normInstant:
    0x0000000000001024 <+4>: nop
0x0000000000001030:
    0x0000000000001028:      nop
    0x000000000000102c:      nop
    0x0000000000001030:      nop
{PROMPT}Killed process 42.
"
        )
    );
}

#[test]
fn the_arena_state_lists_every_field_and_each_non_empty_entry() {
    let mut state = vec![0u8; crate::codegen::error::constants::error_constants::ARENA_STATE_SIZE];
    state[0..8].copy_from_slice(&0x1_0000_4000u64.to_le_bytes());
    // quickBin[2], at 104 + 16.
    state[120..128].copy_from_slice(&0xabcdu64.to_le_bytes());
    // v128Slots[1], a 16-byte entry.
    let v128 = ARENA_STATE_FIELDS
        .iter()
        .find(|field| field.name == "v128Slots")
        .unwrap()
        .offset
        + 16;
    state[v128] = 7;
    let lines = state_lines(&state);
    assert_eq!(lines[0], "Arena State:");
    assert_eq!(lines[1], "  blockHead         0x0000000100004000");
    assert!(lines.contains(&"  quickBin[128]     1 non-empty".to_string()));
    assert!(lines.contains(&"    [2]             0x000000000000abcd".to_string()));
    assert!(lines.contains(&"  largeBin[64]      0 non-empty".to_string()));
    assert!(lines.contains(&"  v128Slots[126]    1 non-empty".to_string()));
    assert!(lines.contains(&"    [1]             0x00000000000000000000000000000007".to_string()));
    assert_eq!(
        lines.last().unwrap(),
        "  largeFlushCredit  0x0000000000000000"
    );
    // One line per word field, plus the three array headers and two entries.
    assert_eq!(lines.len(), 1 + ARENA_STATE_FIELDS.len() + 2);
}

#[test]
fn blocks_print_their_headers_numbered_from_the_oldest() {
    let block = |address, prev, size| Block {
        address,
        prev,
        size,
        usable: size - 32,
        bump: 0,
    };
    assert_eq!(
        block_lines(&[block(0x2000, 0, 0x1000), block(0x3000, 0x2000, 0x2000)]),
        [
            "Arena Block (0):",
            "  prevBlock         0x0000000000000000",
            "  blockSize         0x0000000000001000",
            "  usableCapacity    0x0000000000000fe0",
            "  bumpOffset        0x0000000000000000",
            "Arena Block (1):",
            "  prevBlock         0x0000000000002000",
            "  blockSize         0x0000000000002000",
            "  usableCapacity    0x0000000000001fe0",
            "  bumpOffset        0x0000000000000000",
        ]
    );
}

#[test]
fn a_dump_shows_offsets_bytes_and_printable_text() {
    let bytes = b"MFBASIC arena!\x00\x01\xffxyz";
    assert_eq!(
        dump_lines(0x1_2340, bytes),
        [
            "0x0001_2340 4d 46 42 41 53 49 43 20 61 72 65 6e 61 21 00 01 | M F B A S I C   a r e n a ! . .",
            "0x0001_2350 ff 78 79 7a                                     | . x y z",
        ]
    );
}

#[test]
fn show_arena_parses_its_forms() {
    assert_eq!(
        parse("show arena"),
        Ok(Command::ShowArena { id: 0, dump: None })
    );
    assert_eq!(
        parse("show arena 3"),
        Ok(Command::ShowArena { id: 3, dump: None })
    );
    assert_eq!(
        parse("show arena 3 1"),
        Ok(Command::ShowArena {
            id: 3,
            dump: Some(Dump {
                block: 1,
                offset: 0,
                size: 1024
            })
        })
    );
    assert_eq!(
        parse("show arena 3 1 0x40 32"),
        Ok(Command::ShowArena {
            id: 3,
            dump: Some(Dump {
                block: 1,
                offset: 0x40,
                size: 32
            })
        })
    );
    for (line, message) in [
        (
            "show",
            "'show' needs what to show: show mem <where> [<size>], show type <type> <where>, \
                 show arena [<id>], show error [<thread>], or show report",
        ),
        (
            "show it",
            "cannot show 'it'; 'show mem', 'show type', 'show arena', 'show error' and \
                 'show report' are what can be shown",
        ),
        ("show report now", "'show report' takes no arguments"),
        (
            "show arena x",
            "'x' is not an arena id; type 'list arena' for the arenas",
        ),
        ("show arena 0 b", "'b' is not a block number"),
        ("show arena 0 0 zz", "'zz' is not a number"),
        ("show arena 0 0 0 0 0", "too many arguments to 'show arena'"),
    ] {
        assert_eq!(parse(line), Err(message.to_string()), "{line}");
    }
}

#[test]
fn show_mem_reads_through_memory_in_brackets() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.words = vec![(0x2000, 0x3000), (0x3000, 0x4847_4645_4443_4241)];
    let (out, err) = drive(
        &mut inferior,
        "show mem [0x2000] 8
show mem [x0]+8 8
show mem [0x2000 8
",
        false,
    );
    assert_eq!(
        out,
        format!(
            "{PROMPT}Memory at 0x0000000000003000 ([0x0000000000002000]):
0x0000000000003000 41 42 43 44 45 46 47 48                         | A B C D E F G H
{PROMPT}{PROMPT}{PROMPT}
Killed process 42.
"
        )
    );
    assert_eq!(
        err,
        "error: unmapped 0x2a
\
             error: '[0x2000' opens a '[' it does not close
"
    );
    assert_eq!(
        parse("dis [sp+8]"),
        Ok(Command::Disassemble(Disassembly::At(
            Location::parse("[sp+8]").unwrap(),
            None
        )))
    );
}

#[test]
fn show_arena_without_a_registry_asks_for_a_debug_build() {
    let (_, err, _) = session("show arena\nq\n", Ending::Killed, false);
    assert_eq!(
        err,
        "error: showing an arena needs a debug build; rebuild the program with: mfb build \
             --debug -O0 <project>\n"
    );
}
