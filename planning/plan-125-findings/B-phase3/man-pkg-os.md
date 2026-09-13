### 1. Environment concurrency guarantee is documented backwards
UNIT:      man-pkg:os  
PAGE:      package-wide  
CATEGORY:  consistency  
CLAIM:     “They are not synchronized against a concurrent os::getEnv/os::environ running in another thread:: worker” (overview; repeated on `setEnv` and `unsetEnv`).  
VERDICT:   wrong  
EVIDENCE:  `rg -n 'emit_env_(lock|unlock)' src/codegen/builtins/os` shows `setEnv`, `unsetEnv`, `environ`, `hasEnv`, and the shared `lower_get_env` path all acquire and release the same lock. `src/codegen/builtins/os/gen_env.rs:123` explicitly says `lower_get_env` serializes `getenv` against concurrent `os::setEnv`; `func_set_env.rs:50` and `func_unset_env.rs:39` acquire that lock before changing the environment.  
SUGGESTED: Replace the warnings with: “Environment calls are synchronized with other MFBASIC `os` environment calls, so a concurrent read observes either the state before or after a completed change. A returned `os::environ()` map remains a snapshot.”