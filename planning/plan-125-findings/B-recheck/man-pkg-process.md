The whole unit has been read and the claims I challenged were checked against source and a compiled probe. Findings follow.

### 1. The package intro is a cut-off sentence
UNIT:      man-pkg:process
PAGE:      package-wide (overview intro, and the `mfb man` package index row)
CATEGORY:  discoverability
CLAIM:     "Spawn and manage child processes: run a program, stream its standard I/O,"
VERDICT:   wrong
EVIDENCE:  `mfb man process --all` line 4 and `mfb man` package index row 49 both end at the comma. `MODULE_INTRO` in `src/codegen/builtins/process/mod.rs` is the same truncated literal; `git log -S'stream its standard I/O,'` shows it landed that way in fbf4da4a7 (the migration). The source it was ported from, `planning/old_man/builtins/process/package.md` line 3-4, reads "...stream its standard I/O, wait for it, and signal or detach it." The second line was dropped on port. The intro is the one line a developer sees in the package index when choosing a package, so this is the most visible defect in the unit.
SUGGESTED: "Spawn and manage child processes: run a program, stream its standard I/O, wait for it, and signal or detach it."

### 2. `signal` sends the reader to the wrong page for the platform table
UNIT:      man-pkg:process
PAGE:      signal
CATEGORY:  discoverability
CLAIM:     "The full platform mapping is tabulated on mfb man process types."
VERDICT:   wrong
EVIDENCE:  `mfb man process types` renders no table (`grep -c '┌' types.txt` prints 0); the Signal table is on the overview page (`mfb man process`, the four-column "Sent on Unix / Sent on Windows / Read back" table). The sibling `didSignal` page correctly says "tabulated on the mfb man process overview page" (`func_did_signal.rs:39` vs `func_signal.rs:35`).
SUGGESTED: "The full platform mapping is tabulated on the mfb man process overview page." (match didSignal's wording).

### 3. Two names for the same timeout argument across siblings
UNIT:      man-pkg:process
PAGE:      poll (vs send, sendBytes)
CATEGORY:  consistency
CLAIM:     poll declares `ms AS Integer` but its Description says "A timeoutMs of 0 checks and returns immediately; a negative timeoutMs waits with no deadline at all"; send and sendBytes declare the same quantity as `timeoutMs`.
VERDICT:   inconsistent
EVIDENCE:  `func_poll.rs:122` has `name: "ms"`; `func_poll.rs:29-30` desc uses `timeoutMs`; `mfb man process send` / `sendBytes` declarations show `[timeoutMs AS Integer]`. Parameter names are public surface for named arguments (`.ai/man-content.md` §2.1), so a developer who copies `timeoutMs := 100` from the poll description to a poll call gets a compile error, while the same spelling works on send.
SUGGESTED: Prose fix only is in scope for a docs pass: make poll's description say `ms` where it now says `timeoutMs`. Separately note for the main thread that the parameter name itself diverges from its two siblings; renaming is a code change, not a prose edit.

### 4. `waitFor` and `signal` disagree on what a signalled child returns
UNIT:      man-pkg:process
PAGE:      waitFor, signal
CATEGORY:  consistency
CLAIM:     waitFor: "a child killed by a signal returns -1." signal (same page, two paragraphs apart): Windows terminates "with a POSIX-flavored exit code (128 + signo, so 137/143/134 for Kill/Terminate/Error) that a later process::waitFor can read back", then its example comments "A signalled child has no exit status of its own, so waitFor gives -1."
VERDICT:   inconsistent
EVIDENCE:  Windows `waitFor` stores the raw `GetExitCodeProcess` value into the exit-code slot and returns it unchanged (`func_wait_for.rs:851-876`, `PROC_EXITCODE`), and `func_signal.rs:212-219` emits exit codes 134/137/143. So on Windows `waitFor` after `signal(child, Terminate)` returns 143, not -1, and the signal page's example takes the ELSE path there. The overview gets it right ("-1 on a signal death on Unix") but waitFor's own page drops the qualifier.
SUGGESTED: waitFor: "a child killed by a signal returns -1 on Unix; on Windows, where process::signal ends a child with exit code 128 + signal number, that code (137, 143 or 134) is what comes back." signal example comment: "' On Unix a signalled child has no exit status of its own, so waitFor gives -1; on Windows it returns 143."

### 5. poll's negative timeout is Unix-only behaviour stated as universal
UNIT:      man-pkg:process
PAGE:      poll
CATEGORY:  consistency
CLAIM:     "a negative timeoutMs waits with no deadline at all, until the stream is readable or the child exits."
VERDICT:   misleading
EVIDENCE:  Unix: confirmed by probe. `/tmp/plan-125-scratch/B-recheck/man-pkg-process/pollproj` runs `process::poll(sh, -1)` on `shell("sleep 1; echo late")`; output `late`, wall time 1.168 s. Windows: `func_poll.rs:319-321` computes `DEADLINE = GetTickCount64() + ms` and the loop exits FALSE once `now >= DEADLINE` (`func_poll.rs:357-359`), so a negative ms is a deadline already in the past and behaves as an immediate check, not an infinite wait. Windows runtime not executed here (no Windows host): code-read only, UNVERIFIED at runtime. The `ms` parameter cell also only describes 0 and positive values, so the negative case appears in one place and contradicts the sibling cell.
SUGGESTED: "On Unix a negative ms waits with no deadline; on Windows a negative value behaves like 0, an immediate check." Add the same sentence to the `ms` parameter description.

### 6. No page says how to get a working directory or environment with a shell command
UNIT:      man-pkg:process
PAGE:      shell (and overview)
CATEGORY:  coverage
CLAIM:     The overview says "A four-argument spawn overload adds a working directory, an environment Map OF String TO String, and a replace-vs-merge flag." shell has only the one-argument form and never mentions cwd or env.
VERDICT:   missing
EVIDENCE:  `mfb man process shell` shows a single Declaration `process::shell(cmd AS String)`; no overload. A developer arriving with "run this pipeline in directory X with FOO set" finds the answer on neither page. It is reachable today by spawning the shell yourself: `process::spawn(["/bin/sh", "-c", cmd], cwd, env, FALSE)` on Unix, and the shell page already documents which shell each platform uses, so the route is one sentence.
SUGGESTED: On shell, after the paragraph on the four-argument spawn: "shell has no working-directory or environment form. To run a command line with either, spawn the shell yourself through the four-argument process::spawn: process::spawn(["/bin/sh", "-c", cmd], cwd, env, FALSE) on Unix, process::spawn(["cmd.exe", "/S", "/C", cmd], ...) on Windows."

### 7. Windows `\r` line-ending note lives only on shell, but applies to every child
UNIT:      man-pkg:process
PAGE:      shell vs receive / spawn
CATEGORY:  consistency
CLAIM:     shell: "Windows programs end their lines \r\n, so a line read from a Windows child keeps its trailing \r; trim it if the value is compared or printed inline." receive, the page that actually returns the line, and spawn say nothing about it.
VERDICT:   inconsistent
EVIDENCE:  receive's description states "including the trailing newline" and "reads until it sees a '\n'" with no mention of `\r`; grep of `func_receive.rs` and `func_spawn.rs` for `\\r` finds nothing, only `func_shell.rs` carries the note. A developer reading receive (the natural page for "what does the returned line contain") or spawning a Windows program directly never sees it.
SUGGESTED: Move or copy the sentence to receive's description ("The line keeps whatever ended it: a Windows child writes \r\n, so the returned String ends in \r\n there; trim the \r if the value is compared or printed inline.") and keep a one-line pointer on shell.

What I also checked and found consistent: every page's "detached or out-of-scope handle raises ErrResourceClosed" story matches the overview; closeInput's "handle stays usable" matches the overview's "no early close"; the Signal and Stream enums on the types page match their use on signal, didSignal, poll, receive and receiveBytes; the overview's errors table is the union of the per-page tables; the function table covers all 14 descriptors in `src/codegen/builtins/process/`; waitFor's 16 MiB cap (`gen_shared.rs:127 SPILL_MAX_CAPACITY`) and receive's 1 MiB line cap (`func_receive.rs:144`) match the pages.
