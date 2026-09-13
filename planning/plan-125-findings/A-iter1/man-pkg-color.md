### 1. Public colour constants are undocumented and undiscoverable
UNIT:      man-pkg:color  
PAGE:      package-wide  
CATEGORY:  coverage  
CLAIM:     No rendered page mentions the public `color::black` through `color::orange` constants.  
VERDICT:   missing  
EVIDENCE:  `src/codegen/builtins/color/constants.rs:BASIC` registers 16 public constants through `pkg.add_constant`; `rg -n 'color::(black|white|orange)' /tmp/plan-125-scratch/A-iter1/man-pkg-color/{overview,all}.txt` printed no matches. A probe built with `mfb build /tmp/plan-125-scratch/A-iter1/man-pkg-color/probe` and run at `.../build/probe.out` printed `#000000` and `#ffa500` for `color::black` and `color::orange`.  
SUGGESTED: Add a “Constants” section to the package overview listing the 16 opaque CSS basic colours and calling out that `color::green` is `#008000` while vivid green is available through `color::fromName("lime")`.