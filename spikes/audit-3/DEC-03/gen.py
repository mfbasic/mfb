# DEC-03: json/regex/csv materialize the whole input as a per-element collection.
# Emit a 1.2 MB JSON array of 400000 `1`s.
import sys
n = 400000
sys.stdout.write("[" + ",".join("1" for _ in range(n)) + "]")
