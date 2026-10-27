### 1. Constants table shows the Float value for every Fixed constant, contradicting the overview
UNIT:      man-pkg:math
PAGE:      package-wide (overview Constants table)
CATEGORY:  consistency
CLAIM:     Constants table: `math::piFixed │ Fixed │ 3.141592653589793`; overview prose: "A Fixed form holds the nearest value Fixed can represent ... (math::piFixed is 3.141592653701081)" and "The Constants table below gives every value as written."
VERDICT:   inconsistent
EVIDENCE:  Probe `io::print(toString(math::piFixed, toByte(15)))` prints `3.141592653701081`; `math::eFixed` prints `2.718281828565523`; `math::pi2Fixed` prints `1.570796326734125`. The table's values come from `src/codegen/builtins/math/mod.rs:CONSTANTS`, where the Fixed rows reuse the Float literal ("Both the Float and the Fixed form fold to the same decimal shorthand"). So a reader who trusts the table (which the prose tells them to) gets a value that is wrong from the 10th digit, and the prose's own `3.141592653701081` disagrees with the row immediately below it. The Fixed spec confirms a 1/2^32 fraction, so the Fixed value genuinely cannot equal the Float one.
SUGGESTED: Descriptor-side: give each `*Fixed` row its real value (`3.141592653701081`, `2.718281828565523`, …), or render the Fixed rows as the Float literal they are declared from and say so. Prose-side: replace "gives every value as written" with "gives the Float value each pair is derived from; the Fixed twin holds the nearest Fixed, e.g. math::piFixed is 3.141592653701081" so the two sources no longer contradict.

### 2. The unit never explains `^` versus math::pow
UNIT:      man-pkg:math
PAGE:      package-wide (overview, pow)
CATEGORY:  overview-mismatch
CLAIM:     "The math package provides the numeric functions the language operator set does not spell — ... power ..."
VERDICT:   misleading
EVIDENCE:  The operator set does spell power: probe `2 ^ 10` prints `1024`, `2.0 ^ 3.0` prints `8.00`. But `2.0 ^ 0.5` raises ErrFloatDomain ("a non-whole or negative ^ exponent"), and `mfb spec language operators` says "The ^ operator's domain is distinct from math::pow; do not infer its domain from that function." Neither the overview nor the pow page mentions `^` (`grep -c '\^' src/codegen/builtins/math/func_pow.rs` → 0 prose hits). The one question a developer arrives with — "I have `^`, when do I need pow?" — is unanswered, and the overview's framing denies the operator exists.
SUGGESTED: Overview: "Power: `^` handles whole-number, non-negative exponents (and is the only power form for Integer); math::pow is the Float/Fixed form that also accepts fractional and negative exponents." pow page: one sentence saying the same and a See also to the operators spec.

### 3. No path from an Integer into sqrt/log/exp/trig/pow, and no truncation
UNIT:      man-pkg:math
PAGE:      package-wide (overview; rounding family)
CATEGORY:  coverage
CLAIM:     Overview: "Most members take a single number and give back the same type they were given" — nothing says which types are refused, or how to convert.
VERDICT:   missing
EVIDENCE:  Probe `math::sqrt(4)` and `math::pow(2, 10)` fail to compile (TYPE_CALL_ARGUMENT_MISMATCH). `math::sqrt(toFloat(n))` compiles. The unit never names `toFloat`/`toFixed` (`grep -c toFloat /tmp/.../all.txt` → 0). Likewise the rounding family offers floor/ceil/round but no round-toward-zero; probe `toInt(-2.7)` prints `-2`, `toInt(2.7)` prints `2`, so `toInt` is the truncation the developer is looking for, and no math page points at it.
SUGGESTED: Overview, after the type paragraph: "sqrt, pow, exp, log/log10 and the trigonometric functions take Float or Fixed only; convert an Integer first with toFloat or toFixed. To truncate toward zero rather than floor/ceil/round, use toInt." Add the same toInt pointer to floor/ceil/round as a See also-style sentence.

### 4. math::rand never says it is not cryptographically secure, nor points to crypto::randomInt
UNIT:      man-pkg:math
PAGE:      rand, seed, overview
CATEGORY:  coverage
CLAIM:     rand: "returns a uniformly-distributed value in the inclusive range [min, max], drawn from this thread's random sequence" — no statement of fitness for security use.
VERDICT:   missing
EVIDENCE:  `grep -n crypto src/codegen/builtins/math/func_rand.rs src/codegen/builtins/math/func_seed.rs src/codegen/builtins/math/mod.rs` → no hits. The reverse pointer exists: `src/codegen/builtins/crypto/func_random_int.rs:45-46` says math::rand's helpers "are fast and seedable but **not** cryptographically secure. Use crypto::randomInt whenever the value must be unpredictable." A developer reading only the math unit (tokens, shuffles, lottery picks) is never told this.
SUGGESTED: rand description: "The sequence is fast and reproducible via math::seed, not unpredictable: for tokens, keys or anything an adversary must not guess, use crypto::randomInt." Mirror one clause in the overview's random sentence.

### 5. Three different words for the List overloads
UNIT:      man-pkg:math
PAGE:      package-wide
CATEGORY:  consistency
CLAIM:     Overview: "list forms"; abs/max/min/atan2: "array forms"; asin/atan/exp/tan: "the List OF Float vectorized form"; sin/cos/sqrt/log: "a List OF Float form".
VERDICT:   inconsistent
EVIDENCE:  `grep -ic 'list form' all.txt` → 2, `grep -ic 'array form'` → 4, `grep -ic 'vectorized form'` → 4. MFBASIC has no "array" type and "vectorized" is implementation vocabulary; the overview says "Each function's page lists its forms" using the word "list forms".
SUGGESTED: Use "list form" everywhere ("the List OF Float list form", "the List OF Integer/Float/Fixed list forms"); drop "array" and "vectorized".

### 6. ErrFloatNaN appears in six Errors tables with no condition anywhere in the unit
UNIT:      man-pkg:math
PAGE:      sin, cos, tan, atan, atan2, exp
CATEGORY:  consistency
CLAIM:     Errors tables list `77050013 ErrFloatNaN` on the Float overloads of sin, cos, tan, atan, atan2 and exp; none of those descriptions says when it is raised, while every other error in the package (ErrFloatDomain, ErrInvalidArgument, ErrOverflow, ErrFloatInf) is explained on its page.
VERDICT:   misleading
EVIDENCE:  `awk` over the rendered pages (see Errors rows above) shows ErrFloatNaN on those six pages; `grep -c ErrFloatNaN` in their Description text → only pow mentions it (for a different condition). `src/codegen/builtins/math/mod.rs` comment: "ErrFloatNaN stays on the Float forms of atan/tan/sin/cos even though a NaN operand is unconstructible today (every NaN/Inf-producing expression traps at its own observation boundary first)". Probe confirms: `math::sin(1.0 / z)` with `z = 0.0` fails at the division with 7-705-0015, never reaching sin. So the table promises an error the developer can never see, and the prose is silent.
SUGGESTED: One shared sentence on each of the six pages: "ErrFloatNaN is listed for the Float forms because the kernel checks for a NaN operand; no MFBASIC expression can deliver one (an Inf or NaN fails where it is produced), so in practice this function raises only [ErrOutOfMemory for the list form / ErrFloatInf, ErrOverflow for exp]."

### 7. Inverse-trig pages state their return range inconsistently
UNIT:      man-pkg:math
PAGE:      asin (versus acos, atan)
CATEGORY:  consistency
CLAIM:     acos intro: "returning an angle from 0 through pi radians"; atan desc: "in radians (in [-pi/2, pi/2])"; asin intro and desc: "returning radians" only, no range.
VERDICT:   inconsistent
EVIDENCE:  Rendered asin page (`mfb man math asin`): neither intro nor Description contains "pi/2" (`grep -c 'pi/2'` on that page → 0); acos and atan both state their range.
SUGGESTED: asin intro: "Arcsine (inverse sine), returning an angle from -pi/2 through pi/2 radians." Put the range sentence in the same position (intro or first sentence of desc) on all three pages.
