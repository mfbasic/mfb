# Agent Instructions

Universal rules below. Before a given kind of work, also read the matching `.ai/` file.

## Always

- **Done means verified.** Asked if work is done/complete/verified: answer **yes**
  or **no** on the first line, nothing before it. Say **yes** only after proving the
  actual goal holds (compilation, passing tests, and matching goldens are proxies,
  not verification). When unsure, **no** — then one short line on what's left, no
  status report unless asked.
- **Finish the task — do not stop mid-task.** When asked to finish a plan or to complete
  a plan or to work until done... "Done", "finish", "complete" is the whole task complete
  and verified, not a phase boundary, a plausible stopping point, or a place to hand
  back for confirmation. Stopping early to report progress, ask whether to continue,
  or wait for approval on the next obvious step wastes hours and tokens — keep going
  until the goal holds or you hit a genuine blocker (a destructive irreversible
  action you're unsure about, a real ambiguity that changes the outcome, or an
  external dependency you cannot resolve). At a blocker, state it plainly and
  proceed with the best default where one exists; never declare done while work
  remains. By acting under these instructions you confirm you have read and
  understood this rule and the "Done means verified" rule above.
- **Production-ready only.** Implement the complete behavior with real error
  handling and integration. No stubs, placeholders, mocks, default-result
  fallbacks, simulations, or "unsupported" stand-ins unless explicitly asked. If
  blocked, state the blocker plainly — never fill the gap with non-functional code
  or call it done.
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

## Read before that kind of work

- Compiler / built-ins / IR / native codegen / runtime helpers / diagnostics →
  `.ai/compiler.md` (runtime completion gate, validation & function tests, register
  lifetimes).
- Creating or updating a man page (`src/docs/man/**`) → follow the templates exactly:
  `.ai/man_template.md` for a per-function page, `.ai/man_type_template.md` for a
  package's consolidated `types` page. Keep every section name and order; fill in
  all `<...>` placeholders; omit `[bracketed]` sections only when they do not apply.
  (`scripts/update_man.sh` drives this in bulk and loads the same templates.)
- The embedded spec (`mfb spec`, `src/docs/spec/**`) → `.ai/specifications.md` (keep it
  current with every compiler change).
- Writing a feature plan → `.ai/planning.md` (template `.ai/plan_template.md`).
- Finding a bug while doing any task → `.ai/bugs.md` (test-first fix when small-ish,
  otherwise a `planning/plan-NN` fix).
- Remote test machines → `.ai/remote_systems.md`.
