# api-review spikes

One minimal MFBASIC program per finding from the built-in-package API review
(`bugs/bug-514` … `bugs/bug-535`). Each spike is the *failing reproduction* its
bug document cites: it holds the claim and the counter-claim side by side and
prints which one the compiler or the runtime actually implements.

Build and run any one with:

    ./target/release/mfb build spikes/api-review/<id>
    ./spikes/api-review/<id>/build/mfb_project.out

Two of them are **build-time** spikes and produce no executable — the evidence
is the compiler's own output (`bug-517`, `bug-535`).

| spike | question | answer |
|---|---|---|
| `bug-514-keypair-untagged` | `KeyPair` has no curve tag — does anything catch a 32-byte X25519 pair handed to an Ed25519 operation? | no. `convert` maps it silently; `encrypt` returns a well-formed 62-byte box that **neither** private key can open (`ErrAuthenticationFailed`, 77050016) |
| `bug-517-sha1-advisory-context` | does `CRYPTO_SHA1_INSECURE` distinguish hashing from HMAC? | no. `hash`, `hmac` and `hkdf` each get the identical "not collision-resistant" warning, which is only decisive for the first |
| `bug-518-withzone-doc` | does `withZone` move the instant, as its `zone` parameter row says? | no. instant 1782475200 before and after — the Description is right, the parameter row is wrong |
| `bug-519-parse-normalizes` | does `datetime::parse` reject month 13? | it did not, and it did not "carry it into the result" either: `2026-13-45 25:70:99` silently became `2027-02-15T02:11:39Z`, while `datetime::date(2026,13,45)` raised 77050002. **Fixed (bug-519):** `parse` and `parseIso` now raise `ErrInvalidFormat` (77050003); the spike re-run shows all three refusing |
| `bug-521-toiso-nanos` | is `toIso` → `parseIso` the round trip its man page promises? | it was not, for sub-millisecond values: 123456789 ns in, 123000000 ns out, 456789 ns lost. **Fixed (bug-521):** `toIso(dt, 9)` is lossless; the default form is unchanged and the page now states each form's precision |
| `bug-522-transfer-list-stale` | may a `tcp::Listener` cross a thread, as the package intro says and `thread::transfer` denies? | yes — it compiles, transfers and the worker returns. The `transfer` page's list is stale |
| `bug-523-res-shapes-undocumented` | can a `RES` handle be a record field and a collection element? | yes to both, and no resource type page mentions either — every one of the 11 is a single sentence about scope-exit closing |
| `bug-524-process-close` | does `process::close` close its handle, like every other `close` in the language? | no. `isRunning` is still TRUE, `receive` and `pid` still work |
| `bug-526-tls-poll-res` | is the signature `mfb man tls poll` prints a declaration that compiles? | no. `List OF tls::Socket` is `TYPE_RESOURCE_REQUIRES_RES`; `mfb man tcp poll` prints the same overload correctly |
| `bug-528-pad-display-width` | does padding to a scalar width align a terminal table? | no, in both directions: 3 scalars / 5 columns (emoji), 4 / 6 (CJK), 6 / 5 (NFD) |
| `bug-529-empty-needle` | what does the empty needle mean in `strings`? | four members, four answers: `contains` TRUE, `find` 0, `count` raises 77050002, `replace` a no-op |
| `bug-530-utf8encode-return-overload` | does `mfb man encoding utf8Encode` show both forms of the language's only return-type overload? | no. One `Declaration` line ending `AS List OF Byte`; the `List OF Integer` form appears only in prose, and an unannotated call is `TYPE_OVERLOAD_AMBIGUOUS` |
| `bug-531-find-absence` | how is "not found" reported? | two ways: `strings::find` raises `ErrNotFound` (77050004), `regex::find` returns `-1` |
| `bug-532-regex-span` | can you extract what a regex matched? | not directly. `findAll("a1b22c333", "\d+")` gives starts 1, 3, 6 and no lengths; the spike recovers `[1,2) [3,5) [6,9)` only by re-matching an anchored pattern at every candidate length |
| `bug-533-empty-pattern-replace` | do the two `replace` members agree on an empty needle? | opposites: `strings::replace("abc","","-")` = `"abc"`, `regex::replace("abc","","-")` = `"-a-b-c-"` |
| `bug-535-unused-runtime-helper` | does a `RES` bind off a thread channel, with no other call into the package, build? | no: `error: NIR declares unused runtime helper 'tcp'` — an internal message with no code and no source location |
