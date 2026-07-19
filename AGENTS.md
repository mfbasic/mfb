# Agent Instructions

Universal rules below. Before a given kind of work, also read the matching `.ai/` file.

---

## 🛑🛑🛑 STOP — READ BEFORE YOU TOUCH A FAILING TEST 🛑🛑🛑

> ### **<ins>A FAILING TEST IS A CLAIM THAT YOUR CHANGE IS WRONG.</ins>**
> ### **<ins>THE BURDEN IS ON YOU TO DISPROVE IT — NEVER ON THE TEST TO JUSTIFY ITSELF.</ins>**

**DO NOT edit, delete, weaken, re-baseline, or "update" a test — or a golden — to make
your change pass.** Not until you have *proven* the test is wrong. "This test encoded
the old/broken behavior" is a story that always fits and is never evidence. If you
find yourself writing that sentence, **stop: you are about to destroy the only signal
that your change is broken.**

Before touching any test or golden you must be able to answer all four, from evidence,
not from reasoning about what "should" be true:

1. **When and why was this assertion written?** (`git log -S`, `git blame`, the
   originating bug/plan doc.) Deliberate assertions look exactly like stale ones.
2. **What was it protecting?** State the behavior in one sentence. If you cannot, you
   do not understand it yet.
3. **Who else depends on that behavior?** Grep the whole tree — other layers, other
   crates, downstream consumers, the spec. A feature you think is dead is usually
   modelled somewhere you have not looked.
4. **What independent evidence says the assertion is wrong?** A reproduction, a spec
   citation, a sibling function's contract. Your own change is not evidence.

**If you cannot answer all four: the test wins and your change is suspect.** Editing a
test is the *conclusion of an investigation*, never the first move toward green.

**This is not hypothetical — it nearly shipped a wrong fix (bug-288, 2026-07-18).** A
change to reject `PRIVATE RESOURCE` broke `scope_privates::tests::renames_private_decls_and_rewrites_references`.
The test was edited to fit, with a confident comment claiming it "asserted the
half-applied behavior." That claim was never checked. It was false: `ir::lower` maps
`Visibility::Private` for resources and two IR tests cover that arm *on purpose* —
`PRIVATE RESOURCE` is a modelled feature and the fix was wrong. It was caught only
because a *second* test failed somewhere harder to explain away. Had that second test
not existed, a wrong fix would have landed behind a gutted test and a confident commit
message. **The first test was saying exactly this. It was silenced.**

Two aggravating factors to watch for in yourself:

- **You will be most confident precisely when you are wrong.** By the time a test
  fails you have already concluded your change is correct, so the failure gets filed
  as "stale test" instead of "evidence against me." Skepticism aimed at bug reports
  and other people's code is worthless if none is aimed at your own diff.
- **Narrow test filters compound this.** Running `cargo test <one_module>` instead of
  `cargo test` hides the evidence; editing the test then destroys it. bug-288 was
  landed after running only `cargo test ast::`, so the failure was not even seen until
  a later merge forced a full run.

Tests *do* sometimes enshrine bugs — bug-309 found eleven goldens with a live failure
recorded as expected output, and the suite defended it. That is real, and it is why
"the test is wrong" is such an easy story to reach for. **The difference is proof.**
bug-309 was proven: reproduce the failure, fix the cause, show before/after. bug-288
was merely asserted. Same sentence, opposite epistemic status.

---

## Always

- **Done means verified.** Asked if work is done/complete/verified: answer **yes**
  or **no** on the first line, nothing before it. Say **yes** only after proving the
  actual goal holds (compilation, passing tests, and matching goldens are proxies,
  not verification). When unsure, **no** — then one short line on what's left, no
  status report unless asked.
- **Finish the task — do not stop mid-task.** When asked to finish a plan or to complete
  a plan or to work until done... "Done", "finish", "complete", "in full" is the whole task complete
  and verified, not a phase boundary, a plausible stopping point, or a place to hand
  back for confirmation. Stopping early to report progress, ask whether to continue,
  or wait for approval on the next obvious step wastes hours and tokens — keep going
  until the goal holds or you hit a genuine blocker (a destructive irreversible
  action you're unsure about, a real ambiguity that changes the outcome, or an
  external dependency you cannot resolve). At a blocker, state it plainly and
  proceed with the best default where one exists; never declare done while work
  remains. By acting under these instructions you confirm you have read and
  understood this rule and the "Done means verified" rule above.
- **Never edit a test to fit your change.** See the STOP section at the top of this
  file. A failing test is evidence your change is wrong; disprove it with git history,
  a tree-wide search for other consumers, and independent evidence before you touch
  it. Same rule for goldens: regenerating one while a bug is live enshrines the bug
  and the suite then defends it (bug-309).
- **Production-ready only.** Implement the complete behavior with real error
  handling and integration. No stubs, placeholders, mocks, default-result
  fallbacks, simulations, or "unsupported" stand-ins unless explicitly asked. If
  blocked, state the blocker plainly — never fill the gap with non-functional code
  or call it done.
- **Never blanket-suppress dead code.** A file-level `#![allow(dead_code)]` is
  banned: the tree has none, and `cargo check --all-targets` is clean, so a new
  dead item is reported the moment it appears. If an item must stay without a
  reader, give it a *targeted* `#[allow(dead_code)]` (or `#[cfg(test)]` when a
  test is the only consumer) plus a comment saying what makes it load-bearing —
  a spec `[[path:symbol]]` anchor, a layout/enumeration slot, an integrity
  guard. Never write "consumed by a later phase": bug-326 found a dozen such
  promises whose phases had landed by another route or been dropped, and three
  attributes that had become outright false — the item they suppressed was
  heavily used. If it is neither used nor load-bearing, delete it.
- **Git.** Never create/switch/rename a branch unless asked — commit on the current
  branch (even `main`). Never run tree-wide `git checkout`/`reset`/`restore`/
  `stash`; only touch and commit files you changed this session, leaving all others
  as found (other clients share this tree). Use detailed, itemized commit messages
  (imperative subject + `-` bullets); never include unrelated changes.
- **MCP tools.** The `mfbasic` MCP server (`mfb_man`, `mfb_spec`) and other MCP
  tools arrive deferred — names only, no schemas. At the start of each context run
  `ToolSearch` to load the schemas you need before answering questions about the
  language, spec, or built-ins; prefer `mfb_spec`/`mfb_man` over reading files by
  hand. Schemas load per context, so re-run `ToolSearch` after a fresh context.
- **No compound background jobs.** A background Bash job must be exactly ONE
  command. Chaining (`a && b`, `a; b`, timing wrappers) in a backgrounded job
  dies silently here: later steps never run, the job looks "done", and a
  ~15-minute golden cycle is wasted. A short pipe on one step (`cmd | tail -1`)
  is fine. Sequence long steps as separate jobs, but never park waiting on a
  completion notification — it is lost across a context compaction, stranding
  the run as "No completion record found" and stalling the session. Instead
  poll for the effect (`git status` for a golden sync; `pgrep -f` takes ERE —
  `pgrep -f "a|b"`, never `"a\|b"`, which never matches). On resume, treat any
  no-completion-record job as dead: re-derive state and continue, don't wait.

## Read before that kind of work

- Compiler / built-ins / IR / native codegen / runtime helpers / diagnostics →
  `.ai/compiler.md` (runtime completion gate, validation & function tests, register
  lifetimes).
- Creating or updating a man page (`src/docs/man/**`, Markdown) → follow the templates
  exactly: `.ai/man_template.md` for a per-function page, `.ai/man_type_template.md`
  for a package's consolidated `types` page, `.ai/man_package_template.md` for a
  package overview. Keep every section name and order; fill in all `<...>`
  placeholders; omit optional sections only when they do not apply. The templates are
  bare skeletons — authoring rules live in the driver scripts (`scripts/update_man.sh`
  for function/type pages, `scripts/update_man_package.sh` for package overviews).
- The embedded spec (`mfb spec`, `src/docs/spec/**`) → `.ai/specifications.md` (keep it
  current with every compiler change).
- Remote test machines → `.ai/remote_systems.md`.
