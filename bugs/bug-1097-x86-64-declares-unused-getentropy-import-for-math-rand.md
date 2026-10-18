# bug-1097: linux-x86_64 declares a libc `getentropy` import for `math::rand`/`math::seed` that no emitted code calls

Last updated: 2026-10-17
Effort: small (<1h)
Severity: LOW
Class: Dead-code

Status: Open
Regression Test: src/target/linux_x86_64/plan.rs `tests::math_rand_does_not_import_getentropy` (to be added)

On linux-x86_64 the startup seed for `math::rand` comes from the raw `getrandom`
syscall (nr 318). The import plan still declares libc `getentropy` for
`math.rand`/`math.seed`. Nothing calls it. The import lands in the dynamic symbol table
of both the glibc and musl executables. It is also one of the imports that stop an
x86-64 program from linking as a static ELF (`src/os/linux/link/mod.rs`
`encode_executable_bytes` takes the static path only when `image.imports.is_empty()`).

**The single correct behavior a fix produces:** the linux-x86_64 import plan declares
`getentropy` only for calls whose emitted code calls it (`crypto.randomBytes` does,
directly). A `math::rand` program's x86-64 import list does not contain `getentropy`.
aarch64 and riscv64 keep it, because their entry seed calls libc `getentropy`.

References:

- bug-71 (`bugs/completed/bug-71-stale-dead-imports-comments-backends.md`) fixed the same
  unused import for `fs.createTempFile`. Under "Latent (NOT fixed)" it lists this exact
  case and leaves it because `src/docs/spec/linker/08_linux-x86_64.md` documents the
  import as present. That spec text is itself wrong (see Root Cause), so the spec
  changes with the fix.
- `src/target/linux_common/plan.rs:LinuxPlan::native_call_imports` carries a comment
  that defers the question to "the bug-71 family". This document is that follow-up.
- Sibling: bug-1096 (a program that calls only `math::seed` still draws startup
  entropy); found in the same review.

## Failing Reproduction

```
mkdir -p /tmp/b1097/src
cat > /tmp/b1097/project.json <<'EOF'
{ "name": "r", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [ { "root": "src", "role": "main", "include": ["**/*.mfb"] } ],
  "entry": "main", "targets": ["native"] }
EOF
printf 'IMPORT io\nIMPORT math\n\nFUNC main AS Integer\n  io::print(toString(math::rand(1, 6)))\n  RETURN 0\nEND FUNC\n' > /tmp/b1097/src/main.mfb
for t in linux-x86_64 linux-aarch64 linux-riscv64 macos-aarch64 windows-x86_64; do
  target/release/mfb build -q --ncode --target $t /tmp/b1097 >/dev/null
  python3 -I -c '
import json,sys
d=json.load(open(sys.argv[1]))
imps=[x["symbol"] for x in d["imports"] if "ntropy" in x["symbol"] or "GenRandom" in x["symbol"]]
callers=sorted({f["name"] for f in d["functions"] for i in f["instructions"]
  if i.get("op") in ("bl","call") and ("ntropy" in i.get("target","") or "GenRandom" in i.get("target",""))})
print(sys.argv[2], "imports:", imps, "called from:", callers)' /tmp/b1097/r.ncode $t
done
```

- Observed (measured 2026-10-17, release build of main at 2a29186d0):
  - `linux-x86_64 imports: ['getentropy'] called from: []`  ✗
  - `linux-aarch64 imports: ['getentropy'] called from: ['program.entry']`
  - `linux-riscv64 imports: ['getentropy'] called from: ['program.entry']`
  - `macos-aarch64 imports: ['_getentropy'] called from: ['program.entry']`
  - `windows-x86_64 imports: ['BCryptGenRandom'] called from: ['program.entry']`
- On x86-64 the entry instead emits `mov rax, 318` / `svc` (raw `getrandom`) before
  `bl _mfb_rng_seed_at`.
- Expected: `linux-x86_64 imports: [] called from: []`; the other four lines unchanged.

The import is also visible in a real executable:
`target/release/mfb build --nobj --target linux-x86_64 /tmp/b1097` → `r.nobj` lists
`"library": "libc.so.6"` / `"symbol": "getentropy"`.

## Root Cause

`src/target/linux_common/plan.rs:LinuxPlan::native_call_imports` returns
`self.libc_import("getentropy", ...)` for `"math.rand" | "math.seed"` on every Linux
backend. It does not check `self.abi.raw_getrandom`. The x86-64 backend sets
`raw_getrandom: true` (`src/target/linux_x86_64/plan.rs`, its `LinuxAbi`), and its
`emit_random_bytes` (`src/target/linux_x86_64/code.rs`) is the raw syscall. So the
only consumer of that import, the entry `seed_rng` block
(`src/codegen/engine/function/entry.rs:lower_program_entry`), never references it on
x86-64. The `fs.createTempFile` arm in `LinuxPlan::runtime_imports` already applies
this gate (`if matches!(spec.call, "fs.createTempFile") && !self.abi.raw_getrandom`,
from bug-71). The `math.rand`/`math.seed` arm was left without it.

The spec repeats a false claim. `src/docs/spec/linker/08_linux-x86_64.md` ("Entropy and
app mode") and `src/docs/spec/stdlib/07_math-rng.md` ("Main thread — OS entropy") say
the "`math::rand`/`math::seed` runtime helpers … import/use `getentropy` on all
targets". On every target, `runtime.rng_next` and `runtime.rng_seed_at` call no
entropy source; only `program.entry` does (the "called from" column above). The
helpers are pure PCG64.

## Goal

- The repro prints `linux-x86_64 imports: [] called from: []`; the other targets are
  unchanged.
- `crypto.randomBytes` on x86-64 still imports `getentropy` (it calls it directly;
  bug-71's test `crypto_random_bytes_imports_getentropy` stays green).

### Non-goals (must NOT change)

- The x86-64 seed source (raw `getrandom`), the seed value path, or any emitted
  instruction. Only the import table and the ELF dynamic-symbol metadata change.
- aarch64/riscv64/macOS/Windows imports.
- Do not edit the spec only to match the code, and do not delete the import from the
  generic Linux arm without the `raw_getrandom` gate. aarch64/riscv64 need it.

## Blast Radius

Search: `grep -rn 'getentropy' src/target` and
`grep -rn 'raw_getrandom' src/target`.

- `src/target/linux_common/plan.rs:LinuxPlan::native_call_imports` (`math.rand`/`math.seed` arm) — fixed by this bug.
- `src/target/linux_common/plan.rs:LinuxPlan::runtime_imports` `fs.createTempFile` arm — already
  gated (bug-71); unaffected.
- `crypto.randomBytes` import — unaffected: `crypto::func_random_bytes` calls libc
  `getentropy` directly on every Linux target, so the import is live.
- linux-aarch64 / linux-riscv64 (`raw_getrandom: false`) — unaffected; import live.
- macOS / Windows plans — unaffected (different import tables; seed call is live).
- Spec: `linker/08_linux-x86_64.md` "Entropy and app mode" and `stdlib/07_math-rng.md`
  (the parenthetical about runtime helpers) — corrected by this bug.
  `linker/09_linux-riscv64.md` ("A program that uses `math::rand`/`math::seed` also
  imports `getentropy` for its startup seed (x86-64 uses the `getrandom` syscall
  instead)") — already correct for riscv64 and already implies x86-64 has no import;
  unaffected (bug-1096 may narrow its `math::seed` wording).
- Goldens: `tests/byte-identity/math` (`math.linux-x86_64.ncodesum`) and any other
  x86-64 fixture that calls `math::rand` will change hash. The `.ncode` `imports` list
  loses one entry. That diff is expected; any other byte change is a bug-hunt trigger.

## Fix Design

In `LinuxPlan::native_call_imports`, change the arm to
`if matches!(target, "math.rand" | "math.seed") && !self.abi.raw_getrandom`, mirroring
the `fs.createTempFile` gate. Replace the deferral comment with that rationale. If
bug-1096 lands first, the arm is `"math.rand"` only; the gate is the same.

Rejected: an override in `linux_x86_64/plan.rs` that filters the import out. The
`LinuxAbi.raw_getrandom` field exists so the shared plan can decide this in one place
(bug-321).

## Phases

### Phase 1 — failing test (no behavior change)

- [ ] Add `math_rand_does_not_import_getentropy` beside bug-71's tests in
      `src/target/linux_x86_64/plan.rs` `mod tests`: for both flavors,
      `platform().native_call_imports("math.rand", "_main")` (and `"math.seed"`) contains
      no `getentropy`. Add the converse in `linux_aarch64/plan.rs` and
      `linux_riscv64/plan.rs` (the import is present). Confirm the x86-64 test fails.

Acceptance: the x86-64 test fails for the documented reason; the aarch64/riscv64 tests pass.
Commit: —

### Phase 2 — the fix

- [ ] Gate the arm on `!self.abi.raw_getrandom` in `src/target/linux_common/plan.rs`.
- [ ] Correct the spec text listed in Blast Radius: the runtime helpers use no entropy
      source; only the entry seed does, through `emit_random_bytes`.

Acceptance: Phase 1 tests pass; `crypto_random_bytes_imports_getentropy` still passes.
Commit: —

### Phase 3 — regenerate expected outputs + validation

- [ ] `scripts/artifact-gate.sh <mfb> math`. Confirm the only change is in
      `linux-x86_64` hashes, and that rebuilding the `.ncode` and diffing it against
      the previous one shows only the removed `getentropy` import entry. Correct only
      those `.ncodesum` lines. Before re-baselining, run every golden gate in full
      (`scripts/artifact-gate.sh <mfb> all`, full `scripts/test-accept.sh`) per AGENTS.md.
- [ ] Landing tier `bash scripts/test-fast.sh` + the new tests by name.
- [ ] Re-run the repro; on a Linux x86-64 box confirm a `math::rand` executable still
      runs and prints varying values across runs.

Acceptance: gates green; the golden delta is exactly the removed import; the repro passes.
Commit: —

## Validation Plan

- Regression tests: the plan unit tests above.
- Runtime proof: the repro, plus running the x86-64 `math::rand` executable on a Linux
  x86-64 box (`.ai/remote_systems.md`) and seeing different output across runs.
- Doc sync: the spec topics listed in Blast Radius.
- Landing gate: `bash scripts/test-fast.sh`, the new tests, `scripts/artifact-gate.sh <mfb> math`.
- Full tier: the scheduled background run.

## Open Decisions

- None.

## Summary

A one-condition fix plus a spec correction. The only real work is re-baselining the
x86-64 `.ncodesum` lines and showing their delta is just the dropped import.
