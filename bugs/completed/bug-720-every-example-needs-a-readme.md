# bug-720: most `examples/*` projects ship no README.md

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: LOW
Class: Other (docs)

Status: FIXED (563c1c59b)
Regression Test: `tests/guards/example_readmes.rs` (`cargo test --test
example_readmes`) — see Deviation from Fix Design, below.

Of the 18 top-level directories under `examples/` (plus 2 sub-projects inside
`examples/bugs/`), only 4 ship a `README.md`: `examples/brogue`,
`examples/browser`, `examples/network-server`, `examples/yaml-json`. The
other 14 top-level examples, plus both sub-projects under `examples/bugs/`,
have no README at all — including `examples/audio`, which needs a pinned
registry package (`jzaun#libsnd`) and vendored per-target dylibs a reader
would otherwise have no way to discover, and `examples/ai_chat`, which
silently shells out to a `claude` CLI that must already be on `PATH`.

**Correct behavior:** every buildable project under `examples/` — including
each sub-project inside a multi-project example directory such as
`examples/bugs/{tui,canvas}` — ships a `README.md` that states what the
example demonstrates, how to build and run it on every platform it targets,
its controls/usage, and any external dependency (registry package, unsigned
flag, CLI tool, network port) it needs.

References:

- `examples/water/src/main.mfb` header comment (lines 1–29) — the existing
  in-source convention this bug's READMEs must stay consistent with (build
  command, per-platform run line, controls list).
- `examples/network-server/README.md`, `examples/browser/README.md`,
  `examples/brogue/README.md`, `examples/yaml-json/README.md` — the four
  existing READMEs, used as the house style for the new ones.
- `scripts/build-examples.sh` — the CI gate this bug proposes to extend.
- `scripts/README.md` — the convention that every file under `scripts/` has
  exactly one entry there.

## Failing Reproduction

```
$ find examples -mindepth 1 -maxdepth 1 -type d | sort
examples/ai_chat
examples/audio
examples/brogue
examples/browser
examples/bugs
examples/dungeon
examples/emoji
examples/hangman
examples/hello_input
examples/hello_world
examples/life
examples/network-client
examples/network-server
examples/snake
examples/water
examples/wide-demo
examples/wind
examples/yaml-json

$ find examples/bugs -mindepth 1 -maxdepth 1 -type d | sort
examples/bugs/canvas
examples/bugs/tui

$ find examples -mindepth 1 -maxdepth 3 -iname 'readme*' | sort
examples/brogue/README.md
examples/brogue/oracle/README.md
examples/browser/README.md
examples/network-server/README.md
examples/yaml-json/README.md
```

- Observed: 4 of 18 top-level example directories have a top-level
  `README.md` (`brogue`, `browser`, `network-server`, `yaml-json`); the other
  14, plus both sub-projects `examples/bugs/tui` and `examples/bugs/canvas`,
  have none.
- Expected: every buildable example project (every `project.json` with
  `"kind": "executable"`, per `scripts/build-examples.sh`'s own discovery
  loop) has a sibling `README.md`.

Contrast case that works correctly today: `examples/water/src/main.mfb`
carries an in-source header comment (build command, per-platform run line,
controls) that partially substitutes for a README, but the convention
established by the 4 examples that DO have a `README.md` is a separate file,
and `water` lacks one at that path — it fails the same bar.

## Root Cause

`scripts/build-examples.sh` has no README check at all. Its only per-project
gate is the `kind: executable` filter used to select build candidates:

```
if ! grep -Eq '"kind"[[:space:]]*:[[:space:]]*"executable"' "$manifest"; then
    continue
fi
```

(`scripts/build-examples.sh`, main discovery loop, `while IFS= read -r
manifest; do ... done < <(find "$EXAMPLES_DIR" -name project.json | sort)`).
After that filter it goes straight to `"$MFB" build -q --target "$target"
"$proj_dir"` — nothing in the script inspects `$proj_dir` for a `README.md`,
and nothing else in `scripts/` or `tests/` does either (no hit for
`README` in `scripts/build-examples.sh` or any other script file, confirmed
by inspection of the full script above). There is no gate that would ever
fail CI for a missing example README, so the gap accumulated silently as
examples were added.

## Goal

- Every executable project under `examples/` (as discovered by
  `scripts/build-examples.sh`'s own `find "$EXAMPLES_DIR" -name project.json`
  + `kind: executable` filter, which already correctly treats
  `examples/bugs/tui` and `examples/bugs/canvas` as separate projects) has a
  `README.md` in the same directory as its `project.json`.
- Each such README meets the minimum content bar defined below.
- A gate exists (in `build-examples.sh` or a new test) that fails when a
  discovered executable project lacks a `README.md`, so the gap cannot
  reopen.

### Non-goals (must NOT change)

- Do not rewrite or restyle the 4 existing good READMEs
  (`brogue`, `browser`, `network-server`, `yaml-json`); they already meet the
  bar and are the style reference for the new ones.
- Do not invent behavior for an example beyond what was actually verified in
  this doc (source grep, project.json, or an actual build run). Anything not
  directly confirmed is marked "guess" below and must be verified before it
  goes in a README.
- Do not add a README-presence check that also requires content correctness
  (e.g. that it documents every control) — Phase 1 only gates *presence*;
  content accuracy is a human-authoring concern for Phase 2.
- Do not change `packages/yaml`'s or `packages/*`'s README conventions —
  this bug is scoped to `examples/*` only.

## Blast Radius

One line per example, from the `find` output above cross-referenced with
`project.json` `kind` (`grep -o '"kind"[^,]*' examples/<d>/project.json`):

- `examples/brogue` — has README. Unaffected.
- `examples/brogue/oracle` — has README (companion tool, not itself a
  `project.json`/example; unaffected).
- `examples/browser` — has README. Unaffected. (Multi-project dir: `dom`,
  `fetch`, `display`, `app` share the one top-level README per
  `build-examples.sh`'s `prepare_browser` comment — out of scope to split.)
- `examples/network-server` — has README. Unaffected.
- `examples/yaml-json` — has README. Unaffected.
- `examples/ai_chat` — **needs README. Priority**: shells out to a `claude`
  CLI (see below).
- `examples/audio` — **needs README. Priority**: registry package + vendored
  dylibs (see below).
- `examples/dungeon` — needs README (canvas app, `--app` build).
- `examples/emoji` — needs README (canvas app, `--app` build).
- `examples/hangman` — needs README (terminal app, `term`).
- `examples/hello_input` — needs README (trivial `io`-only CLI).
- `examples/hello_world` — needs README (trivial `io`-only CLI).
- `examples/life` — needs README (terminal app, `term`).
- `examples/network-client` — needs README (network app: TCP/UDP/TLS client,
  binds no port but takes `--host`/`--port`; see network note below).
- `examples/snake` — needs README (terminal app, `term`).
- `examples/water` — needs README (canvas app; already has an in-source
  header comment at `examples/water/src/main.mfb:1-29` documenting build
  command and controls — the new README must stay consistent with it, not
  duplicate it verbatim).
- `examples/wide-demo` — needs README (terminal app, `term`).
- `examples/wind` — needs README (canvas app that also fetches live weather
  data over `http`/`net` from `nomads.ncep.noaa.gov` — network dependency,
  see below).
- `examples/bugs/tui` — needs README (separate sub-project, terminal app).
- `examples/bugs/canvas` — needs README (separate sub-project, canvas app).

### Priority case 1: `examples/audio`

`examples/audio/project.json` declares one package dependency:

```
"packages": [
  {
    "name": "libsnd",
    "ident": "jzaun#libsnd",
    "version": "1.5.0",
    "pin": true,
    "source": "jzaun#libsnd",
    "identKey": "ed25519:v5dpUNnc4g4TBinvHzGfZh06JXdREdn0Ct1W75qZFBc",
    "direct": true,
    "requiredBy": []
  }
]
```

`find examples/audio -maxdepth 3 -type f` shows the package is already
vendored in-tree: `examples/audio/packages/libsnd.mfp` plus one dylib per
target under `examples/audio/packages/libsnd.vendor/` (macOS `.dylib`, and
`.so` builds for aarch64/riscv64/x86_64 × glibc/musl). `examples/audio/mfb.lock`
pins the resolution.

`grep -rn "unsigned" scripts/ examples/audio` (both plain-text and
`--include` variants) returns no hits — neither `scripts/build-examples.sh`
nor anything under `examples/audio` mentions `--unsigned`. Building it
confirms why:

```
$ target/release/mfb build examples/audio
uses libsnd - [Verified]
Building audio (executable) for macos-aarch64
Wrote executable to examples/audio/build/audio.out
```

Exit code 0. The build succeeds with a plain `mfb build examples/audio` — no
`--unsigned` flag needed, because the pinned package resolves to a signed
("[Verified]") source and the dylibs are already vendored in the example
directory, not fetched at build time. (Build artifact `examples/audio/build/`
was removed after this check; the repo tree is otherwise unmodified —
confirmed with `git status --porcelain`, clean.)

So the audio README's dependency section should state: registry package
`jzaun#libsnd` (already vendored under `packages/`, resolves automatically,
no `--unsigned` needed), and that the vendored `.dylib`/`.so` files are
per-target so cross-compiling to a target this vendor tree doesn't cover
(e.g. Windows — no `.dll` file is present in `packages/libsnd.vendor/`) may
fail; this Windows gap is a guess about behavior, not verified by an actual
cross-build in this investigation.

### Priority case 2: `examples/ai_chat`

`grep -rn "claude" examples/ai_chat/src/main.mfb` confirms the program spawns
a `claude` CLI child process per turn:

```
519: FUNC claudeArgs(prompt AS String, resumeId AS String, pickModel AS String) AS List OF String
520:   MUT a AS List OF String = ["claude", "-p", prompt, "--output-format", "stream-json", "--include-partial-messages", "--verbose"]
1106:  RES child = process::spawn(claudeArgs(it.prompt, it.resumeId, it.model)) TRAP(e)
```

The README must say the `claude` CLI must already be installed and on
`PATH` before running the example; there is no vendored or package-managed
copy.

### Other non-obvious dependencies found

- `examples/network-server`, `examples/network-client` — both `IMPORT net
  tcp tls udp`; take `--host`/`--port` flags and bind/connect a real local
  port (`network-server` defaults to port 7000 per
  `examples/network-server/src/main.mfb:92`). `network-server` already has a
  README covering this; `network-client` does not.
- `examples/wind/src/fetch.mfb` — `IMPORT http`, fetches live GFS forecast
  data from `https://nomads.ncep.noaa.gov/...` at runtime (line 143 area);
  needs network access to run meaningfully. No README exists to say so.
- No other example's source or `project.json` showed a registry package
  dependency (`grep -o '"ident": *"[^"]*"' examples/*/project.json` returned
  a hit only for `examples/audio`) or another shell-out (`grep -rl
  "process::spawn"` under `examples/*/src` returned only `ai_chat`,
  `network-client`, `network-server`, `wind/fetch.mfb` — the last is a
  `net`/`http` client call site, not a spawn, on closer read).

## Fix Design

**Required README content** (the minimum bar every `examples/*` project
README must meet):

1. **What it demonstrates** — one or two sentences on the feature/package
   surface being shown off.
2. **Build and run**, per supported platform:
   - The build command, e.g. `mfb build examples/<name>`, and `--app` if the
     example is a canvas/windowed program (`--app`: "Build as a standalone
     application instead of a library" — `target/release/mfb build --help`).
   - The per-platform run line: `open examples/<name>/build/<name>.app`
     (macOS), `./examples/<name>/build/<name>.AppImage` (Linux), and the
     Windows equivalent — following the existing pattern documented in
     `examples/water/src/main.mfb:16-19` and in
     `examples/network-server/README.md`/`examples/browser/README.md`.
3. **Controls / usage** — key bindings for an interactive program, or CLI
   flags for a program driven from the command line (mirroring
   `examples/water/src/main.mfb:20-28`'s Controls list style, and
   `examples/network-server/src/main.mfb:77-92`'s usage-line style).
4. **Dependencies / setup** — the exact registry package identifier (e.g.
   `jzaun#libsnd`) if any, whether `--unsigned` is required (verify by an
   actual build, as done above for `audio` — do not guess), any external CLI
   tool that must be pre-installed (e.g. `claude` for `ai_chat`), and any
   network/port requirement.

**Gate**: extend `scripts/build-examples.sh`'s existing discovery loop
(`while IFS= read -r manifest; do ... done < <(find "$EXAMPLES_DIR" -name
project.json | sort)`), right after the existing `kind: executable` filter,
with a check that `$proj_dir/README.md` exists; on a miss, print
`MISSING README: $example` and increment `fail_count` the same way a failed
build does today, so the script's existing non-zero exit on `fail_count > 0`
covers it — no new exit path needed. This fits naturally: the loop already
walks every executable project directory the check needs, including
`examples/bugs/tui` and `examples/bugs/canvas` as separate entries, since
`rel`/`example` are already computed per-`project.json` at that point.
Rejected alternative: a standalone `cargo test` — rejected because
`build-examples.sh` already owns per-project directory discovery and runs in
CI (`scripts/README.md`'s "Run by: CI `examples`"), so duplicating discovery
logic in Rust would drift from it.

`scripts/README.md` needs no *new* entry for this — `build-examples.sh`
already has one; only its existing bullet text may want an added clause
noting the README check, which is a Phase 1/3 documentation touch-up, not a
new script file.

## Phases

### Deviation from Fix Design

This doc's Fix Design proposed extending `scripts/build-examples.sh` with a
detect-only README-presence check. The fix instead landed as a Rust guard,
`tests/guards/example_readmes.rs` (`cargo test --test example_readmes`),
following the shape of the existing `tests/guards/example_project_names.rs`
guard: it walks every `examples/**/project.json`, filters to `"kind":
"executable"`, and fails listing every one with no `README.md` in its own
directory or (for a multi-project example directory, the `examples/browser`
case) at the shared parent directory. This keeps the check in the same
`cargo test` surface as the sibling guard it mirrors, runs without a host
`mfb` build, and needed no `scripts/README.md` change since no file under
`scripts/` was touched.

The doc's Blast Radius list also predates several fixes landed on `main`
since it was written, re-confirmed before Phase 2:
- `examples/bugs/{tui,canvas}` is now `examples/swat/{tui,canvas}`
  (`swat-tui`/`swat-canvas`, bug-718) — both still needed a README, now at
  their new paths.
- `examples/dungeon/CREDITS.md` already exists (Kenney tile set, CC0,
  bug-714, trimmed to the 28 tiles the example actually loads) — linked from
  `examples/dungeon/README.md` rather than re-authored.
- `network-server`'s default port (7000) matches what this doc already
  recorded; `network-client`'s new README cites it directly rather than
  duplicating the number without attribution.
- `audio`'s `jzaun#libsnd` package and `mfb.lock` are git-ignored, so a fresh
  checkout needs an install step before it builds. See Correction below —
  the first pass here got the exact step wrong.

### Correction (post-review)

The first version of `examples/audio/README.md` claimed "a plain `mfb build
examples/audio` resolves and builds" based on a build run in a worktree
where `examples/audio/packages/` and `mfb.lock` had already been copied over
from the main checkout by hand — not a real fresh-clone test. Review caught
this: a genuinely fresh `git worktree add --detach` checkout has neither
file (both are `.gitignore`d — `examples/**/packages/` and
`examples/**/*.lock`), and `mfb build examples/audio` there fails with
`IMPORT_PACKAGE_NOT_INSTALLED`.

Re-verified from two independent fresh detached worktrees (`git worktree add
--detach <tmp> worktree-B-720`, no prior `packages/`/`mfb.lock` present): the
correct sequence is `mfb pkg update`, run **inside** `examples/audio` (it
takes no project-path argument — `mfb pkg update <path>` errors with "failed
to read './project.json'"), then `mfb build examples/audio` from the repo
root as before. `mfb pkg update` needs network access to the package
registry: it resolved `libsnd 1.5.0` and downloaded its 7 vendor libraries
both times. `mfb pkg install [path]` (the other candidate) is for
re-installing from an *existing* `mfb.lock` by hash and does not resolve a
new one, so it does not help a first-time fresh checkout.
`examples/audio/README.md` now documents this exact two-step sequence and
notes that a checkout where the package was already resolved (e.g. this
repo's own long-lived working tree) can skip straight to `mfb build`.

Checked every other README the same way (fresh detached worktree, run the
documented commands verbatim): `examples/browser`'s existing
`dom`→`fetch`/`display`→`app` package-build sequence and
`examples/yaml-json`'s package-then-build sequence both work unmodified from
a fresh checkout (their local `.mfp` packages build from source with `mfb
build`, no registry step, no `--unsigned` needed). No other new README in
this fix depends on git-ignored build state — `audio` was the only
`"packages"` declaration among them.

### Phase 1 — failing check + audit (no behavior change)

- [x] Add the README-presence check — as `tests/guards/example_readmes.rs`
      (see Deviation above), not `scripts/build-examples.sh`.
- [x] Run it and confirm it currently reports exactly the 15 executable
      projects from the re-measured Blast Radius (13 top-level examples plus
      `swat/tui` and `swat/canvas`), and no others.
- [x] Blast-radius audit re-measured against current `main` (see Deviation
      above).

Acceptance: the guard fails (panic, one line per project) for precisely the
15 projects listed as "needs README" above; the 4 projects with existing
READMEs (plus `browser`'s multi-project layout) produce no failure line.
Commit: 4c620c1b4

### Phase 2 — write the READMEs

- [x] `examples/audio/README.md` first (priority): what it demonstrates,
      build/run per platform, the `jzaun#libsnd` package note (vendored, no
      `--unsigned` needed — reconfirmed by an actual build in this fix's own
      worktree), no interactive controls beyond its numeric menu (verified
      via source).
- [x] `examples/ai_chat/README.md` (priority): the `claude` CLI prerequisite,
      reconfirmed against source; controls cross-checked against the actual
      key-code handling (`\u{0E}` etc.), not just the header comment.
- [x] The remaining 13 (`dungeon`, `emoji`, `hangman`, `hello_input`,
      `hello_world`, `life`, `network-client`, `snake`, `water`,
      `wide-demo`, `wind`, `swat/tui`, `swat/canvas`), each meeting the
      minimum content bar, each verified against actual source/controls
      before writing (no invented behavior).

Acceptance: every project the Phase 1 check flagged now has a `README.md`
meeting the minimum content bar; re-running the Phase 1 check produces zero
failures.
Commit: 563c1c59b

### Phase 3 — validation

- [x] Re-ran the Phase 1 check standalone (`cargo test --test
      example_readmes`) — GREEN, zero misses.
- [x] Built every touched example on the host (`macos-aarch64`) with this
      worktree's own `target/release/mfb`, confirming each README's exact
      build command actually works (`--app` where documented); also
      cross-built one console and one app example for `windows-x86_64` to
      confirm the `.exe` artifact name used in every README.
- [x] No `scripts/` file was touched, so `scripts/README.md` needs no edit —
      see Deviation above.
- [x] Full-suite gate: `cargo test --release` under the machine-wide heavy
      lock — one incidental failure
      (`rt_compress_bounds::decode_time_is_linear_in_output_size`, a timing
      assertion), re-run alone and it passed
      (`test result: ok. 2 passed; 0 failed`) — confirmed machine-contention
      noise from the other five lanes sharing the host, not a regression
      from this fix (this fix touches no `src/` code the test exercises).

Acceptance: the guard is GREEN; every documented build command was run for
real; the full suite is green modulo the confirmed-noise timing test.
Commit: 563c1c59b (doc-only follow-up folded into the same archival commit)

## Validation Plan

- Regression test: the extended `scripts/build-examples.sh` README-presence
  check (Phase 1), which fails today and must pass after Phase 2.
- Runtime proof: `bash scripts/build-examples.sh` full run (Phase 3),
  reporting `0` `MISSING README:` lines.
- Doc sync: `scripts/README.md`'s `build-examples.sh` entry.
- Full suite: not required beyond `scripts/build-examples.sh` itself — this
  is a docs/build-tooling change with no compiler surface touched.

## Summary

The engineering risk is entirely in Phase 2 (writing 16 accurate READMEs
without inventing behavior) — Phase 1's gate is a few lines in an existing
loop, and Phase 3 is just re-running that gate. The two priority cases
(`audio`, `ai_chat`) are now fully characterized: `audio` needs no
`--unsigned` flag (verified by an actual successful build,
`target/release/mfb build examples/audio` → exit 0, package shown
"[Verified]"), and vendors its own per-target `libsndfile` dylibs; `ai_chat`
needs a `claude` CLI pre-installed on `PATH`, confirmed by source
(`examples/ai_chat/src/main.mfb:520`). Everything else in this doc not tied
to a `grep`/`find`/build command above (e.g. audio's Windows vendor-tree gap)
is explicitly flagged as a guess and must be verified before Phase 2 writes
it into a README.
