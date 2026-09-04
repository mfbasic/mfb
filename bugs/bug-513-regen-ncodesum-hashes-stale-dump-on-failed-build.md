# bug-513: `regen-ncodesum.sh` writes the PREVIOUS target's hash into a golden when a build fails

Last updated: 2026-09-03
Effort: small (<1h) — fix landed with bug-497; this doc records the defect and the residual risk
Severity: HIGH (silently ratifies wrong bytes into a drift sentinel)
Class: Tooling / test-integrity

Status: Fixed (guard landed alongside bug-497)
Regression Test: none automated — see "Why no automated test" below

## Mechanism

`scripts/regen-ncodesum.sh` rebuilds each fixture's `-ncode` dump per target and
overwrites the matching `.ncodesum` golden with its sha256. The dump path carries
**no target infix**:

```sh
af="$fixturedir/$name.ncode"        # same path for EVERY target of the fixture
"$MFB" build -q -ncode $mode $targ "$fixturedir" >/dev/null 2>&1
if [ -f "$af" ]; then
  shasum -a 256 "$af" | cut -d' ' -f1 > "$gsum"
```

Two omissions combine:

1. the build's **exit status is never checked** (and its output is sent to
   `/dev/null`), and
2. `$af` is **not removed before the build**.

So when a build fails for one target, `$af` still holds the dump from the
*previous* target in `find | sort` order, and its hash is written into this
target's golden. `macos-aarch64` sorts immediately before `windows-x86_64`, so
the failure mode is specifically "the macOS sum lands in the Windows golden."

## Observed

Running the script under `zsh` (it is `#!/usr/bin/env bash`; zsh does not
word-split `$targ`, so `-target windows-x86_64` is passed as a single argument
and every cross-target build fails):

```
$ zsh scripts/regen-ncodesum.sh target/release/mfb
regen-ncodesum: 55 golden(s) refreshed, 86 missing
```

26 windows-x86_64 goldens that were **correct** were overwritten with the
macos-aarch64 sum. For `tests/byte-identity/bits`:

```
fresh windows-x86_64 build sha: be43083f997a3a855f0f4b2c469ffdac0a99e9c8b5d0b82249c749d58762e030
golden on main (correct):       be43083f997a3a855f0f4b2c469ffdac0a99e9c8b5d0b82249c749d58762e030
golden after the zsh regen:     ea23809f4d2648c10ceedf68826d5f444113b846e02b3cf3c1bedcaa02e42c4f
main's macos-aarch64 golden:    ea23809f4d2648c10ceedf68826d5f444113b846e02b3cf3c1bedcaa02e42c4f
```

The artifact gate then reported 41 diffs where it had reported 25 — the regen
*created* 26 of them. Under `bash` the same command reports
`141 golden(s) refreshed, 0 missing` and the gate is clean.

## Impact

A `.ncodesum` is a drift sentinel: the only thing standing between a codegen
change and an unnoticed cross-target regression. A regen that writes a *wrong but
self-consistent* value is worse than one that fails — the next gate run is green
against a golden that no longer describes the target's real output, and the
Windows/Linux byte-identity signal is silently dead. Nothing else in the harness
recomputes these sums, and the CI boxes that could catch it (`.ai/remote_systems.md`)
only run what the goldens tell them to.

The trigger does not require zsh. Any per-target build failure — a
cross-compilation prerequisite missing, a fixture that legitimately cannot build
for one target, a transient error — produces the same silent corruption.

## Fix (landed)

`scripts/regen-ncodesum.sh` now removes `$af` before each build and refuses to
hash unless *this* target's build succeeded:

```sh
rm -f "$af"
if ! "$MFB" build -q -ncode $mode $targ "$fixturedir" >/dev/null 2>&1; then
  echo "BUILD FAILED for $gsum ($target${mode:+ }$mode) — golden left unchanged"
  missing=$((missing + 1))
  continue
fi
```

A failure is now loud and the golden is left at its previous (correct) value.
This is what surfaced the zsh word-splitting problem: the guarded run reported
`112 BUILD FAILED` lines instead of silently writing 112 wrong sums.

## Recurrence guard (also landed)

The script is bash-only but is **not marked executable in a fresh worktree**
(`./scripts/regen-ncodesum.sh` → `permission denied`), which is precisely what
tempts an operator into `zsh scripts/...`. Rather than rely on the shebang being
honoured, it now re-execs itself under bash:

```sh
if [ -n "${ZSH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi
```

So the corrupting invocation is no longer reachable, and the `BUILD FAILED`
guard above covers every other cause of a per-target build failure.

## The sibling script had it too (fixed)

`scripts/regen-outside-ncode.sh` — which covers every `.ncode`/`.ncodesum`
golden *outside* `tests/byte-identity/` — was audited while filing this and has
the identical omission at its build site:

```sh
"$MFB" build -q -ncode $targ $app "$fixturedir" >/dev/null 2>&1   # status ignored
actual="$fixturedir/$name.ncode"                                   # no target infix
if [ -f "$actual" ]; then
  if [ "$ext" = "ncodesum" ]; then shasum ... > "$golden"
  else cp "$actual" "$golden"                                      # copies the WRONG dump
```

It is strictly worse for a raw `.ncode` golden, where the wrong target's entire
dump is `cp`'d over the golden rather than just its hash. Both the `rm -f` +
exit-status guard and the zsh re-exec guard were applied there as well.

## Follow-ups (not done here)

- Consider having both scripts report *which* goldens moved rather than a bare
  count, so an operator sees a blast radius. A regen that rewrites 26 goldens
  the operator did not expect should be visible without running the gate.

## Why no automated test

The failure needs a build that fails for exactly one target while a sibling
target's dump is present — reproducible by hand (run under zsh, or point `$MFB`
at a binary that rejects `-target`), but a fixture that pins it would have to
ship a deliberately unbuildable target, which the gate would then report as a
permanent MISSING. The guard converts the silent path into a loud one, which is
the property worth having; the census above is the evidence.

## Non-goals

- Do not change the `.ncodesum` golden format or the gate's comparison.
- Do not make the script tolerate a failed build by skipping the fixture
  silently — the whole point is that it must be loud.
