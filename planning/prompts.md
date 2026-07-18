
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

Make a new worktree...
**Base:** `25c38ba1` (`origin/main`), clean tree

In this work tree I want you to review the code and create bugs specifically for cleanup work. duplicate code, file code order, splitting large files, dead-code removal, reorganization of code in a file or of whole files, restructuring or code in a file or of whole features across multiple files, spec document updates, man document updates, etc. This review is specifically for *cleanup* work, not for bug-finding/correctness issues.

You need to take your time and review everything closely.

