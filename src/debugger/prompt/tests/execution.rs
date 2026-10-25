//! plan-176-K: `debugger::prompt::tests` — the execution tests. The fake `Recorder` inferior and\n//! the shared `session`/`drive` helpers live in `prompt/tests/mod.rs`.

use super::*;

fn variables(pairs: &[(&str, &str)]) -> Environment {
    pairs
        .iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect()
}

#[test]
fn env_changes_what_the_next_restart_gives_and_list_env_shows_it() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.environment = Some(variables(&[("PATH", "/bin"), ("HOME", "/h")]));
    let (out, err) = drive(
        &mut inferior,
        "list env\nenv s GREETING  hello,  world \nenv r HOME\nlist env\nrestart\n\
             list env\nenv clear\nenv reset\nrestart\nq\n",
        false,
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}Name | Current | After restart\n\
                 HOME | /h      | /h\n\
                 PATH | /bin    | /bin\n\
                 {PROMPT}Set GREETING=hello,  world ; the program gets it when it restarts.\n\
                 {PROMPT}Removed HOME; the program goes without it when it restarts.\n\
                 {PROMPT}Name     | Current | After restart\n\
                 GREETING |         | hello,  world \n\
                 HOME     | /h      | \n\
                 PATH     | /bin    | /bin\n\
                 {PROMPT}Restarted the program as process 43; paused before its first \
                 instruction.\n\
                 {PROMPT}Name     | Current        | After restart\n\
                 GREETING | hello,  world  | hello,  world \n\
                 PATH     | /bin           | /bin\n\
                 {PROMPT}Cleared the environment; the program starts with none when it \
                 restarts.\n\
                 {PROMPT}Reset the environment to the one the session started the program \
                 with; the program gets it when it restarts.\n\
                 {PROMPT}Restarted the program as process 43; paused before its first \
                 instruction.\n\
                 {PROMPT}Killed process 42.\n"
        )
    );
    assert_eq!(
        inferior.restarts,
        [
            variables(&[("PATH", "/bin"), ("GREETING", "hello,  world ")]),
            variables(&[("PATH", "/bin"), ("HOME", "/h")]),
        ]
    );
}

#[test]
fn env_works_once_the_program_has_ended() {
    let mut inferior = Recorder::new(Ending::AlreadyExited);
    inferior.environment = Some(Vec::new());
    inferior.runs = vec![RunStop::Exited(Exit::Status(0))];
    let (_, err) = drive(
        &mut inferior,
        "run\nenv s A 1\nlist env\nrestart\nq\n",
        false,
    );
    assert_eq!(err, "");
    assert_eq!(inferior.restarts, [variables(&[("A", "1")])]);
}

#[test]
fn env_needs_a_launched_program() {
    let (_, err, _) = session("env s A 1\nlist env\nq\n", Ending::Detached, false);
    assert_eq!(
        err,
        "error: 'env' needs a program the debugger launched; this one was attached to\n\
             error: 'list env' needs a program the debugger launched; this one was attached to\n"
    );
}

#[test]
fn env_refuses_what_it_cannot_do() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.environment = Some(variables(&[("A", "1")]));
    let (_, err) = drive(
        &mut inferior,
        "env\nenv s\nenv set A=B 1\nenv r\nenv remove A B\nenv r B\nenv clear now\n\
             env reset x\nenv frob\nlist env x\nbp 0x1000 list env\nq\n",
        false,
    );
    assert_eq!(
        err,
        "error: 'env' needs what to do: env s <name> <value>, env r <name>, env clear or \
             env reset; 'list env' shows the environment\n\
             error: 'env s' needs a name and a value: env s <name> <value>\n\
             error: 'A=B' is not a variable name: a name holds no '=' and no NUL\n\
             error: 'env r' needs a name: env r <name>\n\
             error: too many arguments to 'env remove'\n\
             error: no variable 'B' is in the environment; type 'list env' for it\n\
             error: 'env clear' takes no arguments\n\
             error: 'env reset' takes no arguments\n\
             error: cannot 'env frob'; env s, env r, env clear and env reset are what env does\n\
             error: too many arguments to 'list env'\n\
             error: 'list env' cannot run at a stop: the environment it lists is the one the \
             program gets when it restarts\n"
    );
}

#[test]
fn env_s_takes_the_rest_of_the_line_as_the_value() {
    assert_eq!(after_words("env s A  b c \r\n", 3), "b c ");
    assert_eq!(after_words("  env\tset A\n", 3), "");
    assert_eq!(
        parse("env set A x=1 y\n", &AARCH64),
        Ok(Command::Env(environment::Change::Set {
            name: "A".to_string(),
            value: "x=1 y".to_string(),
        }))
    );
}

#[test]
fn step_runs_the_main_thread_and_says_where_it_stopped() {
    let mut inferior = Recorder::new(Ending::Killed);
    let (out, err) = drive(&mut inferior, "step\ns 2\nstep 0\nq\n", false);
    assert_eq!(err, "");
    assert_eq!(inferior.steps, [(vec![0], 1), (vec![0], 2)]);
    assert_eq!(
        out,
        format!(
            "{PROMPT}Stepped 1 instruction; thread 0 is at 0x0000000000001004 in next_seed.\n\
                 {PROMPT}Stepped 2 instructions; thread 0 is at 0x000000000000100c in _mfb_rt_print+0x4.\n\
                 {PROMPT}Thread 0 is at 0x000000000000100c in _mfb_rt_print+0x4.\n\
                 {PROMPT}Killed process 42.\n"
        )
    );
}

#[test]
fn step_reports_an_exception_a_wait_or_the_end_of_the_program() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.pc = 0x8000;
    inferior.stop = StepStop::Exception("a breakpoint instruction".to_string());
    let (out, _) = drive(&mut inferior, "s\nq\n", false);
    assert!(
        out.contains("Stopped after 1 instruction by a breakpoint instruction; thread 0 is at 0x0000000000008004.\n"),
        "{out}"
    );
    inferior.stop = StepStop::Waiting;
    let (out, _) = drive(&mut inferior, "s 3\nq\n", false);
    assert!(
        out.contains("Stopped after 3 instructions: thread 0 is waiting in a system call at 0x0000000000008010.\n"),
        "{out}"
    );
    inferior.stop = StepStop::Exited;
    let (out, _) = drive(&mut inferior, "s\nq\n", false);
    assert!(
        out.contains("The program exited after 1 instruction.\n"),
        "{out}"
    );
}

#[test]
fn step_runs_the_thread_asked_for() {
    let mut inferior = Recorder::new(Ending::Killed);
    let (out, err) = drive(&mut inferior, "step 2 1\ns 0 1\nq\n", false);
    assert_eq!(err, "");
    assert_eq!(inferior.steps, [(vec![1], 2)]);
    assert_eq!(
        out,
        format!(
            "{PROMPT}Stepped 2 instructions; thread 1 is at 0x0000000000007000.\n\
                 {PROMPT}Thread 1 is at 0x0000000000007000.\n\
                 {PROMPT}Killed process 42.\n"
        )
    );
}

#[test]
fn step_refuses_a_bad_count_or_thread() {
    let mut inferior = Recorder::new(Ending::Killed);
    let (_, err) = drive(
        &mut inferior,
        "step -1\ns ten\ns 1 x\ns 1 3\ns 1 2 3\nsa -4\nsa 1 2\nq\n",
        false,
    );
    assert_eq!(
        err,
        "error: cannot step -1 instructions: the count must not be negative\n\
             error: 'ten' is not a number of instructions\n\
             error: 'x' is not a thread id; type 'list thread' for the threads\n\
             error: no thread 3: the program has 3 threads; type 'list thread' for them\n\
             error: 's' takes at most two arguments\n\
             error: cannot step -4 instructions: the count must not be negative\n\
             error: 'sa' takes at most one argument\n"
    );
    assert!(inferior.steps.is_empty());
}

#[test]
fn step_all_runs_every_thread_and_reports_each() {
    let mut inferior = Recorder::new(Ending::Killed);
    let (out, err) = drive(&mut inferior, "step-all 2\nsa 0\nq\n", false);
    assert_eq!(err, "");
    assert_eq!(inferior.steps, [(vec![0, 1, 2], 2)]);
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
Thread 0: stepped 2 instructions; at 0x0000000000001008 in _mfb_rt_print.
Thread 1: stepped 2 instructions; at 0x0000000000007000.
Thread 2: stopped after 0 instructions, waiting in a system call; at 0x0000000000007004.
{PROMPT}\
Thread 0 is at 0x0000000000001008 in _mfb_rt_print.
Thread 1 is at 0x0000000000007000.
Thread 2 is at 0x0000000000007004.
{PROMPT}Killed process 42.\n"
        )
    );
}

#[test]
fn step_all_reports_an_exception_or_the_end_of_the_program() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.stop = StepStop::Exception("an illegal instruction".to_string());
    let (out, _) = drive(&mut inferior, "sa\nq\n", false);
    assert!(
        out.contains("Thread 1: stopped after 1 instruction by an illegal instruction; at 0x0000000000007000.\n"),
        "{out}"
    );
    inferior.stop = StepStop::Exited;
    let (out, _) = drive(&mut inferior, "sa 3\nq\n", false);
    assert!(
        out.contains("The program exited after 3 rounds of stepping.\n"),
        "{out}"
    );
}

#[test]
fn breakpoints_are_set_listed_and_removed() {
    let mut inferior = Recorder::new(Ending::Killed);
    let (out, err) = drive(
        &mut inferior,
        "bp next_seed\nbreakpoint 0x1012\nlist breakpoint\nbp r 0x1012\nl breakpoint\nq\n",
        false,
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed.\n\
                 {PROMPT}Set a breakpoint at 0x0000000000001012 in main+0x2.\n\
                 {PROMPT}\
Address            | Count | Symbol              | Written name
0x0000000000001004 | 0     | _mfb_fn_next_5Fseed | next_seed
0x0000000000001012 | 0     | _mfb_fn_main+0x2    | main+0x2
{PROMPT}Removed the breakpoint at 0x0000000000001012 in main+0x2.\n\
                 {PROMPT}Address            | Count | Symbol              | Written name\n\
                 0x0000000000001004 | 0     | _mfb_fn_next_5Fseed | next_seed\n\
                 {PROMPT}Killed process 42.\n"
        )
    );
    assert_eq!(inferior.inserted, [0x1004]);
}

#[test]
fn a_bad_breakpoint_command_is_refused() {
    let (_, err, _) = session(
        "bp\nbp r\nbp 0xzz\nbp a b c\nbp r a b\nbp r nothing\nrun now\nq\n",
        Ending::Killed,
        false,
    );
    assert_eq!(
        err,
        "error: 'bp' needs an address (0x…) or a function name\n\
             error: 'bp' needs an address (0x…) or a function name\n\
             error: '0xzz' is not an address\n\
             error: unknown command 'b'; type 'help' for the list of commands\n\
             error: too many arguments to 'bp r'\n\
             error: there is no breakpoint on 'nothing'\n\
             error: 'run' takes no arguments\n"
    );
}

#[test]
fn run_reports_each_kind_of_stop() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.runs = vec![
        RunStop::Breakpoints(vec![(0, 0x1000), (1, 0x7000)]),
        RunStop::Exception {
            thread: 1,
            what: "a bad memory access at 0x8".to_string(),
        },
        RunStop::Interrupted,
        RunStop::Exited(Exit::Signal(9)),
    ];
    let (out, err) = drive(
        &mut inferior,
        "run\nr\nrun\nrun\nstep\nlist breakpoint\nq\n",
        false,
    );
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
--[BP] T: 0 P: 0x0000000000001000 F: _main
--[BP] T: 1 P: 0x0000000000007000 F: -
{PROMPT}--[XC] T: 1 P: 0x0000000000007000 F: - X: a bad memory access at 0x8
{PROMPT}--[CC] User Break
{PROMPT}--[EX] Program ended by signal (9)
{PROMPT}{PROMPT}no breakpoints
{PROMPT}Killed process 42.\n"
        )
    );
    assert_eq!(
        err,
        "error: the program has exited; type 'quit' to end the session\n"
    );
}

#[test]
fn watchpoint_breakpoint_and_breakerror_take_commands_after_a_semicolon() {
    let commands = |list: &[&str]| list.iter().map(|word| word.to_string()).collect::<Vec<_>>();
    assert_eq!(
        parse("bp fib bt 1; list reg x0 ;", &AARCH64),
        Ok(Command::SetBreakpoint {
            target: Target::Name("fib".to_string()),
            condition: None,
            commands: commands(&["bt 1", "list reg x0"])
        })
    );
    // `finish` may be anywhere among a stop's commands, `run` only last.
    assert_eq!(
        parse("bp fib bt 1; f; list reg x0; r", &AARCH64),
        Ok(Command::SetBreakpoint {
            target: Target::Name("fib".to_string()),
            condition: None,
            commands: commands(&["bt 1", "f", "list reg x0", "r"])
        })
    );
    assert_eq!(
        parse("wp sp+0x10", &AARCH64),
        Ok(Command::SetWatchpoint {
            location: Location::named("sp", 16),
            size: 8,
            commands: Vec::new(),
        })
    );
    assert_eq!(
        parse("watchpoint 0x1000 0x4 show mem 0x1000 4;be", &AARCH64),
        Ok(Command::SetWatchpoint {
            location: Location::number(0x1000),
            size: 4,
            commands: commands(&["show mem 0x1000 4", "be"]),
        })
    );
    assert_eq!(
        parse("wp r x19+8", &AARCH64),
        Ok(Command::RemoveWatchpoint(Location::named("x19", 8)))
    );
    assert_eq!(
        parse("be ErrOverflow 42 show error; bt", &AARCH64),
        Ok(Command::BreakError(
            ErrorCodes::Codes(vec![77_050_010, 42]),
            commands(&["show error", "bt"])
        ))
    );
    assert_eq!(
        parse("be any dis 4", &AARCH64),
        Ok(Command::BreakError(ErrorCodes::Any, commands(&["dis 4"])))
    );
    assert_eq!(
        parse("list watchpoint", &AARCH64),
        Ok(Command::List(Some(Listing::Watchpoints)))
    );
    for (line, message) in [
        (
            "bp fib run; bt",
            "'run' must be the last command of a stop: 'bt' follows it",
        ),
        (
            "bp fib r; finish",
            "'r' must be the last command of a stop: 'finish' follows it",
        ),
        (
            "wp 0x1000 8 bt; step",
            "'step' cannot run at a stop: a stop runs list, show, backtrace, disassemble, \
                 jump, finish, a bare breakerror and, last, run",
        ),
        (
            "be any be clear",
            "'be clear' cannot run at a stop: a stop runs list, show, backtrace, \
                 disassemble, jump, finish, a bare breakerror and, last, run",
        ),
        ("wp 0x1000 8x", "'8x' is not a number"),
        (
            "wp",
            "'wp' needs an address or a register: wp <where> [<size>] [<commands>]",
        ),
        (
            "wp r",
            "'wp' needs an address or a register: wp <where> [<size>] [<commands>]",
        ),
        ("wp r 0x10 8", "too many arguments to 'wp r'"),
        (
            "be bt",
            "'be' needs an error code, a name or 'any' before its commands",
        ),
        ("be clear bt", "'be clear' takes no commands"),
        (
            "list watchpoint 2",
            "too many arguments to 'list watchpoint'",
        ),
    ] {
        assert_eq!(parse(line, &AARCH64), Err(message.to_string()), "{line}");
    }
}

#[test]
fn a_stop_runs_its_commands_and_the_program_runs_on() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.runs = vec![
        RunStop::Breakpoints(vec![(0, 0x1004)]),
        RunStop::Exited(Exit::Status(0)),
    ];
    let (out, err) = drive(
        &mut inferior,
        "bp next_seed list reg x0; show mem 0x10; run\nrun\nq\n",
        false,
    );
    assert_eq!(err, "error: unmapped 0x10\n");
    assert_eq!(
        out,
        format!(
            "{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed. Each stop runs \
                 'list reg x0; show mem 0x10; run'.
{PROMPT}--[BP] T: 0 P: 0x0000000000001004 F: (next_seed) _mfb_fn_next_5Fseed
Register | Hex                | Decimal
x0       | 0x000000000000002a | (42)
--[EX] Program exit (0)
{PROMPT}Killed process 42.
"
        )
    );
}

#[test]
fn a_stop_whose_commands_do_not_end_in_run_stays_stopped_and_is_counted() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.runs = vec![
        RunStop::Breakpoints(vec![(0, 0x1004)]),
        RunStop::Exited(Exit::Status(0)),
    ];
    let (out, err) = drive(
        &mut inferior,
        "bp next_seed list reg x0\nrun\nlist breakpoint\nq\n",
        false,
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed. Each stop runs \
                 'list reg x0'.
{PROMPT}--[BP] T: 0 P: 0x0000000000001004 F: (next_seed) _mfb_fn_next_5Fseed
Register | Hex                | Decimal
x0       | 0x000000000000002a | (42)
{PROMPT}\
Address            | Count | Symbol              | Written name | Commands
0x0000000000001004 | 1     | _mfb_fn_next_5Fseed | next_seed    | list reg x0
{PROMPT}Killed process 42.
"
        )
    );
}

#[test]
fn run_reports_a_changed_watchpoint_and_a_raised_error() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.runs = vec![
        RunStop::Changed(vec![
            Change {
                thread: 0,
                pc: 0x1014,
                address: 0x2000,
                size: 8,
                old: 5,
                new: u64::MAX,
            },
            Change {
                thread: 1,
                pc: 0x7000,
                address: 0x3001,
                size: 2,
                old: 0xab,
                new: 0x1cd,
            },
        ]),
        RunStop::Raised {
            thread: 0,
            code: 77_050_010,
            origin: Some("main.mfb:3:5".to_string()),
        },
        RunStop::Raised {
            thread: 0,
            code: 42,
            origin: None,
        },
    ];
    let (out, err) = drive(&mut inferior, "run\nrun\nrun\nq\n", false);
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
--[WP] T: 0 P: 0x0000000000001014 F: (main) _mfb_fn_main+0x4 M: 0x0000000000002000: 0x0000000000000005 (5) \
-> 0xffffffffffffffff (-1)
--[WP] T: 1 P: 0x0000000000007000 F: - M: 0x0000000000003001: 0x00ab -> 0x01cd
{PROMPT}--[BE] T: 0 P: 0x0000000000001000 F: _main E: errorCode::ErrOverflow (77050010)
{PROMPT}--[BE] T: 0 P: 0x0000000000001000 F: _main E: 42
{PROMPT}Killed process 42.
"
        )
    );
}

#[test]
fn run_reports_an_exit_status() {
    let mut inferior = Recorder::new(Ending::AlreadyExited);
    inferior.runs = vec![RunStop::Exited(Exit::Status(3))];
    let (out, _) = drive(&mut inferior, "run\nq\n", false);
    assert!(out.contains("--[EX] Program exit (3)\n"), "{out}");
    let mut inferior = Recorder::new(Ending::AlreadyExited);
    inferior.runs = vec![RunStop::Exited(Exit::Unknown)];
    let (out, _) = drive(&mut inferior, "run\nq\n", false);
    assert!(out.contains("--[EX] Program exit\n"), "{out}");
}

#[test]
fn breakpointif_takes_a_test_a_place_and_commands() {
    let condition = |location, test, value| Some(Condition::parse(location, test, value).unwrap());
    assert_eq!(
        parse("bpi x0 eq 5 at logPath bt", &AARCH64),
        Ok(Command::SetBreakpoint {
            target: Target::Name("logPath".to_string()),
            condition: condition("x0", "eq", Some("5")),
            commands: vec!["bt".to_string()],
        })
    );
    assert_eq!(
        parse("breakpointif [sp+0x18] in at 0x1000+4", &AARCH64),
        Ok(Command::SetBreakpoint {
            target: Target::Address(0x1004),
            condition: condition("[sp+0x18]", "in", None),
            commands: Vec::new(),
        })
    );
    assert_eq!(
        parse("bpi r [x0]-8 lt -1 at main+0x10", &AARCH64),
        Ok(Command::RemoveBreakpoint {
            target: Target::Offset {
                name: "main".to_string(),
                offset: 0x10
            },
            condition: condition("[x0]-8", "lt", Some("-1")),
        })
    );
    for (line, message) in [
        (
            "bpi",
            "'bpi' needs a test and a place: bpi <where> eq|ne|gt|lt <value> at \
                 <address|function[+offset]> [<commands>], or bpi <where> in at \
                 <address|function[+offset]>",
        ),
        (
            "bpi x0 is 5 at f",
            "'is' is not a test: give eq, ne, gt, lt or in",
        ),
        (
            "bpi x0 eq at f",
            "'eq' needs a value to test against: <where> eq <value>",
        ),
        (
            "bpi x0 eq five at f",
            "'five' is not a value: give a number, such as 5, -1 or 0x10",
        ),
        (
            "bpi [x0] in 5 at f",
            "'in' takes no value: <where> in at <address|function[+offset]>",
        ),
        (
            "bpi x0 in at f",
            "'x0' can always be worked out: 'in' needs a memory read ([…])",
        ),
        (
            "bpi x0 eq 5 f",
            "'bpi' needs 'at' and the breakpoint's place after its test: bpi <where> \
                 <test> at <address|function[+offset]>",
        ),
        (
            "bpi x0 eq 5 at",
            "'bpi' needs a place after 'at': an address (0x…) or a function name",
        ),
        ("bpi r x0 eq 5 at f bt", "too many arguments to 'bpi r'"),
        (
            "bpi x0 eq 5 at f restart",
            "'restart' cannot run at a stop: a stop runs list, show, backtrace, \
                 disassemble, jump, finish, a bare breakerror and, last, run",
        ),
    ] {
        assert_eq!(parse(line, &AARCH64), Err(message.to_string()), "{line}");
    }
}

#[test]
fn a_breakpoint_with_a_test_is_listed_with_it_and_bp_r_takes_every_one_there() {
    let mut inferior = Recorder::new(Ending::Killed);
    let (out, err) = drive(
        &mut inferior,
        "bp next_seed bt
bpi x0 eq 5 at next_seed
bpi [0x12348593] eq 5 at next_seed bt
\
             bpi x9 eq 5 at next_seed
l breakpoint
bpi r x0 eq 5 at next_seed
\
             bp r next_seed
l breakpoint
q
",
        false,
    );
    assert_eq!(
        err,
        "error: no register or function is named 'x9'; type 'list reg' or 'list func' for \
             them
"
    );
    assert_eq!(
        out,
        format!(
            "{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed. Each stop runs 'bt'.
{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed if x0 eq 0x0000000000000005 (5).
{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed if [0x0000000012348593] eq \
                 0x0000000000000005 (5). Each stop runs 'bt'.
{PROMPT}{PROMPT}\
Address            | Count | Symbol              | Written name | If                                             | Commands
0x0000000000001004 | 0     | _mfb_fn_next_5Fseed | next_seed    | -                                              | bt
0x0000000000001004 | 0     | _mfb_fn_next_5Fseed | next_seed    | x0 eq 0x0000000000000005 (5)                   | -
0x0000000000001004 | 0     | _mfb_fn_next_5Fseed | next_seed    | [0x0000000012348593] eq 0x0000000000000005 (5) | bt
{PROMPT}Removed the breakpoint at 0x0000000000001004 in next_seed if x0 eq 0x0000000000000005 (5).
{PROMPT}Removed the breakpoint at 0x0000000000001004 in next_seed.
Removed the breakpoint at 0x0000000000001004 in next_seed if [0x0000000012348593] eq \
                 0x0000000000000005 (5).
{PROMPT}no breakpoints
{PROMPT}Killed process 42.
"
        )
    );
    assert!(inferior.inserted.is_empty());
}

#[test]
fn jump_moves_the_current_thread_and_sets_its_registers() {
    assert_eq!(
        parse("j lr x0=0 x1=-1 sp=[sp+8]", &AARCH64),
        Ok(Command::Jump {
            to: Location::named("lr", 0),
            registers: vec![
                ("x0".to_string(), Location::number(0)),
                ("x1".to_string(), Location::number(u64::MAX)),
                ("sp".to_string(), Location::parse("[sp+8]").unwrap()),
            ],
        })
    );
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.code = true;
    inferior.words = vec![(0x2a, 0x77)];
    let (out, err) = drive(
        &mut inferior,
        "jump 0x1010
j next_seed x0=[x0] nzcv=0x60000000 fpcr=0
list reg x0
q
",
        false,
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}Moved thread 0 to 0x0000000000001010 in main.
{PROMPT}Moved thread 0 to 0x0000000000001004 in next_seed; set x0 = 0x0000000000000077 \
                 (119), nzcv = 0x0000000060000000 (1610612736), fpcr = 0x0000000000000000 (0).
{PROMPT}\
Register | Hex                | Decimal
x0       | 0x0000000000000077 | (119)
{PROMPT}Killed process 42.
"
        )
    );
    assert_eq!(
        inferior.sets,
        [
            (0, vec![("pc".to_string(), 0x1010)]),
            (
                0,
                vec![
                    ("pc".to_string(), 0x1004),
                    ("x0".to_string(), 0x77),
                    ("nzcv".to_string(), 0x6000_0000),
                    ("fpcr".to_string(), 0),
                ]
            ),
        ]
    );
}

#[test]
fn jump_refuses_a_bad_place_register_or_value() {
    let sets = "jump sets x0-x28, fp, lr, sp, nzcv, fpsr and fpcr";
    for (line, message) in [
        (
            "j".to_string(),
            "'j' needs a place to go: j <where> [<register>=<value>]...".to_string(),
        ),
        (
            "j lr pc=4".to_string(),
            format!("'pc' is where the thread goes: give it as the place, jump <where>; {sets}"),
        ),
        (
            "j lr v0=1".to_string(),
            format!("'v0' is a vector register; {sets}"),
        ),
        (
            "j lr x29=1".to_string(),
            format!("no register 'x29'; {sets}"),
        ),
        (
            "j lr x0=1 x0=2".to_string(),
            "'x0' is set twice".to_string(),
        ),
        (
            "j lr x0".to_string(),
            "'x0' is not a register and its value: <register>=<value>, such as x0=5".to_string(),
        ),
        (
            "j lr x0=".to_string(),
            "'x0' needs a value: x0=<value>".to_string(),
        ),
        (
            format!(
                "j lr {}",
                (0..9)
                    .map(|n| format!("x{n}=0"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            "'j' sets at most 8 registers".to_string(),
        ),
    ] {
        assert_eq!(parse(&line, &AARCH64), Err(message), "{line}");
    }
    let mut inferior = Recorder::new(Ending::Killed);
    let (_, err) = drive(
        &mut inferior,
        "j 0x1002
j 0x1010
",
        false,
    );
    assert_eq!(
        err,
        "error: 0x0000000000001002 is not the start of an instruction: they start every 4 \
             bytes
\
             error: cannot jump to 0x0000000000001010: unmapped 0x1010
"
    );
    inferior.code = true;
    let (_, err) = drive(
        &mut inferior,
        "j 0x1010 nzcv=1
j 0x1010 fpsr=0x100000000
j 0x1010 x0=[0x10]
j 0x1010 x0=nothing
",
        false,
    );
    assert_eq!(
        err,
        "error: cannot set nzcv to 0x0000000000000001: it holds only the flags, bits 28 to 31 \
             (0x00000000f0000000)
\
             error: cannot set fpsr to 0x0000000100000000: it holds 32 bits
\
             error: cannot set x0: unmapped 0x10
\
             error: cannot set x0: no register or function is named 'nothing'; type 'list reg' or \
             'list func' for them
"
    );
    assert!(inferior.sets.is_empty());
}

#[test]
fn a_stop_can_jump_the_stopped_thread_and_run_on() {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.code = true;
    inferior.runs = vec![
        RunStop::Breakpoints(vec![(0, 0x1004)]),
        RunStop::Exited(Exit::Status(0)),
    ];
    let (out, err) = drive(
        &mut inferior,
        "bp next_seed j 0x1010 x0=7; run
run
q
",
        false,
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed. Each stop runs 'j \
                 0x1010 x0=7; run'.
{PROMPT}--[BP] T: 0 P: 0x0000000000001004 F: (next_seed) _mfb_fn_next_5Fseed
Moved thread 0 to 0x0000000000001010 in main; set x0 = 0x0000000000000007 (7).
--[EX] Program exit (0)
{PROMPT}Killed process 42.
"
        )
    );
}
