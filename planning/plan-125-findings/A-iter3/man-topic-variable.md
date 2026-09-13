### 1. “Copyable record” has two incompatible meanings
UNIT:      man-topic:variable
PAGES:     mfb man variable; mfb man types
CATEGORY:  divergence
QUOTE-A:   mfb man variable: “A record is a value like any other, so it is copied on assignment — with one exception: a RES field is still an alias after the copy”
QUOTE-B:   mfb man types: “records whose fields are all copyable” are copyable.
VERDICT:   The pages use “copyable” differently for records containing RES fields. The variable page’s behavior is real: the scratch program assigning a `Holder` record containing `RES fs::File` compiled successfully with `mfb build /tmp/plan-125-scratch/A-iter3/man-topic-variable`. `types` must distinguish assignment of such a record from a fully independent copy.
SUGGESTED: Records with RES fields may be assigned; their value fields copy, while each RES field remains an alias. See mfb man variable.

### 2. Resource type pages re-explain variable’s owned model
UNIT:      man-topic:variable
PAGES:     mfb man variable; mfb man fs types; mfb man tcp types
CATEGORY:  redundancy
QUOTE-A:   mfb man variable: “This page is the one place the whole model is written down, so that no other page has to explain it.”
QUOTE-B:   mfb man fs types / mfb man tcp types: “A second name for one is an alias, not a copy, and it closes itself when its binding's scope ends. A handle may be a field of a record and an element of a collection — written List OF RES <Type>, never a bare List OF <Type>.”
VERDICT:   `variable` explicitly owns this model, but each resource types page repeats four model rules before linking there. Keep only resource-specific behavior on each package page.
SUGGESTED: “For RES-handle aliases, scope closing, and record or collection use, see mfb man variable.”