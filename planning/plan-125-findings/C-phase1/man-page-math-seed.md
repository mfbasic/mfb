### 1. Unseeded draws are not guaranteed to differ
UNIT:      man-page:math/seed
CLAIM:     "Until a program calls seed, the sequence starts from a fresh, automatically chosen seed, so unseeded draws differ from run to run."
VERDICT:   misleading
EVIDENCE:  An unseeded `math::rand(1, 1)` necessarily prints `1` on every execution; distinct initial generator state does not guarantee distinct bounded draws. `src/codegen/engine/function/entry.rs:405-426` initializes the RNG automatically, but `math::rand` may map different generator outputs to the same result.
SUGGESTED: Until a program calls `seed`, the sequence is automatically seeded, so do not rely on unseeded draws being reproducible.

### 2. Zero and negative seeds are undocumented
UNIT:      man-page:math/seed
CLAIM:     "The value to seed the generator with. The same seed replays the same sequence, which is what makes a run reproducible."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/math/func_seed.rs:29` accepts every `Integer` and declares no errors; `src/codegen/builtins/math/gen_math.rs:1074-1097` passes the integer directly to the seeding helper. Probe `seed-values/src/main.mfb` printed `zero=4014,4014` and `negative=230833,230833`, proving both `0` and `-1` are accepted and deterministic.
SUGGESTED: Any `Integer` seed is valid, including zero and negative values. Reusing the same value replays the same sequence.

### 3. Worker threads do not inherit the parent stream
UNIT:      man-page:math/seed
CLAIM:     "Seeding is per-execution context: a worker thread inherits the spawning thread's stream and then diverges independently."
VERDICT:   wrong
EVIDENCE:  `src/codegen/runtime/thread/runtime_helpers.rs:724-737` advances the spawning thread’s generator once, then uses that result to seed a separate worker generator. The thread probe printed `expected-child=158026869`, `actual-child=72961198`, `expected-parent=733666462`, and `actual-parent=733666462`: the worker’s first draw is not the parent stream’s first draw, while creating it advances the parent stream once.
SUGGESTED: Each thread has its own random sequence. Starting a worker derives its sequence from the spawning thread’s generator and advances that generator once; later draws in either thread proceed independently.