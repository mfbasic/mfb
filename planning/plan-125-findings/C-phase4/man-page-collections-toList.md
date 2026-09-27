### 1. First example does not list elements
UNIT:      man-page:collections/toList
CLAIM:     "List the elements of a set:"
VERDICT:   misleading
EVIDENCE:  Compiling and running the example verbatim with the specified release binary printed only `3`; it prints `len(elems)`, not any element or their insertion order.
SUGGESTED: Print the elements, for example: “Print the elements of a set in insertion order:” followed by a `FOR EACH n IN elems` loop that calls `io::print(toString(n))`.