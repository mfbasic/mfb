### 1. “Non-blocking” exchange begins with a blocking connect/write
UNIT:      man-pkg:http
PAGE:      package-wide
CATEGORY:  overview-mismatch
CLAIM:     “the five-call non-blocking client (`startRead`/`ready`/`pump`/`done`/`finish`) drives an exchange without blocking the calling thread.”
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/http/helper_start_exchange.rs:20,30` calls `tls::connect`/`tcp::connect` before returning the stream, with `__HTTP_CONNECT_TIMEOUT_MS`; `src/codegen/builtins/http/helper_limits.rs:20` sets it to 30000. `src/codegen/builtins/tcp/func_connect.rs:15-34` says a positive timeout waits until connection completion or the deadline. The rendered `mfb man http startRead` also says the whole request is written before `startRead` returns.
SUGGESTED: Replace with: “The five-call client performs connection setup and sends the request in `startRead`; after that returns, `ready`/`pump`/`done`/`finish` drive receipt of the response without blocking the calling thread.”

### 2. Overview omits the custom-response starting point
UNIT:      man-pkg:http
PAGE:      package-wide
CATEGORY:  discoverability
CLAIM:     “the response constructors (`ok`/`status`/`json`/`withHeader`/`respondFile`/`respondPath`) build a `http::Response`.”
VERDICT:   missing
EVIDENCE:  `mfb man http responseDefault` renders `http::responseDefault()` as the base for `WITH` edits; `src/codegen/builtins/http/func_response_default.rs:9-21` says it is the function to use when the other constructors do not produce the desired response shape. The overview list in `src/codegen/builtins/http/mod.rs:193-194` omits it.
SUGGESTED: Add `responseDefault` to the response-construction group, e.g. “the response builders (`responseDefault`/`ok`/`status`/`json`/`withHeader`/`respondFile`/`respondPath`) build a `http::Response`.”