# PCG64 Random Number Generator

`math::rand` and `math::seed` are backed by a **PCG64 XSL-RR 128/64** generator:
a 128-bit linear-congruential state with a 64-bit permuted output. This topic
specifies the *algorithm* — the state advance, the output permutation, the
seeding dance, and how the main thread and each spawned thread obtain their
seeds. The 128-bit state is **stored** in the per-arena arena-state words at
offsets 88/96; that storage (and the *separate* memory-fill stream at 16/24) is
owned by `./mfb spec memory arenas` and is not duplicated here.

The per-function API (signatures, parameters, errors) is owned by
`./mfb man math`. This topic specifies the model behind it.

## Constants

The generator uses the canonical PCG64 128-bit constants. Each is held as two
64-bit limbs (high, low). [[src/target/shared/code/error_constants.rs:PCG_MULT_HI]]

| Name | Value (128-bit) | Role |
|------|-----------------|------|
| `MULT` | `0x2360ED051FC65DA4_4385DF649FCCF645` | LCG multiplier |
| `INC`  | `0x5851F42D4C957F2D_14057B7EF767814F` | LCG increment (fixed default stream) |

The stream is fixed: every generator on every thread uses the same `INC`, so
two generators seeded identically produce identical sequences. There is no
selectable PCG stream; independence between threads comes from distinct seeds,
not distinct increments. [[src/target/shared/code/error_constants.rs:PCG_INC_HI]]

## State advance

One step is the truncated 128-bit LCG recurrence

```text
state := state * MULT + INC          ; all arithmetic mod 2^128
```

computed from the two 64-bit limbs `(lo, hi)` held in registers. The product is
the low 128 bits of `state * MULT`; the increment is added with carry across the
limbs. [[src/target/shared/code/entry_and_arena.rs:emit_pcg_step]]

```text
emit_pcg_step(lo, hi):
  ; 128-bit truncated product state * MULT
  p_lo  := MULT_LO * lo                       ; mul   -> low limb of product
  carry := umulh(MULT_LO, lo)                 ; high half of MULT_LO*lo
  p_hi  := carry + (MULT_LO * hi) + (MULT_HI * lo)
  ; add 128-bit INC with carry between limbs
  lo := p_lo + INC_LO   (adds, sets carry)
  hi := p_hi + INC_HI + carry   (adc)
```

The cross term `MULT_HI * hi` is dropped because it contributes only to bits
≥128 and the state is truncated to 128 bits. The step reads `(lo, hi)` at entry
and rewrites them in place; `x11`–`x16` are scratch (caller-saved). The aarch64
encoders are `mul`, `umulh`, `adds`, `adc`.
[[src/arch/aarch64/abi.rs:unsigned_multiply_high_registers]]

## Output function (XSL-RR 128/64)

After advancing, the 64-bit result is the **XSL-RR** permutation of the new
128-bit state: XOR the two halves, then rotate right by a count taken from the
top 6 bits of the high half. [[src/target/shared/code/entry_and_arena.rs:lower_rng_next]]

```text
rot := hi >> 58                 ; top 6 bits of the high limb (0..63)
xsl := hi XOR lo                ; fold the 128-bit state to 64 bits
out := rorv(xsl, rot)           ; rotate-right by rot (low 6 bits of rot used)
```

`rorv` rotates by the low 6 bits of its count operand, which exactly matches the
6-bit `rot` value. The advance-then-output order means the value returned for a
call reflects the *post-step* state; the pre-seed/initial state is never
emitted. [[src/arch/aarch64/abi.rs:rotate_right_registers]]

`_mfb_rng_next` loads the state from the calling thread's arena (`x19`, offsets
88/96), runs `emit_pcg_step`, stores the advanced state back, and returns the
XSL-RR output in `x0`. It is a leaf helper and clobbers the caller-saved
registers, so `math::rand` spills its bounds across the call.
[[src/target/shared/code/error_constants.rs:RNG_NEXT_SYMBOL]]

## Seeding dance

Reseeding from a single 64-bit `seed` follows the canonical PCG initialization:
zero the state, step once, add the seed, step again. The result is stored to the
arena RNG words. [[src/target/shared/code/entry_and_arena.rs:lower_rng_seed_at]]

```text
_mfb_rng_seed_at(arena_ptr, seed):
  lo := 0 ; hi := 0
  step(lo, hi)                    ; state := INC
  lo := lo + seed (adds)          ; mix the 64-bit seed into the low limb
  hi := hi + 0 + carry (adc)
  step(lo, hi)                    ; state := (INC + seed) * MULT + INC
  store (lo, hi) to arena_ptr+88 / +96
```

The seed mixes into the low limb only; the carry propagates into the high limb.
The two `step` calls scramble the seed so that adjacent seed values (e.g. 41 and
42) yield well-separated streams. Any 64-bit value is a valid seed — including
0 and negative `Integer`s (interpreted as their two's-complement bit pattern).
Equal seeds produce identical subsequent `math::rand` sequences on the same
build. [[src/builtins/math.rs:SEED]]

## Per-thread seeding

Each OS thread owns its own arena, hence its own 128-bit RNG state and an
independent stream. The two threads' streams use the same `INC`; independence is
purely a function of distinct seeds.

### Main thread — OS entropy

Before any user code runs (including global initializers, which may call
`math::rand`), the program entry seeds the main thread's generator from the OS
entropy pool. The 8-byte seed scratch is pre-filled with the arena address so a
`getentropy` failure still yields a varying seed; `getentropy`/`getrandom`
overwrites it on success. The seed is then passed to `_mfb_rng_seed_at`.
[[src/target/shared/code/error_constants.rs:RNG_SEED_SYMBOL]]

```text
entry:
  scratch := arena_address          ; getentropy fallback
  getentropy(&scratch, 8)           ; overwrite with 8 OS-random bytes
  _mfb_rng_seed_at(arena, scratch)  ; seed words 88/96
```

The platform random-bytes seam is `getentropy` on macOS and Linux-aarch64; only
Linux-x86_64 uses the `getrandom` syscall. [[src/target/macos_aarch64/plan.rs:514]]

### Spawned thread — drawn from the parent stream

A thread started with `thread::start` is seeded from the *spawning* thread's
generator. The parent draws one 64-bit value with `_mfb_rng_next` (advancing the
parent's own stream) and uses it as the child's seed via `_mfb_rng_seed_at`. The
draw runs in the parent, where `x19` is the parent arena, so it is race-free; the
child arena pointer is reloaded from the thread control block afterward (the draw
clobbers `x0`–`x18`). [[src/target/shared/code/error_constants.rs:RNG_NEXT_SYMBOL]]

```text
parent (at thread spawn):
  seed  := _mfb_rng_next()                 ; advances PARENT stream
  child := control_block.arena_state
  _mfb_rng_seed_at(child, seed)            ; seed CHILD words 88/96
```

Consequences: a child's seed is determined by the parent's stream position, so a
parent that has `math::seed`-ed itself produces a deterministic child seed at
each spawn; and spawning a child advances the parent's own sequence by one draw.

## `math::rand(min, max)` bounding

`math::rand` validates `min <= max` (reporting `ErrInvalidArgument` otherwise),
computes the inclusive span, draws one raw 64-bit value from `_mfb_rng_next`, and
maps it into range. [[src/builtins/math.rs:RAND]]

```text
if min > max: report ErrInvalidArgument
span := (max - min) + 1                     ; wraps to 0 only for the full domain
raw  := _mfb_rng_next()
if span == 0:                               ; min=INT_MIN, max=INT_MAX
    result := raw                           ; return the raw draw directly
else:
    result := min + (raw mod span)          ; unsigned modulo
```

The reduction is a plain unsigned `raw mod span` (`udiv` + `msub`), so the
mapping is *not* rejection-sampled; for `span` values that do not divide `2^64`
evenly there is a negligible modulo bias. The full-domain case
(`min = INT_MIN`, `max = INT_MAX`) detects the `span == 0` wrap and returns the
raw 64-bit draw unmodified, covering every `Integer` uniformly.
[[src/target/shared/code/builder_math.rs:794]]

## `math::seed(value)` semantics

`math::seed(value)` reseeds **only the calling thread's** generator via
`_mfb_rng_seed_at(x19, value)` and returns `Nothing`; it produces no draw of its
own. Because each thread owns an independent generator and stream, reseeding one
thread never disturbs another. Calling `math::seed` is optional — every thread is
already seeded automatically (main from the OS, children from the parent) — and
is needed only to make a thread's subsequent sequence reproducible.
[[src/target/shared/code/builder_math.rs:883]]

## Relationship to the memory-fill RNG

The arena also holds a **second, separate** PCG64 stream (the memory-fill RNG at
offsets 16/24) used to scrub freed chunks and poison fresh blocks. It uses the
same step/seed algorithm but a different, independently-seeded state and its
output is never observable, so advancing it on every alloc/free never perturbs
the reproducible `math::rand` sequence at 88/96. That stream and the full
arena-state layout are specified by `./mfb spec memory arenas`.

## See Also

- `./mfb man math` — per-function API for `math::rand`, `math::seed`, and the rest of `math::`.
- `./mfb spec memory arenas` — arena-state layout, the 88/96 RNG storage words, and the separate memory-fill stream at 16/24.
- `./mfb spec language types` — the `Integer` (signed 64-bit) domain that bounds `math::rand`.
- `./mfb spec architecture frontend` — how `math::` builtins are lowered and the runtime RNG helpers are emitted.
