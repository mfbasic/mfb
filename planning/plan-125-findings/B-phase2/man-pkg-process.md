### 1. Windows `didSignal` result contradicts its function page
UNIT:      man-pkg:process  
PAGE:      package-wide  
CATEGORY:  overview-mismatch  
CLAIM:     “Windows has no signals, so every delivered signal is the same forced termination there and `didSignal` reports `process::Signal.None` for every child.”  
VERDICT:   wrong  
EVIDENCE:  `rg -n -C 5 "error-severity|NTSTATUS|STATUS_ACCESS|didSignal|did_signal" src/codegen/builtins/process/func_did_signal.rs src/codegen/builtins/process/gen_windows.rs src/codegen/builtins/process/mod.rs` prints `func_did_signal.rs:36-38`: an NTSTATUS error-severity exit code, such as `STATUS_ACCESS_VIOLATION`, maps to `process::Signal.Error`; only other Windows outcomes map to `process::Signal.None`. The overview is `src/codegen/builtins/process/mod.rs:144-145`.  
SUGGESTED: Replace the final sentence with: “On Windows, every delivered terminating bucket uses forced termination. `process::didSignal` reports `process::Signal.Error` for an error-severity NTSTATUS exit code and `process::Signal.None` for other outcomes.”