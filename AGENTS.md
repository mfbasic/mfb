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

## A claim is measured or a guess

* Number/count/status/"X does Y" → give the command behind it in the same sentence,
  else say "guess" (not "~").
* Green gate = nothing *covered* changed.
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
  commit on current branch.
  Never tree-wide `checkout`/`reset`/`restore`/`stash`;
  touch+commit only files you changed. Itemized commits.
* MCP tools arrive deferred. Run `ToolSearch` each context to
  load `mfbasic` (`mfb_man`,`mfb_spec`); prefer over reading files.
* No compound background jobs — one command each. Don't wait on completion notices;
  poll the effect (`pgrep -f` ERE `"a|b"`). No-completion-record job = dead; re-derive.

## Read before that kind of work

* Compiler / built-ins / IR / native codegen / runtime helpers / diagnostics →
  `.ai/compiler.md` (runtime completion gate, validation & function tests, register
  lifetimes).
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
