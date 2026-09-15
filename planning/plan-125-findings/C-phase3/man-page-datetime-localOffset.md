### 1. Negative epoch seconds omitted
UNIT:      man-page:datetime/localOffset
CLAIM:     "The instant, in seconds since the epoch, to ask about. The offset is not constant — a zone with daylight saving gives different answers at different times of year."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_local_offset.rs:register` accepts an unrestricted `Integer`; scratch probe `main.mfb` run with `TZ=America/New_York` printed `at=-1 off=-18000` and `at=0 off=-18000`. Negative values name instants before the epoch; zero is accepted.
SUGGESTED: Zero names the Unix epoch, and negative values name instants before it. Any whole-second Integer is accepted until the host cannot convert that instant, which raises `ErrInvalidArgument`.

### 2. macOS range estimate is materially wrong
UNIT:      man-page:datetime/localOffset
CLAIM:     "An instant outside the range the host can convert (roughly beyond ±10^16 seconds on macOS and glibc, and outside the FILETIME range on Windows) raises ErrInvalidArgument."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/datetime/func_local_offset.rs:lower_local_offset` delegates non-Windows conversion to `localtime_r` and raises `ErrInvalidArgument` only on its NULL result. On macOS, the scratch probe run with `TZ=UTC` printed `at=60000000000000000 off=0` and `at=70000000000000000 error=77050002`; the stated ±10^16 estimate understates the observed usable range by roughly sixfold.
SUGGESTED: An instant the host cannot convert raises `ErrInvalidArgument`. The exact range is platform-dependent.

### 3. Implementation relationship leaks into a developer page
UNIT:      man-page:datetime/localOffset
CLAIM:     "`localOffset` is the low-level intrinsic that backs `datetime::offsetAt` for local zones and `datetime::toLocal`; most code should prefer those higher-level functions, which operate on `datetime::Instant` and `datetime::Zone` values rather than a raw epoch-seconds `Integer`."
VERDICT:   out-of-scope
EVIDENCE:  The “backs” / “low-level intrinsic” relationship is an implementation fact (`src/codegen/builtins/datetime/func_local_offset.rs:lower_local_offset`), not information needed to call the function. The developer-relevant distinction is the argument type, confirmed by `register`’s `epochSeconds: Integer` descriptor.
SUGGESTED: `localOffset` accepts a raw epoch-seconds `Integer`; most code should prefer `datetime::offsetAt` or `datetime::toLocal`, which work with `datetime::Instant` and `datetime::Zone` values.