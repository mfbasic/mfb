### 1. Fixed overload names the wrong error

UNIT:      man-page:math/asin  
CLAIM:     “outside that domain a Float or List OF Float argument raises ErrFloatDomain, and a Fixed argument raises ErrInvalidArgument.”  
VERDICT:   wrong  
EVIDENCE:  Release-binary probe `/tmp/plan-125-scratch/C-phase1/man-page-math-asin/probe-project/src/main.mfb` printed `fixed-error=77050012` for `math::asin(1.1f)`, i.e. `ErrFloatDomain`, not `ErrInvalidArgument`. The same probe printed `scalar-error=77050012` and `list-error=77050012`.  
SUGGESTED: “Outside that domain, every overload raises ErrFloatDomain.”

### 2. Errors table advertises an error the binary does not raise

UNIT:      man-page:math/asin  
CLAIM:     “77050002 ErrInvalidArgument … Overloads 1, 2, 3”  
VERDICT:   wrong  
EVIDENCE:  The release-binary probe `/tmp/plan-125-scratch/C-phase1/man-page-math-asin/probe-project/src/main.mfb` trapped out-of-range Float, Fixed, and `List OF Float` calls. Its output was `scalar-error=77050012`, `fixed-error=77050012`, and `list-error=77050012`; none raised `77050002`.  
SUGGESTED: Remove the `ErrInvalidArgument` row from this member’s declared errors; retain `ErrFloatDomain` for overloads 1, 2, and 3.