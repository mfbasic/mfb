
please review src/docs/spec/language and verify it, correct it and expand it to it is accurate to the actual current compiler codebase.

you can use `/mfb spec language --all` at any time to view the current specification (after a fresh build)

This documentation should be:
1) Accurate, no inaccurate information
2) Complete, no missing information
3) Targeted towards compiler implementers, not developers


---


I would like you to convert specifications/package_format.md into a the new src/docs/spec/package/* layout
everything in the package_format.md should end up in src/docs/spec/package/* nothing missing or dropped.


--

Everything is now internal and internally referenced, nothing external remains. The next part is a larger re-org...
I want one source of truth for anything. For example if the language specificaiton talks about threading in details, and the threading spec covers all that and more, then things live in 2 places.

I dont mind if small things life in 2 places, I dont want a rats-nest of references. Having a summary or even a comprehensive (detailed) summary in one location then referncing the main location is fine.

small facts being inlined is fine. again I dont want a rats nest of references.

to this end, I would liks a complete reorganiaiton of the specifications. You may move things between specifications, you make create new sub topics, you may remove exisitng sub topics, you may merge sub topics, you may add whole new specificaitons or whole new sub topics where needed.

1) Everything must be accurate to the actual compiler code (most important)
2) Everything must be complete, I would rather have an accurate stub than something missing or incorrect
3) No, non-verifiyable information.
    - You may add [[path/**/file.*:line]] references in the document.
    - Update the MD display engine to not display [[ ]] references.

Thoughts? Agreed?

## Security Review

/goal Produce a code-grounded security review of the language, compiler, runtime, linker, and package registry as they are implemented today — not a general bug hunt and not a spec-only read. Every finding must be verified against source (and, where practical, reproduced against a built `mfb`). Write artifacts under `./planning` and file bugs under `./bugs`; do not modify other trees.

This is a **security** review: prioritize attacker-reachable impact (memory unsafety, trust/auth bypass, injection, privilege escalation, supply-chain substitution, sandbox escape, DoS that an untrusted party can trigger, missing crypto/verification, weak executable hardening). Skip pure correctness, polish, or missing features unless they create a security boundary failure.

### Scope

In scope (read and cite):
- `src/**` — frontend, IR/package decode, typecheck/monomorph, codegen, runtime helpers, custom linker, CLI install/build/import paths
- `repository/**` — package registry HTTP service, store, crypto, auth
- Specs under `src/docs/spec/**` only where they define security-relevant intended behavior you are checking the code against

Out of scope for edits: everything except `./planning/**` and `./bugs/**`. Do not fix issues in this pass — document them.

Prior art (read first; do not re-open fixed items as new findings without re-verifying against current code):
- `planning/security-review-1.md` (early, mostly spec-level)
- `planning/old-plans/audit-1-*.md` (previous code-grounded audit)

### Outputs

1. **Audit files**, split by surface (package decode, codegen/memory, frontend, fs/net/thread, linker, repository, …) as needed:
   - `planning/audit-<N>-<surface>.md` where `<N>` is the next free audit series number
   - One index: `planning/audit-<N>-summary.md` with a master finding table (ID, severity, title, location, cross-links)
2. **Bug documents** via the write-bug skill for every **CRITICAL** and **HIGH** finding (and for MEDIUM when the fix is not small-ish). Use `bugs/bug-NN-shortname.md`; do not implement fixes here.

### Finding requirements

Each finding must include:
- **ID** and **severity**: CRITICAL / HIGH / MEDIUM / LOW / NTH (nice-to-have)
- **Title** and **location** cited as `path/file.ext:line` (or symbol) after a real source read
- **Threat / impact** — who can trigger it, and what breaks (confidentiality, integrity, availability, trust)
- **Mechanism** — why the code is wrong, not just that it feels risky
- **Reproduction** — preferred: a minimal MFBasic (or HTTP/CLI) example that triggers it against a built binary; if pure decode/linker, a concrete byte/command repro. Record observed vs expected.
- **Best fix** — implementation-level; **must not change the MFBasic language** (syntax, observable runtime semantics, or spec surface). Registry fixes are ordinary service-code changes.
- **Non-goals** for that fix (what must stay the same)

Label only what you can support. "Not demonstrated" is allowed when the code path exists but you could not exercise it; do not promote those to CRITICAL without evidence.

### Method

1. Map trust boundaries first: untrusted `.mfp`/IR, untrusted source, network peers, registry clients, generated native binaries, filesystem paths, cross-thread ownership.
2. Fan out by surface (parallel subagents are fine); each agent returns findings only, with citations.
3. Re-verify every finding yourself against current source before writing it down — discard hallucinations and fixed-already items.
4. Write the audit files and summary; file write-bug docs for CRITICAL/HIGH (and qualifying MEDIUM).
5. Do not implement fixes in this pass.

### Done

You are done when: the summary index exists; every in-scope surface has been covered or explicitly marked out-of-scope with a reason; every CRITICAL/HIGH has both an audit entry and a bug document; each finding meets the requirements above; and no code outside `./planning` and `./bugs` was modified.

## Specification Review

/goal Verify the embedded specification is up to date and 100% correct to the current code. "Correct" means each file's factual and implementation claims match what the code actually does — not merely that its citations resolve. Do a deep, per-file audit of every file, not a skim.

A specification describes *behavior*, not the code that implements it. Outside of `[[…]]` citations, the prose must not name source files or functions, and must not describe incidental, of-the-moment implementation details — it states what the compiler does and cites where the behavior lives, so the citation carries the provenance and the prose stays durable. If a claim can only be phrased as "this particular implementation happens to…", it belongs in a citation's target, not in the spec. Any such prose that already exists — a bare file or function name outside a citation, a reference to project history or planning, or a passage about this specific implementation's incidentals — must be removed or rewritten as part of this pass, not just left in place; cleaning up existing violations is as much the job as fixing inaccuracies.

The specs must be self-contained: they reference nothing outside the compiler source and the spec tree itself. This applies to citations too, not only prose. A `[[…]]` citation may target only compiler source or in-tree spec files — never a path under `planning/**` (or any planning, history, audit, or old-plan document). Such a citation is itself a violation: re-cite the claim to the source that implements the behavior, or drop the claim. This is not caught by the prose rules above, so treat it as its own class of violation to hunt for and fix.

Method (proven; follow it):
1) Fan out ~10 parallel subagents, each owning one spec package. Split the two largest packages across two agents each so no agent owns more than ~15 files. Instruct each agent to (a) verify EVERY factual claim in its files against the source and report provable mismatches, one per line: `<file>:<line> — CLAIM "<quote>" — CODE <what the source actually does, with a citation> — FIX <correction>`; and (b) flag every purity violation — prose outside a `[[…]]` citation that names a source file or function, references project history or planning, or describes an of-the-moment implementation incidental, AND any `[[…]]` citation whose target resolves under `planning/**` or any planning/history/audit/old-plan document — as `PURITY <file>:<line> — "<quote>" — <suggested rewrite>`. "<file>: OK" only when a file is both accurate and clean; a separate UNVERIFIED section for suspicions it cannot prove. Its final message is the findings list only.
2) Verify every finding yourself against the source before editing — agents hallucinate line numbers and occasionally invert a fact. A contradicting read means the agent was wrong; discard it.
3) Fix and commit per package, with itemized commit messages. Commit as you go.
4) Also hunt for real, reachable behavior that has no spec page at all — an undocumented capability is a correctness gap. When adding a page, seat it logically among its siblings.

Gates (all must pass before done): the project builds cleanly; the specification and generated-artifact test suites pass; every citation resolves across the whole tree (the path exists, a line anchor is in range, a symbol anchor names a real definition), with a non-trivial count and zero unresolved; zero citations resolve to a path under `planning/**` (or any planning/history/audit/old-plan document); and every new or edited topic renders through the spec viewer.

Only `src/docs/spec/**` may be modified; leave all other files as found. This is a documentation pass, not a code-fixing one — so if the audit turns up an actual bug or defect in the source (behavior that is wrong, not merely undocumented), do not fix it here: capture it as a bug file and document the spec to the *correct* intended behavior, noting the discrepancy in the bug file.

## Bug fix

/goal Create a tests/* to verify the bug, the fix and so regressions can't happen, then fix each of the bugs in the ./bugs/* folder

**verify on:**
- ssh -p 2223 test@127.0.0.1 # Kali (libc)
- ssh -p 2227 test@127.0.0.1 # Alipine x86_64 (musl)
- ssh -p 2228 test@127.0.0.1 # Ubuntu x86_64 gtk (libc)
- ssh -p 2229 test@127.0.0.1 # Alipine riscv64 (musl)

## Benchmark Review

I would like you to review the benchmark/*.log files I just ran. then Review the benchmark code and finally the runtime code in src/

Verify yourself, but startup time should already be **excluded** from the logged values. We are working on aarch64/macOS as the target for this work.

### These are my goals in priority order:

1) All MFB (MED) times are less than python times.
2) All MFB (MED) times are not more than 10ms more than c-O0 times.
3) All MFB (MED) times are not more than 5ms more than c-O0 times.
4) All MFB (MED) times are not more than 5ms more than c-O2 times.

Those priorities should define what order things are worked on. If a time doesnt pass #1 its a higher priority to work on... if it passed #3 its a lower priority, if it passed #4 then its already complete.

*Note:* The <= 5ms rule overrides the goals. If a MFB (MED) time is <= 5ms it is already complete regardless of #1–#4, as it is within the margin of error.

*Note:* Otherwise, a benchmark is "complete" only when it beats all 4 goals. A benchmark could beat Python yet fail #2–4, or (rarely) the reverse — either way it is not complete.

*Note:* MED is the median of the same run-count captured in the existing logs; when re-measuring after a fix, use the same run-count so the comparison is fair.

### Constraints on the fixes:

1) It must produce correct output.
2) It must *not* change the MFBasic language — neither its observable runtime semantics nor its syntax/grammar/spec.

### Tasks:

1) Create an ordered list of what benchmarks and their priority.
2) Create a new plan with sub-plans for each performance fix to work on. You can group multiple benchmarks together if a single performance fix will work for them all.
2a) Do some exploring before writing the plan as I would like more than "guessing"
3) Create a new plan with a list of additional benchmarks to add that should be tracked as critical MFB features. Cover the hot paths users actually hit — map/collection churn, float & transcendental math, string/unicode, io buffering, regex, arena stress — and do not duplicate what the current suite already exercises.

### Done:

You are done when all three artifacts exist: the ordered priority list (Task 1), a fix plan whose sub-plans cover every non-"complete" benchmark (Task 2), and the new-benchmark plan (Task 3). This is a plan-authoring task — do not implement the fixes.

---

## cleanup review

Make a new worktree...
**Base:** `25c38ba1` (`origin/main`), clean tree

In this work tree I want you to review the code and create bugs specifically for cleanup work. duplicate code, file code order, splitting large files, dead-code removal, reorganization of code in a file or of whole files, restructuring or code in a file or of whole features across multiple files, spec document updates, man document updates, etc. This review is specifically for *cleanup* work, not for bug-finding/correctness issues.

You need to take your time and review everything closely.

---

## security review

Make no changes - there is another agent working in tree.

I would like you to review the **runtime** code looking for buffer overruns, use after free, and other vulnerabilities that need closed so apps compiled with this compiler (using the runtime) are secure. Any vulnerabilities should be logged as bugs/sec-NN using the existing bug template.

---

## Bug Fixing 1st:

review all the currently open bugs.  Give me an overall order (phases) to work through them.


## Bug Fixing 2nd:

You coordinate fixing the open bugs in bugs/. Agents fix; you review their code and
re-check every claim. You are in charge: nothing reaches main that you have not verified.

WORK ORDER: phase <N> of the bug order (list the bug numbers here, or point at the doc).

## Scope freeze (the rule that outranks every other step)
The bug list of a phase is FIXED the moment its first agent starts. Write the list down
(bugs/status.md) before assigning anything. After that, NOTHING is added to the running
phase, for any reason:
- not a bug found during the phase, however severe or however small;
- not a bug another session files, and not one the owner mentions mid-phase;
- not a bug pulled forward from a later phase because it "pairs with" one in this phase;
- not extra scope on a bug in the list. If an owner decision makes a bug bigger than its
  doc, the bigger part becomes a new bug for a later phase.
All of those are filed (step D) and wait. Do not ask the owner whether to add something
to the running phase; the answer is no. Only the owner can change this, and only by
saying so unprompted.

The only work that may be added is a fix for something THIS phase's own changes broke
(a review finding, an integration clash, a gate failure). That is finishing the list, not
growing it.

Things leave the list freely. A bug that stops on an owner decision, cannot be made
green, or is still not done when the rest is ready is moved OUT to a later phase; it
never holds the merge. Never start an agent on later-phase work while this phase has
anything unmerged: it competes for the one build lock and delays the merge.

The goal of a phase is its fixes on main. Land the list, then start the next one.

## Setup (once per run)
1. Old staging: list every `bug-staging*` branch and worktree with its merged/unmerged
   status (`git branch --merged main`, `git log --oneline main..<branch>`). Remove only
   the ones fully merged into main. Report any with unmerged commits and leave them.
   Never touch any other worktree or branch; other sessions own them.
2. Create one staging worktree, branch `bug-staging-<NN>` from main, with
   CARGO_TARGET_DIR=/tmp/bs<NN>-target. Symlink <tree>/target/release/mfb to that build:
   test-accept, man-run-examples and man-census default to ./target/release/mfb
   (man-run-examples also honors MFB=).
3. Main build and debugger: build main once (CARGO_TARGET_DIR=/tmp/main-dbg-target
   cargo build --release), then `sh scripts/sign-debugger.sh
   /tmp/main-dbg-target/release/mfb`. This binary is both the debugger and the
   "main compiler" for golden proofs. Rebuild it if main moves, and re-sign after every
   rebuild (the signature drops).
4. Baseline: if there's no current one, run the full gate (step F) once on main. Record
   main's SHA and which tests already fail there. Don't chase those during the phase.

## Phase loop
A. Assign bugs: at most 2 agents at a time (never a third, not even for a "quick"
   follow-up; queue it), at most 5 bugs each, worked one bug at a time. A phase can take
   several rounds, but every round draws only from the phase's fixed list. Choose the model per .ai/sub-agents.md (Opus
   for concurrency, ABI and memory-safety bugs). Each agent gets its own worktree
   branched from staging, its own CARGO_TARGET_DIR, and the same target/release/mfb
   symlink as step 2.
B. For each bug, the agent:
   - reproduces the bug and writes a RED test, then fixes it;
   - runs ONLY the targeted checks for that bug: `cargo test --test <name>`,
     `scripts/test-accept.sh <mfb> <outdir> <fixtures>`, `scripts/artifact-gate.sh
     <mfb> all` (~3 min) if codegen moved, man examples for touched packages,
     spec-census --render/--citations for spec edits;
   - also runs any tests/guards/* covering the files it touched (no_type_strings, the
     declared-errors guards, drop-count/helper-scratch tests);
   - does NOT run the full suite and does NOT regenerate goldens. It records in the
     bug doc which goldens it expects to change, and why;
   - STOPS that bug and reports if the fix would change an observable contract beyond
     what the bug doc already settles. The coordinator asks the owner, and an answer is
     not a go-ahead (AGENTS.md "Questions");
   - updates the spec topic in the same change and moves the bug doc to bugs/completed/;
   - commits itemized, with explicit paths only (never -A); commits never mix two bugs;
   - builds probe programs with its OWN worktree compiler (`mfb build --debug -O0`)
     and runs them under the signed debugger /tmp/main-dbg-target/release/mfb
     (`printf 'cmd\n…' | <dbg> dbg prog`; useful: wp, bp X list reg …, be any /
     show error, finish, bt). It reports every debugger issue, failure or feature
     request to the coordinator, with a "how it functioned / issues / recommended
     features" note;
   - at the end of its round (not per bug): `rustup run 1.96.0 cargo fmt --all --check`
     and `cargo clippy --all-targets`. Never `cargo clippy --fix`: it silently degrades
     the hi/lo float constants in gen_pow.rs and builder_simd_float_math.rs;
   - ends with a report: commits, tests run with their results, and every claim with
     the command behind it.
   Probe syntax reminders: `Map OF K TO V { k := v }`, `END WHILE` (not WEND), `toInt`
   (not toInteger).
C. Review every bug yourself: read the diff, re-run the repro and the RED→GREEN test,
   re-run at least one decisive claim from the report, check the spec update, and check
   that no stale comment contradicts the new behavior. Never accept "done" on trust.
   Merge accepted bugs into staging, and add a separate integration commit when parallel
   fixes clash (a retired field, a new parameter). Add debugger feedback to
   debugger-report.md (untracked, main checkout).
D. New bugs found during the phase: reproduce them yourself before writing a bug doc,
   file each one, and place it in a LATER phase in bugs/status.md. Filing is all that
   happens to it now: no agent starts on it, and it is never added to the running phase
   (see "Scope freeze"). Check `ls bugs` in every worktree before taking a bug number;
   other sessions file bugs too.
E. Goldens. Once every bug in the phase is merged into staging:
   1. run `scripts/artifact-gate.sh <mfb> all`. Its diff list is the set to prove;
   2. prove each diff with the 4-question proof (AGENTS.md): a function-level diff of
      old vs new (normalize labels and data symbols); trace each changed function to a
      bug's predicted list; show that main's compiler (/tmp/main-dbg-target/release/mfb)
      reproduces the old golden. An unpredicted diff means you hunt the bug, not
      re-baseline;
   3. regenerate only the proven fixtures (`scripts/sync-goldens.sh <mfb> [globs]`,
      `scripts/regen-native-goldens.sh <mfb> [fixture-dir...]`). Then `git diff --stat`
      must match E.1's proven list exactly; anything extra is an unpredicted diff → hunt
      it. Commit with the proof.
   The full gate in F then satisfies the "full suite before re-baselining" rule; this
   order is deliberate.
F. Full gate. Freeze staging, then:
     cargo build --release
     cargo test --no-fail-fast                          (~65 min)
     scripts/test-accept.sh target/release/mfb <out>    (~8 min)
     scripts/man-examples-gate.sh target/release/mfb     (~12 min)
     sh scripts/check-generated.sh
     rustup run 1.96.0 cargo fmt --all --check
     cargo clippy --all-targets
   Log each step with rc and seconds; grep the logs for FAILED/panicked. Never modify
   the tree while the gate runs, and no builds by you or agents. Timing tests
   (rt_big_int_cost, rt_compress_bounds) false-fail under load, so the next phase's
   agents wait for the gate to finish. To restart, stop the gate and pkill its
   cargo/test-accept processes first.
G. Failures: fix them, then go back to E. A failure counts as a flake only if it also
   fails on main over N runs (300 for anything under ~1 s; otherwise as many as fit in
   ~10 min); then file a bug for it. If a bug's fix can't be made green, revert it out
   of the phase rather than hold the merge.
H. Merge: if main gained commits since the gate, run `git merge --no-edit main` in
   staging, rebuild, and run artifact-gate all plus the targeted tests for the files
   main changed. If main changed compiler/codegen broadly, re-run the full gate. Then in
   the main checkout, `git merge --ff-only bug-staging-<NN>`; if it fails because main
   moved, repeat from staging. Don't push. Leave other sessions' uncommitted and
   untracked files alone. Delete /tmp/bs<NN>-target and the agents' targets afterwards.
I. Move on to the next phase.

## Discipline
- A started phase never grows (see "Scope freeze"). When in doubt, file it and move on.
- One cargo build at a time machine-wide; check `uptime` first.
- Perf bugs run alone, measured at load < 4 against a main build from git archive
  into /tmp.
- macOS has no `timeout` command. In zsh, split SHA lists with ${=L}.
- Status questions: answer from `git log main`, never from plans.
- Memory edits only through a sub-agent (AGENTS.md).
