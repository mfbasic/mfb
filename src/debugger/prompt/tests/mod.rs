use super::*;

mod execution;
mod finishing;
mod heap;
mod inspection;
mod listing;
use crate::debugger::names::Origin;
use crate::debugger::{Change, FunctionRange, Step};

/// An [`Inferior`] answering from fixed tables: two threads, a few
/// functions, three registers. It records how often it was ended and every
/// step it was asked for, and refuses every read when `unreadable`. With
/// `code` its code is all `nop`s, in functions placed as its symbols say.
struct Recorder {
    ending: Ending,
    ends: usize,
    unreadable: bool,
    code: bool,
    steps: Vec<(Vec<usize>, u64)>,
    stop: StepStop,
    pc: u64,
    inserted: Vec<u64>,
    watched: Vec<(u64, u64)>,
    runs: Vec<RunStop>,
    /// Every `set_registers`: the thread and its values. Thread 0's `pc`
    /// moves; its `x0` is the last value set, when any is.
    sets: Vec<(usize, Vec<(String, u64)>)>,
    x0: Option<u64>,
    /// Memory a `[…]` can read: 8-byte words by address.
    words: Vec<(u64, u64)>,
    /// The environment a launched program started with; `None` for one
    /// attached to, which cannot restart.
    environment: Option<Environment>,
    /// The environment each restart was given.
    restarts: Vec<Environment>,
}

impl Recorder {
    fn new(ending: Ending) -> Self {
        Recorder {
            ending,
            ends: 0,
            unreadable: false,
            code: false,
            steps: Vec::new(),
            stop: StepStop::Finished,
            pc: 0x1000,
            inserted: Vec::new(),
            watched: Vec::new(),
            runs: Vec::new(),
            sets: Vec::new(),
            x0: None,
            words: Vec::new(),
            environment: None,
            restarts: Vec::new(),
        }
    }
}

/// A C function's symbol as the host's format spells it.
fn c(name: &str) -> String {
    format!("{}{name}", crate::debugger::names::C_PREFIX)
}

fn symbol(name: &str, linked: bool, address: Option<u64>, size: Option<u64>) -> FunctionSymbol {
    FunctionSymbol {
        name: name.to_string(),
        linked,
        loaded: false,
        address,
        size,
    }
}

impl Inferior for Recorder {
    fn pid(&self) -> u32 {
        42
    }
    fn end(&mut self) -> Result<Ending, String> {
        self.ends += 1;
        Ok(self.ending)
    }
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        if self.unreadable {
            return Err("cannot read process 42's memory".to_string());
        }
        Ok(vec![
            symbol("_main", false, Some(0x1000), Some(4)),
            symbol("_mfb_fn_main", false, Some(0x1010), Some(0x10)),
            symbol(&c("malloc"), true, None, None),
            symbol("_mfb_rt_print", false, Some(0x1008), Some(8)),
            symbol("_mfb_fn_next_5Fseed", false, Some(0x1004), Some(4)),
            symbol(
                "_mfb_ifn_datetime_5FnormInstant",
                false,
                Some(0x1020),
                Some(8),
            ),
            symbol(&c("free"), true, Some(0x9000), None),
            symbol(&c("malloc"), true, None, None),
            FunctionSymbol {
                loaded: true,
                ..symbol(&c("vkCreateInstance"), true, Some(0xa000), None)
            },
        ])
    }
    fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn read_code(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if !self.code {
            return Err(format!("unmapped {address:#x}"));
        }
        Ok(0xd503_201f_u32.to_le_bytes().repeat(length / 4))
    }
    fn function_at(&mut self, address: u64) -> Result<Option<FunctionRange>, String> {
        if !self.code {
            return Ok(None);
        }
        Ok(self.functions()?.into_iter().find_map(|function| {
            let (start, size) = (function.address?, function.size?);
            (start <= address && address < start + size).then(|| FunctionRange {
                image: "prog".to_string(),
                name: Some(function.name),
                start,
                size: Some(size),
            })
        }))
    }
    fn data_symbol_at(&mut self, _: u64) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        match self.words.iter().find(|(at, _)| *at == address) {
            Some((_, word)) if length <= 8 => Ok(word.to_le_bytes()[..length].to_vec()),
            _ => Err(format!("unmapped {address:#x}")),
        }
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        if self.unreadable {
            return Err("cannot list the threads of process 42".to_string());
        }
        let thread = |id, name: Option<&str>, pc| ThreadFacts {
            id,
            handle: id,
            name: name.map(str::to_string),
            pc,
            sp: 0,
            arena: 0,
            stack: None,
        };
        Ok(vec![
            thread(0x501, None, self.pc),
            thread(0x502, Some("com.apple.worker"), 0x7000),
            thread(0x503, None, 0x7004),
        ])
    }
    fn registers(&mut self, thread: usize) -> Result<Vec<Register>, String> {
        if self.unreadable {
            return Err("cannot read thread 0's registers".to_string());
        }
        let x0 = match self.x0 {
            Some(x0) if thread == 0 => x0,
            _ => 0x2a + thread as u64,
        };
        Ok(vec![
            Register {
                name: "x0".to_string(),
                group: RegisterGroup::General,
                value: RegisterValue::Bits64(x0),
                role: Some("MFB arg, MFB result, C arg, C result"),
            },
            Register {
                name: "nzcv".to_string(),
                group: RegisterGroup::System,
                value: RegisterValue::Bits64(0x6000_0000),
                role: None,
            },
            Register {
                name: "v31".to_string(),
                group: RegisterGroup::Simd,
                value: RegisterValue::Bits128(1 << 64),
                role: None,
            },
        ])
    }
    fn set_registers(&mut self, thread: usize, values: &[(String, u64)]) -> Result<(), String> {
        self.sets.push((thread, values.to_vec()));
        for (name, value) in values {
            match name.as_str() {
                "pc" if thread == 0 => self.pc = *value,
                "x0" if thread == 0 => self.x0 = Some(*value),
                _ => {}
            }
        }
        Ok(())
    }
    /// Thread 0 moves 4 bytes an instruction; the others do not move.
    fn step(&mut self, threads: &[usize], count: u64) -> Result<Vec<Step>, String> {
        self.steps.push((threads.to_vec(), count));
        if threads.contains(&0) {
            self.pc += 4 * count;
        }
        Ok(threads
            .iter()
            .map(|thread| Step {
                taken: if *thread == 2 { 0 } else { count },
                stop: if *thread == 2 {
                    StepStop::Waiting
                } else {
                    self.stop.clone()
                },
            })
            .collect())
    }
    /// A launched program (one with an `environment`) restarts as
    /// process 43, recording the environment it was given.
    fn restart(
        &mut self,
        environment: &[(std::ffi::OsString, std::ffi::OsString)],
    ) -> Result<u32, String> {
        if self.environment.is_none() {
            return Err("restart needs a program the debugger launched".to_string());
        }
        self.restarts.push(environment.to_vec());
        Ok(43)
    }
    fn entry(&mut self) -> Result<Option<u64>, String> {
        Ok(Some(0x1000))
    }
    fn insert_breakpoint(&mut self, address: u64) -> Result<(), String> {
        self.inserted.push(address);
        Ok(())
    }
    fn insert_watchpoint(&mut self, address: u64, size: u64) -> Result<(), String> {
        self.watched.push((address, size));
        Ok(())
    }
    fn remove_watchpoint(&mut self, address: u64) -> Result<(), String> {
        self.watched.retain(|(other, _)| *other != address);
        Ok(())
    }
    fn remove_breakpoint(&mut self, address: u64) -> Result<(), String> {
        self.inserted.retain(|other| *other != address);
        Ok(())
    }
    fn run(&mut self) -> Result<RunStop, String> {
        Ok(self.runs.remove(0))
    }
}

const CONSOLE: Program = Program { app: false };

fn session(input: &str, ending: Ending, echo: bool) -> (String, String, usize) {
    let mut inferior = Recorder::new(ending);
    let (out, err) = drive(&mut inferior, input, echo);
    (out, err, inferior.ends)
}

fn drive(inferior: &mut Recorder, input: &str, echo: bool) -> (String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let environment = inferior.environment.clone();
    run(
        inferior,
        CONSOLE,
        environment,
        &mut input.as_bytes(),
        &mut out,
        &mut err,
        echo,
    )
    .expect("session runs");
    (
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn help_lists_every_command_in_both_spellings() {
    for line in ["help\nq\n", "h\nq\n"] {
        let (out, err, _) = session(line, Ending::Killed, false);
        assert_eq!(
            out,
            format!("{PROMPT}{HELP}\n{PROMPT}Killed process 42.\n"),
            "{line:?}"
        );
        assert_eq!(err, "");
    }
}

#[test]
fn quit_ends_the_session_once_in_both_spellings() {
    for line in ["quit\n", "q\n", "  q  \n"] {
        let (out, _, ends) = session(line, Ending::Detached, false);
        assert_eq!(
            out,
            format!("{PROMPT}Detached from process 42; it continues running.\n")
        );
        assert_eq!(ends, 1, "{line:?}");
    }
}

#[test]
fn end_of_input_quits() {
    let (out, _, ends) = session("", Ending::Killed, false);
    assert_eq!(out, format!("{PROMPT}\nKilled process 42.\n"));
    assert_eq!(ends, 1);
}

#[test]
fn echo_repeats_each_command_after_the_prompt() {
    let (out, _, _) = session("h\r\n\nq\n", Ending::Killed, true);
    assert_eq!(
        out,
        format!("{PROMPT}h\n{HELP}\n{PROMPT}\n{PROMPT}q\nKilled process 42.\n")
    );
}

#[test]
fn a_bad_command_reports_and_the_prompt_continues() {
    let (out, err, ends) = session("frob\nhelp me\nq now\nq\n", Ending::Killed, false);
    assert_eq!(
        out,
        format!("{PROMPT}{PROMPT}{PROMPT}{PROMPT}Killed process 42.\n")
    );
    assert_eq!(
        err,
        "error: unknown command 'frob'; type 'help' for the list of commands\n\
             error: 'help' takes no arguments\n\
             error: 'q' takes no arguments\n"
    );
    assert_eq!(ends, 1);
}

#[test]
fn an_externally_ended_process_is_reported() {
    let (out, _, _) = session("q\n", Ending::AlreadyExited, false);
    assert_eq!(out, format!("{PROMPT}Process 42 had already exited.\n"));
}

#[test]
fn a_bare_list_shows_what_can_be_listed_in_both_spellings() {
    for line in ["list\nq\n", "l\nq\n"] {
        let (out, err, _) = session(line, Ending::Killed, false);
        assert_eq!(
            out,
            format!("{PROMPT}{LIST_USAGE}\n{PROMPT}Killed process 42.\n")
        );
        assert_eq!(err, "");
    }
}

#[test]
fn list_function_is_list_func() {
    assert_eq!(parse("list function"), parse("list func"));
    assert_eq!(parse("l function MU fib"), parse("l func MU fib"));
}

#[test]
fn list_func_groups_by_origin_then_address_and_keeps_repeats() {
    let (out, err, _) = session("l func\nq\n", Ending::Killed, false);
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
Type | Address            | Symbol                          | Written name
MU   | 0x0000000000001004 | _mfb_fn_next_5Fseed             | next_seed
MU   | 0x0000000000001010 | _mfb_fn_main                    | main
MB   | 0x0000000000001020 | _mfb_ifn_datetime_5FnormInstant | datetime::#normInstant
MR   | 0x0000000000001000 | _main                           | -
MR   | 0x0000000000001008 | _mfb_rt_print                   | -
SL   | 0x0000000000009000 | {free:<31} | free
SL   | -                  | {malloc:<31} | malloc
SD   | 0x000000000000a000 | {vk:<31} | vkCreateInstance

MU  {MU}
MB  {MB}
MR  {MR}
SL  {SL}
SD  {SD}
{PROMPT}Killed process 42.\n",
            MU = Origin::User.legend(),
            free = c("free"),
            malloc = c("malloc"),
            vk = c("vkCreateInstance"),
            MB = Origin::Builtin.legend(),
            MR = Origin::Runtime.legend(),
            SL = Origin::Linked.legend(),
            SD = Origin::Dynamic.legend(),
        )
    );
}

#[test]
fn list_thread_names_each_thread_by_name_frame_or_id() {
    let (out, err, _) = session("list thread\nq\n", Ending::Killed, false);
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
ID | Name             | PC                 | Task
0  | _main            | 0x0000000000001000 | MFB Main
1  | com.apple.worker | 0x0000000000007000 | Other
2  | thread-0x503     | 0x0000000000007004 | Other
{PROMPT}Killed process 42.\n"
        )
    );
}

#[test]
fn list_reg_alone_shows_the_registers_with_a_role_then_its_usage() {
    for (line, value) in [
        ("list reg\nq\n", "2a | (42)   "),
        ("l reg 2\nq\n", "2c | (44)   "),
    ] {
        let (out, err, _) = session(line, Ending::Killed, false);
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
Register | Hex                | Decimal | Role
x0       | 0x00000000000000{value} | MFB arg, MFB result, C arg, C result

{LIST_REG_USAGE}
{PROMPT}Killed process 42.\n"
            )
        );
        assert_eq!(err, "");
    }
    let (out, err, _) = session("list reg 3\nq\n", Ending::Killed, false);
    assert_eq!(out, format!("{PROMPT}{PROMPT}Killed process 42.\n"));
    assert_eq!(
        err,
        "error: no thread 3: the program has 3 threads; type 'list thread' for them\n"
    );
}

#[test]
fn list_reg_alone_shows_only_its_usage_after_the_program_ends() {
    let mut inferior = Recorder::new(Ending::AlreadyExited);
    inferior.runs = vec![RunStop::Exited(Exit::Status(0))];
    let (out, err) = drive(&mut inferior, "run\nlist reg\nlist reg gpr\nq\n", false);
    assert!(
        out.contains(&format!("{PROMPT}{LIST_REG_USAGE}\n")),
        "{out}"
    );
    assert!(!out.contains("Register |"), "{out}");
    assert_eq!(
        err,
        "error: the program has exited; type 'quit' to end the session\n"
    );
}

#[test]
fn list_reg_shows_each_group_of_the_main_thread_or_the_one_asked_for() {
    let (out, err, _) = session(
        "list reg simd\nl reg 2 gpr sys simd\nq\n",
        Ending::Killed,
        false,
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
Register | Hex                                | Decimal
v31      | 0x00000000000000010000000000000000 | (18446744073709551616)
{PROMPT}\
Register | Hex                                | Decimal
x0       | 0x000000000000002c                 | (44)
nzcv     | 0x0000000060000000                 | (1610612736)
v31      | 0x00000000000000010000000000000000 | (18446744073709551616)
{PROMPT}Killed process 42.\n"
        )
    );
}

#[test]
fn list_reg_shows_the_main_thread_or_the_one_asked_for() {
    let (out, _, _) = session(
        "list reg gpr sys simd\nl reg 2 gpr sys simd\nq\n",
        Ending::Killed,
        false,
    );
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
Register | Hex                                | Decimal
x0       | 0x000000000000002a                 | (42)
nzcv     | 0x0000000060000000                 | (1610612736)
v31      | 0x00000000000000010000000000000000 | (18446744073709551616)
{PROMPT}\
Register | Hex                                | Decimal
x0       | 0x000000000000002c                 | (44)
nzcv     | 0x0000000060000000                 | (1610612736)
v31      | 0x00000000000000010000000000000000 | (18446744073709551616)
{PROMPT}Killed process 42.\n"
        )
    );
}

#[test]
fn list_reg_refuses_a_thread_or_register_that_does_not_exist() {
    let (_, err, _) = session(
        "list reg 3 gpr\nlist reg x9\nlist reg 1 x0, w5\nq\n",
        Ending::Killed,
        false,
    );
    assert_eq!(
        err,
        "error: no thread 3: the program has 3 threads; type 'list thread' for them\n\
             error: no register 'x9'; type 'list reg' for the groups\n\
             error: no register 'w5'; type 'list reg' for the groups\n"
    );
}

#[test]
fn list_reg_shows_just_the_registers_named_by_spaces_or_commas() {
    for line in [
        "list reg 2 nzcv x0\nq\n",
        "list reg 2 nzcv, x0\nq\n",
        "list reg 2,nzcv,x0\nq\n",
    ] {
        let (out, err, _) = session(line, Ending::Killed, false);
        assert_eq!(err, "", "{line}");
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
Register | Hex                | Decimal
nzcv     | 0x0000000060000000 | (1610612736)
x0       | 0x000000000000002c | (44)
{PROMPT}Killed process 42.\n"
            ),
            "{line}"
        );
    }
}

#[test]
fn a_negative_64_bit_value_shows_as_a_signed_decimal() {
    let register = Register {
        name: "x8".to_string(),
        group: RegisterGroup::General,
        value: RegisterValue::Bits64(u64::MAX),
        role: None,
    };
    assert_eq!(
        register_lines(&[register]),
        [
            "Register | Hex                | Decimal",
            "x8       | 0xffffffffffffffff | (-1)",
        ]
    );
}

#[test]
fn a_listing_that_fails_reports_and_the_prompt_continues() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.unreadable = true;
    let (out, err) = drive(
        &mut inferior,
        "list func\nlist reg gpr\nlist thread\nq\n",
        false,
    );
    assert_eq!(
        out,
        format!("{PROMPT}{PROMPT}{PROMPT}{PROMPT}Killed process 42.\n")
    );
    assert_eq!(
        err,
        "error: cannot read process 42's memory\n\
             error: cannot list the threads of process 42\n\
             error: cannot list the threads of process 42\n"
    );
    assert_eq!(inferior.ends, 1);
}

#[test]
fn a_bad_list_argument_is_reported() {
    let (_, err, _) = session("list frames\nlist thread reg\nq\n", Ending::Killed, false);
    assert_eq!(
        err,
        "error: cannot list 'frames'; type 'list' for what can be listed\n\
             error: too many arguments to 'list thread'\n"
    );
}

#[test]
fn empty_listings_print_nothing() {
    assert!(function_lines(&[], &FunctionFilter::default()).is_empty());
    assert!(register_lines(&[]).is_empty());
    assert!(thread_lines(&[], &[], &[]).is_empty());
    assert!(table(&["A", "B"], Vec::new()).is_empty());
}

#[test]
fn a_table_aligns_every_column_to_its_widest_entry_by_characters() {
    assert_eq!(
        table(
            &["ID", "Name", "Task"],
            vec![
                vec!["0".to_string(), "héllo".to_string(), "MFB Main".to_string()],
                vec!["10".to_string(), "x".to_string(), "Other".to_string()],
            ]
        ),
        [
            "ID | Name  | Task",
            "0  | héllo | MFB Main",
            "10 | x     | Other",
        ]
    );
}

#[test]
fn list_func_keeps_the_types_and_names_asked_for() {
    let (out, err, _) = session(
        "list func next_seed\nlist func MR MB\nlist func *_5F* free\nlist func MU free\nq\n",
        Ending::Killed,
        false,
    );
    assert_eq!(
        err,
        "error: no function matches 'MU free'; type 'list func' for the functions\n"
    );
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
Type | Address            | Symbol              | Written name
MU   | 0x0000000000001004 | _mfb_fn_next_5Fseed | next_seed

MU  {user}
{PROMPT}\
Type | Address            | Symbol                          | Written name
MB   | 0x0000000000001020 | _mfb_ifn_datetime_5FnormInstant | datetime::#normInstant
MR   | 0x0000000000001000 | _main                           | -
MR   | 0x0000000000001008 | _mfb_rt_print                   | -

MB  {builtin}
MR  {runtime}
{PROMPT}\
Type | Address            | Symbol                          | Written name
MU   | 0x0000000000001004 | _mfb_fn_next_5Fseed             | next_seed
MB   | 0x0000000000001020 | _mfb_ifn_datetime_5FnormInstant | datetime::#normInstant
SL   | 0x0000000000009000 | {free:<31} | free

MU  {user}
MB  {builtin}
SL  {linked}
{PROMPT}{PROMPT}Killed process 42.
",
            user = Origin::User.legend(),
            free = c("free"),
            builtin = Origin::Builtin.legend(),
            runtime = Origin::Runtime.legend(),
            linked = Origin::Linked.legend(),
        )
    );
}

#[test]
fn a_glob_matches_the_whole_name() {
    for (pattern, text, matches) in [
        ("fib", "fib", true),
        ("fib", "fibs", false),
        ("fib", "_fib", false),
        ("*fib*", "_mfb_fn_fib", true),
        ("f?b", "fob", true),
        ("f?b", "fb", false),
        ("*", "", true),
        ("a*b*c", "aXbYbZc", true),
        ("a*b*c", "aXbYbZ", false),
        ("**x", "yyx", true),
        ("é?", "éa", true),
    ] {
        assert_eq!(glob(pattern, text), matches, "{pattern} {text}");
    }
}
