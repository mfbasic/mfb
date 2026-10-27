### 1. Overview's "single-line form" claim sends readers into a trap the if page never defuses
UNIT:      man-topic:flow
PAGE:      package-wide (overview Description; `if`)
CATEGORY:  overview-mismatch
CLAIM:     "Blocks either use a dedicated terminator such as END IF, END WHILE, or LOOP UNTIL, or a single-line form whose statements are separated with :."
VERDICT:   misleading
EVIDENCE:  Probe `/tmp/plan-125-scratch/B-recheck/man-topic-flow/p3_ifcolon` (`IF x > 0 THEN io::print("a") : io::print("b")` with `x = -1`) printed `b` then `end`: the statement after `:` is a separate following statement, not part of the IF. Probe `p8_ifblock_oneline` shows the actual one-line multi-statement form is the terminated block written with colons (`IF x > 0 THEN : io::print("a") : io::print("b") : END IF`, and `MATCH x : CASE 1 : ... : END MATCH`), which printed `a`, `b`, `one`. Every colon example on the `do`, `for`, `forEach`, `while` pages also keeps its terminator (`NEXT`, `END WHILE`, `LOOP`). So the overview's "either/or" is false in both directions: the colon forms are terminated blocks, and the only terminator-less form (single-line IF) takes exactly one statement, which the if page states but never contrasts with `:`.
SUGGESTED: Overview: "Every block form ends with its terminator (END IF, NEXT, END WHILE, LOOP). Any block can be written on one line by separating its statements with `:`, terminator included. The one exception is single-line IF ... THEN statement [ELSE statement], which takes exactly one statement per branch and no END IF; a `:` after it starts a new statement outside the IF." Add the same one sentence to the `if` page's single-line paragraph, with the `IF c THEN : a : b : END IF` spelling as the way to get two statements on one line.

### 2. Named EXIT FOR / CONTINUE FOR on a FOR EACH variable is documented only on the `for` page
UNIT:      man-topic:flow
PAGE:      forEach
CATEGORY:  consistency
CLAIM:     "EXIT FOR leaves the loop and CONTINUE FOR skips to the next item." (the whole of forEach's exit coverage)
VERDICT:   inconsistent
EVIDENCE:  `for` page: "They act on the FOR or FOR EACH loop whose variable is i". Probe `p5_exitforeach` (`EXIT FOR x` where `x` is a FOR EACH variable, from inside a nested counted FOR) compiled and printed `1,1 1,2 1,3 2,1`, confirming it works on FOR EACH. `mfb spec language control-flow` lines 77-81 says the same. The forEach page says nothing about the named form or `UNKNOWN_FOR_IDENTIFIER`, and the overview's EXIT paragraph also only describes the bare "innermost enclosing loop" form. A developer who reads forEach (the page the overview routes collection users to) cannot discover the feature.
SUGGESTED: On forEach: "EXIT FOR item and CONTINUE FOR item name this loop by its variable, so an inner loop can leave or continue it directly; see mfb man flow for." On the overview EXIT paragraph add: "EXIT FOR name and CONTINUE FOR name target the FOR or FOR EACH loop whose variable is name."

### 3. No page says what the loop variable is: immutable, scoped to the loop, bounds evaluated once
UNIT:      man-topic:flow
PAGE:      for (and forEach)
CATEGORY:  coverage
CLAIM:     gap: the `for` page describes type, STEP, overflow and float non-progress but never says whether the body may assign the loop variable, whether it is visible after NEXT, or when start/end/step are evaluated.
VERDICT:   missing
EVIDENCE:  Probe `p2_assign` (`i = i + 3` in the body) fails with `TYPE_ASSIGN_REQUIRES_MUT: Binding i is immutable`. Probe `p1_scope` (`io::print(toString(i))` after NEXT) fails with `SYMBOL_UNKNOWN_IDENTIFIER: Identifier i is not declared in this scope`. `mfb spec language control-flow` line 23-24: "Each operand is evaluated once, before the first test." All three are the first questions a BASIC developer brings (classic BASIC lets you alter the counter and read it after the loop). Nothing in `mfb man flow --all` or `mfb man variable` (grep for "loop variable" returned nothing) answers them.
SUGGESTED: Add to `for` Description: "The loop variable is immutable inside the body and exists only until NEXT; assigning it is TYPE_ASSIGN_REQUIRES_MUT and reading it after the loop is an unknown identifier. start, end and STEP are each evaluated once before the first test, so changing a variable they were computed from does not change the loop." Add the scope/immutability sentence to forEach for the item variable (probe p2's rule applies there too, same binding kind).

### 4. forEach never says what happens to the collection you are iterating when you change it
UNIT:      man-topic:flow
PAGE:      forEach
CATEGORY:  coverage
CLAIM:     gap: no sentence on reassigning or rebuilding the source collection inside the loop.
VERDICT:   missing
EVIDENCE:  Probe `p7_mutate` (reassign `xs = collections::append(xs, 99)` inside `FOR EACH x IN xs`) printed `1 2 3` for the running loop and `after 1 2 3 99` for a second loop: the loop iterates the value the source had when the loop began, and reassignment never affects it. This is the one FOR EACH behaviour every developer coming from a language with "concurrent modification" errors asks about, and it follows directly from the value model the overview's see-also (`mfb man variable`) teaches, but forEach does not say it.
SUGGESTED: "FOR EACH iterates the value the source expression had when the loop started. Assigning a new value to the source variable inside the body does not change the sequence being visited; the next loop over that variable sees the new value."

### 5. The pipeline page is written for the compiler contributor and sits oddly in a "statements" topic
UNIT:      man-topic:flow
PAGE:      pipeline; overview
CATEGORY:  overview-mismatch
CLAIM:     Overview title "Control-flow statements and block forms" and Description "MFBASIC control flow is statement-based and structured" never mention `|>`; pipeline page: "the parser substitutes the left-hand expression into the _ placeholder on the right-hand side and emits the resulting call AST with no pipeline node remaining".
VERDICT:   misleading
EVIDENCE:  Rendered `mfb man flow --all`: the overview Description does not introduce the pipeline operator at all; only the Topics bullet does. "AST", "pipeline node", "parse time", "no runtime cost" are §3 internals vocabulary (`.ai/man-content.md` §3, §9: "No IR ... no lowering"); the page's first See also is `mfb spec language operators`, the contributor surface, rather than a man page. A developer looking for `|>` will search operators/types, not "flow statements", and when they arrive they get a parser description instead of the two rules they need (left-to-right, `_` evaluated once per occurrence).
SUGGESTED: Overview Description: add one sentence "The pipeline operator |> is an expression form, not a statement; it is documented here because it chains the calls a flow statement would otherwise nest (mfb man flow pipeline)." Pipeline page: replace the parse-time/AST paragraph with "a |> f(_, x) means exactly f(a, x); the pipeline is spelling, not a value or a type, so there is nothing to declare and nothing to import." Point See also at `mfb man types comparisons`/`mfb man lambda` and keep the spec link last, if at all.

### 6. `ELSE IF` (two words) is a parse error and the unit never warns
UNIT:      man-topic:flow
PAGE:      if
CATEGORY:  coverage
CLAIM:     gap: the if page shows `ELSEIF` but never says the two-word spelling is rejected.
VERDICT:   missing
EVIDENCE:  Probe `p4_elseif` (`ELSE IF x = 2 THEN` in a block IF) fails with `MFB_PARSE_UNEXPECTED_TOKEN: Expected end of statement after ELSE`. Most BASIC dialects and all C-family languages accept `ELSE IF`; the error text does not mention ELSEIF, so the page is the only place a developer can learn the fix.
SUGGESTED: In the block-form paragraph: "The keyword is the single word ELSEIF; ELSE IF on one line is a parse error, because ELSE must end its line."

Checked and found consistent, for the audit trail: EXIT of the wrong kind inside a loop is a compile error (`p6`, `p9`: `EXIT_NO_MATCHING_LOOP`), matching the overview's "of the matching kind"; every `mfb man` cross-reference on all eight pages renders; map/set ordering wording on forEach matches `mfb man types map` and `types set` verbatim; RETURN inside a loop (`p10`) works and is owned by the errors/lambda/variable topics rather than this unit, which is acceptable given the overview's see-also to `errors`.
