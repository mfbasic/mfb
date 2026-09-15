### 1. `zero` is not an MFBASIC duration expression
UNIT:      man-page:datetime/negate  
CLAIM:     "Negation is the same operation as `datetime::minus(zero, d)`."  
VERDICT:   misleading  
EVIDENCE:  `/tmp/plan-125-scratch/C-phase3/man-page-datetime-negate/zero-identifier/src/main.mfb` fails to build: `SYMBOL_UNKNOWN_IDENTIFIER: Identifier 'zero' is not declared in this scope.` The implementation in `src/codegen/builtins/datetime/func_negate.rs:BODY` computes the inverse by normalizing negated fields.  
SUGGESTED: `Negation is the same operation as \`datetime::minus(datetime::duration(0), d)\`.`