### Tier 1 — pure-source, reuses encoding's `Mfb` machinery verbatim (0 `src/target` files)

Cheapest wins; each just relocates the descriptor, converts `Rewrite`→`Mfb`, migrates docs, wires man2. No native code to touch.

1. **csv** — 0 target files, `csv_package.mfb`
2. **json** — 0 target files, `json_package.mfb`
3. **regex** — 0 target files, `regex_package.mfb`

### Tier 2 — source companion + light native (Mfb bodies + a handful of `Native` fns)

4. **datetime** — 2 files, large source companion
5. **process** — 4 files
6. **money** — 3 files (`builder_money_math`), `money_package.mfb`
7. **app** — 3 files
8. **vector** — 1 file, but SIMD value-record types (`Vec2/3`) add descriptor-type work

### Tier 3 — coupled clusters (migrate together to avoid half-cut seams)

9. **net** + **http** — `net::Url`/`http::Response` types are shared; http's source leans on net (net 7 files, http 0)
10. **astrings** + **term** + **strings** — bound by `term_astrings_bridge.mfb` and strings' Tier-B `__astrings_*` transforms; **strings also unblocks the shared `find`/`mid`/`replace` List overloads still pending from collections** (7 files each)
11. **crypto** — 5 files, five `.mfb` companions (hash/aead/ecdsa/ed25519/util)
12. **audio** — 5 files, MML + render source

### Tier 4 — descriptor / data-only (no `.mfb`, little-to-no lowering to move; mostly relocate + docs)

13. **errorcode** — data-only table
14. **testing** — descriptor + desugar
15. **general** — overridable builtins (`toString`/`len`); touches every package's override table
16. **resource** — RES subsystem
17. **bits** — 3 files, inline bit ops (small `Native`/inline)

### Tier 5 — heavy native leaves (collections::get/`Native` pattern at scale; the big `src/target` payloads)

Do these last — most code to move, highest byte-identity risk, most arch-specific.

18. **os** — 5 files (syscalls)
19. **math** — 7 files, ~6,222 lines (SIMD/transcendental/fixed-point) *(you deferred this)*
20. **fs** — 9 files (filesystem syscalls)
21. **io** — 11 files (print/read/stdin, per-arch)
22. **thread** — 9 files (concurrency runtime)
23. **tls** — 13 files (TLS/network runtime, most target-coupled)