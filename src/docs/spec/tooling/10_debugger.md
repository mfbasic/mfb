# Debugger

`mfb dbg` is the compiler's native debugger. It is a property of the compiler's
**host**, not of a compiled program: an `mfb` built for macOS debugs macOS
programs and nothing else, and there is no cross-debugging. Today it starts or
attaches to a program, pauses it, lists its functions, threads, registers and
arenas, single-steps one thread or all of them, runs it to software
breakpoints, disassembles it, and unwinds its threads' calls; it is the seam later debugger features build
on.

The developer-facing guide is `./mfb man debug`.

## Invocation

```text
mfb dbg <binary> [-- <args>...]
mfb dbg <pid>
```

The debugger's own arguments end at the first `--`; everything after it is
passed to a launched program verbatim, including a second `--` or a `--help`.
A `--help`/`-h` before the first `--` prints `DBG_HELP` and exits `0`.
[[src/cli/dispatch.rs:dispatch]] [[src/cli/help.rs:DBG_HELP]]

The single operand is classified in this order:[[src/cli/dbg.rs:parse]]

1. A path that exists is a `<binary>` to launch. An existing file named only by
   digits is therefore a binary, not a PID.
2. An operand starting with `-` is an unknown option (usage error).
3. An operand of ASCII digits is a `<pid>`. `0`, and a value that does not fit
   `u32`, fail with `no process with PID <operand>`. A `<pid>` followed by `--`
   is a usage error.
4. Anything else fails with `'<operand>' is neither a file nor a process ID`.

Zero operands, or more than one before `--`, is a usage error.

## Exit codes

| Code | When |
| --- | --- |
| 0 | the session ended by `quit`/`q` or end of input; also `--help` |
| 1 | the host has no backend; the program is unreadable, not MFBASIC, or not native; the process does not exist; launch or attach failed; the session could not resume a detached program or write its output |
| 2 | usage: no operand, extra operands, an unknown option, program arguments after a `<pid>` |

Errors print as `error: <message>` on stderr through the shared command-error
renderer.[[src/cli/mod.rs:dispatch_command_error]]

## Host support

`host_backend` answers the platform backend compiled into this `mfb`, or none.
Only macOS has one; on every other host, after argument parsing succeeds,
`mfb dbg` fails with `mfb dbg is not supported on <std::env::consts::OS>`.
[[src/debugger/mod.rs:host_backend]] A new platform implements the `Backend`
trait (`executable_of`, `launch`, `attach`) and an `Inferior` (`pid`, `end`,
`functions`, `data_symbol`, `read_u64`, `read_bytes`, `threads`, `registers`, `step`,
`restart`, `entry`, `insert_breakpoint`, `remove_breakpoint`, `run`, `read_code`,
`function_at`, `library_functions`, `data_symbol_at`) and is returned from
`host_backend`. Symbol classification, the reading of the runtime's thread
table and arena registry (`src/debugger/runtime.rs`), breakpoint resolution and
pending placement (`src/debugger/breakpoints.rs`), the listings' format, the
prompt and the CLI are platform-neutral.
[[src/debugger/mod.rs:Backend]] [[src/debugger/mod.rs:Inferior]]

## The debuggability check

Before any process is touched — before a launch, and between resolving a PID's
executable and attaching — the file is read and checked by `check_debuggable`:
[[src/debugger/mod.rs:check_debuggable]]

- It must carry the MFBASIC provenance note, found by the same reader as
  `mfb info` (`./mfb spec linker provenance-marker`); otherwise
  `'<path>' is not an MFBASIC executable`.[[src/os/inspect.rs:inspect]]
- Its container format and architecture must equal the host's: Mach-O on
  macOS, PE on Windows, ELF elsewhere; the architecture spelled as `inspect`
  spells it (`x86_64` → `x86-64`). Otherwise
  `'<path>' is built for <format> <arch>; this debugger runs <format> <arch> programs`.
  [[src/debugger/mod.rs:host_image]]

A file whose loaded libraries include `AppKit.framework` is an **app**
(`Program::app`): its initial thread runs the window system and `main` runs on
a thread of its own (`./mfb spec app macos-runtime`).

For a `<pid>`, the executable is the one the kernel reports for the process
(`proc_pidpath` on macOS); a process that does not exist fails with
`no process with PID <pid>`.[[src/debugger/macos.rs:MacOs]] Every path printed
by the debugger passes through the terminal sanitizer.[[src/debugger/mod.rs:shown]]

## macOS backend

[[src/debugger/macos.rs:MacOs]]

- **Launch.** `posix_spawn` with `POSIX_SPAWN_START_SUSPENDED` (`0x0080`), the
  operand as `argv[0]`, the arguments after `--`, and this process's
  environment and stdio. The kernel creates the task suspended, so no
  instruction runs before the prompt — not even `dyld`'s. The launched program
  is a child of `mfb` in a process group of its own (`POSIX_SPAWN_SETPGROUP`,
  group 0), so a Ctrl-C typed at the terminal reaches `mfb` and not the
  program; the program is not the terminal's foreground group, so it cannot
  read the terminal while it runs.
- **Attach.** `task_suspend` on the task, which stops every thread at once.
- **Task port.** Both obtain the program's task port with `task_for_pid`.
  macOS grants it only to a caller signed with the
  `com.apple.security.cs.debugger` entitlement or running as root, and only for
  a target without the hardened runtime; `mfb build`'s ad-hoc Mach-O signature
  does not enable the hardened runtime. On refusal the error names the Mach
  error and the signing command `sh scripts/sign-debugger.sh <current_exe>`,
  and a launched child is killed and reaped.[[src/debugger/macos.rs:task_port]]
- **Ending.** `quit` on a launched program first reaps it if it has already
  exited (`AlreadyExited`), otherwise sends `SIGKILL` and reaps it (`Killed`). On
  an attached program it calls `task_resume` (`Detached`); if that fails and the
  process is gone, `AlreadyExited`. The task port's send right is released
  either way. An `Inferior` dropped without `end` ends itself.
  [[src/debugger/macos.rs:Task]]
- **Symbols.** `list func` reads the executable's `LC_SYMTAB` from the file
  (`super::macho::read`): every `nlist_64` that is not a stab and is either
  `N_SECT` in a section with `S_ATTR_PURE_INSTRUCTIONS` or
  `S_ATTR_SOME_INSTRUCTIONS` (a function; data such as `_mfb_str_*` is
  skipped), or `N_UNDF|N_EXT` (an import). Entries keep symbol-table order and
  repeats: each import record has its own undefined entry
  (`./mfb spec linker import-selection`).[[src/debugger/macho.rs:read]]
  After them come the functions of every library the program loaded while
  running, in address order: a library is one when its `LC_ID_DYLIB` install
  name is not reached from the executable's dependencies (`LC_LOAD_DYLIB`,
  `LC_LOAD_WEAK_DYLIB`, `LC_REEXPORT_DYLIB`, `LC_LOAD_UPWARD_DYLIB`, read from
  its file by `crate::os::inspect`) through the loaded libraries' own (the same
  commands, read from each header in memory), or it has no install name (a
  bundle). Its functions are the `N_EXT` `N_SECT` symbols of its `LC_SYMTAB` in a
  section with `S_ATTR_PURE_INSTRUCTIONS` or `S_ATTR_SOME_INSTRUCTIONS`. A
  framework that libSystem's own dependencies reach (CoreFoundation, Security,
  Network) is loaded with the program, so a `dlopen` of it names no new
  library. [[src/debugger/macos.rs:loaded_functions]]
- **Addresses.** A defined function's address is its `n_value` plus the load
  offset: the live address of the first readable region (walking
  `mach_vm_region`) that starts with an `MH_MAGIC_64` header of filetype
  `MH_EXECUTE`, minus `__TEXT`'s `vmaddr`. The kernel maps the executable before
  `dyld` runs, so this holds at the first instruction. An import's address is
  read from its `S_NON_LAZY_SYMBOL_POINTERS`/`S_LAZY_SYMBOL_POINTERS` slot, found
  through `LC_DYSYMTAB`'s indirect symbol table: a slot whose live bytes still
  equal the file's is unbound (`-`), otherwise its 64-bit value is the bound
  function. A launched program has run no `dyld` code, so every import is
  unbound there. [[src/debugger/macos.rs:Task]]
- **Data symbols.** `data_symbol` answers a symbol `LC_SYMTAB` defines in a
  section without instructions, plus the load offset.
- **Threads.** `threads` lists `task_threads` in the kernel's order (index 0 is
  the initial thread). Each thread's `id` and `pthread_t` come from
  `thread_info(THREAD_IDENTIFIER_INFO)`, its name from
  `proc_pidinfo(PROC_PIDTHREADID64INFO)`'s `pth_name`, `pc`, `sp` and the arena
  register (`x19`) from `ARM_THREAD_STATE64`, and its stack from the
  `mach_vm_region` holding `sp`.
- **Registers.** `registers` reads one thread with `thread_get_state` flavors
  `ARM_THREAD_STATE64` and `ARM_NEON_STATE64`.
  [[src/debugger/macos.rs:arm64_registers]]
- **Step.** `step` takes a set of threads and a round count. It holds every
  thread outside the set with `thread_suspend`, installs an exception port for
  `EXC_BAD_ACCESS`, `EXC_BAD_INSTRUCTION`, `EXC_ARITHMETIC` and
  `EXC_BREAKPOINT` (`task_swap_exception_ports`, behavior
  `EXCEPTION_DEFAULT | MACH_EXCEPTION_CODES`), and per round: sets
  `MDSCR_EL1.SS` through `ARM_DEBUG_STATE64` on every thread still stepping,
  `task_resume`s, collects exceptions until each of those threads has raised
  one or the wait gives up, `task_suspend`s, clears `SS`, takes any
  exception that arrived late, and answers every exception `KERN_SUCCESS`. A
  thread that has raised its exception waits for the answer, so each runs one
  instruction a round. A single-step and a `brk` both arrive as
  `EXC_BREAKPOINT` with code `EXC_ARM_BREAKPOINT`; a `brk` is the one whose
  instruction is `brk` and whose `pc` did not move. A thread that raised
  another exception (`StepStop::Exception`; the faulting instruction is retried
  when the program next runs) or none in time (`Waiting`) is held with
  `thread_suspend` for the remaining rounds. The wait runs in 2000 ms slices
  (`STEP_TIMEOUT_MS`); after each, it gives up only when every thread still
  without an exception is blocked in a system call — its `pc` just past an
  `svc` — or 30 s (`STEP_LIMIT_MS`) have passed in all. A thread executing an
  ordinary instruction that is merely slow (its code paging in, a loaded
  machine) is waited for, not reported as waiting. A
  `task_suspend` that fails ends every stepping thread as `Exited`. Afterwards
  every hold is given back and the program's own exception handlers put back.
  [[src/debugger/macos.rs:ExceptionPort]] [[src/debugger/macos.rs:Task]]
- **Breakpoints.** `insert_breakpoint` saves the 4 bytes at the address and
  writes `brk #0` (`0xd4200000`): the page (`getpagesize()`, aligned down) is
  made `VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY` with `mach_vm_protect` —
  a private copy, so a shared-cache library page changes for this process
  only — written with `mach_vm_write`, and set back to
  `VM_PROT_READ | VM_PROT_EXECUTE`. `remove_breakpoint` writes the saved bytes
  back the same way. Neither needs ptrace: the debugger entitlement's task
  port suffices, and the program is not killed for the changed page. `step`
  lifts every breakpoint for its duration and writes them again afterwards, so
  a stepped thread runs the original instructions. Ending an attached session
  lifts them before `task_resume`, so the program never meets a `brk` with no
  debugger. `entry` is `LC_MAIN`'s `entryoff` plus `__TEXT`'s address and the
  load offset. [[src/debugger/macos.rs:Task]]
- **Run.** `run` first steps every thread whose `pc` is on a breakpoint one
  instruction (so it does not stop on the breakpoint it is paused at), then
  installs the exception port, clears the interrupt flag, and `task_resume`s.
  It waits in 100 ms slices for an exception, a set interrupt flag, or the end
  of the program, then `task_suspend`s. Exceptions are classified: every
  `EXC_BREAKPOINT` whose second code (the `brk`'s address) is a breakpoint's
  address is a hit (`RunStop::Breakpoints`, every such thread); otherwise the
  first exception is `RunStop::Exception`. Every exception is answered
  `KERN_SUCCESS`, which leaves a thread on its `brk` (`pc` is the `brk`'s
  address) or retries a faulting instruction. A launched program has ended
  when `waitpid(WNOHANG)` reaps it (exit status or signal); an attached one
  when `pid_for_task` fails or `kill(pid, 0)` finds no process
  (`Exit::Unknown`). After that the session refuses every command but `help`,
  `list`, `list breakpoint`, a bare `list reg` and `quit`, and `quit` reports
  `AlreadyExited`.
- **Libraries.** `function_at` and `library_functions` read the loader's image
  list (`task_info(TASK_DYLD_INFO)` → `dyld_all_image_infos.infoArray`), and
  for each image but the executable its Mach-O header in memory: `__TEXT`'s
  range and sections, `__LINKEDIT` (whose `vmaddr` + slide − `fileoff` locates
  every table offset), `LC_FUNCTION_STARTS`, `LC_SYMTAB` and `LC_DYSYMTAB`.
  Function starts are ULEB128 deltas from `__TEXT`'s start; a library function
  runs to the next start and never past its section. Names come from the
  `N_SECT` symbols, whose strings (in the shared cache, one pool shared by every
  library) are read in runs of nearby offsets. An address in a
  `S_SYMBOL_STUBS` section is the stub named through the indirect symbol table
  (`symbol stub for: <symbol>`). The list is re-read when its count changes;
  each library's starts and symbols are read on first use.
  [[src/debugger/macos.rs:Library]]
- **Restart.** `restart` refuses an attached program. For a launched one it
  kills and reaps the process (unless it has already ended), releases its task
  port, spawns the program again with the same arguments exactly as `launch`
  does, and forgets everything about the old process: the load offset, the
  breakpoints written into it, its exit, its libraries, and the executable's
  symbols (read again, so a rebuilt file is seen).
- **Ctrl-C.** For the session's length `mfb` handles `SIGINT` with a handler
  that only sets an atomic flag (and puts the previous handler back when the
  session ends). A `run` that sees the flag suspends the task and answers
  `RunStop::Interrupted`; at the prompt the signal does nothing.
- **A debugger that dies.** The kernel drops a `task_suspend` hold when the
  holding process exits, so an attached program resumes; a program still
  suspended at spawn does not outlive its parent. Neither case leaves a program
  frozen. A launched program that has been stepped is held by `task_suspend`
  from then on, so it too resumes. (Observed on macOS 15; not a documented
  Apple guarantee.)

`scripts/sign-debugger.sh` ad-hoc signs a given `mfb` (default
`target/debug/mfb` and `target/release/mfb`) with that one entitlement. A cargo
build relinks `mfb` and drops the signature, so it is re-run after each build.

## The prompt

[[src/debugger/prompt.rs:run]]

The launch banner is
`Launched <path> as process <pid>; paused before its first instruction.`; the
attach banner is `Attached to process <pid> (<executable>); paused.`, both on
stdout.[[src/cli/dbg.rs:session]] When the program's symbols read and name no
function it defines — a build without `--debug` carries only its imports — two
more lines follow:[[src/cli/dbg.rs:debug_build_note]]

```text
note: this is not a debug build, so the program's function names are missing.
      Rebuild it with: mfb build --debug -O0 <project>
```

Then, repeatedly:

1. `DBG> ` is written to stdout and flushed, with no newline.
2. One line is read from stdin. When stdin is not a terminal, the line (less
   its line ending) and a newline are echoed to stdout.
3. The line is split on whitespace. An empty line does nothing.

| Command | Effect |
| --- | --- |
| `help`, `h` | prints the `HELP` listing to stdout |
| `list`, `l` | prints the `LIST_USAGE` block |
| `list func` | prints the function listing |
| `list thread` | prints the thread listing |
| `list reg [<id>]` | prints the `LIST_REG_USAGE` block |
| `list reg [<id>] <what>...` | prints thread `<id>`'s registers (default 0) of each group or name in `<what>` |
| `list arena` | prints the arena listing |
| `show mem <where> [<size>]` | dumps `<size>` bytes (default 64) of memory |
| `show type` | prints the `TYPE_USAGE` block |
| `show type <type> <where> [...]` | prints the arena value at `<where>` read as `<type>` |
| `show arena [<id>]` | prints arena `<id>`'s state and block headers (default 0) |
| `show arena <id> <block> [<offset>] [<size>]` | dumps `<size>` bytes (default 1024) of a block from `<offset>` (default 0) |
| `step [<count>] [<thread>]`, `s …` | runs thread `<thread>` (default 0) `<count>` instructions (default 1) |
| `step-all [<count>]`, `sa …` | runs every thread `<count>` rounds of one instruction (default 1) |
| `breakpoint <target>`, `bp …` | sets breakpoints at `<target>` |
| `breakpoint r <target>`, `bp r …` | removes breakpoints at `<target>` |
| `list breakpoint` | prints the breakpoint listing |
| `run`, `r` | runs the program to the next stop |
| `finish`, `f` | runs the current thread until its function returns |
| `backtrace [<count>] [<id>]`, `bt …` | prints thread `<id>`'s frames (default the current thread), the innermost `<count>` (default all) |
| `backtrace all`, `bt all` | prints every thread's frames |
| `restart`, `rs` | starts a launched program again |
| `disassemble`, `dis` | disassembles the current function |
| `dis <count>` | disassembles `<count>` instructions from the current `pc`; a negative count, that many before it and then the `pc` |
| `dis <where> [<count>]` | disassembles from an address or register (either `±<offset>`): to the end of its function, or `<count>` as above |
| `dis <function> [<count>]` | disassembles each function `<function>` names, or `<count>` from its start as above |
| `quit`, `q` | ends the session |

`list` may be spelled `l` in every form. Commands are case-sensitive.
[[src/debugger/prompt.rs:parse]] Refusals, each printed as `error: <text>`:

| Input | Text |
| --- | --- |
| an unknown command | `unknown command '<name>'; type 'help' for the list of commands` |
| `help` or `quit` with arguments | `'<name>' takes no arguments` |
| `list <what>` for an unknown `<what>` | `cannot list '<what>'; type 'list' for what can be listed` |
| `list func`/`thread`/`arena` with an argument, `list reg` with two | `too many arguments to '<name> <what>'` |
| `step <count> <id>` where `<id>` is not a `usize` | `'<id>' is not a thread id; type 'list thread' for the threads` |
| `list reg` naming a register the thread does not have | `no register '<name>'; type 'list reg' for the groups` |
| `show mem` with no location | `'show mem' needs an address or a register: show mem <where> [<size>]` |
| `show mem` with a location that is neither `0x…` nor `<register>[±<n>]` | `'<where>' is not an address or a register` |
| `show mem` with a size of 0 or over 65536 | `cannot show <size> bytes; give 1 to 65536` |
| `show mem` at a vector register | `'<name>' is a vector register, not an address` |
| `show type <type>` with no location | `'show type <type>' needs an address or a register: show type <type> <where>` |
| `show type <word>` for a `<word>` that is no type | `cannot show type '<word>'; the types are: int, float, fixed, money, bool, byte, scalar, string, list, set, map, resource` |
| a scalar, `string` or `resource` view with more than a location | `too many arguments to 'show type <type>'` |
| a `list`/`set` span or type that reads as neither | `'<word>' is not an index or a range (3, 0..10, 2..) or a type` |
| a `map` argument but `keys`, `values`, an index or `get` | `'<word>' is not an index, 'keys', 'values' or 'get <key>'` |
| `finish` or `restart` with arguments | `'<name>' takes no arguments` |
| `restart` of an attached program | `restart needs a program the debugger launched; this one was attached to` |
| `finish` where no known function holds the `pc` | `cannot finish: no known function holds 0x…` |
| `list reg <id>` or `step <count> <id>` past the last thread | `no thread <id>: the program has <n> thread(s); type 'list thread' for them` |
| a `step`/`step-all` count that is a negative integer | `cannot step <count> instructions: the count must not be negative` |
| a `step`/`step-all` count otherwise not a `u64` | `'<count>' is not a number of instructions` |
| `step` with three arguments | `'<name>' takes at most two arguments` |
| `step-all` with two arguments | `'<name>' takes at most one argument` |
| `breakpoint` or `breakpoint r` with no target | `'<name>' needs an address (0x…) or a function name` |
| `breakpoint` with more than `r <target>` | `too many arguments to '<name>'` |
| a target starting `0x` that is not hex | `'<target>' is not an address` |
| `run` with arguments | `'<name>' takes no arguments` |
| a `backtrace` count that is not a `usize` of 1 or more | `'<count>' is not a number of frames (1 or more), or 'all'` |
| `backtrace` with three arguments | `'<name>' takes at most two arguments` |
| `backtrace <count> <id>` past the last thread | `no thread <id>: the program has <n> thread(s); type 'list thread' for them` |
| a `dis` count of 0 or not an `i64` | `cannot show <count> instructions; give a count other than 0 (negative counts back)` |
| `dis` with three arguments | `'<name>' takes at most two arguments` |
| `dis` at an address not a multiple of 4 | `0x<address> is not the start of an instruction: they start every 4 bytes` |
| `dis` counting back past address 0 | `there are not <n> instructions before 0x<address>` |
| an address `0x<hex>±<offset>` below 0 or past `u64::MAX` | `'<word>' is below address 0` / `'<word>' is past the last address` |
| `dis <function>` naming nothing | `no function is named '<function>'; type 'list func' for the functions` |
| `dis <function>` naming only an unbound import | `'<function>' is not loaded yet; run the program until it has loaded its libraries` |
| `show` alone | `'show' needs what to show: show mem <where> [<size>], show type <type> <where>, or show arena [<id>]` |
| `show <what>` but `mem`, `type` or `arena` | `cannot show '<what>'; 'show mem', 'show type' and 'show arena' are what can be shown` |
| `show arena` with more than four arguments | `too many arguments to 'show arena'` |
| an arena id not a `usize` | `'<id>' is not an arena id; type 'list arena' for the arenas` |
| a block number not a `usize` | `'<block>' is not a block number` |
| an offset or size not a `u64` (decimal, or hex after `0x`) | `'<word>' is not a number` |
| any command but `help`, `list`, `list breakpoint`, a bare `list reg`, `restart`, `quit` after the program ended | `the program has exited; type 'quit' to end the session` |

Every listing below is laid out by `prompt::table`: a header line naming its
columns, then one line per row; the columns are one space apart and each but
the last is left-aligned to its widest entry, header included, counted in
characters. A listing with no rows prints no header.
[[src/debugger/prompt.rs:table]]

A listing or step the backend cannot produce prints `error: <reason>`. All go
to stderr after stdout is flushed; the prompt continues. End of input writes a
newline and ends the session as `quit` does.

### `list func`

One line per symbol `Inferior::functions` answers, except that a symbol
repeated at the same address (an import's repeated records, or the same
unbound import) is shown once; the same name at two addresses is shown twice:
[[src/debugger/prompt.rs:function_lines]]

```text
Type Address Symbol Written name
<type> <address> <symbol> <written name>

<type>  <legend>
```

- `<type>` is `MU`, `MB`, `MR`, `SL` or `SD`, and lines are grouped in that
  order; within a group they are in address order with an unbound import last,
  and ties keep symbol-table order.
- `<address>` is `0x` and 16 lowercase hex digits, or `-` for an unbound import.
- `<symbol>` is the symbol as the executable spells it.
- `<written name>` is the name the developer wrote, or `-`.
- After the table, an empty line and one legend line per type the listing
  shows, in group order: the code, two spaces, and `Origin::legend` —
  `MU  MFB User: code you wrote, in your program or a package it imports`,
  `MB  MFB Builtin: a built-in package's code; a #name is internal to the
  package (you can break on it, not call it)`, `MR  MFB Runtime: the MFBASIC
  runtime every program carries`, `SL  System Linked: a system library
  function the program imports (via the linker)`, `SD  System Dynamic: a
  function in a library the program loaded while running (via the dynamic
  loader)`.
  [[src/debugger/names.rs:Origin]]

Classification and the written name come from the symbol alone
(`src/debugger/names.rs:classify_defined`), inverting
`src/target/shared/nir/symbols.rs:function_symbol` and, for a runtime helper,
`src/target/shared/runtime/mod.rs:symbol_for_call` over the helper catalog
(`supported_helper_specs`): [[src/debugger/names.rs:classify_defined]]
[[src/debugger/names.rs:package_member]]
[[src/debugger/names.rs:builtin_name]]

| Symbol | Type | Written name |
| --- | --- | --- |
| `_mfb_fn_<escaped>` | `MU` | `<escaped>` with each `_XX` (two uppercase hex digits) decoded to its byte, then reduced as below |
| `_mfb_ifn_<escaped>` decoding to `<16 hex>$<name>` (a file-`PRIVATE` function) | `MU` | `<name>`, reduced as below |
| `_mfb_ifn_<escaped>` decoding to `<package>_<name>` (less any `$…`), `<package>` a registry package's import name: a built-in package's MFBASIC function, which its source spells `__<package>_<name>` | `MB` | `<package>::<name>` when the package registers a member `<name>` that is not `internal_only`; else `<package>::#<name>` (`#` is `crate::internal_name::INTERNAL_SIGIL`): `_mfb_ifn_datetime_5FnormInstant` is `datetime::#normInstant` |
| `_mfb_rt_<family>_<call>` that `symbol_for_call` makes from a catalog spec whose `call` is a registry package member (`registry().owning_package` answers) | `MR` | `call` with its first `.` written `::`: `_mfb_rt_io_io_print` is `io::print` |
| any other defined symbol (other `_mfb_ifn_*`, other `_mfb_rt_*`, `_mfb_arena_*`, `_mfb_debug_*`, `_main`, …) | `MR` | `-` |
| an import | `SL` | the symbol less one leading `_` |
| a function of a library loaded while the program ran | `SD` | the symbol less one leading `_` |

A trailing `$own<hex>` (a hand-over copy; the mask is not escaped) is dropped
before decoding. The decoded IR name is reduced: `<16 hex>.<package>.<name>`
becomes `<package>::<name>`; `$lambda<N>` becomes `lambda`;
`$default$<function>$<index>` becomes `<function>`; otherwise the text before
the first `$` (an overload or generic instance), or the whole name when that is
empty. A `_` not followed by two uppercase hex digits is kept literally.

### `list thread`

One line per thread, in the backend's order, `<id> <name> <pc> <task>`, under
the header `ID Name PC Task`:
[[src/debugger/prompt.rs:thread_lines]]

- `<id>` is the thread's index, the id `list reg` and `list arena` use.
- `<name>` is the name the thread gave itself; else the function holding its
  `pc` (as `<written name or symbol>+0x<offset>`, the offset omitted when 0),
  for a function the executable defines; else `thread-0x<system id>`.
- `<pc>` is `0x` and 16 lowercase hex digits.
- `<task>` is the thread's role, decided by `runtime::roles`:
  [[src/debugger/runtime.rs:roles]]

| Role | Thread |
| --- | --- |
| `App` | thread 0 of an app |
| `MFB Main` | thread 0 of a console program; in an app, the thread whose stack region holds the address in `_mfb_rt_main_arena`, or — when the program has no such symbol — the one thread whose `x19` points into its own stack at an arena state with `workerThread` 0 and a `blockHead` block whose `blockSize` is a non-zero multiple of `ARENA_DEFAULT_BLOCK_SIZE` (more than one such thread: none) |
| `User Thread` | a thread whose `pthread_t` is the `CONTROL_OFFSET_OS_HANDLE` of a used `_mfb_rt_thread_table` slot, or whose `x19` holds a worker arena state whose control block's `CONTROL_OFFSET_WORKER_ARENA` is that same state |
| `Other` | everything else: the Canvas graphics thread, system threads, a `process::detach` reaper |

The register tests read `x19` as the thread left it, which is MFBASIC's arena
state only while the thread is in MFBASIC code; a build without `--debug` names
no globals, so an app's main thread paused in a system call is `Other` there.

### `list reg`

The arguments are split at whitespace and at commas, so `x0, x1,x2` names
three registers. A first argument of all digits is the thread id `<id>`
(default 0). With no other argument the command prints the `LIST_REG_USAGE`
block, even after the program has ended; otherwise every other argument picks
registers, in the order given: `gpr`, `simd` and `sys` pick every register of
that `RegisterGroup` in the backend's order, and any other word picks the one
register of that name. [[src/debugger/prompt.rs:RegisterPick]]

One line per register picked, `<register> 0x<hex> (<decimal>)`, under the
header `Register Hex Decimal`: the value as `0x` plus 16 lowercase hex digits
(64-bit) or 32 (128-bit), and the same value in decimal, signed for a 64-bit
register (as MFBASIC's `Integer`) and unsigned for a 128-bit one.
[[src/debugger/prompt.rs:register_lines]]

On AArch64 every register of the thread state is read
(`src/debugger/macos.rs:arm64_registers`):

| Group | Registers |
| --- | --- |
| `gpr` (`General`) | `x0`–`x28`, `fp` (x29), `lr` (x30) |
| `sys` (`System`) | `sp`, `pc`, `nzcv`, `fpsr`, `fpcr` |
| `simd` (`Simd`) | `v0`–`v31` |

`nzcv` is CPSR with every bit but 31–28 cleared, as the `NZCV` system register
reads; `fpsr` and `fpcr` are the 32-bit words of `ARM_NEON_STATE64`.

### `list arena`

One line per live arena in the `--debug` registry,
`<id> <thread id> <address> <size in KB>`, under the header
`ID Thread Address Size (KB)`: [[src/debugger/runtime.rs:arenas]]
[[src/debugger/prompt.rs:arena_lines]]

- `<id>` is the registry slot (`./mfb spec tooling debug-report`); a slot whose
  state pointer is 0 (a joined worker) is skipped.
- `<thread id>` is the thread that allocates from it: for kind 0 (main) the
  `MFB Main` thread; kind 1 (worker) the thread whose `pthread_t` its control
  block (`ArenaState.workerThread`) records; kind 2 (graphics) the thread whose
  `pthread_t` is `_mfb_rt_canvas_graphics`'s `GRAPHICS_OFFSET_TID`; for any of
  these with no match, a thread whose `x19` is the state. Kind 3 (a thread
  block arena) is `-`.
- `<address>` is the arena state's address.
- `<size in KB>` is the sum of `blockSize` over the chain from `blockHead`
  through each `prevBlock` (`./mfb spec memory arenas`), divided by 1024 and
  rounded up.

A program with no `_mfb_rt_debug_arena_base` symbol (no `--debug`) answers
`error: listing arenas needs a debug build; rebuild the program with: mfb build
--debug -O0 <project>`; one whose registry is not yet mapped prints
`no arenas yet: the program's runtime has not started`.

### `step`

Runs thread `<t>` `<count>` instructions through `Inferior::step`, every other
thread held, then prints one line, where `<place>` is `0x<pc>` and, inside a
function the executable defines, ` in <function>` as `list thread` spells it:
[[src/debugger/prompt.rs:step]]

| Outcome | Line |
| --- | --- |
| `<count>` is 0 (nothing runs) | `Thread <t> is at <place>.` |
| every instruction ran | `Stepped <n> instruction(s); thread <t> is at <place>.` |
| an exception | `Stopped after <n> instruction(s) by <exception>; thread <t> is at <place>.` |
| a system call that waits | `Stopped after <n> instruction(s): thread <t> is waiting in a system call at <place>.` |
| the program ended | `The program exited after <n> instruction(s).` |

### `step-all`

Runs every thread `list thread` shows in `<count>` rounds through one
`Inferior::step`, then prints one line per thread in id order:
[[src/debugger/prompt.rs:step_all]]

| Outcome | Line |
| --- | --- |
| `<count>` is 0 (nothing runs) | `Thread <t> is at <place>.` |
| every round ran | `Thread <t>: stepped <n> instruction(s); at <place>.` |
| an exception | `Thread <t>: stopped after <n> instruction(s) by <exception>; at <place>.` |
| a system call that waits | `Thread <t>: stopped after <n> instruction(s), waiting in a system call; at <place>.` |
| the thread is gone afterwards | `Thread <t> ended during the step.` |

A program that ended prints only `The program exited after <r> round(s) of
stepping.`, `<r>` being the most instructions any thread took. A thread the
program starts during the step is not stepped and runs until the round ends.

`<exception>` is `a bad memory access at 0x<address>`, `an illegal
instruction`, `an arithmetic exception`, `a breakpoint instruction`, or
`exception <n>`.

### `show mem`

[[src/debugger/prompt.rs:show_memory]] `show mem <where> [<size>]` reads
`<size>` bytes (default 64, at most 65536; decimal or `0x` hex) at `<where>`:
an address (`0x…`), or a register of the current thread, either with an
optional offset (`0x1000-0x10`, `sp`, `sp+0x18`, `x19-16`; a 64-bit register
only). [[src/debugger/prompt.rs:address]] It prints
`Memory at 0x<address>:` — with ` (<register>±0x<offset> of thread <t>)` for a
register — then the bytes as `show arena`'s dump does, each line led by its
full address (`0x` + 16 digits). [[src/debugger/prompt.rs:memory_lines]]

### `show type`

[[src/debugger/values.rs:show]] `show type <type> <where> [...]` reads the value
at `<where>` (as `show mem` resolves it: an address, or a register's value plus
an offset) as `<type>`; `show type` alone prints the `TYPE_USAGE` block, even
after the program has ended. `<type>` is `int`/`integer`, `float`, `fixed`,
`money`, `bool`/`boolean`, `byte`, `scalar`, `string`, `list`, `set`, `map` or
`resource`, in any case. [[src/debugger/values.rs:parse_view]]

Every byte read must lie in the usable space (`blockAddress + 32` up to
`blockAddress + blockSize`) of one block of one arena in the `--debug` registry
(`Region`); without a registry the command fails with `'show type' reads values
in the arena, which needs a debug build; …`, and before the runtime has started
with `no arenas yet: …`. An address in no block is `0x<address> is not in an
arena block, so it holds no <Type>; 'show mem' reads any memory`; one whose
bytes run past their block is `not a <Type> at 0x<address>: its <n> bytes run
past the end of their arena block`. Layouts are `./mfb spec memory
heap-values`, `scalar-storage` and `collections`.

| Type | Bytes | Prints |
| --- | --- | --- |
| `int` | 8 | `Value: 0x<16 hex> (<signed decimal>)` |
| `float` | 8 | `Value: 0x<16 hex> (<f64, as Rust's Debug>)` |
| `fixed` | 8 | `Value: 0x<16 hex> (<as toString prints it: 2 places, rounded half away from zero>)` |
| `money` | 8 | `Value: 0x<16 hex> (<raw / 100000, 5 places>)` |
| `bool` | 1 | `Value: 0x00 (FALSE)` or `0x01 (TRUE)`; any other byte fails |
| `byte` | 1 | `Value: 0x<2 hex> (<decimal>)` |
| `scalar` | 4 | `Value: 0x<8 hex> (U+<hex> '<char>')`; a non-codepoint fails |
| `string` | 8 + length + 1 | `Length: <n> Value: "<text>"` |

A `string`'s length word, its bytes and its NUL must lie in the block
(`not a String … its length, <n>, runs past the end of its arena block`), its
bytes must be UTF-8 (`not a String … byte <i> of its value, 0x<b>, is not
UTF-8`), and at most 65536 bytes are shown, then ` (the first 65536 bytes)`. Text
is escaped as `char::escape_debug` does ([[src/debugger/values.rs:quoted]]).

`list`, `set` and `map` read a 40-byte `CollectionHeader` and check it: the kind
byte (0 or 2 for `list`, 3 for `set`, 1 for `map`; else `not a <Type> … its kind
byte is <k>, a <Kind>` or `…, which no collection has`), the type codes (a list's
key type 0 and a known value code; a set's known key code and Boolean value; a
map's known codes), `count <= capacity`, `dataLength <= dataCapacity`, a kind-2
list's `count * width <= dataCapacity`, and that the block —
`40 + capacity*40 (none for kind 2) + dataCapacity`, the hash index excluded — is
in one arena block. With no further argument each prints one line:

```text
List Type: <type> Length: <count> Capacity: <capacity> Data: <dataLength>/<dataCapacity> bytes Block: 0x<start>-0x<end> (<size> bytes)
Set Type: <type> Length: <count - dead> Removed: <dead> Capacity: … Data: … Block: …
Map Keys: <count - dead> Key Type: <type> Value Type: <type> Removed: <dead> Capacity: … Data: … Block: …
```

`<type>` names a runtime type code (code 22 is `object (Set, record, enum,
resource or function)`). `list|set <where> [<type>] <span>` prints the elements
`<span>` picks — `<i>`, `<a>..<b>` (`a < b`, `b` excluded) or `<a>..` — as a
table `Index Address Bytes Value` (`Element` for a set), skipping removed
entries; a span past the last element fails with `no element <i>: the <Type> has
<n> (numbered from 0)`. Each entry's flags must be used (1), or removed (2) in a
set or map, and its payloads within `dataLength`. `map <where> keys [<type>]`
and `values [<type>]` print every key or value the same way; `map <where> <i>`
prints `Index: <i>`, `Key: <value> at 0x<address> (<n> bytes)` and `Value: …`;
`map <where> get <key>` prints the same for the entry whose key bytes equal
`<key>` encoded as the key type stores it (the rest of the line, less one pair of
surrounding quotes; Integer decimal or `0x` hex, Float, Fixed and Money decimal,
Boolean `TRUE`/`FALSE`, Byte, Scalar as one character or `U+<hex>`; codes 20–22
cannot be looked up), else `the map has no key <key>`.

A payload is decoded by its type code: Nothing as `Nothing`, a scalar as above
(its size must be the type's width), String bytes as `"<text>"`, a nested List
or Map (codes 20/21) as its header line plus ` (inline)` for a block inlined in
the data region or ` (stored at 0x…)` for an 8-byte payload holding the block's address. Code 22 needs
`<type>`: `set` (inline or pointer, as above) or `resource` (an 8-byte pointer,
shown as `resource` shows it); without one it prints `(type code 22: name it, as
set or resource, to read it)`. A `<type>` that contradicts a known code fails
with `the elements are <Type>, not <Type>`.

`resource` reads a 112-byte resource record and prints one line:
`Type: <kind> Closed: Y|N Moved: Y|N <handle>: <value> State: 0x<16 hex> Scope:
<scope>` then the kind's own fields (`File`: `Write Buffer`, `Write Filled`,
`Write Enabled`, `Read Buffer`, `Read Position`, `Read Filled`, `At EOF`; the
TLS and audio tails of the layout tables), counts in decimal and addresses in
hex. `Closed` and `Moved` are bits 0 and 1 of the word at `+16`; `<handle>` is
`Handle (fd)` in decimal for the fd-backed kinds; `<scope>` is the owner word at
`+32`: `none (not bound to a scope yet)` for 0, else the token with its depth and
token slot decoded (`((sp + token_slot) << 12) -
depth`). A tag outside the resource tag table fails with `not a resource …: its
tag, <t>, is no resource kind`.

### `show arena`

[[src/debugger/prompt.rs:show_arena]] The arena is the live registry slot
`<id>` that `list arena` shows, so like it this needs a `--debug` build
(`showing an arena needs a debug build; …`); otherwise `no arenas yet: …` or
`no arena <id>; type 'list arena' for the arenas`.

`show arena [<id>]` reads all `ARENA_STATE_SIZE` bytes of the arena state and
prints `Arena State:`, then one line per field of
`runtime::ARENA_STATE_FIELDS` — the layout of `./mfb spec memory arenas`, from
the compiler's own offset constants — as the name left-aligned in 17 columns
and the value in hex at its width (`0x` + 16 digits; `v128Slots` entries 32).
An array prints `<name>[<count>]` and `<n> non-empty`, then a line
`    [<index>]` per entry that is not all zero bytes.
[[src/debugger/prompt.rs:state_lines]] Then each block, **oldest first**
(numbered from 0: the chain from `blockHead` through `prevBlock`, reversed), as
`Arena Block (<n>):` and its `prevBlock`, `blockSize`, `usableCapacity` and
`bumpOffset`. [[src/debugger/runtime.rs:blocks]]

`show arena <id> <block> [<offset>] [<size>]` dumps block `<block>`
(numbered as above; past the last, `arena <id> has <n> block(s), numbered from
0, the oldest`). `<offset>` and `<size>` are decimal or `0x` hex; `<size>` of 0
fails with `a dump needs a size of 1 or more bytes`, and `<offset> + <size>`
past the block's `blockSize` with `offset 0x… + size 0x… runs past the end of
block <n>, whose blockSize is 0x…`. The bytes are read from the block's
address plus `<offset>` and printed under `Arena Block (<id>,<block>):` as one
line per 16 bytes: the line's offset in the block as `0x` and hex digits
grouped in fours by `_` (at least 8 digits), the bytes as two hex digits each
(a short last line padded), `|`, and each byte as its ASCII character (`.`
outside 0x20–0x7e), all space-separated. [[src/debugger/prompt.rs:dump_lines]]

### Breakpoints

[[src/debugger/breakpoints.rs:Breakpoints]] A `<target>` starting `0x` (or
`0X`) is an address in hex, perhaps `+` or `-` an offset (decimal, or hex after
`0x`), as `show mem` reads one; anything else is a name.

- **By address**: one breakpoint at that address, described by the function
  that holds it (`<symbol>+0x<offset>` and `<written>+0x<offset>`), a linked
  function bound exactly there, or `-`.
- **By name**: the function symbols spelled exactly `<target>` (as `list func`
  shows them: `_mfb_rt_print`, `_exit`); only when there are none, those whose
  written name is `<target>` (`isPrime`, `exit` — which can name both a
  developer's function and a linked one). One breakpoint per distinct address
  among them; a linked function not yet bound gives one **pending**
  breakpoint per symbol, unless another record of it is bound. No match:
  `no function is named '<target>'; type 'list func' for the functions`.

Each new breakpoint prints `Set a breakpoint at <place>.`, where `<place>` is
`0x<address>` and ` in <written name or symbol>` when it has one; a pending one
prints `Set a pending breakpoint on <symbol>: it is placed when the program
has loaded its libraries.` A target whose every breakpoint exists already
fails with `there is already a breakpoint at <address>` / `on '<target>'`.

`breakpoint r <target>` removes, by address, the breakpoint there; by name, the
breakpoints whose symbol is `<target>`, or else whose written name is. Each
prints `Removed the breakpoint at <place>.`; none fails with `there is no
breakpoint at <address>` / `on '<target>'`.

`list breakpoint` prints `<address> <symbol> <written name>` per breakpoint
under the header `Address Symbol Written name`, in address order with pending
ones last (`-` for the address); with none, `no breakpoints`.

### `run`

`run` places every pending breakpoint whose function is now bound, printing
`Placed the pending breakpoint on <symbol> at 0x<address>.` While any stays
pending and the program has not been seen at its entry point, the entry point
gets a breakpoint of the debugger's own (unless one is there): a stop at that
breakpoint alone places the pending ones and runs on, unseen. Then one line per
outcome, `<place>` as in `step`:

| Stop | Line |
| --- | --- |
| a breakpoint (one line per thread) | `Thread <t> hit a breakpoint at <place>.` |
| another exception | `Thread <t> stopped by <exception> at <place>.` |
| Ctrl-C | `Paused: thread 0 is at <place>.` |
| a launched program exited | `The program exited with status <n>.` |
| a launched program ended by a signal | `The program was ended by signal <n>.` |
| an attached program ended | `The program exited.` |

### `disassemble`

[[src/debugger/disassemble.rs:disassemble]] The **current thread** is the one
the last `run` stopped (the first breakpoint hit, or the exception's thread)
or the last `step` ran; thread 0 before either, or when it has ended.

- `dis`: the function holding the current thread's `pc`, from its start.
- `dis <count>` (digits, perhaps after `-`): a count, never 0. A positive count
  is that many instructions from the `pc`; a negative one is that many before
  it, then the `pc`'s own.
- `dis <where> [<count>]`, `<where>` starting `0x`, holding `+` or `-`, or a
  register name (`x0`–`x30`, `fp`, `lr`, `sp`, `pc`): the address it resolves to
  as `show mem` resolves one, which must be a multiple of 4. With no count, the
  listing runs from there to the end of the function holding it (or to the first
  `ret` or 64 instructions when its extent is unknown); with one, as for
  `dis <count>`. The header names the function holding that address.
- `dis <function> <count>`: `<count>` counted from each function's start, as
  for `dis <count>`.
- `dis <function>`: resolved as a breakpoint name is — symbols spelled
  `<function>` (the executable's, its imports' bound addresses, the libraries'
  via `library_functions`), else written names (the executable's functions,
  its imports less `_`, and the libraries' `_<function>`) — one listing per
  distinct start, separated by an empty line.

A function's extent is the first known of: the executable's symbol size
(`--debug` builds), its import stub's size, the library's function starts.
With none, the listing runs to the first return (`ret`, `retaa`, `retab`) or 64
instructions (`UNSIZED_LIMIT`). Code is read with `read_code`, which puts each
breakpoint's saved instruction back, so a listing never shows the `brk`.

The listing is lldb's: a header `<image>`<name>:` (`<image>`0x<start>:` when
unnamed; `0x<start>:` when no image holds it), where the name is the written
name for the developer's functions and otherwise the symbol less one leading
`_`; then per instruction `->  ` at the current `pc` or four spaces, the
address as `0x` and 16 hex digits, ` <+<offset>>:` (` <-<offset>>:` before the
function's start, when a count runs back past it) padded to the widest in the listing (omitted
when no image holds it), and the instruction. A comment follows from column 33
of the instruction text: `=<decimal>` for a `mov` of an immediate (the value it
sets, signed at the register's width, `mov w0, #-0x2 ; =-2`); for a branch,
call, `adr` or literal load (`ldr x0, 0x…`), `<+<offset>>` inside this
function, else the function at the address it names (`<name>` or
`<name> + <offset>`); for an `add #imm` whose register an earlier
`adrp` in the listing set, the data symbol at the address the pair forms, else
the function there.

Instructions are decoded by `crate::arch::aarch64::decode`, which knows every
form `crate::arch::aarch64::encode` emits and, so that code in dyld and the
system libraries reads too, the A64 base instruction set: integer, loads and
stores in every addressing mode (pairs, exclusive, ordered, LSE atomics,
`ldraa`), branches (`br`, `cbz`, `tbz`, `bc.<cond>`), system instructions
(hints, barriers, `mrs`/`msr` with LLVM's register names, `dc`/`ic`), pointer
authentication, scalar floating point (half, single, double), Advanced SIMD
vector and scalar forms, AES, SHA-1, SHA-256, SHA-512 and SHA-3. It does not
decode SVE, SME, memory tagging or the `cpy`/`set` memory operations; a word it
does not know prints as `.word  0x<word>`. Text is spelled as lldb prints it on
macOS, NEON in Apple's syntax (`fadd.2d v1, v0, v2`, `mov.16b v24, v0`,
`mov.d x8, v1[0]`). Its round-trip test encodes every emitter form and decodes
it back, and fails when the emitter gains a form it does not cover; a second
test holds words from dyld and the system libraries to lldb's text for them,
and an ignored test compares a whole corpus against lldb
(`tools/oracles/aarch64-decode`).
[[src/arch/aarch64/decode/mod.rs:decode]]

### `finish`

[[src/debugger/breakpoints.rs:Breakpoints]] `finish` runs the current thread
until the function holding its `pc` returns. It finds every return
instruction (`ret`, `retaa`, `retab`) in the function (through its extent, or
the first within 64 instructions as `dis` does, `disassemble::returns`), writes a breakpoint of its own on each that
holds none, and runs as `run` does. A stop at one of them by the current
thread with its stack pointer at or above the one it had (so a deeper,
recursive call's return does not count) removes them and steps the `ret`,
then prints
`Thread <t> returned from <function> to <place>: x0 = <value>, x1 = <value>.`
— MFBASIC returns an error code in `x0` and the value in `x1`, each shown as
`list reg` shows it. A stop at them by another thread or a deeper frame runs
on. Any other stop — a user breakpoint, an exception, Ctrl-C, the end —
removes them and is reported as `run` reports it. With no function at the
`pc`: `cannot finish: no known function holds 0x…`; with no `ret` found:
`cannot finish: no ret instruction found in the function at 0x…`.

### `backtrace`

[[src/debugger/backtrace.rs:frames]] Unwinds thread `<id>` (default the
current thread, falling back to 0 when it has ended) from its `pc`, `sp`,
`fp` and `lr`. Executables carry no unwind tables and MFBASIC code keeps no
frame-pointer chain (`./mfb spec memory native-calling-convention`), so each
frame is unwound by reading its function's code from its start to the frame's
`pc` (`state_at`), as lldb does for code without unwind tables:
[[src/debugger/backtrace.rs:state_at]]

- The state tracked is `sp - CFA` (the CFA is the `sp` on entry), where `lr`
  and the caller's `fp` are saved (offsets from the CFA), and `fp - CFA` once
  the function points `fp` at a frame record.
- `sub`/`add sp, sp, #imm{, lsl #12}` move `sp`; `str` (unsigned offset, pre-
  and post-index) and `stp` (offset, pre- and post-index) through `sp`, `fp`,
  or a register set by `add x<n>, sp, #imm` record where `x29`/`x30` went;
  `ldr`/`ldp` of `x29`/`x30` put them back in their registers; `add x29, sp,
  #imm` sets `fp`; `sub sp, x29, #imm` sets `sp` from it; any other write to
  `sp` makes it unknown, and the CFA then comes from `fp`.
- The prologue is the leading run of `sp` decrements, stores through `sp` (or a
  register set from it), `add x<n>, sp, #imm` and hints. The state it leaves is
  the body's: `ret`, `br`, or `b` outside the function's extent restores it, so
  an epilogue in the middle of a function does not leak into the code after it.

A frame's caller resumes at the saved `lr` (the thread's `lr` for the
innermost frame while it is unsaved), with `sp` the CFA and `fp` the saved one
(else unchanged); the address has bits 63–47 cleared. The function for a frame
after the first is looked up at its return address less 4, so a call that is a
function's last instruction is attributed to it. The walk ends after the
program entry `_main` (whose caller's return address is gone once it calls),
at a frame no function holds (the innermost one is unwound as a function that
has not touched the stack), when the next return address is 0, when the next
frame's `sp` is below this one's (or equal, past the innermost frame), or at
4096 frames.

Output follows lldb's `bt`: per thread,
`<mark> thread #<id>[, name = '<name>'], task = '<task>'` (`<mark>` `*` for the
current thread, else a space; `<name>` only for a thread that named itself;
`<task>` as `list thread`), then per frame
`  <mark> frame #<n>: 0x<pc> <image>`<function>[ + <offset>]`, `<mark>` `*` on
the current thread's frame 0, `<function>` named as `dis` names it
(`disassemble::display_name`), else `0x<start>`; the offset is decimal and
omitted when 0; a frame no image names prints only its address.
[[src/debugger/backtrace.rs:lines]]

### `restart`

`restart` asks the backend to start the program again (an attached program
cannot) and prints `Restarted the program as process <pid>; paused before its
first instruction.` Every breakpoint is then placed again by the function
symbol and offset it was set at — the new process is loaded at a new address —
a linked function's becoming pending until its library loads; one at an
address no function held is dropped with `Dropped the breakpoint at <place>:
no function holds it.`, and one whose function the program no longer has with
`Dropped the breakpoint on <symbol>: the program no longer has it.` The
current thread is 0 again, and the session runs on even after the program had
ended.

### Addresses

Every address the debugger prints itself — listings, stops, breakpoints,
registers, memory dumps, errors — is `0x` and 16 lowercase hex digits, so a
script can match one against another. Operands inside a disassembled
instruction keep LLVM's spelling (`b.lt 0x104864780`). Offsets and sizes
(`+0x44`, `0x2d bytes`) are not addresses and print as short as they are.

## Ending

Ending the session prints one line to stdout and exits `0`:

| `Ending` | Line |
| --- | --- |
| `Killed` | `Killed process <pid>.` |
| `Detached` | `Detached from process <pid>; it continues running.` |
| `AlreadyExited` | `Process <pid> had already exited.` |

A launched program shares the debugger's stdout and stderr. While it is paused
it writes nothing; whatever `step` or `run` lets it write interleaves with the
debugger's own output on the same streams.
