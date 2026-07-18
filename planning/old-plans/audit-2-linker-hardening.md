# Audit 2 — Surface 6: Custom linker & emitted-binary hardening (Mach-O / ELF)

Last updated: 2026-07-14
Untrusted party: an attacker exploiting a runtime memory bug in an emitted
binary. Must not benefit from disabled mitigations (non-PIE / no-NX / no-RELRO /
no-canary) the platform should provide by default.

Scope read: `src/os/linux/link/{elf,mod}.rs`, `src/os/{linux,macos}/object.rs`,
`src/os/macos/link/{macho,commands,mod}.rs`, `src/os/macos/icon.rs`,
`src/arch/*/encode`. Evidence from freshly built binaries (`readelf -h/-l`,
`otool -hv/-l`).

Observed headers:
- Linux aarch64/x86_64 (dynamic): `ET_EXEC`, entry `0x401000`, base `0x400000`,
  5 phdrs (`PT_PHDR, PT_INTERP, PT_LOAD R-X, PT_LOAD RW-, PT_DYNAMIC`) — **no**
  `PT_GNU_STACK`, `PT_GNU_RELRO`, `PT_GNU_PROPERTY`.
- macOS aarch64: flags `NOUNDEFS DYLDLINK TWOLEVEL PIE`; `__TEXT` R+X;
  `__DATA_CONST` `SG_READ_ONLY` (0x10); `__DATA` RW; `__PAGEZERO` present.
- `grep stack_chk|canary|GNU_STACK|GNU_RELRO|gnu.property|BTI` over `src/`: zero
  hits — none generated on any target.

## Verdict on prior audit-1 findings (re-verified)

| ID | Prior sev | Verdict | Evidence |
|----|-----------|---------|----------|
| LNK-01 | HIGH | **STILL OPEN** | Linux `ET_EXEC` (`elf.rs:30,98,168`) at fixed `IMAGE_BASE=0x400000` (`mod.rs:7`). macOS is PIE. → **bug-186**. |
| LNK-02 | MEDIUM | **STILL OPEN** | no `PT_GNU_STACK` in any Linux phdr list (observed 5 phdrs, none GNU_STACK). |
| LNK-03 | MEDIUM | **PARTIAL — macOS FIXED, Linux STILL OPEN** | macOS `__DATA_CONST` now carries `SG_READ_ONLY` (`commands.rs:65`) → GOT read-only after fixups. Linux emits no `PT_GNU_RELRO`; GOT/`.dynamic` sit in the writable `PT_LOAD` despite `DT_FLAGS=DF_BIND_NOW` (`elf.rs:632`). |
| LNK-04 | MEDIUM | **FIXED** | static ELF now emits two PT_LOADs: text `p_flags=5` (R+X) + separate data `p_flags=6` (R+W) — `elf.rs:44-62,112-130`. W^X holds; constant data no longer executable. |
| LNK-05 | LOW | **STILL OPEN** | no `.note.gnu.property`/BTI/PAC note; no landing pads at indirect-branch targets (`emit_import_stub`, `linux/link/mod.rs:459-467`). |
| LNK-06 | LOW | **FIXED at linker; residual in object encoder → LNK-11** | linker encoders reach-check + return `Err` (`mod.rs:507,524,538,396`; macOS `:487,504`; x86 PLT `:450`). |
| LNK-07 | LOW | **STILL OPEN (build-time only)** | `read_u32`/`write_u32` index-panic on bad offset (`linux/link/mod.rs:551-557`, `macos/link/mod.rs:515-521`). Offsets are compiler-generated → not attacker-reachable; build-robustness only. |

## New findings

### LNK-08 — MEDIUM — No read-only data segment: program constants are writable at runtime (both platforms)
- Location — Linux: all constant data (string literals, constant tables) is in the
  single R+W `PT_LOAD` (`elf.rs:54-67`, `p_flags=6`); no read-only load. Location
  — macOS: constants populate the writable `__DATA` (`macho.rs:152-158`,
  `commands.rs:103-132`, initprot/maxprot `0x3`); only GOT/init-pointers get
  read-only `__DATA_CONST`.
- Threat/impact: an arbitrary-write primitive can corrupt constants an attacker
  should not be able to touch (rewrite a format string, a dispatch table, a
  constant used in a security check) — amplifies a contained bug.
- Mechanism: the linker segregates only the GOT into a read-only region; the
  zero-init arena global is co-located with constants in `image.data` and must be
  writable, but constants and the arena global are never separated, so the whole
  blob maps writable.
- Best fix: partition `image.data` (constant vs mutable) at codegen and emit the
  constant partition read-only (Linux: separate R-only `PT_LOAD` / RELRO; macOS:
  read-only `__DATA_CONST`/`__TEXT,__const`). → **bug-187**.
- Non-goals: the arena model; program behavior.

### LNK-09 — LOW (informational) — No stack-smashing protection anywhere in emitted code
- Location: no canary/`__stack_chk_*` emission (grep zero hits); frames generated
  directly by `src/arch/*/encode`. Stack buffer overflows in generated frames are
  undetected at return. The compiler would have to add canaries itself (platform
  won't auto-insert), so this is a design gap, not a disabled default. LOW absent
  a demonstrated overflowable generated stack buffer (the model relies on
  bounds-checked collection/string runtime).

### LNK-10 — LOW (informational) — macOS ad-hoc signature omits hardened runtime; `__DATA_CONST` maxprot stays RW
- Location: code-directory flags `0x20002` = `CS_ADHOC|CS_LINKER_SIGNED`
  (`commands.rs:522-523`); `CS_RUNTIME (0x10000)` not set. `__DATA_CONST`
  `maxprot=0x3` (`commands.rs:62`) permits a runtime `mprotect` back to writable
  despite `SG_READ_ONLY`. Defense-in-depth; ad-hoc signing is normal for local
  builds. Best fix for distributed builds: set `CS_RUNTIME` and lower
  `__DATA_CONST` maxprot to R.

### LNK-11 — LOW (build-time codegen correctness) — Object-encoder `branch_imm26`/`branch_imm19` truncate silently
- Location: `src/arch/aarch64/encode/sizing.rs:138-141` (`branch_imm26`),
  `:143-146` (`branch_imm19`) mask (`& 0x03ff_ffff` / `& 0x0007_ffff`) with **no
  reach check**, unlike the linker copies that now `Err` (LNK-06). An intra-function
  branch exceeding ±128 MiB / ±1 MiB wraps to a wrong target — miscompile, not
  attacker-controlled (compiler's own control flow). Same silent-truncation class
  LNK-06 eliminated; left unfixed here. Best fix: add the same reach check.

## Non-applicable notes
- iOS Mach-O codesigning: plan-30 written but not implemented; no arm64e /
  entitlement/provisioning path exists to audit.
- riscv64 ELF shares `encode_static_elf`/`encode_dynamic_elf` with aarch64 — same
  `ET_EXEC`/phdr posture (LNK-01/02/03/05 apply); only `e_machine=243`/`e_flags`
  differ, neither security-relevant.
- `icon.rs`: build-time PNG→icns decode, size/format-validated; nothing reaches
  emitted programs.

## Verdict

LNK-01 (Linux non-PIE) remains the single HIGH → bug-186. LNK-02 and the Linux
half of LNK-03 remain MEDIUM (hardening; fold into the PIE rework). New LNK-08
(writable program constants) is the most actionable new MEDIUM → bug-187.
LNK-04, the macOS half of LNK-03 (`SG_READ_ONLY`), and the linker-level LNK-06
are fixed. LNK-05/07/09/10/11 are LOW.
