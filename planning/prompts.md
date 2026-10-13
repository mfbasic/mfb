
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

## Cause before fix (once per phase, before step A)
An agent is never handed a symptom to make go away. Before assigning anything, YOU read
the code for every bug in the list (no agent; about 15 minutes a bug, less for the
obvious ones) and write a "Cause" note at the top of its bug doc, five lines at most:
- the rule the code is breaking (cite the spec section or the invariant in `.ai/`; if the
  spec itself states the opposite of what the owner has ruled, say so);
- the one place that rule lives in the code (function and file);
- what following the rule changes there, and what existing special handling it makes
  redundant and deletes.
Then:
- Bugs with the same cause are ONE fix: give them to one agent as one piece of work, in
  one brief, not split across agents and not fixed bug by bug.
- The agent's brief carries the Cause note and says: restore the rule in the place it
  lives; a fix that decides behaviour by enumerating cases (statement shapes, positions,
  wrapper or callee names, type spellings) is rejected at review whatever it measures.
- You need the owner before starting ONLY for a bug where (a) its doc records an earlier
  attempt that failed, was reverted or was narrowed, or (b) you cannot name an existing
  rule and the fix needs a new one. Put all such bugs in ONE message (cause, proposed
  rule, what gets deleted) and wait for the answer on those bugs only; every other bug in
  the phase starts at once. This is the only planned stop of a phase.
A measured experiment beats a paragraph: if the cause is one predicate or one check, try
it in a scratch worktree (`git worktree add --detach /tmp/...`, removed afterwards) and
put the before/after numbers in the note.

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
     bug doc which goldens it expects to change, and why. "Full suite" includes the
     unfiltered unit binary: an agent NEVER runs `cargo test` or `cargo test --bin mfb`
     without a filter (~40 min each, under the one build lock, and the gate in F runs
     them anyway). Unit tests are run filtered to the modules touched
     (`cargo test --bin mfb -- <module>::`). The coordinator must not tell an agent
     otherwise, in a brief or a follow-up, whatever a gate failure seemed to teach;
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
   Review the APPROACH before the claims. Reject, unmerged, a fix that enumerates cases
   instead of restoring the rule in its Cause note, even when every counter is right.
   Three tripwires, checked here and nowhere else (an agent never stops for them, and you
   never stop for anything that is not one of them):
   1. the fix decides behaviour case by case (see "Cause before fix");
   2. after a fix that was reported done, your re-run or a sibling sweep finds more
      failing cases OF THE SAME KIND (another position, another wrapper, another type);
   3. the fix adds more than about 300 lines to one analysis or ownership file, or more
      than about 600 non-test lines for one bug.
   When one fires, do NOT send the agent back for the next case and do NOT ask the owner
   yet. Stop that bug's agent, and spend up to 30 minutes yourself on the code: find the
   rule and the place, and test it in a scratch worktree. If that gives a general fix
   inside what the bug doc and the owner's rulings already settle, write the new Cause
   note, re-brief ONE agent from a clean base (not on top of the rejected work), and carry
   on without stopping. Tell the owner only if it does not, or if the general fix changes
   a contract or replaces a mechanism the owner has not ruled on: one message with the
   cause, the per-case approach you rejected, the general alternative, and the agent
   tokens the bug has used. The rest of the phase keeps running while you wait. A
   tripwire is the ONLY reason to raise the approach; a choice the prompt, the bug doc or
   a ruling already settles is made and noted, never asked.
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
G. Failures: fix them, then go back to E. When the gate catches something an agent's
   targeted checks missed, add that ONE named test or guard to the targeted list in the
   agent brief. Never respond by making agents run the full suite. A failure counts as a flake only if it also
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
- The full suite runs in exactly one place: the coordinator's gate (step F). Agents run
  targeted checks only. Any brief the coordinator writes for agents must repeat step B's
  rules without loosening them.
- Budget: a bug gets at most TWO agent rounds (the fix, and one follow-up for review
  findings). There is no third round. If the second round does not pass review, treat it
  as a tripwire (step C): find the cause yourself, and re-brief once from a clean base
  only with a new Cause note; otherwise the bug leaves the phase with what was tried
  written in its doc. Keep a running total of the tokens each agent's completion notice
  reports, per bug, and put it in every status answer; a bug that passes about one
  million agent tokens is reported to the owner in that answer, with why.
- A status answer says, for each bug in progress, the rule being restored and where
  (from its Cause note), before any counts.
- Watch your own gate: when the full gate (step F) or a merge check runs detached, poll
  its summary file and report the result when it finishes, not when asked.
- One cargo build at a time machine-wide; check `uptime` first.
- Perf bugs run alone, measured at load < 4 against a main build from git archive
  into /tmp.
- macOS has no `timeout` command. In zsh, split SHA lists with ${=L}.
- Status questions: answer from `git log main`, never from plans.
- Memory edits only through a sub-agent (AGENTS.md).

---

## canvas-demo rewrite

### What this is

`examples/canvas-demo` is the project's canvas showcase: a menu ("The Enchanted Grove",
ten scenes, a QUIT button) and ten scenes, about 6,900 lines across 18 files in
`examples/canvas-demo/src/`. It was written against the canvas API as it was before
plan-172 (the shape model) and plan-164 (Bézier curves), and has only been kept
compiling since. It uses none of what those two plans added.

I want it **rewritten, not ported**. The owner's words: "Do a rewrite, not a port… the
canvas-demo is a showcase so it should be done right." A port keeps the old structure
and swaps field names. A rewrite asks, for each scene and each helper, how it would be
written today if the current model had always existed, and writes that.

**This prompt is your go-ahead to edit files under `examples/canvas-demo/`.** It is not
a go-ahead to change the compiler, the canvas package, tests or goldens. If the rewrite
exposes a compiler or canvas bug, follow `AGENTS.md`: reproduce it, then fix it or file
it with `/write-bug`. Do not work around it silently in the demo.

### Read first

1. `AGENTS.md` (repo root). Its rules apply throughout.
2. `planning/new_canvas.md`, everything above `## Future`. This is plan-172's type
   surface. It predates the curve shapes, so the manual is the authority for those.
3. `./target/release/mfb man canvas --all`, then the `types`, `pollMouse`, `setGroup`,
   `present`, `flatten` and `nurbsToBeziers` pages individually. Build the compiler
   first if needed (`cargo build --release`).
4. `./target/release/mfb spec app canvas`. Read "The item unions", "Placement",
   "AutoPaint", "Hit-testing", "A picture is drawn inside a shape", and whatever it
   says of curves.
5. `planning/completed/plan-164-A-bezier-types-reduction.md`, `-B-curves-software.md`
   and `-C-curves-gpu.md`, for what the curve shapes are and where their limits are.
6. The demo itself. Start with `main.mfb`, `demos.mfb`, `menu.mfb`, `kit.mfb`, `ui.mfb`
   and `svg.mfb`, then the ten scenes: `dawn`, `spring`, `twilight`, `alchemist`,
   `canopy`, `wanderer`, `scribe`, `bestiary`, `cathedral`, `lantern`.
7. Small, correct examples of the new features, under `tests/canvas/scenes/`:
   `picture_shapes_on_black.mfb`, `autopaint_on_black.mfb`, `curves.mfb`,
   `curve_strokes_on_black.mfb`, `curves_placed_on_black.mfb`,
   `curve_pictures_on_black.mfb`.

### What changed in the canvas model

Verify each of these against the man pages; do not take them from this list alone.

- **Every shape carries `id`, `position`, `paint`, `transform`.** The transform is on
  the shape, not on `Paint`, and pivots about the shape's `position`. A `Rectangle`
  pivots about the point its `bind` names (`TopLeft` or `Center`). Points of a `Line`,
  `Polygon`, `PolyLine`, `PolyCurve` and `CurvePolygon` are offsets from `position`, so
  moving a shape is changing `position` alone.
- **`Rectangle`** has four radii (`radiusTL/TR/BL/BR`) and `bind`. **`Polygon`** has
  `contours` (a list of rings, so holes are possible) and `rule`.
- **Bézier curves are shapes now (plan-164).**
  - `canvas::BezierCurve` is one segment: `points` (two for a line, three for a
    quadratic, four for a cubic, up to seventeen) and optional `weights`, one per
    point, which make it rational. A quarter circle is three points with weights 1.0,
    0.7071067811865476, 1.0. It is not drawn on its own.
  - `canvas::PolyCurve` is an open path of curves joined end to end, stroked: the
    curved counterpart of `PolyLine`. Joins, caps and dashes apply along the whole
    path. It has no interior and ignores `paint.fill`.
  - `canvas::CurvePolygon` is a closed shape with a curved outline, filled and
    stroked: the curved counterpart of `Polygon`, with `contours` (each a list of
    `BezierCurve`) and `rule` for holes.
  - Curves stay smooth however far the **shape's own** transform enlarges them.
    Enlarging a `Group` they are in does not redraw them at the new size, so a cubic
    can show slight flats there. Zoom with the shape's transform, not the group's.
  - `canvas::flatten` turns a curve into points and `canvas::nurbsToBeziers` makes
    curves from a NURBS curve, for the cases that still need points.
- **`Group` nodes take a full affine transform** (`id`, `position`, `transform`,
  `name`). A group inside a group composes. A figure built once with
  `canvas::setGroup` can be placed, turned and scaled many times for the cost of one
  node each.
- **`canvas::AutoPaint`** wraps a shape and swaps its paint on enter, hover, press and
  exit. The graphics thread does this with no program code running. Fields: `shape`,
  `useEnter/useExit/useHover/usePressed`, `enterPaint/exitPaint/hoverPaint/pressedPaint`.
  It has no `id` of its own; it reports its shape's.
- **`canvas::Picture`** is an image drawn inside a shape: `id`, `shape`, `clip`
  (`PictureClip.ClipOnly` or `ClipAndTransform`), `fit` (`PictureFit.Stretch`,
  `Contain` or `Cover`), `image`. The shape's paint tints, fades, blends and outlines
  it. The shape may be a circle, ellipse, polygon, rounded rectangle or
  `CurvePolygon`, and may itself be an `AutoPaint`. A shape with no inside (a line,
  arc, `PolyLine`, `PolyCurve`, text) draws nothing.
- **The unions.** `Shape` has ten members (the eight original shapes, `PolyCurve`,
  `CurvePolygon`); `AutoShape` adds `AutoPaint`; `DrawItem` adds `Picture` and `Group`.
  An exhaustive `MATCH` over `DrawItem` breaks every time a variant is added — it has
  broken this demo three times.
- **`MouseEvent.overID`** reports which item the mouse is over as an id path joined by
  `::` (group id, then picture id, then shape id, empty parts left out). One group
  drawn twice under two node ids is told apart by its path.
- **Defaults.** `MUT r AS canvas::Rectangle` then `r = WITH r { … }` works for records
  that hold no union or resource. `AutoPaint`, `Text` and `Picture` need full literals.
  There are no constructor functions; do not add any.

### What "done right" means here

These are the places the old demo does by hand what the model now does for it. Treat
them as the minimum, and look for more.

1. **Curves.** The demo flattens every curve to points itself: `kit.mfb` has
   `quadPts` and `cubicPts`, `svg.mfb` flattens every SVG path (`cubicPts`,
   `curveSteps`, `arcPts`) into polygon rings, and several scenes build curved outlines
   as long point lists. Draw curves as curves. An SVG cubic or quadratic segment is a
   `BezierCurve`; an SVG arc is a rational one; a closed path is a `CurvePolygon`
   contour and an open stroked one a `PolyCurve`. Hand-flattening should remain only
   where points are genuinely what is needed, and then through `canvas::flatten`.
   Because curves are redrawn smooth under the shape's own transform, a scene that
   zooms is where this shows best — make sure at least one does, and that it zooms the
   shape and not an enclosing group.
2. **Hit testing.** `ui.mfb` and `menu.mfb` test the mouse against rectangles by hand
   (`inside(px, py, x, y, w, h)`). Replace this with `id`s and `overID`. Clickable
   things get an `id`; the menu's ten tiles should be one group drawn ten times, told
   apart by node id.
3. **Hover and press feedback.** Buttons animate hover in program code today. Use
   `AutoPaint` where a paint swap is the effect wanted. Keep hand animation only where
   the effect is something `AutoPaint` cannot express (a growing or eased transition),
   and say so in a comment.
4. **Placement.** `kit.mfb:placed` and `ui.mfb:restyleItem` are `MATCH`es over every
   `DrawItem` that rewrite each shape's transform so a figure built at the origin can
   be moved as a whole. That is what a `Group` node with a transform is for. Build
   figures as groups and place the node. These two functions are the ones that keep
   breaking; they should not survive the rewrite, and nothing that replaces them
   should `MATCH` exhaustively over `DrawItem`.
5. **Pictures.** `wanderer.mfb` draws every image as a plain rectangle. Find places
   where a shaped picture is the natural thing (the menu's ten circular thumbnails are
   the obvious one; a framed portrait or a clipped scene window is another) and use it.
   Show `Contain` or `Cover` at least once where the proportions matter, and a picture
   inside a curved outline (`CurvePolygon`) at least once.
6. **Local geometry.** Anywhere the demo adds an offset to every point of a polygon,
   line or curve to move it, set `position` instead.
7. **Rounded corners.** Anything faking a rounded or pill shape with extra items
   should use the four radii.

The demo must visibly contain at least: an `AutoPaint` button, a shaped picture, a
rotated or scaled group, a filled `CurvePolygon` and a stroked `PolyCurve` — each
reachable from the menu without special setup.

### Constraints

- **Keep what the demo is.** Ten scenes, the same art direction, the same menu, the
  same keys (number keys, `F` for full screen, `Q` to quit), the same assets under
  `resources/`, and `CREDITS.md`. This is a rewrite of the code, not a redesign of the
  content. If you think a scene should change materially, ask first.
- **Keep reading the keyboard every loop.** `canvas::pollMouse` reports nothing unless
  the program also calls a key reader such as `io::readKey(0)` (open bug-1015). The
  demo already does; do not lose it.
- **Mind code size.** A defaulted record updated with `WITH` costs about 27 KB of
  machine code per statement (open bug-1080). Prefer small builder functions called
  many times over long runs of inline `MUT`/`WITH` pairs. Measure the binary before
  and after, and report both numbers.
- **Mind frame cost.** The demo animates from wall-clock time. Things that do not
  change between frames belong in a group installed once, not rebuilt per frame.
  Compare frame cost before and after on the scenes whose curves you converted: a
  curve is not free, and "fewer points" is not automatically "faster".
- **Do not touch** `src/`, `tests/`, goldens, or other examples, except to fix a bug
  you can reproduce outside the demo.
- Update `examples/canvas-demo/README.md` to describe the demo as rewritten. The repo
  has guards for example READMEs and project names:
  `cargo test --test example_readmes --test example_project_names`.

### How to work

1. **Baseline first, before changing anything.** Build the demo and capture a frame of
   the menu and of each of the ten scenes, so you have something to compare against:

   ```sh
   cargo build --release
   ./target/release/mfb build --debug examples/canvas-demo
   ```

   Run headless with a frame dump and stats. Hold stdin open, because the demo quits
   on end-of-input:

   ```sh
   sleep 6 | MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 \
     MFB_CANVAS_DUMP=/tmp/demo/frame.rgba MFB_CANVAS_STATS=/tmp/demo/stats.txt \
     examples/canvas-demo/build/canvas-demo.app/Contents/MacOS/canvas-demo
   ```

   The dump is raw RGBA8, 900 pixels wide. `MFB_CANVAS_DUMP` and `MFB_CANVAS_STATS`
   need a `--debug` build. That command shows the menu; work out how to reach each
   scene headless (read `main.mfb`; add a small, documented environment switch if
   there is no way). Keep scratch files in `/tmp`, never in `scripts/`.

2. **Tell me the plan before the bulk of the work.** After reading, send a short
   outline: the new file layout, what replaces `kit.mfb`, `ui.mfb` and the flattening
   in `svg.mfb`, which scenes get which new features, and anything you want to change
   about content. Then carry on unless I object; do not wait for a reply.

3. **Rewrite scene by scene**, building and looking at a dumped frame after each one.
   The scene should look like its baseline except where a new feature is deliberately
   on show — and where a flattened curve became a real one, which should look the
   same or smoother, never different in shape. Convert a frame to PNG and actually
   look at it; a build that compiles proves nothing about a picture.

4. **Verify at the end:**
   - `./target/release/mfb build examples/canvas-demo` and the `--debug` build both
     succeed with no warnings.
   - Every scene and the menu render headless on the software renderer and with
     `MFB_CANVAS_GPU=1`. On the GPU run the stats line shows `gpuFrames` equal to
     `frames`; a scene that silently falls back to software is a finding, not a pass.
   - Hover and click work. Use the `--debug` pointer script, for example
     `MFB_CANVAS_POINTER="0,x,y,0,1"` (milliseconds, x, y, buttons, inside), and check
     the `auto=` field of the stats line changes for an `AutoPaint` button. For clicks,
     `MFB_MOUSE_INJECT` feeds SGR mouse reports to `pollMouse`; see
     `tests/canvas/rt_canvas_hit.rs` for the format.
   - `cargo test --test example_readmes --test example_project_names` passes.
   - A real-window run on this Mac if I am not using the machine. Ask before taking
     over the pointer.

5. **Commit** on the current branch, itemised, only the files you changed. Do not
   create or switch branches.

### Machines

Use this Mac for everything. If you want a second platform, box 206
(`test@192.168.1.206`, Windows, real GPU, x86_64) is the only remote worth using; read
`.ai/remote_systems.md` first. Do not run anything on an emulated x86_64 box, and ask
before using any other remote machine.

### Report back

- What each scene now uses from the current model, one line per scene.
- What you removed (helpers that no longer exist) and what replaced them.
- Line count and binary size before and after, with the commands that produced them.
- Frame cost before and after on the scenes whose curves changed.
- Anything in the canvas API that made the rewrite awkward. That is useful feedback on
  the model itself.
- Any bug found, with its repro.
- What you did not verify, said plainly.
