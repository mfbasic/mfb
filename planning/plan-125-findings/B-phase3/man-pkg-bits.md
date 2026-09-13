### 1. Bitwise functions are called Boolean operations
UNIT:      man-pkg:bits
PAGE:      package-wide
CATEGORY:  consistency
CLAIM:     “The Boolean operations are named `band`/`bor`/`bxor`/`bnot`…”
VERDICT:   inconsistent
EVIDENCE:  `mfb man bits` says `AND`/`OR`/`XOR`/`NOT` are Boolean operators and these functions supply the omitted integer bitwise operations. `rg -n 'name: "(band|bor|bxor|bnot)"|ty: ParameterType::Integer' src/codegen/builtins/bits/func_{band,bor,bxor,bnot}.rs` shows each function takes `Integer`, not `Boolean`.
SUGGESTED: “The bitwise operations are named `band`/`bor`/`bxor`/`bnot` because `and`/`or`/`xor`/`not` are reserved logical keywords and cannot be package member identifiers.”

### 2. `ctz` gives an unusable bitwise-AND idiom
UNIT:      man-pkg:bits
PAGE:      bits::ctz
CATEGORY:  consistency
CLAIM:     “`ctz` composes with the lowest-set-bit idiom `value AND -value`…”
VERDICT:   inconsistent
EVIDENCE:  `mfb man bits` says `AND` is Boolean-only. A probe containing `LET lowest AS Integer = 40 AND -40` failed with `TYPE_BINARY_OPERATOR_MISMATCH`: “Operator `AND` requires Boolean operands, got Integer and Integer.” Replacing it with `bits::band(40, -40)` built and ran successfully. The source is `src/codegen/builtins/bits/func_ctz.rs:33`.
SUGGESTED: “`ctz` composes with the lowest-set-bit idiom `bits::band(value, -value)`, which clears every bit but the lowest one…”

### 3. The overview sends readers to a declaration that lacks the promised sign information
UNIT:      man-pkg:bits
PAGE:      package-wide
CATEGORY:  overview-mismatch
CLAIM:     “The functions do not interpret sign except where a signature says so — `sra`, the arithmetic right shift.”
VERDICT:   misleading
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man bits sra` renders the declaration as `bits::sra(value AS Integer, count AS Integer) AS Integer`; it contains no signed annotation. The sign behavior appears only in the parameter description and prose. `src/codegen/builtins/bits/func_sra.rs` likewise declares `ParameterType::Integer`.
SUGGESTED: “The functions treat inputs as bit patterns except `sra`, which shifts its value as a signed two’s-complement quantity.”