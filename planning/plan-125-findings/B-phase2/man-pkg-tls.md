### 1. Overview contradicts `tls::close` on repeat close

UNIT:      man-pkg:tls  
PAGE:      package-wide  
CATEGORY:  overview-mismatch  
CLAIM:     “`tls::close` closes the handle and treats an already-closed handle as success rather than an error.”  
VERDICT:   wrong  
EVIDENCE:  `mfb man tls` prints this claim, while `mfb man tls close` lists `ErrResourceClosed` and says “a second close raises ErrResourceClosed.” `src/codegen/builtins/tls/func_close.rs:register` declares `ErrResourceClosed` for both overloads; all platform close lowerings emit that error for a closed handle.  
SUGGESTED: Replace with: “`tls::close` closes a handle early; calling it again on that handle raises `ErrResourceClosed`.”

### 2. `read` and `write` imply accepted sockets are unsupported

UNIT:      man-pkg:tls  
PAGE:      tls::read and tls::write  
CATEGORY:  consistency  
CLAIM:     The `sock` parameters say a connected socket is “as returned by `tls::connect`.”  
VERDICT:   inconsistent  
EVIDENCE:  `mfb man tls accept` says its result is “indistinguishable from a client Socket: read and write it with tls::read and tls::write.” `src/codegen/builtins/tls/func_accept.rs:register` returns `tls.Socket`; `func_read.rs:register` and `func_write.rs:register` accept that same type. A no-network probe assigning `tls::accept(...)` to `tls::Socket` and passing it to both calls compiled successfully: `mfb build /tmp/plan-125-scratch/B-phase2/man-pkg-tls/type-probe` printed `Wrote executable`.  
SUGGESTED: On both pages, say: “A connected TLS socket to receive from/send on, as returned by `tls::connect` or `tls::accept`.”