### 1. `poll` is described as Boolean/no-error despite its list overload

UNIT:      man-pkg:tcp  
PAGE:      tcp::setReadTimeout  
CATEGORY:  consistency  
CLAIM:     “Use `tcp::poll` instead when the question is ‘is there data?’ rather than ‘read, but not forever’: poll answers with a `Boolean` and raises nothing.”  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/tcp/func_set_read_timeout.rs:DESC` contains the claim. `src/codegen/builtins/tcp/func_poll.rs:DESC` and the rendered `mfb man tcp --all` state that `tcp::poll(List OF RES Socket, timeoutMs)` returns a ready `Socket` and raises `ErrTimeout` when its deadline expires.  
SUGGESTED: “For one socket, use `tcp::poll` when the question is ‘is there data?’: it returns a `Boolean` and an expired deadline is `FALSE`. The list form instead returns a ready socket or raises `ErrTimeout`; see `tcp::poll`.”

### 2. Overview reverses the `IMPORT net` distinction

UNIT:      man-pkg:tcp  
PAGE:      package-wide  
CATEGORY:  overview-mismatch  
CLAIM:     “Only the address-valued members are affected: `tcp::connect`, `tcp::listen`, `tcp::read`, and `tcp::write` need nothing but `IMPORT tcp`.”  
VERDICT:   misleading  
EVIDENCE:  Rendered by `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man tcp --all`. `src/codegen/builtins/tcp/func_local_address.rs:register` and `func_remote_address.rs:register` return `net::Address`; `func_read.rs:register` and `func_write.rs:register` do not involve an address. A no-network probe compiled without `IMPORT net` when it passed `tcp::remoteAddress(sock)` directly to `tcp::connect(address, 0)` (`mfb build /tmp/plan-125-scratch/B-phase3/man-pkg-tcp/import_probe_project`); adding `address.port` produced `TYPE_UNKNOWN_VALUE` instructing “add `IMPORT net`.”  
SUGGESTED: “This requirement applies when reading fields of an address returned by `tcp::localAddress` or `tcp::remoteAddress`. You can pass an address directly to `tcp::connect` without importing `net`; `tcp::read` and `tcp::write` do not use addresses.”