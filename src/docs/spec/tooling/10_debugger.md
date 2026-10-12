# Debugger

`mfb dbg` is the compiler's native debugger. It is a property of the compiler's
**host**, not of a compiled program: an `mfb` built for macOS debugs macOS
programs and nothing else, and there is no cross-debugging. Today it starts or
attaches to a program, pauses it, lists its functions, threads, registers and
arenas, single-steps one thread or all of them, runs it to software
breakpoints (each perhaps with a test) and hardware watchpoints (running
commands at a stop), moves a thread and sets its registers, disassembles it,
and unwinds its threads' calls; it is the seam later debugger features build
on.

The developer-facing guide is `./mfb man debug`.

## Invocation

```text
mfb dbg <binary> [-- <args>...]
mfb dbg <pid>
```

The debugger's own arguments end at the first `--`; everything after it is
passed to a launched program verbatim, including a second `--` or a `--help`.
A `--help`/`-h` before the first `--` prints `dbg_help()` — an introduction, the
prompt's `HELP` listing, and the arguments — and exits `0`.
[[src/cli/dispatch.rs:dispatch]] [[src/cli/help.rs:dbg_help]]

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
macOS has one, and Linux on AArch64; on every other host, after argument
parsing succeeds, `mfb dbg` fails with `mfb dbg is not supported on <os>
<arch>; it runs on macOS and on Linux on AArch64` (`std::env::consts`). The
disassembler and the unwinder read AArch64 code, so another architecture has
no backend. [[src/cli/dbg.rs:run_dbg_command]]
[[src/debugger/mod.rs:host_backend]] A new platform implements the `Backend`
trait (`executable_of`, `launch`, `attach`) and an `Inferior` (`pid`, `end`,
`functions`, `data_symbol`, `read_u64`, `read_bytes`, `threads`, `registers`,
`set_registers`, `step`,
`restart`, `entry`, `insert_breakpoint`, `remove_breakpoint`,
`insert_watchpoint`, `remove_watchpoint`, `run`, `read_code`, `function_at`,
`library_functions`, `data_symbol_at`) and is returned from `host_backend`.
Symbol classification, the reading of the runtime's thread table and arena
registry (`src/debugger/runtime.rs`), breakpoint resolution, pending
placement and tests, the watchpoints' change test and commands at a stop
(`src/debugger/breakpoints.rs`), locations and their values
(`src/debugger/location.rs`), the listings' format, the prompt and the CLI are
platform-neutral.
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

A file whose loaded libraries include `AppKit.framework`, or `libgtk-4.so.1`
(`GTK_LIBRARY`), is an **app** (`Program::app`): its initial thread runs the
window system and `main` runs on a thread of its own (`./mfb spec app
macos-runtime`, `./mfb spec app linux-runtime`). A Linux app is an AppImage;
it is debugged through the executable inside it (`usr/bin/<name>` of
`--appimage-extract`), or by attaching to it.

For a `<pid>`, the executable is the one the kernel reports for the process
(`proc_pidpath` on macOS, `/proc/<pid>/exe` on Linux); a process that does not exist fails with
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
  [[src/debugger/arm64.rs:registers]] `set_registers` reads
  `ARM_THREAD_STATE64`, replaces the words it names (`x0`–`x28`, `fp`, `lr`,
  `sp`, `pc`; for `nzcv`, CPSR's bits 31–28, `arm64::NZCV_BITS`, the rest of the word
  kept), and writes it back with `thread_set_state`; `fpsr` and `fpcr` are read,
  replaced and written back the same way in `ARM_NEON_STATE64`, only when one
  is named. The process is paused, so the thread runs from its new state when
  the program next runs. [[src/debugger/macos.rs:set_thread_state]]
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
- **Watchpoints.** `insert_watchpoint` takes one of the processor's watchpoint
  register pairs: as many as `sysctlbyname("hw.optional.watchpoint")` says
  (4 on Apple silicon), the next past those in use failing with `all <n> of the
  processor's watchpoints are in use; remove one with 'wp r <where>'`.
  `DBGWVR` is the address rounded down to 8; `DBGWCR` is enabled, for EL0
  (`PAC` 0b10), on stores (`LSC` 0b10), with byte-address-select bits for the
  `size` bytes from the address's offset in its 8 (`WCR_STORE_EL0`,
  `write_watchpoints`). They are written into every thread's
  `ARM_DEBUG_STATE64` with `thread_set_state`, and into the task's with
  `task_set_state`, which the kernel copies into each thread the program
  starts later; `remove_watchpoint` writes the rest again the same way. A
  stepping thread's are written disabled for the step and enabled after it,
  so a step can carry a thread past a watched store. Ending an attached
  session clears them before `task_resume`, so the program never meets a
  watchpoint with no debugger. [[src/debugger/macos.rs:write_watchpoints]]
- **Run.** `run` first steps every thread whose `pc` is on a breakpoint one
  instruction (so it does not stop on the breakpoint it is paused at — nor on
  one `jump` has put it on), then
  installs the exception port, clears the interrupt flag, and `task_resume`s.
  It waits in 100 ms slices for an exception, a set interrupt flag, or the end
  of the program, then `task_suspend`s. Exceptions are classified: every
  `EXC_BREAKPOINT` whose second code (the `brk`'s address) is a breakpoint's
  address is a hit (`RunStop::Breakpoints`, every such thread); else every
  thread whose `EXC_BREAKPOINT` has the first code `EXC_ARM_DA_DEBUG` (`0x102`)
  is about to write watched memory (`RunStop::Watchpoints`); otherwise the
  first exception is `RunStop::Exception`. Every exception is answered
  `KERN_SUCCESS`, which leaves a thread on its `brk` (`pc` is the `brk`'s
  address), retries a faulting instruction, or retries a watched store — which
  stops again on the next `run` when a breakpoint hit at the same time took
  precedence. A launched program has ended
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
  breakpoints and watchpoints written into it, its exit, its libraries, and
  the executable's symbols (read again, so a rebuilt file is seen).
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

## Linux backend

[[src/debugger/linux.rs:LinuxAarch64]] Linux on AArch64, the `ET_DYN`
executables of `./mfb spec linker linux-aarch64` in either libc world. Linux
needs no signing: a process may trace its own children, and others as
`kernel.yama.ptrace_scope` allows.

- **Launch.** `fork`; the child puts itself in a process group of its own
  (`setpgid(0, 0)`), stops itself (`SIGSTOP`), and `execve`s the program with
  the arguments after `--` and this process's environment and stdio; a failed
  `execve` reports its errno through a close-on-exec pipe (`cannot launch
  '<path>': <error>`). The parent waits for the stop, seizes the child
  (`PTRACE_SEIZE` with `PTRACE_O_TRACECLONE | PTRACE_O_TRACEEXEC |
  PTRACE_O_EXITKILL`), continues it, and waits for `PTRACE_EVENT_EXEC`: the
  program is stopped at its exec, before the loader (`ld.so`) runs.
  [[src/debugger/linux.rs:spawn]] [[src/debugger/linux.rs:seize_launched]]
- **Attach.** Every thread in `/proc/<pid>/task` is seized with
  `PTRACE_O_TRACECLONE` (again until no new one appears), then interrupted
  (`PTRACE_INTERRUPT`) and waited for. A refused seize names its errno; for
  `EPERM` with `ptrace_scope` not 0 the message adds that the system lets a
  debugger attach only to its own children, and to launch the program instead
  or run `mfb dbg` as root. [[src/debugger/linux.rs:seize_all]]
  [[src/debugger/linux.rs:seize_refusal]]
- **Threads.** The session holds every traced thread, leader first. A thread
  the program starts is traced from its first instruction and appended when
  its first stop, or its parent's `PTRACE_EVENT_CLONE`, is seen; one that ends
  is dropped. `threads` reads `pc`, `sp` and `x19` from `NT_PRSTATUS`, the stack
  from the `/proc/<pid>/maps` region holding `sp`, and the name from
  `/proc/<pid>/task/<tid>/comm` — none for the leader, or for a thread whose
  `comm` is still the program's. The `pthread_t` is the thread pointer
  (`NT_ARM_TLS`) less glibc's `struct pthread` size, which the loaded libc
  exports as `_thread_db_sizeof_pthread`, or, for musl (an interpreter path
  naming `musl`), less the gap at which the descriptor's first word, `self`,
  holds its own address, tried in steps of 8 up to 4096 (`MAX_PTHREAD_SIZE`);
  0 when neither is found, as before libc has set the thread pointer.
  [[src/debugger/linux.rs:GLIBC_PTHREAD_SIZE]] [[src/debugger/linux.rs:Tracee]]
- **Memory.** `/proc/<pid>/mem`, read and written at an offset; a breakpoint's
  `brk #0` is written over read-only code through it, and the kernel makes the
  page a private copy. [[src/debugger/linux.rs:BREAKPOINT_INSTRUCTION]]
- **Symbols.** The executable is read by `super::elf`: `.symtab`'s functions
  and objects (a `--debug` build); each undefined dynamic symbol, an import,
  with the GOT slot an `R_AARCH64_JUMP_SLOT` or `R_AARCH64_GLOB_DAT` relocation
  names, the dynamic table reached through `PT_DYNAMIC` (a build without
  `--debug` has no section table); and the import stubs at the end of the
  code (`adrp x16` / `ldr x17, [x16]` / `br x17`), each named by the GOT slot it
  loads. A slot holding 0 is unbound. The load bias is where
  `/proc/<pid>/maps` maps the executable's first page, less that page's
  link-time address; `entry` is `e_entry` plus it. [[src/debugger/elf.rs:executable]]
- **Libraries.** Each file `/proc/<pid>/maps` maps at offset 0, but the
  executable, is read from its file (through `/proc/<pid>/root` when the path
  does not resolve here). Its functions are the defined `STT_FUNC` and
  `STT_GNU_IFUNC` entries of its dynamic symbol table and of `.symtab` when it
  has one, with their sizes; an address in its `.plt` is `symbol stub for:
  <symbol>`, by the entry's `R_AARCH64_JUMP_SLOT`. A library was loaded while
  the program ran (`SD`) when the executable's `DT_NEEDED` closure, through
  each library's `DT_SONAME` and `DT_NEEDED`, does not reach it and it is not
  the loader. The list is read again when the maps change.
  [[src/debugger/elf.rs:read]]
- **Registers.** `NT_PRSTATUS` (`user_pt_regs`) and `NT_FPREGSET`
  (`user_fpsimd_state`), laid out as macOS's thread and NEON states;
  `set_registers` writes them back with `PTRACE_SETREGSET`.
  [[src/debugger/arm64.rs:registers]]
- **Run.** A thread paused on a breakpoint steps over it first, as on macOS;
  then every stopped thread is continued (`PTRACE_CONT`) with any signal held
  for it, and the session waits (`waitpid(-1, __WALL)`). A `SIGTRAP` with
  `si_code` `TRAP_BRKPT` at a breakpoint's address is a hit, elsewhere `a
  breakpoint instruction`; one with `TRAP_HWBKPT` while watchpoints are set is
  a watched write. `SIGSEGV` and `SIGBUS` are `a bad memory access at
  0x<si_addr>`, `SIGILL` `an illegal instruction`, `SIGFPE` `an arithmetic
  exception`. On any of these every other thread is interrupted and waited
  for, and what they stopped for joins it, classified as macOS's are. The
  kernel's own traps — a single step's (`TRAP_TRACE`, or `SI_USER` from pid 0
  for a step over a system call) and a removed watchpoint's — are dropped; so
  is a `SIGSTOP`. Any other signal (`SIGCHLD`, `SIGPIPE`, a timer, a `SIGTRAP`
  a process sent) is passed on at once and the program runs on, unseen. An
  exception's signal is held and delivered when the program next runs, so its
  handler — or the default action, which ends it — runs then; macOS retries
  the faulting instruction instead. A job-control stop (`PTRACE_EVENT_STOP`)
  is continued. The program has ended when its leader's end is reaped: its
  exit status or signal, for an attached program too.
  [[src/debugger/linux.rs:signal_stop]]
- **Step.** Every breakpoint is lifted and the stepping threads' watchpoints
  turned off, as on macOS. Each round, every stepping thread gets
  `PTRACE_SINGLESTEP` and the session polls (`waitpid(WNOHANG)`) in 2000 ms
  slices (`STEP_SLICE`) for each to stop. Threads still running after a slice
  are given up when each is blocked in the kernel (`S` or `D` in
  `/proc/<pid>/task/<tid>/stat`) or 30 s (`STEP_LIMIT`) have passed: they are
  interrupted and are `Waiting`. A signal that stops a stepping thread is held
  and the instruction tried again; a clone event lets the step finish.
  [[src/debugger/linux.rs:step_rounds]]
- **Watchpoints.** The processor's watchpoint register pairs (`dbg_info`'s
  count, from `NT_ARM_HW_WATCH`), encoded as on macOS and written into every
  thread with `PTRACE_SETREGSET`; the kernel does not copy them into a new
  thread, so each gets them at its first stop.
  [[src/debugger/linux.rs:set_thread_watchpoints]]
- **Ending.** A launched program is killed (`SIGKILL`; `PTRACE_O_EXITKILL`
  kills it too if `mfb` dies) and every thread reaped until the leader is. An
  attached one has its breakpoints lifted and watchpoints cleared, and every
  thread is detached (`PTRACE_DETACH`) with any signal held for it.
  [[src/debugger/linux.rs:kill_and_reap]]
- **Ctrl-C.** A `SIGINT` handler installed without `SA_RESTART` sets a flag;
  landing on any thread but the session's — `mfb` runs a command on a thread
  of its own, and the session's thread is the one that waits — it sends the
  signal on to it with `tgkill`, so the wait returns `EINTR`. A `run` that sees
  the flag interrupts every thread and answers `RunStop::Interrupted`.
  [[src/debugger/linux.rs:on_interrupt]]
- **Restart.** The old process is killed and reaped, the program launched
  again, and everything about the old process forgotten, as on macOS.

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
| `list func [<filter>...]`, `list function [<filter>...]` | prints the function listing, kept to the types and names `<filter>` gives |
| `list thread` | prints the thread listing |
| `list reg [<id>]` | prints thread `<id>`'s registers (default 0) that have a role, a blank line, then the `LIST_REG_USAGE` block; only the block once the program has ended |
| `list reg [<id>] <what>...` | prints thread `<id>`'s registers (default 0) of each group or name in `<what>` |
| `list arena` | prints the arena listing |
| `show mem <where> [<size>]` | dumps `<size>` bytes (default 64) of memory |
| `show type` | prints the `TYPE_USAGE` block |
| `show type <type> <where> [...]` | prints the arena value at `<where>` read as `<type>` |
| `show arena [<id>]` | prints arena `<id>`'s state and block headers (default 0) |
| `show error [<thread>]` | prints the error thread `<thread>` (default the current thread) has raised and not yet caught |
| `show arena <id> <block> [<offset>] [<size>]` | dumps `<size>` bytes (default 1024) of a block from `<offset>` (default 0) |
| `step [<count>] [<thread>]`, `s …` | runs thread `<thread>` (default 0) `<count>` instructions (default 1) |
| `step-all [<count>]`, `sa …` | runs every thread `<count>` rounds of one instruction (default 1) |
| `breakpoint <target> [<commands>]`, `bp …` | sets breakpoints at `<target>`, each running `<commands>` at a stop |
| `breakpoint r <target>`, `bp r …` | removes every breakpoint at `<target>`, with a test or not |
| `breakpointif <where> <test> [<value>] at <target> [<commands>]`, `bpi …` | sets breakpoints at `<target>` that stop a thread only when the test holds for it |
| `breakpointif r <where> <test> [<value>] at <target>`, `bpi r …` | removes the breakpoints at `<target>` with that test |
| `jump <where> [<register>=<value>]...`, `j …` | moves the current thread to `<where>`, setting up to 8 registers; it stays paused |
| `breakerror <code>... [<commands>]`, `be …` | stops when a thread raises an error whose code is one of `<code>` (a number or an `errorCode` name) |
| `breakerror any [<commands>]`, `be any …` | stops when a thread raises any error |
| `watchpoint <where> [<size>] [<commands>]`, `wp …` | stops when a thread changes the `<size>` bytes (default 8) at `<where>` |
| `watchpoint r <where>`, `wp r …` | removes the watchpoint at `<where>` |
| `list watchpoint` | prints the watchpoint listing |
| `breakerror clear`, `be clear` | stops watching for errors |
| `breakerror`, `be` | prints what is watched for |
| `list breakpoint` | prints the breakpoint listing |
| `run`, `r` | runs the program to the next stop |
| `finish`, `f` | runs the current thread until its function returns |
| `backtrace [<count>] [<id>]`, `bt …` | prints thread `<id>`'s frames (default the current thread), the innermost `<count>` (default all) |
| `backtrace all`, `bt all` | prints every thread's frames |
| `restart`, `rs` | starts a launched program again |
| `disassemble`, `dis` | disassembles the current function |
| `dis <count>` | disassembles `<count>` instructions from the current `pc`; a negative count, that many before it and then the `pc` |
| `dis -<back> <length>` | disassembles `<length>` instructions from `<back>` before the current `pc` |
| `dis <where> [<count>]`, `dis <where> -<back> <length>` | disassembles from an address, register or function (each `±<offset>`): to the end of its function, or the instructions a count picks as above |
| `dis <function> [<count>]` | disassembles each function `<function>` names, or `<count>` from its start as above |
| `quit`, `q` | ends the session |

`list` may be spelled `l` in every form. Commands are case-sensitive.
[[src/debugger/prompt.rs:parse]] Refusals, each printed as `error: <text>`:

| Input | Text |
| --- | --- |
| an unknown command | `unknown command '<name>'; type 'help' for the list of commands` |
| `help` or `quit` with arguments | `'<name>' takes no arguments` |
| `list <what>` for an unknown `<what>` | `cannot list '<what>'; type 'list' for what can be listed` |
| `list thread`/`arena`/`breakpoint`/`watchpoint` with an argument | `too many arguments to '<name> <what>'` |
| `list func <filter>...` that keeps no function | `no function matches '<filter>...'; type 'list func' for the functions` (the type codes first, then the names) |
| `step <count> <id>` where `<id>` is not a `usize` | `'<id>' is not a thread id; type 'list thread' for the threads` |
| `list reg` naming a register the thread does not have | `no register '<name>'; type 'list reg' for the groups` |
| `show mem` with no location | `'show mem' needs an address or a register: show mem <where> [<size>]` |
| a location that is neither `0x…`, `<name>` nor `[<location>]`, each `[±<n>]` | `'<where>' is not an address, a register or a function` |
| a location with a `[` and no matching `]` | `'<where>' opens a '[' it does not close` |
| a location whose value is outside the address space | `'<where>' is outside the address space` |
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
| `breakerror` with a word that is no integer and no `errorCode` constant | `'<word>' is not an error code: give its number (77050010) or its errorCode name (ErrOverflow)` |
| `breakerror` on a program that does not name `_mfb_rt_park_error` (a build without `--debug`) | `'breakerror' stops in the runtime's error function, which only a debug build names; rebuild the program with: mfb build --debug -O0 <project>` |
| `breakerror clear` with nothing watched | `no errors are being watched for` |
| `breakpoint r` with more than a target | `too many arguments to '<name> r'` |
| `breakpointif` without a location and a test | `'<name>' needs a test and a place: <name> <where> eq\|ne\|gt\|lt <value> at <address\|function[+offset]> [<commands>], or <name> <where> in at <address\|function[+offset]>` |
| a test but `eq`, `ne`, `gt`, `lt` or `in` | `'<test>' is not a test: give eq, ne, gt, lt or in` |
| `eq`, `ne`, `gt` or `lt` with no value | `'<test>' needs a value to test against: <where> <test> <value>` |
| `in` with a value | `'in' takes no value: <where> in at <address\|function[+offset]>` |
| `in` on a location with no `[…]` | `'<where>' can always be worked out: 'in' needs a memory read ([…])` |
| a value that is no whole number | `'<value>' is not a value: give a number, such as 5, -1 or 0x10` |
| a negative value below `-0x8000000000000000` | `'<value>' is below the smallest value, -0x8000000000000000` |
| `breakpointif` without `at` after its test | `'<name>' needs 'at' and the breakpoint's place after its test: <name> <where> <test> at <address\|function[+offset]>` |
| `breakpointif` with nothing after `at` | `'<name>' needs a place after 'at': an address (0x…) or a function name` |
| `breakpointif r` with words after its target | `too many arguments to '<name> r'` |
| a test naming a register or function the current thread does not have (checked when it is set) | `no register or function is named '<name>'; type 'list reg' or 'list func' for them`, or the function name's own refusal |
| `breakpointif` where the same test is at that place | `there is already a breakpoint at <target> if <test>` / `on '<target>' if <test>` |
| `breakpointif r` where no breakpoint has that test | `there is no breakpoint at <target> if <test>` / `on '<target>' if <test>` |
| `jump` with no place | `'<name>' needs a place to go: <name> <where> [<register>=<value>]...` |
| `jump` with more than 8 registers | `'<name>' sets at most 8 registers` |
| a `jump` word without `=` | `'<word>' is not a register and its value: <register>=<value>, such as x0=5` |
| a `jump` register with nothing after `=` | `'<register>' needs a value: <register>=<value>` |
| a `jump` register named twice | `'<register>' is set twice` |
| `jump` setting `pc` | `'pc' is where the thread goes: give it as the place, jump <where>; jump sets x0-x28, fp, lr, sp, nzcv, fpsr and fpcr` |
| `jump` setting `v0`–`v31` | `'<register>' is a vector register; jump sets x0-x28, fp, lr, sp, nzcv, fpsr and fpcr` |
| `jump` setting any other name | `no register '<register>'; jump sets x0-x28, fp, lr, sp, nzcv, fpsr and fpcr` |
| `jump` to an address not a multiple of 4 | `0x<address> is not the start of an instruction: they start every 4 bytes` |
| `jump` to an address with no readable code | `cannot jump to 0x<address>: <reason>` |
| a `jump` value that cannot be worked out | `cannot set <register>: <reason>` |
| `nzcv` given bits but 31–28 | `cannot set nzcv to 0x<value>: it holds only the flags, bits 28 to 31 (0x00000000f0000000)` |
| `fpsr` or `fpcr` given more than 32 bits | `cannot set <register> to 0x<value>: it holds 32 bits` |
| a stop's command that is not `list`, `show`, `backtrace`, `disassemble`, `jump` or a bare `breakerror` | `'<command>' cannot run at a stop: a stop runs list, show, backtrace, disassemble, jump and a bare breakerror` |
| a stop's command that does not parse | the command's own refusal |
| `breakerror` with commands but no code | `'<name>' needs an error code, a name or 'any' before its commands` |
| `breakerror clear` with commands | `'<name> clear' takes no commands` |
| `watchpoint` or `watchpoint r` with no place | `'<name>' needs an address or a register: <name> <where> [<size>] [<commands>]` |
| `watchpoint r` with more than a place | `too many arguments to '<name> r'` |
| a `watchpoint` size that starts with a digit and is not a number | `'<size>' is not a number` |
| a watchpoint of 0 or more than 8 bytes, or crossing a multiple of 8 | `cannot watch <size> bytes at 0x<address>: a watchpoint covers 1 to 8 bytes that do not cross an address that is a multiple of 8` |
| a watchpoint where one is | `there is already a watchpoint at 0x<address>` |
| `watchpoint r` where none is | `there is no watchpoint at 0x<address>` |
| a target starting `0x` that is not hex | `'<target>' is not an address` |
| `run` with arguments | `'<name>' takes no arguments` |
| a `backtrace` count that is not a `usize` of 1 or more | `'<count>' is not a number of frames (1 or more), or 'all'` |
| `backtrace` with three arguments | `'<name>' takes at most two arguments` |
| `backtrace <count> <id>` past the last thread | `no thread <id>: the program has <n> thread(s); type 'list thread' for them` |
| a `dis` count of 0 or not an `i64` | `cannot show <count> instructions; give a count other than 0 (negative counts back)` |
| two `dis` counts, the first not negative | `'<back>' does not count back; with two counts the first is negative, as in dis -6 8` |
| two `dis` counts, the second not positive | `cannot show <length> instructions; the count after a negative one must be positive` |
| `dis` with four arguments | `'<name>' takes at most three arguments` |
| `dis` at an address not a multiple of 4 | `0x<address> is not the start of an instruction: they start every 4 bytes` |
| `dis` counting back past address 0 | `there are not <n> instructions before 0x<address>` |
| an address `0x<hex>±<offset>` below 0 or past `u64::MAX` | `'<word>' is below address 0` / `'<word>' is past the last address` |
| `dis <function>` naming nothing | `no function is named '<function>'; type 'list func' for the functions` |
| `dis <function>` naming only an unbound import | `'<function>' is not loaded yet; run the program until it has loaded its libraries` |
| `show` alone | `'show' needs what to show: show mem <where> [<size>], show type <type> <where>, show arena [<id>], or show error [<thread>]` |
| `show error` with more than a thread | `too many arguments to 'show error'` |
| `show <what>` but `mem`, `type`, `arena` or `error` | `cannot show '<what>'; 'show mem', 'show type', 'show arena' and 'show error' are what can be shown` |
| `show arena` with more than four arguments | `too many arguments to 'show arena'` |
| an arena id not a `usize` | `'<id>' is not an arena id; type 'list arena' for the arenas` |
| a block number not a `usize` | `'<block>' is not a block number` |
| an offset or size not a `u64` (decimal, or hex after `0x`) | `'<word>' is not a number` |
| any command but `help`, `list`, `list breakpoint`, a bare `list reg`, `restart`, `quit` after the program ended | `the program has exited; type 'quit' to end the session` |

Every listing below — and `show type`'s element table — is laid out by
`prompt::table`: a header line naming its columns, then one line per row; the
columns are separated by ` | ` (`COLUMN_SEPARATOR`) and each but the last is
left-aligned to its widest entry, header included, counted in characters, so
every row's separators fall under the header's. A listing with no rows prints
no header. [[src/debugger/prompt.rs:table]]

A listing or step the backend cannot produce prints `error: <reason>`. All go
to stderr after stdout is flushed; the prompt continues. End of input writes a
newline and ends the session as `quit` does.

### `list func`

One line per symbol `Inferior::functions` answers, except that a symbol
repeated at the same address (an import's repeated records, or the same
unbound import) is shown once; the same name at two addresses is shown twice:
[[src/debugger/prompt.rs:function_lines]]

```text
Type   | Address   | Symbol   | Written name
<type> | <address> | <symbol> | <written name>

<type>  <legend>
```

- `<type>` is `MU`, `MB`, `MR`, `SL` or `SD`, and lines are grouped in that
  order; within a group they are in address order with an unbound import last,
  and ties keep symbol-table order.
- `<address>` is `0x` and 16 lowercase hex digits, or `-` for an unbound import.
- `<symbol>` is the symbol as the executable spells it.
- `<written name>` is the name the developer wrote, or `-`.
- Each word after `func` is a filter: one of the type codes `MU`, `MB`, `MR`,
  `SL`, `SD` (exactly, `Origin::parse`) keeps that type; any other word is a
  pattern a symbol or its untagged written name must match in full, `*`
  matching any run of characters and `?` any one, every other character
  itself. A function is listed when it is of one of the types given (any, with
  none) and matches one of the patterns given (any, with none). The collision
  tags are worked out over every function, not just those listed. A filter
  that keeps nothing is refused. [[src/debugger/prompt.rs:FunctionFilter]]
  [[src/debugger/prompt.rs:glob]]
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
| an import | `SL` | its C name: the symbol less `names::C_PREFIX` (`_` on Mach-O, nothing on ELF) |
| a function of a library loaded while the program ran | `SD` | its C name, likewise |

Everywhere a function is named — breakpoint messages, `step` and `finish`
places, `dis` headers and comments, `bt` frames — it is shown by its written
name, else by its symbol exactly as `list func` spells it (`_main`,
`_mfb_arena_alloc`); a symbol that is not the executable's own (`_main`,
`_mfb…`) is a system library's and is shown by its C name
(`names::plain_name`). A stop line shows both the written name and the
symbol (`run`, below). [[src/debugger/names.rs:plain_name]]

A name an MFBASIC function (`MU`, `MB`) and a native one (`MR`, `SL`, `SD`)
share — a function's name being the name it is shown by, over every function
`Inferior::functions` answers — is shown with
a tag: ` <mfb>` after the MFBASIC one's, ` <native>` after the native one's, so
a developer's `exit` is `exit <mfb>` and the C library's `exit <native>`. The
runtime's `_main` is shown by its symbol, so it shares no name with the
program's `main`. The tag follows any offset (`exit+0x24 <mfb>`,
`exit + 8 <native>`) and is applied the same way in `list func`'s written
names, `list breakpoint` and breakpoint messages, `step` and `finish` places,
`dis` headers and comments, and `bt` frames; it is never part of a name a
command looks up, and a stop line carries none.
[[src/debugger/names.rs:Collisions]]

A trailing `$own<hex>` (a hand-over copy; the mask is not escaped) is dropped
before decoding. The decoded IR name is reduced: `<16 hex>.<package>.<name>`
becomes `<package>::<name>`; `$lambda<N>` becomes `lambda`;
`$default$<function>$<index>` becomes `<function>`; otherwise the text before
the first `$` (an overload or generic instance), or the whole name when that is
empty. A `_` not followed by two uppercase hex digits is kept literally.

### `list thread`

One line per thread, in the backend's order, `<id> | <name> | <pc> | <task>`,
under the header `ID | Name | PC | Task`:
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
(default 0). Every other argument picks registers, in the order given: `gpr`,
`simd` and `sys` pick every register of that `RegisterGroup` in the backend's
order, and any other word picks the one register of that name.
[[src/debugger/prompt.rs:RegisterPick]]

With no other argument the command prints every register of the thread whose
`Register::role` is set, in the backend's order, one line each as
`<register> | 0x<hex> | (<decimal>) | <role>` under the header
`Register | Hex | Decimal | Role`, then an empty line, then the
`LIST_REG_USAGE` block. A thread id past the last thread is refused as it is
with arguments. After the program has ended it prints the `LIST_REG_USAGE`
block alone. [[src/debugger/prompt.rs:common_register_lines]]

On AArch64 the roles are the compiler's own register assignments
(`./mfb spec memory native-calling-convention`), and a role is a fixed label
for the register, not a reading of what it holds at the stop:
[[src/debugger/arm64.rs:role]]

| Register | Role |
| --- | --- |
| `x0`, `x1` | `MFB arg, MFB result, C arg, C result` |
| `x2`, `x3` | `MFB arg, MFB result, C arg` |
| `x4`–`x7` | `MFB arg, C arg` |
| `x19` | `Arena` |
| `x20` | `Current Thread` |
| `x28` | `Closure` |

One line per register picked, `<register> | 0x<hex> | (<decimal>)`, under the
header `Register | Hex | Decimal`: the value as `0x` plus 16 lowercase hex digits
(64-bit) or 32 (128-bit), and the same value in decimal, signed for a 64-bit
register (as MFBASIC's `Integer`) and unsigned for a 128-bit one.
[[src/debugger/prompt.rs:register_lines]]

On AArch64 every register of the thread state is read
(`src/debugger/arm64.rs:registers`), the same on macOS and Linux:

| Group | Registers |
| --- | --- |
| `gpr` (`General`) | `x0`–`x28`, `fp` (x29), `lr` (x30) |
| `sys` (`System`) | `sp`, `pc`, `nzcv`, `fpsr`, `fpcr` |
| `simd` (`Simd`) | `v0`–`v31` |

`nzcv` is CPSR with every bit but 31–28 cleared, as the `NZCV` system register
reads; `fpsr` and `fpcr` are the 32-bit words of `ARM_NEON_STATE64`.

### `list arena`

One line per live arena in the `--debug` registry,
`<id> | <thread id> | <address> | <size in KB>`, under the header
`ID | Thread | Address | Size (KB)`: [[src/debugger/runtime.rs:arenas]]
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

### Locations

[[src/debugger/location.rs:Location]] A `<where>` — what `show mem`,
`show type`, `watchpoint`, `dis`, `jump` and a `breakpointif` test take — is a
term and an optional offset, `+<n>` or `-<n>` (decimal, or hex after `0x`):

- `0x…`: a number. An offset is added in when the word is read, failing past 0
  or `u64::MAX` as an address does (`0x1000-0x10`).
- a name: a register of the thread the location is worked out for, when it has
  one so named (a 64-bit register only: a vector register fails with
  `'<name>' is a vector register, not an address`); else a function, resolved
  as `dis <function>` resolves one, to its start. A name is letters, digits,
  `_`, `:`, `#`, `$` and `.`, so a place `bt` or a stop prints can be typed
  back. A function name that names several functions fails with `'<name>' names
  <n> functions, at 0x…, 0x…; give one by its address`; one that names none
  (and no register) with `no register or function is named '<name>'; type
  'list reg' or 'list func' for them`.
  [[src/debugger/location.rs:is_register]] [[src/debugger/prompt.rs:function_start]]
- `[<location>]`, nested to any depth: the 8 bytes, little-endian, in memory at
  the inner location's value.

The **value** is the term's — the number, the register's value, the
function's start, the bytes read — plus the offset: `sp+0x18` is a number,
`[sp+0x18]` what is stored there, `[sp]+0x18` what is stored at `sp` plus
0x18, `[[sp+0x10]+8]` what is stored 8 past the value stored at `sp+0x10`. A
read that fails, or an offset that goes below 0 or past `u64::MAX` (`'<where>'
is outside the address space`), leaves it without a value.
[[src/debugger/location.rs:Fault]] A location is shown with numbers as `0x`
and 16 hex digits and offsets in short hex (`[0x0000000012348593]`,
`[sp+0x18]`, `x19-0x10`). [[src/debugger/location.rs:Location]]

### `show mem`

[[src/debugger/prompt.rs:show_memory]] `show mem <where> [<size>]` reads
`<size>` bytes (default 64, at most 65536; decimal or `0x` hex) at the value of
`<where>` worked out for the current thread (`0x1000-0x10`, `sp`, `sp+0x18`,
`x19-16`, `main+0x110`, `isPrime+16`, `[sp+0x10]`). It prints
`Memory at 0x<address>:` — with ` (<where> of thread <t>)` when a register
went into the value, ` (<where>)` for any other location but a number — then
the bytes as `show arena`'s dump does, each line led by its full address (`0x` +
16 digits). [[src/debugger/prompt.rs:resolve]] [[src/debugger/prompt.rs:memory_lines]]

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

A `string` view of an address the executable names with a `_mfb_str_*` data
symbol (`data_symbol_at`) — a `String` literal the compiler emits, laid out as
an arena `String` (its length, its bytes, a NUL) in the file's data — reads it
there instead, without the arena check or any arena, and adds ` (the literal
<symbol less one '_'>)`: `Length: 17 Value: "function handler " (the literal
mfb_str_2)`. [[src/debugger/values.rs:show_literal]]

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
is escaped as `char::escape_debug` does. [[src/debugger/values.rs:quoted]]

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

### `show error`

[[src/debugger/prompt.rs:show_error]] `show error [<thread>]` reads the error
thread `<thread>` (a `list thread` id; default the current thread) has raised
and not yet caught: the flat `Error` block a failing operation parks in its
arena's `ARENA_CURRENT_ERROR_OFFSET` slot for its catcher, which takes it and
clears the slot (`./mfb spec memory fallible-call-abi`, *Parked error
ownership*). The arena is the `--debug` registry's whose thread is
`<thread>`, so this needs a `--debug` build (`'show error' reads the thread's
arena, which needs a debug build; …`); a thread with none fails with `thread
<t> has no arena: it is running no MFBASIC code`. An empty slot prints `Thread
<t> has no error in flight.` — as does a loose error (tag `1`, an error with no
block because building one ran out of memory), which lives only in the
registers. [[src/debugger/runtime.rs:current_error]]

When the slot is empty and the thread is stopped on a fault, the line names it
instead: `Thread <t> has no error in flight: it stopped on <exception>, which
is a fault, not an error a TRAP can catch. Use bt and list reg to see where.`
`<exception>` is the text after `X:` on the thread's `--[XC]` line, or after
`by` in a `step`'s or `step-all`'s `Stopped after … by <exception>` line. The
session remembers it per thread (`prompt::Faults`) until the thread can have
left the fault: `run`, `finish`, `restart` and a `step-all` of one round or
more forget every thread's, a `step` of that thread and a `jump` of it forget
its own. [[src/debugger/prompt.rs:Faults]]

With an error in the slot:

```text
Thread <t>'s error in flight (the Error at 0x<address>):
Code: <code> (errorCode::<name>)
Message: "<message>"
Location: <file>:<line>:<char>
```

The block is read as `./mfb spec memory heap-values` lays out `Error` and
`ErrorLoc` (each text at most 4096 bytes). `(errorCode::<name>)` names the
`errorCode` package constant whose value is the code and is left out for any
other code [[src/debugger/runtime.rs:error_code_name]]; an error with no origin
(a zero `ErrorLoc` offset) prints `Location: - (the error has no origin)`.

### Breakpoints

[[src/debugger/breakpoints.rs:Breakpoints]] A `<target>` starting `0x` (or
`0X`) is an address in hex, perhaps `+` or `-` an offset (decimal, or hex after
`0x`), as `show mem` reads one; `<name>+<offset>` is a place in a function;
anything else is a name.

- **By address**: one breakpoint at that address, described by the function
  that holds it (`<symbol>+0x<offset>` and `<written>+0x<offset>`), a linked
  function bound exactly there, or `-`.
- **By name**: the function symbols spelled exactly `<target>` (as `list func`
  shows them: `_mfb_rt_print`, `_exit`); only when there are none, the
  function whose written name is `<target>` (`isPrime`). A written name more
  than one function has — overloads and generic instances (`show`), lambdas
  (`lambda`), a developer's function and a linked one (`exit` on Mach-O; on
  ELF `exit` is the C library's symbol, so it names that one) — is refused
  with `'<target>' is ambiguous: it names <symbol>, <symbol>…; name one by
  its symbol`, the symbols sorted. A function and its hand-over copies
  (`<symbol>$own<hex>`) are one function: the written name names all of them,
  and the refusal lists the function once (`names::ambiguity`). One breakpoint
  per distinct address among the matches; a linked function not yet bound
  gives one **pending** breakpoint per symbol, unless another record of it is
  bound. No match:
  `no function is named '<target>'; type 'list func' for the functions`.
  [[src/debugger/breakpoints.rs:find]]
- **By place** (`<name>+<offset>`, the offset decimal or `0x` hex, as `bt` and
  stops print a place: `main+0x110`, `main+272`): `<name>` matched as by name,
  then one breakpoint `<offset>` bytes into each bound function it matches,
  described as by address. None bound: `'<name>' is not loaded yet; run the
  program until it has loaded its libraries`.

Each new breakpoint prints `Set a breakpoint at <place>.`, where `<place>` is
`0x<address>` and ` in <written name or symbol>` when it has one; a pending one
prints `Set a pending breakpoint on <symbol>: it is placed when the program
has loaded its libraries.` A target whose every breakpoint exists already
fails with `there is already a breakpoint at <address>` / `on '<target>'`.

**Tests.** `breakpointif <where> <test> [<value>] at <target> [<commands>]`
(`bpi`) sets the breakpoints `breakpoint <target>` would, each with a test
(`location::Condition`): `eq`, `ne`, `gt` or `lt` and a value — a whole number,
perhaps negative, decimal or `0x` hex (`5`, `-1`, `0x10`), taken as its 64 bits
(`location::signed`) — or `in` with none. When a thread reaches the breakpoint,
`<where>` is worked out for that thread (its registers; memory as it is then);
`eq` and `ne` compare the 64 bits, `gt` and `lt` compare them as signed numbers
(`i64`), and a value that cannot be worked out passes none of them. `in` holds
just when it cannot: a location without a `[…]` cannot fail so, and `in` on one
is refused. The test's names are checked when it is set, against the current
thread; a memory read that fails then is no fault. A breakpoint's place and
messages carry ` if <test>`, the test written `<where> <test> <value>` with the
location as Locations shows one and the value as an 8-byte watchpoint value
(`x0 eq 0x0000000000000005 (5)`, `[x0+0x8] in`).
[[src/debugger/location.rs:Condition]] [[src/debugger/breakpoints.rs:Breakpoint]]

Several breakpoints may be at one address, each with a different test (or
none); setting one with the test, and at the place, of one already there is
the refusal above, with ` if <test>`. One `brk` serves every breakpoint at an
address — the user's, `breakerror`'s on the park, a `finish`'s on a `ret` —
written when the first is set and taken out when the last goes
(`Breakpoints::armed`). A thread there stops when any of them stops it: one
with no test, or one whose test holds; each of those runs its own commands. A
thread no breakpoint there stops runs on, unseen. [[src/debugger/breakpoints.rs:Breakpoints]]

Every word after the target is the breakpoints' **commands** (see Commands at
a stop below); with any, the line goes on ` Each stop runs '<commands>'.`,
`<commands>` joined by `; `.

`breakpoint r <target>` removes, by address or by place, the breakpoints there; by name, the
breakpoints whose symbol is `<target>`, or else whose written name is — every
one, with a test or not, unless those written so are on more than one function,
which is refused as ambiguous as setting by name is. `breakpointif r <where> <test> [<value>] at <target>`
removes just those of them with that test. Each prints `Removed the breakpoint
at <place>.` (with ` if <test>`); none fails with `there is no breakpoint at
<address>` / `on '<target>'` (with ` if <test>` for `breakpointif r`).

`list breakpoint` prints `<address> | <count> | <symbol> | <written name>` per
breakpoint under the header `Address | Count | Symbol | Written name`, in
address order with pending ones last (`-` for the address), and at one address
the one without a test first, then in the order they were set; with none,
`no breakpoints`. `<count>` is the stops the breakpoint has made since the
program last started: one more each time a stop is reported for it, a thread
whose test failed there not counting, and 0 again after `restart`
(`Breakpoints::count`). The listing is still printed after the program has
ended, so a breakpoint never reached reads 0 there. When any
breakpoint has a test, a column `If` gives each one's (`-` for none); when any
has commands, a last column `Commands` gives each one's, joined by `; `, or
`-`.

### Watchpoints

[[src/debugger/breakpoints.rs:Watch]] `watchpoint <where> [<size>]
[<commands>]` (`wp`) watches the `<size>` bytes at `<where>`: `<where>` is
resolved once, now, as `show mem` resolves one (`0x…`, a register of the
current thread, a function, `[…]`, each `±<offset>`); `<size>` is the word after it
when that starts with a digit (decimal, or hex after `0x`), else 8 (an
`Integer`). The bytes must be 1 to 8 that do not cross an address that is a
multiple of 8 (`breakpoints::watchable`) and readable; the processor holds a
fixed number of watchpoints (the backend refuses one more). Setting one prints
`Set a watchpoint at 0x<address> (<size> byte[s]): <value>.`, with ` Each stop
runs '<commands>'.` when it has commands. `<value>` is the bytes
read little-endian as `0x` and two hex digits per byte, and for 8 bytes the
signed decimal after, in parentheses (`breakpoints::value_shown`):
`0x05`, `0x0000000000000005 (5)`. `watchpoint r <where>` removes the one at
that address, printing `Removed the watchpoint at 0x<address> (<size>
byte[s]).`

A watchpoint stops a thread **before** it writes any of the bytes. `run` then
reads every watchpoint's bytes, steps that thread over the write (the
backend's `step`, which runs with watchpoints off), and reads them again: each
watchpoint whose bytes changed is a `Changed` stop (`RunStop::Changed`, one
`Change` per watchpoint, the write's `pc` the instruction that wrote it); a
write that changed none runs on, unseen. Several threads stopped at once are
stepped in turn. A step that raises an exception or ends the program is that
stop instead. A thread the step leaves on one of the user's breakpoints stops
there at the next `run`, before anything runs. Only stores the thread makes
are seen: the kernel writing the memory (a `read` into it) is not. After the
stop the current thread is the writer, its `pc` past the write.

`list watchpoint` prints `<address> | <count> | <size> | <value>` per
watchpoint under the header `Address | Count | Size | Value`, in address order,
`<count>` the stops it has made, the value read now (`-` when it cannot be
read); a last column `Commands` as for `list breakpoint` when any has
commands; with none, `no watchpoints`. `restart` drops every watchpoint,
each with `Dropped the watchpoint at 0x<address>: a restarted program's memory
is laid out anew.`

### Commands at a stop

[[src/debugger/prompt.rs:stop_commands]] The words after a breakpoint's target,
a watchpoint's place and size, or `breakerror`'s codes (which end at the first
word that is a command name, `COMMAND_NAMES`) are joined with spaces and split
at `;` into commands; empty ones are dropped. Each is parsed now, and must be
one that reads the program — `list`, `show`, `backtrace`, `disassemble`, or a
bare `breakerror` — `jump`, which moves the stopped thread, `finish` (`f`),
or `run` (`r`). `run` must be the last command (`'<run>' must be the last
command of a stop: '<command>' follows it`); any other command is refused
with `'<command>' cannot run at a stop: a stop runs list, show, backtrace,
disassemble, jump, finish, a bare breakerror and, last, run`.

A stop prints its lines, then runs the commands of each breakpoint,
watchpoint or watched error it stopped for in turn, each with its own thread
current, writing their output and errors as at the prompt
(`prompt::hear_stop`). The program then stays stopped and the prompt returns,
unless the stop runs on: every breakpoint, watchpoint or watched error it
stopped for has commands and each one's last command is `run`. A stop that
runs on does so as `run` again would — inside a `finish` too, which goes on
toward its return.

When no command of the stop is `finish`, its commands run as the stop is
heard, before anything else. When one is, none runs there: they are queued,
each with its thread, with one `run` after them when the stop runs on, and
the prompt takes them one at a time before it reads another line, as if they
were typed (no prompt or echo is written for them). A queued `finish` is the
`finish` command for that thread: it prints where the thread returned and its
`x0` and `x1`, and the commands after it see the thread there. A stop heard
while queued commands wait — a breakpoint inside the function being finished
— discards them and runs or queues its own; the program's end discards them.
[[src/debugger/prompt.rs:hear_stop]]

### `breakerror`

[[src/debugger/breakpoints.rs:ErrorCodes]] `breakerror <code>...` (`be`) watches
for errors by code — each `<code>` an integer, or an `errorCode` constant's name
with or without `errorCode::` (`runtime::error_code_by_name`) — and joins them
to any already watched; `breakerror any` watches for every error; `breakerror
clear` ends the watch; `breakerror` alone prints the watch line, or `No errors
are being watched for.` Setting one prints `Stopping when a thread raises
<codes>.` (`<codes>` each `<code>[ (errorCode::<name>)]`, comma-separated, or
`any error`), which `list breakpoint` also prints after its table (alone, in
place of `no breakpoints`, when there are no breakpoints). Words after the
codes (from the first command name) are the watch's commands; the line then
goes on ` Each stop runs '<commands>'.` A `breakerror` with
commands replaces the watch's; one without keeps them.

The watch is a breakpoint of the debugger's own at the entry of
`_mfb_rt_park_error` (`PARK_ERROR`), the runtime function a failing operation
calls to park its new `Error` block (`./mfb spec memory fallible-call-abi`): its
code in `x1`, its origin `ErrorLoc` in `x3`, returning to the function that
raised the error. A stop there by itself is the debugger's: `run` reads `x1`, and
for a code not watched runs on, unseen; for a watched one it writes a second
breakpoint of its own at the thread's `lr` and runs until that thread reaches it
— back in the raising function, the error parked and the `TRAP` dispatch not
yet taken — and reports the `Raised` stop, the current thread becoming that
thread. A stop at a breakpoint of the user's before then is reported instead (and
the raise is not), and a raise by another thread meanwhile is let go. A
breakpoint of the user's on `_mfb_rt_park_error` shares the address: a stop there
is theirs, and removing it keeps the watch's. `restart` places the watch again
in the new process. A `finish` the raise stops ends early, as for any other
stop. [[src/debugger/breakpoints.rs:Breakpoints]]

Errors the watch cannot see, since they do not pass through the park: an error
re-raised from an `Error` value (`FAIL e`, a cleanup's re-raise), which stores
its block in the slot directly; and a loose error (tag `1`), which has no block.
A worker's error that `thread::waitFor` passes to its caller is parked twice —
by the worker and again in the caller — so it stops twice. Every raise in the
program stops the process for the comparison, watched or not.

### `jump`

[[src/debugger/prompt.rs:jump]] `jump <where> [<register>=<value>]...` (`j`)
moves the current thread — at a stop, the stopped one — to the value of
`<where>` worked out for it, and gives up to 8 registers (`MAX_JUMP_REGISTERS`)
new values: `x0`–`x28`, `fp`, `lr`, `sp`, `nzcv`, `fpsr`, `fpcr`
(`prompt::settable`). A value is a number as a `breakpointif` value is, else a
location's value for the thread (`x0=x1`, `sp=[sp+8]`). Every value is worked out
before any register is set, so `j lr x0=0` goes to the `lr` the thread had.
The place must be a multiple of 4 holding readable code; `nzcv` takes only bits
31–28 (`NZCV_FLAGS`); `fpsr` and `fpcr` take 32 bits. The registers and the
`pc` are written with `Inferior::set_registers`, and the thread stays paused;
`run` and `step` carry on from there. It prints `Moved thread <t> to
<place>.`, `<place>` as `step` prints one, or with registers `Moved thread <t>
to <place>; set <register> = <value>, ….`, each value as an 8-byte watchpoint
value. A thread moved onto a breakpoint runs it, unseen, when the program next
runs, as a thread paused on one always does; one a watched write's step left on
a breakpoint and `jump` then moved does not stop there.

### `run`

`run` places every pending breakpoint whose function is now bound, printing
`Placed the pending breakpoint on <symbol> at 0x<address>.` (a pending one whose
test, or lack of one, is already at that address is dropped instead, with `The
pending breakpoint on <symbol>[ if <test>] is at 0x<address>, which already has
one.`) While any stays
pending and the program has not been seen at its entry point, the entry point
gets a breakpoint of the debugger's own (unless one is there): a stop at that
breakpoint alone places the pending ones and runs on, unseen.
[[src/debugger/breakpoints.rs:Ended]] Each stop is printed as it happens, after
the notes made since the last, as one line per thread it stopped: [[src/debugger/prompt.rs:stop_lines]]

| Stop | Line |
| --- | --- |
| a breakpoint | `--[BP] T: <t> P: <pc> F: <function>` |
| a watched change (one per watchpoint) | `--[WP] T: <t> P: <pc> F: <function> M: 0x<address>: <old> -> <new>` |
| a watched error raised (`breakerror`) | `--[BE] T: <t> P: <pc> F: <function> E: errorCode::<name> (<code>)`, or `E: <code>` when no constant names it |
| another exception | `--[XC] T: <t> P: <pc> F: <function> X: <exception>` |
| Ctrl-C | `--[CC] User Break` |
| a launched program exited | `--[EX] Program exit (<n>)` |
| a launched program ended by a signal | `--[EX] Program ended by signal (<n>)` |
| an attached program ended | `--[EX] Program exit` |

`<t>` is the thread's index; `<pc>` is `0x` and 16 hex digits — the
breakpoint's address, the instruction that wrote a watched change, else the
thread's `pc`; `<function>` is the function holding it as
`(<written name>) <symbol>`, with `+0x<offset>` after the symbol when the stop
is not at its start, and the symbol alone when the function has no written
name or is written as its symbol is spelled (`names::stop_name`:
`(fib) _mfb_fn_fib`, `(fib) _mfb_fn_fib+0x1c`, `(show) _mfb_fn_show_24Integer`,
`_mfb_arena_alloc+0x4`, `(exit) _exit`) — the executable's functions and a
linked function bound exactly there (`runtime::locate_stop`), else a loaded
library's (`(__cxa_finalize_ranges) ___cxa_finalize_ranges+0x8`), else `-`. The
symbol is what tells apart functions that share a written name, and it is the
spelling `breakpoint` takes. [[src/debugger/prompt.rs:stop_place]]
`T: - P: - F: -` stands for a thread no longer there. `<old>` and `<new>` are
shown as watchpoint values are. The error's origin is not printed; `show
error` reads it. A thread at breakpoints whose tests all fail is no stop and
gets no line; at the error park it is still looked at for a watched raise. A
stop whose every part has commands that end in `run` runs them and runs on
(Commands at a stop); the first stop that does not ends the `run`, its thread
current.

### `disassemble`

[[src/debugger/disassemble.rs:disassemble]] The **current thread** is the one
the last `run` stopped (the first breakpoint hit, or the exception's thread)
or the last `step` ran; thread 0 before either, or when it has ended.

- `dis`: the function holding the current thread's `pc`, from its start.
- `dis <count>` (digits, perhaps after `-`): a count, never 0. A positive count
  is that many instructions from the `pc`; a negative one is that many before
  it, then the `pc`'s own. [[src/debugger/disassemble.rs:Window]]
- `dis -<back> <length>`: two counts, the first negative and the second
  positive: `<length>` instructions from `<back>` before the `pc`, so
  `dis -6 8` shows the six before it, the `pc`'s own and one after. Wherever a
  count is taken below, these two counts may stand for it.
- `dis <where> [<count>]`, `<where>` starting `0x` or `[`, holding `+` or `-`,
  or a register name (`x0`–`x30`, `fp`, `lr`, `sp`, `pc`): the address it
  resolves to as `show mem` resolves one (so `dis main+0x110` is a place in `main`), which must be a multiple of 4. With no count, the
  listing runs from there to the end of the function holding it (or to the first
  `ret` or 64 instructions when its extent is unknown); with one, as for
  `dis <count>`. The header names the function holding that address.
- `dis <function> <count>`: `<count>` counted from each function's start, as
  for `dis <count>`.
- `dis <function>`: resolved as a breakpoint name is — symbols spelled
  `<function>` (the executable's, its imports' bound addresses, and the
  libraries' via `library_functions`), else the one function written so (the
  executable's functions, its imports' C names, and on Mach-O the libraries'
  `<C_PREFIX><function>`), refused as ambiguous when several functions are —
  one listing per distinct start, separated by an empty line.
  [[src/debugger/disassemble.rs:resolve]]

A function's extent is the first known of: the executable's symbol size
(`--debug` builds), its import stub's size, the library's function starts.
With none, the listing runs to the first return (`ret`, `retaa`, `retab`) or 64
instructions (`UNSIZED_LIMIT`). Code is read with `read_code`, which puts each
breakpoint's saved instruction back, so a listing never shows the `brk`.

The listing is lldb's: a header `<image>`<name>:` (`<image>`0x<start>:` when
unnamed; `0x<start>:` when no image holds it), where the name is the written
name for the developer's functions and otherwise the plain name; then per instruction `->  ` at the current `pc` or four spaces, the
address as `0x` and 16 hex digits, ` <+<offset>>:` (` <-<offset>>:` before the
function's start, when a count runs back past it into code no function holds)
padded to the widest in the listing (omitted, `:` alone padded the same way,
when no image holds it), and the instruction. A counted listing that runs out
of the function it starts in — back before its start, or on past its end —
puts a new header before the first instruction of each other function
`function_at` finds holding one, as lldb does, and counts those
instructions' offsets, and the `<+<offset>>` comments, from that function; the
listing's first header is the first instruction's function. An instruction no
function holds stays under the function the listing started in. An `adrp`
seen before a new header is forgotten after it.
[[src/debugger/disassemble.rs:listing]] A comment follows from column 33
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
holds none, and runs as `run` does; a `ret` that holds one of the user's is
waited on all the same (`Breakpoints::finishing`), so a stop there that runs on
for its commands still ends the finish. A stop at one of them by the current
thread with its stack pointer at or above the one it had (so a deeper,
recursive call's return does not count) removes them and steps the `ret`,
then prints
`Thread <t> returned from <function> to <place>: x0 = <value>, x1 = <value>.`
— MFBASIC returns an error code in `x0` and the value in `x1`, each shown as
`list reg` shows it. A stop at them by another thread or a deeper frame runs
on. A stop that runs on for its commands lets the finish go on. Any other
stop — a user breakpoint, a watched change or error, an exception, Ctrl-C,
the end — removes them and is reported as `run` reports it, and, unless the
program ended, followed by `The finish from <function> did not complete; type 'finish'
to run it again from here.` (`<function>` named as `dis` names it; `The finish
did not complete; …` when the function has no name).
[[src/debugger/breakpoints.rs:Finish]] With no function at the
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
(else unchanged); the address has every bit above the host's user addresses
cleared — bits 63–47 on macOS, 63–48 on Linux, which also strips
pointer-authentication bits (`ADDRESS_MASK`). The function for a frame
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
`Dropped the breakpoint on <symbol>: the program no longer has it.` A
breakpoint keeps its test, and one `brk` is written per address, the park's
included. A number in a test (`[0x…]`) is not moved to the new process's
layout. Every watchpoint is dropped first, as Watchpoints says. The
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
