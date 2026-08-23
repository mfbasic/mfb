# Agent Instructions

## Never edit a test/golden to pass

Don't edit/weaken/re-baseline a test/golden until PROVEN wrong.

* First answer 4 from evidence:
  (1) when/why written (`git log -S`, blame, bug/plan doc);
  (2) behavior it protects, 1 sentence;
  (3) who else depends (grep tree+spec);
  (4) proof it's wrong (repro/spec cite/sibling contract; your change is never proof).

* Not all 4 → test wins, STOP.
* Run full `cargo test`, never one module.
* Once proven wrong: fix the bug AND correct only the disproved line (never re-baseline a whole file); show proof in commit.
* This rule guards *behavioral/semantic* tests (correctness contracts). It is NOT about
  byte-identity codegen goldens (`.ncode`/`.ncodesum`): those are DRIFT SENTINELS, not
  behavior — a churn from a correct change means "regenerate the golden" (`sync-goldens.sh`
  / `regen-ncodesum.sh`), NEVER "revert the change." The 4-question gate does not apply to them.
* Byte-identity is NOT a design constraint or a goal in itself (`.ai/testing-gates.md`): it was
  the explicit north star ONLY for the builtin-migration project (pure code motion). For all
  other work, do NOT shape, shrink, or abandon a correct change to keep bytes identical, and do
  NOT revert because a change "can't be byte-identical." Make the right change, then regenerate
  and PROVE the golden delta is only yours. Reverting a correct change to preserve bytes is a bug.

## A claim is measured or a guess

* Number/count/status/"X does Y" → give the command behind it in the same sentence,
  else say "guess" (not "~").
* Green gate = nothing *covered* changed.
* Unexpected golden/`.ncode`/objdump diff (incl. a step predicted byte-identical/neutral)
  = bug-hunt trigger, NEVER proof a design is dead: objdump ONE fixture to localize before
  concluding. Almost always a bug you just introduced or a wrong prediction — fix/correct it,
  continue. A diff on a target you EXPECTED to change is the plan working, not failing.
* Cite symbol+command, never a line alone.
* Sources disagree → run the command.
* Before calling a citation dangling, check all: `bug-N` in `bugs/`|`completed-bugs/`|`skipped/`;
  `plan-N` in `planning/`|`old-plans/`; a fixed bug may have no doc.

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
* MCP tools arrive deferred. Run `ToolSearch` each context to
  load `mfbasic` (`mfb_man`,`mfb_spec`); prefer over reading files.
* No compound background jobs — one command each. Don't wait on completion notices;
  poll the effect (`pgrep -f` ERE `"a|b"`). No-completion-record job = dead; re-derive.
* Run `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`
  at the end of any session with Rust code changes (root `--all` does NOT reach the
  `repository/` path dependency — no `[workspace]` table; see `.ai/build-tooling.md`).

## Auto memory rules
* Record only durable, transferable lessons — things that would burn a future session
  writing code (gotchas, ABI/codegen invariants, tooling traps, rules the source doesn't reveal).
* Never record ticket/plan STATUS in memory: no "DONE", "MERGED", "ARCHIVED", "IN PROGRESS",
  "next: ...", commit hashes, or worktree state. Git and the bug tracker own that.
* When a bug is fixed: do NOT leave it in memory as a status line. If the fix taught a durable lesson,
  record ONLY the lesson, stripped of ticket state. If it didn't, record nothing. Never keep a completed/archived plan in MEMORY.md.

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
  * `.ai/net-tls.md` — networking, TLS readiness/timeout, repository-client transport security.
  * `.ai/testing-gates.md` — artifact-gate, byte-identity, acceptance golden harness,
    perf-golden and concurrency hazards, citation sweeps.
  * `.ai/build-tooling.md` — rustfmt/clippy policy, cross-compile + vendor rebuild mechanics.
* Creating or updating a man page (`src/docs/man/**`, Markdown) → follow the templates
  exactly: `.ai/man_template.md` for a per-function page, `.ai/man_type_template.md`
  for a package's consolidated `types` page, `.ai/man_package_template.md` for a
  package overview. Keep every section name and order; fill in all `<...>`
  placeholders; omit optional sections only when they do not apply. The templates are
  bare skeletons — authoring rules live in the driver scripts (`scripts/update_man.sh`
  for function/type pages, `scripts/update_man_package.sh` for package overviews).
* The embedded spec (`mfb spec`, `src/docs/spec/**`) → `.ai/specifications.md` (keep it
  current with every compiler change).
* Remote test machines → `.ai/remote_systems.md`.
* Starting an agent or sub-agent  → `.ai/sub-agents.md`.
