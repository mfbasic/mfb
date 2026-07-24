# plan-47-A: make every platform branch exhaustive

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: nothing. **This is the first thing plan-47 lands.**
Produces: `PlatformFamily` (an enum with a `Windows` variant), and 29 exhaustive
`match` sites + 4 helper functions converted off raw target-string comparison. Consumed
by every other plan-47 sub-plan — it is what makes registering `windows-x86_64` safe.

Shared lowering decides OS behavior by comparing `platform.target()` against a string:
`== "macos-aarch64"`, `.starts_with("linux")`, `.contains("macos")`. **Every one of
those comparisons is binary.** Not one has a Windows arm, and none of them can acquire
one by accident — a new OS silently takes whichever side the author happened to write
last.

The single behavioral outcome: after this sub-plan, adding a new OS to
`NATIVE_BACKENDS` produces a **compile error at every site that must make a decision
about it**, and zero behavior change for the four existing targets — proven byte-identical.

This is the highest-leverage sub-plan in plan-47 and it delivers no Windows code at all.
It converts the feature's dominant silent-failure class into a build failure, *before*
47-B registers the target and makes all 29 sites reachable.

References (read first):

- `src/target/shared/code/types.rs:212` — `CodegenPlatform`, whose `target()` returns
  the `String` every one of these sites compares against.
- The master, §3.2 — the enumeration of what Windows silently inherits today.
- `planning/plan-47-H-threads.md` §Phase 1 — the same "collapse to one chokepoint, prove
  zero-byte diff" technique, applied to pthread symbol selection. This sub-plan is that
  technique applied to the branch sites rather than the symbol literals.

## Prerequisites

See the master §Prerequisites for the feature-wide gate. This sub-plan adds one of its
own, because it edits code every backend compiles:

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| Byte-identity goldens exist for all four existing targets | `find tests -path '*/golden/*' -name '*.ncode*' \| while read f; do b="${f##*/}"; b="${b%.*}"; echo "${b##*.}"; done \| sort -u` | **MET 2026-07-23 (`bb3ba1c5f`) for 6/7 cover surfaces** — `linux-riscv64` `.ncodesum` seeded for audio/tls/os/crypto/net + crypto-ec-valid. **fs excluded (bug-381).** Phase 2 (`open_flag_set`) and Phase 4 (`fs_helpers_paths`/`fs_helpers_io`) fs edits select the shared Linux arm, guarded by linux-x86_64 + linux-aarch64 cover-fs goldens — riscv64-neutral by construction. |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run it
> before you continue and again before you decide to stop. If you stop, report the
> status of every row.

**This row is a real blocker for this sub-plan specifically.** The entire acceptance
criterion here is "0 diffs on every target". A target with no goldens cannot produce a
diff, so for riscv64 the criterion is vacuous — the sub-plan would report success
having proven nothing about the backend it is most likely to break silently. Seed
riscv64 `.ncodesum` goldens first, mirroring the six `codegen-cover` fixtures that
`ff163ddeb` added for linux-x86_64 and linux-aarch64.

## 1. Goal

- A `PlatformFamily` enum (`Linux`, `MacOS`, `Windows`) reachable from
  `CodegenPlatform`, and **29 `if`/`else` chains on `platform.target()` replaced by
  exhaustive `match` on it**.
- The 4 helper functions that take a raw `target: &str` and branch internally take a
  `PlatformFamily` instead.
- Adding a variant to `PlatformFamily` fails to compile at every site that must decide,
  with no `_ =>` fallthrough anywhere in the converted set.
- **Zero behavior change.** All four existing targets emit byte-identical code.
- No Windows arm is added here — every `Windows` arm is `unreachable!()` or a compile
  error until 47-B registers the target. This sub-plan is pure mechanism.

### Non-goals (explicit constraints)

- **No Windows behavior.** Not one Win32 symbol, not one Windows-correct value. The
  `Windows` variant exists so later sub-plans get compile errors; its arms are filled in
  by C/D/E/G/H, each in its own sub-plan.
- **No new `CodegenPlatform` methods**, and no change to the 54 required ones. This adds
  one method (`family()`) with a default derived from `target()`, so no backend is
  forced to change.
- **No change to `platform.target()` itself.** It still returns the full target string;
  other consumers (diagnostics, plan tables, artifact naming) are untouched.
- **Do not convert the 26 non-branching `target()` sites** (§2.1) — those pass the
  string through to a helper or emit it as data. Converting them is scope creep and
  churns code with no safety gain.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| `platform.target()` consumption sites in shared lowering | **55** | `rg -c 'platform\.target\(\)' src/target/shared/code/ \| awk -F: '{s+=$2} END {print s}'` |
| — of those, **comparison-shaped** (the ones that silently pick an arm) | **29** | `rg -c 'platform\.target\(\)\s*(==\|\.starts_with\|\.contains)' src/target/shared/code/` |
| — the remaining pass-through/data sites (out of scope) | 26 | 55 − 29 |
| Files containing a comparison-shaped branch | **10** | same `rg -c`, count of lines |
| Helper fns that take a raw `target: &str` and branch inside | **4** | `rg -n 'fn (open_flag_set\|temp_file_open_flags\|os_family\|os_arch)' src/target/shared/code/` |
| Existing backends that must stay byte-identical | 4 | `src/target.rs:197` |

Branch sites by file:

| File | Branches | What they decide |
|---|---|---|
| `tls/openssl.rs` | 7 | macOS vs default spelling throughout the OpenSSL backend |
| `mod.rs` | 6 | TLS/EC/ALSA data objects, `skip_entry_arena_destroy`, audio callback |
| `term.rs` | 3 | `TIOCGWINSZ` request value (×3) |
| `runtime_helpers.rs` | 3 | `thread_symbol`, `pthread_create`, `pthread_attr_*` underscore prefix |
| `fs_helpers_io.rs` | 3 | raw-syscall write, `openat2` nofollow (×2) |
| `os.rs` | 2 | `_SC_NPROCESSORS_ONLN`, executable-path mechanism |
| `fs_helpers_paths.rs` | 2 | dirent `d_namlen` vs strlen |
| `datetime.rs` | 1 | `CLOCK_MONOTONIC` value |
| `crypto_ec.rs` | 1 | **which crypto backend entirely** |
| `audio/mod.rs` | 1 | CoreAudio vs ALSA |

The 4 helper functions: `open_flag_set` (`fs_helpers_io.rs:2738`),
`temp_file_open_flags` (`fs_helpers_atomic.rs:247`), `os_family` (`os.rs:188`),
`os_arch` (`os.rs:197`). These are the sneakiest sites: the branch is *inside* the
helper, so a reader at the call site sees only `open_flag_set(platform.target(), false)`
and cannot tell an OS decision is being made. `open_flag_set` alone is called from **6**
places.

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| Every comparison-shaped site is binary (no existing Windows arm) | **CONFIRMED** | read all 29; each is `if <os> { … } else { … }` or an `else if` chain terminating in an unconditional `else` |
| A new OS therefore takes a POSIX/macOS arm with no diagnostic | **CONFIRMED** | there is no `_ =>` to fail on and no exhaustiveness check — the `else` absorbs it |
| `open_flag_set`'s wrong arm has shipped as a bug before | **CONFIRMED** | the comment at `fs_helpers_io.rs:2739-2743` documents linux-x86_64 having received the macOS `O_*` bits |
| The 26 pass-through sites do not decide OS behavior | **CONFIRMED** | they forward the string to a helper (counted separately) or emit it as `os.name`/`os.arch` data |
| Converting to `match` cannot change emitted bytes | **UNVERIFIED — this is the acceptance criterion** | proven per phase by 0-diff goldens, not by reasoning |

## 3. Design Overview

One enum, one defaulted trait method, and a mechanical site-by-site conversion.

```rust
// src/target/shared/code/types.rs
pub(crate) enum PlatformFamily { Linux, MacOS, Windows }

// on CodegenPlatform, defaulted so no backend is forced to change:
fn family(&self) -> PlatformFamily {
    let t = self.target();
    if t.starts_with("linux") { PlatformFamily::Linux }
    else if t.starts_with("macos") { PlatformFamily::MacOS }
    else if t.starts_with("windows") { PlatformFamily::Windows }
    else { unreachable!("unregistered target {t}") }
}
```

Each of the 29 sites becomes:

```rust
match platform.family() {
    PlatformFamily::MacOS => DARWIN_TIOCGWINSZ,
    PlatformFamily::Linux => LINUX_TIOCGWINSZ,
    PlatformFamily::Windows => unreachable!("47-G owns the Windows console path"),
}
```

**The `unreachable!` is the point.** It is not a placeholder to tidy up later — it is the
marker that says "this site has an unanswered Windows question, and 47-G owns it".
When 47-B registers the target, any site still holding `unreachable!` that a Windows
program actually reaches panics loudly in a test rather than emitting a wrong constant.
Sites Windows genuinely never reaches keep it permanently.

**Where design uncertainty concentrates:** nowhere. This is mechanical. The only
question is whether the conversion is byte-neutral, and that is *measured* per phase,
not argued.

**Where correctness risk concentrates:** in a conversion that silently changes an arm —
transcribing `starts_with("macos")` as `MacOS` where the original `else` also caught an
unregistered target, or flipping a negated condition (`!contains("macos")` at
`mod.rs:688` is the one to watch). Mitigation: one phase per file, 0-diff gate after
each, so a mistake is attributable to a 3-line change rather than a 29-site sweep.

**Rejected alternative:** *a `has_termios()` / `is_posix()` style boolean on the trait.*
Rejected: it is the same binary shape with a new name. A third OS with a fourth answer
reintroduces the problem, and booleans do not produce exhaustiveness errors.

**Rejected alternative:** *do this inside 47-B, as part of registration.* Rejected:
A is already `large` and its acceptance is byte-identity; folding a 29-site sweep into it
makes any diff unattributable. Landing P first means A's diff is only the ABI change.

**Rejected alternative:** *convert all 55 sites for consistency.* Rejected as scope
creep — the 26 pass-through sites make no OS decision, so converting them churns shared
code with zero safety gain and dilutes the 0-diff signal.

## 4. Detailed Design

### 4.1 Where the enum lives

`PlatformFamily` goes in `src/target/shared/code/types.rs` beside `CodegenPlatform`, not
in `src/target.rs`. Reason: it is a *codegen* concept (which lowering arm to emit), not a
build-target concept. `BuildTarget` already exists for the latter and is parsed from user
input; conflating them would put a codegen enum in the CLI's dependency path.

### 4.2 The helper functions

The 4 helpers change signature from `target: &str` to `family: PlatformFamily`. This is
the highest-value part of the sub-plan despite being the smallest: it moves the decision
from *inside* a helper (invisible at the call site) to the type system. After it,
`open_flag_set(family, false)` cannot be called with an unhandled OS.

### 4.3 What the `Windows` arms say

Three legitimate contents, and the choice is per site:

- `unreachable!("47-<letter> owns this")` — Windows will reach this path, and the named
  sub-plan fills it in. The majority.
- A real value — only where the answer is genuinely OS-independent and the original
  `else` was already correct for Windows. **Prove it, don't assume it**; this is where a
  conversion silently ships a wrong arm.
- `compile_error!`-adjacent: leave the arm out entirely, so the file does not compile
  until the owning sub-plan adds it. Use for the highest-consequence sites
  (`crypto_ec.rs:113`, `audio/mod.rs:106`) where a wrong arm is worse than a build break.

## Compatibility / Format Impact

- **Changed:** internal only. `CodegenPlatform` gains one defaulted method; 29 sites and
  4 helper signatures change shape.
- **Unchanged:** every existing target's emitted bytes (the acceptance criterion);
  `platform.target()`'s return type and every non-branching consumer; the 54 required
  trait methods; the language, IR, and plan/NIR/MIR schemas.

## Phases

One phase per file, smallest and least consequential first, so that the 0-diff gate
attributes any regression to a handful of lines. **Every phase's acceptance is the same
and is non-negotiable: `scripts/artifact-gate.sh` reports 0 diffs on all four targets.**

> **Keep these checkboxes current as you go — tick `- [x]` in the same commit as the
> work, never batched at the end.** An unticked box means NOT DONE.

### Phase 1 — the enum and the defaulted `family()`

- [ ] Add `PlatformFamily` and `CodegenPlatform::family()` to
      `src/target/shared/code/types.rs`, defaulted from `target()` as in §3.
- [ ] Tests: `family()` returns the right variant for all four registered targets, and
      panics for an unregistered one.

Acceptance: the enum exists and is derivable for every registered backend; no call site
uses it yet; `scripts/artifact-gate.sh` 0 diffs on all four targets.
Commit: —

### Phase 2 — the 4 helper functions (highest value per line)

- [ ] `open_flag_set` (`fs_helpers_io.rs:2738`) takes `PlatformFamily`; update its 6
      call sites.
- [ ] `temp_file_open_flags` (`fs_helpers_atomic.rs:247`), `os_family` (`os.rs:188`),
      `os_arch` (`os.rs:197`) likewise.
- [ ] Each becomes an exhaustive `match` with a `Windows` arm per §4.3.

Acceptance: no helper in shared lowering takes a raw target string; 0 diffs on all four
targets. Watch `open_flag_set` specifically — its wrong arm has shipped before
(`fs_helpers_io.rs:2739-2743`).
Commit: —

### Phase 3 — the single-branch files

- [ ] `datetime.rs:59` (`CLOCK_MONOTONIC`), `audio/mod.rs:106` (backend selection),
      `crypto_ec.rs:113` (**backend selection — omit the Windows arm entirely per §4.3**).

Acceptance: 0 diffs on all four targets; `crypto_ec.rs` and `audio/mod.rs` fail to
compile if a `Windows` variant is added without a decision.
Commit: —

### Phase 4 — the small multi-branch files

- [ ] `fs_helpers_paths.rs:922`, `:1039` (dirent shape).
- [ ] `os.rs:1116`, `:1334` (`_SC_NPROCESSORS_ONLN`, executable path).
- [ ] `runtime_helpers.rs:63`, `:612`, `:617` (thread symbol + underscore prefix).
- [ ] `term.rs:233`, `:316`, `:800` (`TIOCGWINSZ`).
- [ ] `fs_helpers_io.rs:33`, `:599`, `:938`.

Acceptance: 0 diffs on all four targets, after **each** file, not at the end.
Commit: —

### Phase 5 — `mod.rs` and `tls/openssl.rs` (largest blast radius last)

`mod.rs`'s 6 branches govern data-object emission — what strings land in `.rdata`.
`tls/openssl.rs`'s 7 govern symbol spelling throughout the TLS backend. Both are where a
flipped condition is least visible, and `mod.rs:688` is a **negated** test
(`!contains("macos")`) — transcribe it deliberately.

- [ ] `mod.rs:680`, `:688`, `:703`, `:712`, `:1036`, `:1052`.
- [ ] `tls/openssl.rs:15`, `:924`, `:1453`, `:1814`, `:2069`, `:2215`, `:2380`.

Acceptance: 0 diffs on all four targets. Additionally diff the emitted `.rdata` strings
for a TLS-using and an audio-using fixture explicitly — a flipped arm here changes which
soname is baked into the binary, which a `.ncodesum` would catch but not explain.
Commit: —

## Validation Plan

- Tests: `family()` unit coverage per Phase 1. The conversion itself is not
  unit-testable — its correctness *is* byte-identity.
- Coverage check: **this is the sub-plan where the coverage question decides the
  outcome.** `linux-riscv64` has zero native goldens, so "0 diffs" is vacuous for it
  (§Prerequisites). Seed them first, or this sub-plan cannot prove the thing it exists
  to prove.
- Runtime proof: none applicable and none claimed — behavior is unchanged by
  construction. The proof is the gate.
- Doc sync: none. `PlatformFamily` is internal and appears in no spec.
- Acceptance: the project's full suite plus `scripts/artifact-gate.sh` at 0 diffs on all
  four targets, re-run after every phase.

## Open Decisions

1. **`unreachable!` vs. omitting the `Windows` arm**, per site (§4.3). Recommended:
   omit the arm for `crypto_ec.rs:113` and `audio/mod.rs:106` (backend selection — a
   wrong arm is catastrophic and a build break is cheap); `unreachable!` with an owning
   sub-plan named everywhere else.
2. **Whether `family()` should be defaulted or required.** Recommended defaulted, so
   this sub-plan touches no backend. The cost is that a future backend could forget to
   override it and get the string-derived answer — acceptable, since the default derives
   from the registered target string and is correct by construction.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The branch count is 29, not the 20 the master's first pass enumerated.**
  Measured: `rg -c 'platform\.target\(\)\s*(==|\.starts_with|\.contains)'
  src/target/shared/code/` → 29 across 10 files, out of 55 total `target()` sites. The
  master's §3.2 table listed a representative subset, not the population.
- 2026-07-20 — **4 helper functions branch on a raw target string internally**
  (`open_flag_set`, `temp_file_open_flags`, `os_family`, `os_arch`) and were missed by
  every earlier pass, because the call site shows only `platform.target()` being passed
  through. `open_flag_set` has 6 call sites and its wrong arm has already shipped once.

## Summary

The engineering risk here is a transcription error in a mechanical sweep, which is why
the sub-plan is cut one file per phase with an unconditional 0-diff gate after each
rather than as a single commit.

The value is disproportionate to the effort: it is the only change in plan-47 that
converts the feature's dominant failure mode — 29 branches silently handing Windows a
POSIX arm — into a compile error, and it delivers zero Windows code to do it.

Its own prerequisite is unmet: `linux-riscv64` has no byte-identity goldens, so the
acceptance criterion is vacuous for the backend this sub-plan is most likely to break
without anyone noticing. Seed those goldens before starting.
