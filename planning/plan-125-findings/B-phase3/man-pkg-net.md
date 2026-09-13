### 1. URL rendering is named inconsistently, and one spelling does not exist
UNIT:      man-pkg:net  
PAGE:      net types  
CATEGORY:  consistency  
CLAIM:     “A parsed URL, produced by net::toUrl and rendered back with net::toString.”  
VERDICT:   inconsistent  
EVIDENCE:  `mfb man net` and `mfb man net toUrl` call the operation bare `toString`; `mfb build /tmp/plan-125-scratch/B-phase3/man-pkg-net/url-render-probe` reports `Built-in package net does not export net.toString` for `net::toString(u)`. `src/codegen/builtins/net/helper_url_to_string.rs` routes universal `toString(url)`.  
SUGGESTED: “A parsed URL, produced by `net::toUrl` and rendered back with `toString`.”

### 2. Overview overstates which packages accept `net::Address`
UNIT:      man-pkg:net  
PAGE:      package-wide  
CATEGORY:  overview-mismatch  
CLAIM:     “net::Address is the shared endpoint record every transport speaks … [and] can be handed straight to any of them.”  
VERDICT:   misleading  
EVIDENCE:  `rg -n 'net::Address|ADDRESS_TYPE' src/codegen/builtins/http` prints no matches: the listed `http` package has no `net::Address` parameter or result. The same search finds Address support in `tcp`, `udp`, and `tls`.  
SUGGESTED: Say that `net::Address` is shared by `tcp`, `udp`, and `tls`, and can be passed directly to their applicable endpoint operations; keep `http` separate as its URL/request API.