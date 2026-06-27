# Agent Instructions

Universal rules below. Before a given kind of work, also read the matching `.ai/` file.

## Always

- **Done means verified.** Asked if work is done/complete/verified: answer **yes**
  or **no** on the first line, nothing before it. Say **yes** only after proving the
  actual goal holds (compilation, passing tests, and matching goldens are proxies,
  not verification). When unsure, **no** — then one short line on what's left, no
  status report unless asked.
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

## Read before that kind of work

- Compiler / built-ins / IR / native codegen / runtime helpers / diagnostics →
  `.ai/compiler.md` (runtime completion gate, validation & function tests, register
  lifetimes).
- The embedded spec (`mfb spec`, `src/spec/**`) → `.ai/specifications.md` (keep it
  current with every compiler change).
- Writing a feature plan → `.ai/planning.md` (template `.ai/plan_template.md`).
- Remote test machines → `.ai/remote_systems.md`.
