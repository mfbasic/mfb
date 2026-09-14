### 1. `min` omits zero and negative-bound behavior
UNIT:      man-page:math/rand
CLAIM:     “The lowest value the result may take, inclusive.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/math/gen_math.rs:CodeBuilder::lower_math_rand` rejects only `min > max`; the scratch probe printed `zero=0`, `negative=-10`, `money-zero=0.00`, and `money-negative=-1.00`.
SUGGESTED: The inclusive lower bound. Zero and negative values are valid; `min` must not exceed `max`.

### 2. `max` omits zero and negative-bound behavior
UNIT:      man-page:math/rand
CLAIM:     “The highest value the result may take, inclusive. Must not be below min.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/math/gen_math.rs:CodeBuilder::lower_math_rand` compares only `min > max`; the scratch probe successfully ran equal zero and negative Integer and Money bounds.
SUGGESTED: The inclusive upper bound. Zero and negative values are valid; it must not be below `min`.