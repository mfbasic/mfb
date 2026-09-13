### 1. Overview promises a wider command reference than it provides
UNIT:      man-topic:tooling  
PAGE:      package-wide  
CATEGORY:  overview-mismatch  
CLAIM:     “This topic documents the developer-facing commands that read or rewrite source rather than produce a build artifact.”  
VERDICT:   misleading  
EVIDENCE:  `mfb man tooling --all` lists only `fmt`, while `mfb --help` lists `audit [options] [path]` as “Report security and code audit findings”; `src/cli/dispatch.rs` dispatches it through `audit::parse_options` and `audit::run`. `mfb man tooling audit` prints `error: unknown tooling topic page 'audit'`.  
SUGGESTED:  Either narrow the overview to “This topic documents `mfb fmt`,” or add discoverable developer pages for the other source-reading commands, beginning with `mfb audit`.

### 2. Formatter indent range is undocumented
UNIT:      man-topic:tooling  
PAGE:      fmt  
CATEGORY:  coverage  
CLAIM:     The `--indent <N>` entry says only “Number of spaces per indentation level (default: 2).” No page states its accepted range or that zero is valid.  
VERDICT:   missing  
EVIDENCE:  `src/cli/fmt.rs:parse_indent` accepts `0..=256`; probing `mfb fmt --indent 0 /tmp/plan-125-scratch/B-phase1/man-topic-tooling/indent0.mfb` exited `0` and produced unindented source, while `mfb fmt --indent 257` printed “must be between 0 and 256.”  
SUGGESTED:  Change the option entry to: “Number of spaces per indentation level, from 0 through 256 (default: 2). A value of 0 removes computed indentation.”