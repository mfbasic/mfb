# plan-00-D — Abstract Arena Base & Neutral Relocations

Last updated: 2026-06-29

> **Status: DONE (all phases).** `RelocIntent` (`Call`/`DataAddr{Hi,Lo}`/
> `GotLoad{Hi,Lo}`) replaces the AArch64 `kind` strings on `CodeRelocation`;
> binding stays. The AArch64 intent→kind table (`src/arch/aarch64/reloc.rs`)
> realizes each intent back to `branch26`/`page21`/`pageoff12` — used by the
> encoder (`emit_bl`/`emit_symbol_ref`) and the `-ncode` serializer, so `.ncode`
> and the linked binary stay byte-identical (the linker/`EncodedRelocation` are
> untouched). `arena_base` is a neutral operand: `lower_to_mir` renames the
> pinned realization register (`Aarch64RegisterModel::arena_base` → `x19`) to
> `arena_base`, `select_aarch64` renames it back — identity for codegen. The
> `-mir` dump now carries a neutral `relocations` array (intent names) and shows
> `arena_base`, never `x19`/`branch26`. Validation: full bin+integration tests
> green; codegen-selfdiff byte-identical except the pre-existing `bug-01`
> resource-union-drop non-determinism (flaky direct-vs-direct); all `.ncode`
> goldens (incl. GOT-reloc app fixtures) byte-identical; both `.mir` goldens
> regenerated; acceptance suite 975/975. The 36-fixture op-family round-trip
> sweep gained an `arena_base` load/store + neutrality asserts.

Two runtime/linking couplings still hard-wire AArch64 into the MIR: the **pinned arena-state
register** (`x19`) and the **AArch64 relocation-kind strings** in `CodeRelocation`.
Neutralize both (`mir.md §7`, §8; §12.4 resolved: arena base abstract, per-ISA pin-or-TLS).

Depends on plan-00-A/C. Stays AArch64-**byte-identical** under `-codegen mir`.

## 1. Goal

- **`arena_base`** — an abstract MIR source for the arena pointer. MIR code that reaches the
  arena references `arena_base`; it never names a global register. The AArch64 backend
  realizes it as the pinned `x19` (byte-identical); x86_64 will realize it as a TLS/memory
  load, rv64 as a pinned register (their plans).
- **Neutral relocation intents** — `CodeRelocation.kind` (today `branch26`/page21/pageoff12/
  GOT, all AArch64) becomes a neutral enum: `{Call, DataAddrHi, DataAddrLo, GotLoad, …}` +
  symbol + binding + library. The AArch64 backend maps intent → the concrete
  `R_AARCH64_*` it emits today.

### Non-goals

- No x86_64/rv64 realization here (those are H/I); AArch64 keeps pinning `x19` and emitting
  the same relocs — byte-identically. No object-format change (mach-o/elf writers untouched
  except for the intent→kind indirection).

## 2. Current State

`x19 = ARENA_STATE_REGISTER`, pinned program-wide and written at entry (`add x19, sp, #0` +
the arena-state stores; `_mfb_rt_main_arena` holds the base). Every helper + arena access
assumes `x19`. `CodeRelocation{from,to,kind:String,binding,library}` carries AArch64 kind
strings; the object writers consume them.

## 3. Design

- Replace `ARENA_STATE_REGISTER` uses in MIR-emitting code with an `arena_base` operand/op.
  The AArch64 backend's RegisterModel declares `arena_base → x19` (pinned, reserved from
  allocation) and the entry sequence initializes it — the existing behavior, now expressed
  as "the AArch64 realization of `arena_base`."
- Relocations: MIR/`CodeRelocation` carries a `RelocIntent` enum; a per-(ISA,OS) table maps
  intent → concrete reloc kind. AArch64+mach-o / AArch64+elf tables reproduce today's output.

## 4. Phases

1. `RelocIntent` enum + AArch64 intent→kind tables; retarget the reloc emit sites. Byte-id.
2. `arena_base` abstraction; AArch64 RegisterModel realizes it as pinned `x19`; entry init
   expressed through it. Byte-id.
3. Byte-identical gate (the entry sequence + every arena_alloc call site are the proof).

## 5. Validation

- Suite **byte-identical** under `-codegen mir` (the arena is touched by nearly every
  allocating program; the entry sequence + relocations are in every binary).
- `-mir` dumps reference `arena_base` and `RelocIntent`, not `x19`/`branch26`.

## Summary

Removes the last two "AArch64 leaks into the neutral layer": the pinned arena register and
the relocation vocabulary. Both become abstractions the AArch64 backend *realizes* (pinned
`x19`, `R_AARCH64_*`) byte-identically — and that x86_64 (TLS arena, ELF `R_X86_64_*`) and
rv64 (pinned arena, `R_RISCV_*`) realize differently, without the MIR knowing.
