# bug-1082: `scripts/test-appimage.sh` fails three checks on box 2226, the only glibc GTK box that was up

Last updated: 2026-10-12
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Regression Test: scripts/test-appimage.sh (run against box 2226)

`scripts/test-appimage.sh <mfb> --box 2226 --libc glibc --gui` reports three failures on
box 2226 (Debian 12 aarch64 glibc, GTK4) with a correct compiler. None of the three is a
defect in the program under test: two checks need `readelf`, which the box does not have,
and the third assumes an ssh session can never reach a display, which is false on this
box. The script's default glibc box, 2228, refused connections on 2026-10-12, so 2226 was
the only glibc GTK box available and the Linux AppImage proof could not go green.

**The single correct behavior a fix produces:** run against box 2226 with a correct
compiler, the script exits 0, and it still exits non-zero when the inner ELF's
`DT_NEEDED` or `DT_RUNPATH` is wrong or when the AppImage does not start.

References:

- `.ai/compiler.md`, "App-mode proof runs on the GTK boxes" — why the script asserts on
  the inner ELF's `DT_NEEDED` (a wrongly linked musl binary launches identically).
- `.ai/remote_systems.md` — box 2226 has "no C compiler or linker on the box" and "a live
  GNOME Wayland session (`/run/user/1001/wayland-0`)".
- Found while moving the test-only environment variables behind `--debug` (branch
  `worktree-env`); the same three failures occur with the unmodified `main` checkout at
  `471aaff16`, so that change did not cause them.
- bug-987 (the Windows scripts default to a retired box) is the same class: a box script
  whose assumptions about its box went stale.

## Failing Reproduction

```
cargo build --release --bin mfb
bash scripts/test-appimage.sh target/release/mfb --box 2226 --libc glibc --gui
```

- Observed (identical on `main` at `471aaff16` and on `worktree-env`, 2026-10-12):

  ```
  --- glibc on box 2226 (linux-aarch64) ---
      (FUSE mount available)
  FAIL: glibc: could not read DT_NEEDED (is readelf installed on the box?)
  FAIL: glibc: never reached the GTK bootstrap (exit 0):
  FAIL: glibc: expected DT_RUNPATH $ORIGIN/../lib, got ''
  skip: glibc GUI case (no reachable X display; set MFB_APPIMAGE_DISPLAY)
  Linux AppImage runtime tests failed: 3
  ```

- Expected: every check passes or is skipped with a stated reason, exit 0.

The other 13 checks of the same run pass (mode 0755, extract, tree match,
`desktop-file-validate`, the corrupted-superblock negative, the vendored-library pair, and
the six window-member rows).

| Environment | Details | Result |
| --- | --- | --- |
| box 2226 | Debian 12 aarch64 glibc, GTK4, live Wayland session, no binutils | fails ✗ (3 checks) |
| box 2228 | Ubuntu x86_64 glibc, the script's default | not measured — connection refused on 2026-10-12 |
| boxes 2227, 2225, 2224 | the musl boxes | not measured — connection refused on 2026-10-12 |

## Root Cause

Three checks, two causes. Both were confirmed on the box on 2026-10-12.

**1. `readelf` is not installed on box 2226** (failures 1 and 3).
`ssh -p 2226 test@127.0.0.1 'command -v readelf objdump; echo rc=$?'` prints `rc=1` and
nothing else. `scripts/test-appimage.sh:check_needed` runs `readelf -d` **on the box**
and fails when the output is empty; the `DT_RUNPATH` check in the vendor case does the
same and compares the empty string with `$ORIGIN/../lib`. Both send stderr to
`/dev/null`, so "command not found" is indistinguishable from "the ELF has no such
entry", which is why the second message reads as a linker defect.

**2. An ssh session on box 2226 reaches the Wayland session** (failure 2).
The liveness check runs the release AppImage and accepts the run only if its output
contains `appimage-marker-ok`, `Failed to open display` or `Gtk-WARNING`. That list
assumes the box has no display, so GTK always fails to open one. On 2226 the ssh
session has `XDG_RUNTIME_DIR=/run/user/1001`, which holds the `wayland-0` socket, and
GTK connects to it with `WAYLAND_DISPLAY` unset. Measured with a `--debug` app-mode
binary over plain ssh: `WAYLAND_DISPLAY=[] DISPLAY=[]`, and `timeout 6 ./tr` exits 124
(the window opened and the program parked); with `env -u XDG_RUNTIME_DIR` the same binary
exits 1 with `Gtk-WARNING **: cannot open display:`. So on this box the release program
opens a real window, prints its marker **into the window**, and exits 0 with empty
stdout — a success the check reports as "never reached the GTK bootstrap (exit 0)".

Related, same cause: the `--gui` case probes only X displays (`xdpyinfo` on `:0`–`:4`),
finds none, and skips — on the one box that has a display, because that display is
Wayland. It is a skip and not a failure, but it means the windowed case has never run
there.

The script's box table still says `2226  aarch64 glibc   offline at time of writing`;
nothing in it was written against that box.

## Goal

- The reproduction above exits 0 on box 2226 with a correct compiler.
- A wrong `DT_NEEDED` or `DT_RUNPATH` still fails the script on every box, including one
  without `readelf`.
- A box where a tool the script needs is missing produces a message that names the
  missing tool, not a message about the program under test.

### Non-goals (must NOT change)

- The `DT_NEEDED` and `DT_RUNPATH` assertions must not become skips on a box without
  `readelf`. They are the only thing that distinguishes a wrongly linked musl binary
  from a correct one (`.ai/compiler.md`), so skipping them where they cannot run turns a
  red script green for the wrong reason. This is the tempting wrong fix.
- The liveness check must not accept "exit 0 with empty output" unconditionally: on a
  box with no display that outcome means the program never ran.
- No change to the compiler or to what an AppImage contains.

## Blast Radius

Found with `grep -n "readelf\|xdpyinfo\|Failed to open display" scripts/*.sh`.

- `scripts/test-appimage.sh:check_needed` — fixed by this bug.
- `scripts/test-appimage.sh`, the vendor case's `DT_RUNPATH` read — fixed by this bug.
- `scripts/test-appimage.sh`, the liveness check — fixed by this bug.
- `scripts/test-appimage.sh`, the `--gui` display probe — same assumption (X only);
  in scope, since fixing the liveness check needs the same "which display, if any"
  answer.
- Other box scripts (`test-canvas-vulkan.sh`, `test-canvas-pointer.sh`,
  `test-canvas-gpu-rows.sh`) — unaffected: all three passed against box 2226 on
  2026-10-12 (`test-canvas-pointer.sh --target linux-aarch64`, exit 0;
  `test-canvas-vulkan.sh --box 2226 --target linux-aarch64`, exit 0). They run headless
  (`MFB_HEADLESS=1`) and use no ELF tool on the box. `test-canvas-gpu-rows.sh` was not
  run that day, but `grep -n "xdpyinfo\|readelf" scripts/test-canvas-gpu-rows.sh` finds
  neither, so it shares neither assumption.

## Fix Design

1. **Read the ELF on the host, not on the box.** The script already has the AppDir
   locally (it compares the extracted tree against it by sha256, and that check passes),
   so the inner ELF it would inspect on the box is byte-identical to a local file. Read
   `DT_NEEDED` and `DT_RUNPATH` from the local copy with a host tool. macOS has no
   `readelf` by default, so the choice of tool is the open decision below. This removes
   the box-side dependency entirely rather than adding a probe for it.
2. **Make the liveness check say what it saw.** Decide first whether the box has a
   display the ssh session reaches (the measurement above: `XDG_RUNTIME_DIR` holding a
   `wayland-*` socket, or a reachable X display). With no display, keep today's rule.
   With one, force the no-display path for this check (run with `XDG_RUNTIME_DIR` and
   `DISPLAY` unset, so GTK reports that it cannot open a display) and leave the windowed
   proof to the `--gui` case.
3. **Teach the `--gui` case about Wayland**, so it runs on 2226 instead of skipping.
4. Correct the box table in the script header.

Rejected: installing binutils on 2226 without root (`apt-get download` + `dpkg -x`, as
was done for `strace`). It fixes one box and leaves the script failing the same way on
the next box without it.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Record the reproduction above as the failing case (it is the script itself; there
      is no unit under it to test separately).
- [x] Blast-radius audit: every site has a verdict above.

Acceptance: the script fails on 2226 for the three documented reasons; the audit has a
verdict for every site.
Commit: —

### Phase 2 — the fix

- [ ] Host-side `DT_NEEDED` / `DT_RUNPATH` reads in `scripts/test-appimage.sh`.
- [ ] Display-aware liveness check.
- [ ] Wayland in the `--gui` display probe.
- [ ] Correct the header's box table.

Acceptance: the reproduction exits 0 on 2226; nothing in Non-goals changed.
Commit: —

### Phase 3 — validation

- [ ] Negative proof: point the host-side read at an ELF with a wrong `DT_NEEDED`
      (a musl binary checked as glibc) and confirm the script goes red.
- [ ] Run the script on 2228 and 2227 when they are reachable, `--libc both`.
- [ ] `cargo test --test scripts_index_is_complete` if a helper file is added under
      `scripts/`.

Acceptance: green on 2226; still green on 2228/2227; red on the negative case.
Commit: —

## Validation Plan

- Regression test(s): `scripts/test-appimage.sh` against box 2226, plus the negative
  case in Phase 3.
- Runtime proof: the reproduction command, exit 0, on 2226.
- Doc sync: `.ai/remote_systems.md` (the app-mode proof table lists 2226 as "often
  down"; update it with what the script now needs from a box) and `scripts/README.md`
  if the usage line changes.
- Full suite: not affected; the script is not part of `cargo test`.

## Open Decisions

- Which host tool reads the dynamic section — a few lines of Python parsing
  `PT_DYNAMIC` (recommended: no new dependency, works on macOS and Linux hosts) vs.
  requiring `llvm-readelf` on the host.
- Whether the liveness check on a box with a display should force the no-display path
  (recommended, above) or assert on the window instead. The second duplicates the
  `--gui` case.

## Summary

The risk is entirely in not weakening the script: both fixes have an easy version that
makes the failures disappear by skipping the assertion. The compiler and the AppImage
format are untouched.
