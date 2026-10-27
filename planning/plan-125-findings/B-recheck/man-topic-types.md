### 1. comparisons page says unions are comparable; the overview and the compiler say they are not
UNIT:      man-topic:types
PAGE:      types comparisons (§ Comparable and orderable), versus types overview (§ Comparability and copying)
CATEGORY:  consistency
CLAIM:     "Boolean, Nothing, enums, unions, and records are comparable but not orderable." (comparisons.md:77)
VERDICT:   wrong
EVIDENCE:  Overview says the opposite: "List, Map, Set, unions, functions, lambdas, threads, and resource handles are neither comparable nor orderable." Probe `/tmp/plan-125-scratch/B-recheck/man-topic-types/proj_p1_union_eq` (UNION Shape of Circle/Square, `IF a = b` on two Shape values) built with `mfb build -q` fails: `error[2-203-0061 TYPE_REQUIRES_COMPARABLE]: Operator '=' requires compatible comparable operands, got Shape and Shape.` The overview is correct; the subtopic is not.
SUGGESTED: In comparisons.md: "Boolean, Nothing, enums, and records are comparable but not orderable. Unions are neither: compare a union by MATCHing on the member first." This also matches the map and set pages, which already list unions as non-comparable.

### 2. Overview claims `Point[]` is the all-defaults record; the compiler rejects it
UNIT:      man-topic:types
PAGE:      types overview (§ Defining types)
CATEGORY:  overview-mismatch
CLAIM:     "Point[x := 5] is a Point whose y is 0, and Point[] is the all-defaults Point." (package.md:241)
VERDICT:   wrong
EVIDENCE:  Probe `proj_p3_defaults` with `LET p = Point[]` fails at build: `error[2-203-0024 TYPE_CONSTRUCTOR_ARITY_MISMATCH]: Constructor 'Point' has 0 argument(s), expected 2.` The other half of the sentence is right: with `LET q = Point[x := 5]` and `MUT m AS Point` the probe prints `0 0` (q.y and m.x).
SUGGESTED: Delete the `Point[]` clause. If the all-defaults value is worth naming, point at `MUT p AS Point`, which the same paragraph already gives as the source of the defaults. (If `Point[]` was intended to work, that is a compiler bug to file, not a sentence to keep.)

### 3. map and set pages promise behaviour for NaN keys that the numeric page says can never exist
UNIT:      man-topic:types
PAGE:      types map (§ Keys) and types set (§ Elements), versus types numeric (§ Float rules)
CATEGORY:  consistency
CLAIM:     "Key equality is a bitwise comparison, so Float keys distinguish +0.0 from -0.0 and treat NaN as equal to NaN" (map); "Element equality is a bitwise comparison, so Float elements ... treat NaN as equal to NaN" (set)
VERDICT:   misleading
EVIDENCE:  numeric page: "no user-accessible Float is non-finite. Finiteness is enforced at observation boundaries — where a Float becomes observable by being ... stored into a collection". Probe `proj_p4_nan_key` building `Map OF Float TO Integer { 0.0 / z := 1 }` runs and dies with `Error: 7-705-0013 Floating-point operation produced a NaN result` at the map literal, so no map or set can ever hold a NaN. The NaN half of the sentence describes an unreachable state and invites a developer to think NaN keys are possible. The signed-zero half is real and is the only part worth keeping (the comparisons page's table already says so).
SUGGESTED: On both pages: "Key equality is a bitwise comparison, so Float keys distinguish +0.0 from -0.0 (the = operator treats them as equal). A NaN can never reach a key or element: see mfb man types numeric." Same wording on the set page with "element".

### 4. map page's non-comparable list omits Set; the overview and set page include it
UNIT:      man-topic:types
PAGE:      types map (§ Keys)
CATEGORY:  consistency
CLAIM:     "List, Map, unions, functions, lambdas, threads, and resource handles are not comparable and cannot be used as keys."
VERDICT:   inconsistent
EVIDENCE:  Overview: "List, Map, Set, unions, functions, lambdas, threads, and resource handles are neither comparable nor orderable." Set page: "List, Map, Set, unions, ... are not comparable and cannot be set elements" and "A Set is itself not comparable, so it cannot be a Map key". Three pages, two lists; the map page is the odd one out and is exactly the page a developer asking "can a Set be a key" would open.
SUGGESTED: Add Set to the map page's list so all three pages carry the same sentence.

### 5. Overview calls Set "unordered"; the set page promises insertion order and the probe confirms it
UNIT:      man-topic:types
PAGE:      types overview (§ Containers) and types set (title + § Iteration)
CATEGORY:  consistency
CLAIM:     Overview: "Set OF T — an unordered, deduplicated collection". Set page title "Unordered Set values", then "FOR EACH over a set yields each element T once, in insertion order" and toList gives "the elements as a List OF T in insertion order".
VERDICT:   inconsistent
EVIDENCE:  Probe `proj_p2_set_order`: literal `{30, 10, 20}`, remove 10, add 10, add 5 iterates `30 20 10 5`; `Set OF String {"zeta","alpha","mid"}` iterates `zeta alpha mid`. So Set has a documented, observable order (insertion, with a re-added element moving to the end), unlike Map, whose order the unit calls implementation-defined. A developer reading only the overview gets "unordered" and would reasonably assume the Map rule applies to Set too.
SUGGESTED: Overview bullet: "Set OF T — a deduplicated collection that iterates in insertion order: each distinct element appears at most once ...". Set page title: "Deduplicated Set values" (or keep "Unordered" but open the Iteration section with "Unlike Map, a Set's order is defined: ..."). Also state on the set page what the probe shows, that removing and re-adding an element moves it to the end.

### 6. The string page uses `&` in every example but no page in the unit (or in `mfb man`) says what it is
UNIT:      man-topic:types
PAGE:      types string
CATEGORY:  coverage
CLAIM:     gap: the String page has Literals, Comparison, Conversions and Defaults sections but never names the concatenation operator; the overview's Nothing example and Defining-types example both rely on `&` with `toString(...)`.
VERDICT:   missing
EVIDENCE:  `grep -rn -i 'concaten\|the & operator\|\`&\`' src/docs/man/` prints nothing; `mfb man strings` mentions concatenation only under join/repeat. Probe `io::print("a" & 1)` fails `TYPE_BINARY_OPERATOR_MISMATCH: Operator '&' requires String operands, got String and Integer`, which is exactly the rule a newcomer needs and that explains the ubiquitous `toString` wrapping.
SUGGESTED: Add to the string page's Description: "Two Strings join with &; both operands must be String, so a number is converted first: "n=" & toString(n). & never converts for you, and + is not string concatenation." Then the Nothing and Defining-types examples in the overview have a page to lean on.

### 7. Overview names function types and never says where they are explained
UNIT:      man-topic:types
PAGE:      types overview (§ Description, § See also)
CATEGORY:  discoverability
CLAIM:     "Callable signatures such as FUNC(Integer) AS String are first-class function type forms rather than named built-in data types."
VERDICT:   missing
EVIDENCE:  That is the only mention of function types in the whole unit; the See also list has sixteen entries and `mfb man lambda` is not one of them. `mfb man lambda` line 36 onward is where the form is actually taught ("The type of a lambda ... is FUNC(argTypes) AS ..."). The string page similarly discusses grapheme clusters and never links `mfb man unicode`, which renders.
SUGGESTED: Append "(mfb man lambda)" to that sentence and add `mfb man lambda` to the overview's See also. Add `mfb man unicode` to the string page's See also next to `strings toScalars`.

Checked and found consistent: all 25 cross-references the unit makes (`mfb man flow forEach/match`, `tooling fmt`, `general toInt`, `collections zip/add/union/partition/toList/sort`, `strings toScalars/graphemes`, `money`, `bits`, `link`, `errors`, `thread`, `general`, `variable`, and the four `mfb spec` targets) render; Pair of comparable members is comparable (probe prints `pair equal`), agreeing with the "records whose fields are all comparable" rule on both the overview and comparisons page; a union-typed record field cannot be omitted from a constructor (probe error), agreeing with "A field with no default (... a union) must be given"; the Error/ErrorLoc shape on the overview matches `mfb man errors`; the Partition and Pair field names match the overview; the numeric and comparisons pages agree on Money and Scalar operand rules.
