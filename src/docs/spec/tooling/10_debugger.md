# Debugger

`mfb dbg` is the compiler's native debugger. It is a property of the compiler's
**host**, not of a compiled program: an `mfb` built for macOS debugs macOS
programs and nothing else, and there is no cross-debugging. Today it starts or
attaches to a program, pauses it, lists its functions, threads, registers and
arenas, single-steps one thread or all of them, and runs it to software
breakpoints; it is the seam later debugger features build on.

The developer-facing guide is `./mfb man debug debugger`.

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
`entry`, `insert_breakpoint`, `remove_breakpoint`, `run`) and is returned from
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
  one or 2000 ms have passed, `task_suspend`s, clears `SS`, takes any
  exception that arrived late, and answers every exception `KERN_SUCCESS`. A
  thread that has raised its exception waits for the answer, so each runs one
  instruction a round. A single-step and a `brk` both arrive as
  `EXC_BREAKPOINT` with code `EXC_ARM_BREAKPOINT`; a `brk` is the one whose
  instruction is `brk` and whose `pc` did not move. A thread that raised
  another exception (`StepStop::Exception`; the faulting instruction is retried
  when the program next runs) or none in time (`Waiting`: a system call that
  blocks) is held with `thread_suspend` for the remaining rounds. A
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
  `list`, `list breakpoint` and `quit`, and `quit` reports `AlreadyExited`.
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
| `list reg [<id>]` | prints thread `<id>`'s registers (default 0) |
| `list arena` | prints the arena listing |
| `show arena [<id>]` | prints arena `<id>`'s state and block headers (default 0) |
| `show arena <id> <block> [<offset>] [<size>]` | dumps `<size>` bytes (default 1024) of a block from `<offset>` (default 0) |
| `step [<count>] [<thread>]`, `s …` | runs thread `<thread>` (default 0) `<count>` instructions (default 1) |
| `step-all [<count>]`, `sa …` | runs every thread `<count>` rounds of one instruction (default 1) |
| `breakpoint <target>`, `bp …` | sets breakpoints at `<target>` |
| `breakpoint r <target>`, `bp r …` | removes breakpoints at `<target>` |
| `list breakpoint` | prints the breakpoint listing |
| `run`, `r` | runs the program to the next stop |
| `quit`, `q` | ends the session |

`list` may be spelled `l` in every form. Commands are case-sensitive.
[[src/debugger/prompt.rs:parse]] Refusals, each printed as `error: <text>`:

| Input | Text |
| --- | --- |
| an unknown command | `unknown command '<name>'; type 'help' for the list of commands` |
| `help` or `quit` with arguments | `'<name>' takes no arguments` |
| `list <what>` for an unknown `<what>` | `cannot list '<what>'; type 'list' for what can be listed` |
| `list func`/`thread`/`arena` with an argument, `list reg` with two | `too many arguments to '<name> <what>'` |
| `list reg <id>` or `step <count> <id>` where `<id>` is not a `usize` | `'<id>' is not a thread id; type 'list thread' for the threads` |
| `list reg <id>` or `step <count> <id>` past the last thread | `no thread <id>: the program has <n> thread(s); type 'list thread' for them` |
| a `step`/`step-all` count that is a negative integer | `cannot step <count> instructions: the count must not be negative` |
| a `step`/`step-all` count otherwise not a `u64` | `'<count>' is not a number of instructions` |
| `step` with three arguments | `'<name>' takes at most two arguments` |
| `step-all` with two arguments | `'<name>' takes at most one argument` |
| `breakpoint` or `breakpoint r` with no target | `'<name>' needs an address (0x…) or a function name` |
| `breakpoint` with more than `r <target>` | `too many arguments to '<name>'` |
| a target starting `0x` that is not hex | `'<target>' is not an address` |
| `run` with arguments | `'<name>' takes no arguments` |
| `show` alone | `'show' needs what to show: show arena [<id>]` |
| `show <what>` but `arena` | `cannot show '<what>'; 'show arena' is what can be shown` |
| `show arena` with more than four arguments | `too many arguments to 'show arena'` |
| an arena id not a `usize` | `'<id>' is not an arena id; type 'list arena' for the arenas` |
| a block number not a `usize` | `'<block>' is not a block number` |
| an offset or size not a `u64` (decimal, or hex after `0x`) | `'<word>' is not a number` |
| any command but `help`, `list`, `list breakpoint`, `quit` after the program ended | `the program has exited; type 'quit' to end the session` |

A listing or step the backend cannot produce prints `error: <reason>`. All go
to stderr after stdout is flushed; the prompt continues. End of input writes a
newline and ends the session as `quit` does.

### `list func`

One line per symbol `Inferior::functions` answers, repeats included:
[[src/debugger/prompt.rs:function_lines]]

```text
<type> <address> <name> <written name>
```

- `<type>` is `MU`, `MR` or `LN`, and lines are grouped in that order; within a
  group they are in address order with an unbound import last, and ties keep
  symbol-table order.
- `<address>` is `0x` and 16 lowercase hex digits, or `-` for an unbound import,
  left-aligned in 18 columns.
- `<name>` is the symbol as the executable spells it, left-aligned to the
  longest name in the listing.
- `<written name>` is the name the developer wrote, or `-`.

Classification and the written name come from the symbol alone
(`src/debugger/names.rs:classify_defined`), inverting
`src/target/shared/nir/symbols.rs:function_symbol`: [[src/debugger/names.rs:classify_defined]]

| Symbol | Type | Written name |
| --- | --- | --- |
| `_mfb_fn_<escaped>` | `MU` | `<escaped>` with each `_XX` (two uppercase hex digits) decoded to its byte, then reduced as below |
| `_mfb_ifn_<escaped>` decoding to `<16 hex>$<name>` (a file-`PRIVATE` function) | `MU` | `<name>`, reduced as below |
| any other defined symbol (`_mfb_ifn_` built-in internals, `_mfb_rt_*`, `_mfb_arena_*`, `_mfb_debug_*`, `_main`, …) | `MR` | `-` |
| an import | `LN` | the symbol less one leading `_` |

A trailing `$own<hex>` (a hand-over copy; the mask is not escaped) is dropped
before decoding. The decoded IR name is reduced: `<16 hex>.<package>.<name>`
becomes `<package>::<name>`; `$lambda<N>` becomes `lambda`;
`$default$<function>$<index>` becomes `<function>`; otherwise the text before
the first `$` (an overload or generic instance), or the whole name when that is
empty. A `_` not followed by two uppercase hex digits is kept literally.

### `list thread`

One line per thread, in the backend's order, `<id> <name> <pc> <task>`:
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

One line per register of thread `<id>` (default 0), `<register> <value>`,
the name left-aligned to the longest and the value `0x` plus 16 lowercase hex
digits (64-bit) or 32 (128-bit). [[src/debugger/prompt.rs:register_lines]] On
AArch64 the set is the registers generated code can use
(`src/arch/aarch64/regmodel.rs`): `x0`–`x17`, `x19`–`x28`, `lr`, `sp`, `pc`,
`nzcv`, `v0`–`v31`. `x18` (platform-reserved) and `x29` (generated code keeps
no frame pointer) are omitted; `nzcv` is CPSR with every bit but 31–28
cleared, as the `NZCV` system register reads.

### `list arena`

One line per live arena in the `--debug` registry,
`<id> <thread id> <address> <size in KB>`: [[src/debugger/runtime.rs:arenas]]
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
`0X`) is an address in hex; anything else is a name.

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

`list breakpoint` prints `<address> <symbol> <written name>` per breakpoint, in
address order with pending ones last (`-` for the address), the symbol column
left-aligned to the longest; with none, `no breakpoints`.

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
