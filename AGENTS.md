# Agent Instructions

## Questions

**"Thoughts?" means discuss.** A design question is not a request to implement.
Answering clarifying questions settles the design; it is not a green light.
Wait for an explicit go-ahead before editing any file. This outranks "finish the task."

**A question is not permission.** Any question, not just "thoughts?", asks for an answer and
grants no permission to change anything. Nothing on disk changes until an explicit go-ahead:
"do it", "implement it", "go". Unsure whether you have one → you don't. Ask.

**The user answering your questions is not permission to make changes.** Asking clarifying
questions and getting answers is still discussion. Collecting decisions is not being handed a
spec, and an answered question is not a go-ahead. Go back to discussing, and ask for one.

## Never edit a test/golden to pass

Don't edit/weaken/re-baseline a test/golden until PROVEN wrong.

* First answer 4 from evidence:
  (1) when/why written (`git log -S`, blame, bug/plan doc);
  (2) behavior it protects, 1 sentence;
  (3) who else depends (grep tree+spec);
  (4) proof it's wrong (repro/spec cite/sibling contract; your change is never proof).

* Not all 4 → test wins, STOP.
* Run the full suite, never one module.
* Once proven wrong: fix the bug AND correct only the disproved line (never re-baseline a whole file); show proof in commit.

## A claim is measured or a guess

* Number/count/status/"X does Y" → give the command behind it in the same sentence,
  else say "guess" (not "~").
* Green gate = nothing *covered* changed.
* Unexpected diff in a golden or generated artifact (incl. a step predicted neutral)
  = bug-hunt trigger, NEVER proof a design is dead: inspect ONE fixture to localize before
  concluding. Almost always a bug you just introduced or a wrong prediction — fix/correct it,
  continue. A diff on a target you EXPECTED to change is the plan working, not failing.
* Cite symbol+command, never a line alone.
* Sources disagree → run the command.
* Before calling a citation dangling, check every directory that kind of document can live in,
  including archive/completed ones; a fixed bug may have no doc.

## Always
* Done means verified. Asked if done: yes/no on line 1;
  yes only after proving the goal (compile/tests/goldens are proxies);
  unsure→no + what's left.
* Finish the task. Done/finish/complete = whole task done+verified, not a phase boundary.
  Continue until goal holds or a genuine blocker (irreversible action, real ambiguity,
  unresolvable dep) — state it, use best default.
* Never leave a bug you found — fix it now, outranking scope. Not excused by
  out-of-scope/another-doc/churn/pre-existing (verify at HEAD via `git worktree add --detach`).
  Too large = blocker on line 1 with repro.
* Production-ready only. No stubs/placeholders/mocks/fallbacks/simulations/"unsupported"
  unless asked. Blocked → say so, no dead-code filler.
* No blanket dead-code suppression. No file-level `#![allow(dead_code)]`;
  use targeted `#[allow]`/`#[cfg(test)]` + comment why load-bearing
  (never "consumed by a later phase"). Else delete.
* Git. Never create/switch/rename a branch unless asked;
  Never tree-wide `checkout`/`reset`/`restore`/`stash`;
  touch+commit only files you changed. Itemized commits.
* No compound background jobs — one command each. Don't wait on completion notices;
  poll the effect (`pgrep -f` ERE `"a|b"`). No-completion-record job = dead; re-derive.

## Auto memory rules
* Record only durable, transferable lessons — things that would burn a future session
  writing code (gotchas, ABI/codegen invariants, tooling traps, rules the source doesn't reveal).
* Never record ticket/plan STATUS in memory: no "DONE", "MERGED", "ARCHIVED", "IN PROGRESS",
  "next: ...", commit hashes, or worktree state. Git and the bug tracker own that.
* When a bug is fixed: do NOT leave it in memory as a status line. If the fix taught a durable lesson,
  record ONLY the lesson, stripped of ticket state. If it didn't, record nothing. Never keep a completed/archived plan in MEMORY.md.
* Edit memory in a sub-agent, never on the main thread (`.ai/sub-agents.md`). A memory write
  drags the index in with it — read it, rewrite it, keep it under its size limit — and none of
  that concerns the task the user actually asked for, so doing it inline floods the
  conversation with index content. Dispatch ONE agent with the whole job; take back a one-line
  confirmation, not the file. Apparent size is no excuse to skip this: a one-line index edit
  can trip a size limit and become a full-file rewrite. If a rule forbids delegating, say so
  and ask before spending the context.

## Read before that kind of work

* Compiler / built-ins / IR / native codegen / runtime helpers / diagnostics →
  `.ai/compiler.md` (runtime completion gate, validation & function tests, register
  lifetimes), plus the hard-won invariant docs — read the one(s) matching the work:
  * `.ai/codegen-invariants.md` — arch-neutral codegen/IR/regalloc invariants
    (register clobbers, record layout, vreg-alloc order, desugars, monomorph, diagnostics).
  * `.ai/arch-abi.md` — per-architecture ABI/codegen traps (x86-64 SysV, Win64,
    riscv64, macOS AArch64, Windows PE/console/audio).
  * `.ai/collections.md` — List/Map/Set codegen (memory mgmt, in-place mutation,
    native lowering, HOF-rewrite tradeoffs).
  * `.ai/resources-packages.md` — the RES resource system, the package/import
    subsystem, and builtin-package authoring seams.
  * `.ai/canvas-threading.md` — the `Mode.Canvas` three-thread model: arena state is
    PER-THREAD (so a graphics thread cannot see the worker's published scene), the
    scene ring, the resize handshake, and the closed-flag texture-free rule. Read it
    before touching the graphics thread, the ring, or a texture free.
  * `.ai/net-tls.md` — networking, TLS readiness/timeout, repository-client transport security.
  * `.ai/testing-gates.md` — artifact-gate, byte-identity, acceptance golden harness,
    perf-golden and concurrency hazards, citation sweeps.
  * `.ai/build-tooling.md` — rustfmt/clippy policy, cross-compile + vendor rebuild mechanics.
* Creating or updating `mfb man` content. Two separate sources — pick by page kind:
  * **Built-in package / function / type pages are rendered from the clean-room
    registry descriptors**, not from any Markdown file (`src/cli/man.rs:1-15`,
    `crate::codegen::registry`). Edit the prose fields on the descriptor in
    `src/codegen/builtins/<pkg>/`: package `MODULE_INTRO`/`MODULE_DESC`
    (`mod.rs`), per-member `intro`/`desc`/`example` on `RegistryFunction`
    (`func_*.rs`), `Parameter.desc`, and the `description` on
    `RegistryRecord`/`RegistryResource`/`EnumVariant`/`UnionVariant`. Verify by
    rendering: `mfb man <pkg>`, `mfb man <pkg> <func>`, `mfb man <pkg> types`,
    `mfb man <pkg> --all`.
  * **Narrative guide topics still live as Markdown** under `src/docs/man/**`
    (`errors`, `flow`, `lambda`, `link`, `optimizations`, `tooling`, `tour`,
    `types`, `unicode`, `variable`) — a directory with a `package.md` is a topic,
    embedded at build time (`src/docs/man/mod.rs`). Only reached when the first
    positional is not a known package.
  * **`.ai/man-content.md` is the content standard — read it before writing a
    page.** It defines who the page is for, what it must and must not contain,
    the four-step authoring workflow, and the verification instruments
    (`scripts/man-census.sh`, `scripts/man-run-examples.sh`). The retired
    `.ai/man_*template*.md` files are deleted; nothing in them survived the
    registry migration (the renderer derives Synopsis/Parameters/Return/Errors/
    See-also itself, so a page author writes only intro, description and
    examples).
  * **No C/Rust memory vocabulary on a man page.** The only permitted words are
    **copy**, **mutate**, **value**, and **alias** (the last for a `RES` handle
    only). Not: borrow, ownership, move, consume, free, heap, refcount,
    lifetime, dangling, allocate, deep/shallow copy, by reference, drop. Say
    what a developer observes — "the handle stays open — you still close it",
    "you get a copy" — and link `mfb man variable` for the model itself; the
    precise contract lives in `mfb spec` §14. Check with
    `scripts/man-census.sh --memory-scope` (and `--banned-list` for the full
    list); it must report 0 unclassified hits.
  * Prose fields are `&'static str` the compiler never reads, so no compiler gate
    catches a doc error — `mfb man` output is the only verification. Render it:
    `scripts/man-census.sh --fill <pkg>` for coverage,
    `scripts/man-run-examples.sh <pkg> --run` to compile and run every example
    on the page.
* The embedded spec (`mfb spec`, `src/docs/spec/**`) → `.ai/specifications.md` (keep it
  current with every compiler change).
* Remote test machines → `.ai/remote_systems.md`.
* Starting an agent or sub-agent  → `.ai/sub-agents.md`.
