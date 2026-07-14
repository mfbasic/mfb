# plan-30-E: `iap::` package over StoreKit 2

Last updated: 2026-07-07
Effort: medium (1h–2h)

This sub-plan adds the **`iap::`** package: MFBASIC In-App-Purchase support backed by
**StoreKit 2** (no legacy StoreKit 1), calling `Product.products(for:)`,
`Product.purchase(options:)`, `Transaction.updates`, and `Transaction.finish()`
through the 30-D Swift async bridge, marshalled to the MFBASIC worker thread over the
existing inbound/outbound resource channel (30-C). Purchases are verified from
StoreKit 2's `VerificationResult.verified` (JWS-signed transactions) — **no receipt
server, no on-device receipt parsing.**

The single behavioral outcome: an MFBASIC program on iOS can list products, start a
purchase, receive a verified transaction, finish it, and observe entitlement updates —
with a synchronous-looking API even though StoreKit is async underneath.

It complements:

- `mfb spec package` (`src/docs/spec/**` — where the new `iap::` surface is documented)
- `mfb spec diagnostics` (new `iap::` runtime error codes)
- source-package precedents `datetime::`/`csv::` (`bindings/**`, generated source companions)

## 1. Goal

- `iap::products(ids OF List OF String) -> List OF Product` — fetch product metadata.
- `iap::purchase(id OF String) -> PurchaseOutcome` — start a purchase; outcome is one
  of success (with a verified `Transaction`), userCancelled, pending, or failed.
- `iap::finish(txn OF Transaction)` — finish a verified transaction.
- `iap::observe(handler)` — deliver `Transaction.updates` (renewals, restores,
  ask-to-buy approvals) to a callback.
- `iap::currentEntitlements() -> List OF Transaction` — active entitlements.
- Types: `Product` (id, displayName, description, displayPrice, price),
  `Transaction` (id, productId, purchaseDate, isVerified), `PurchaseOutcome`.

### Non-goals (explicit constraints)

- **StoreKit 2 only.** No StoreKit 1, no `SKPaymentQueue`, no receipt blob.
- No server-side verification and no receipt-file parsing — trust
  `VerificationResult.verified` (Apple-signed JWS), expose `isVerified`; a
  `.unverified` result is surfaced, never silently trusted.
- No `async`/`await` in MFBASIC — `iap::` calls block the worker (30-D bridge); the
  main thread stays responsive.
- No new value/copy/move/transfer semantics — `Product`/`Transaction` are ordinary
  mfb record values marshalled at the bridge boundary; the Swift objects never escape.
- Simulator-first (StoreKitTest `.storekit` config); real-device sandbox purchasing is
  a deployment concern, not this plan.

## 2. Current State

- No IAP anywhere. The stack beneath this plan: 30-D (Swift async bridge), 30-C
  (worker/main threading + the inbound/outbound resource channel at offsets 104/112,
  `src/target/shared/code/runtime_helpers.rs:26`).
- StoreKit symbols the oracle enumerated (SDK 26.2): `Product.products(for:)…Tu`,
  `Product.purchase(options:)…Tu`, `Transaction.updates` (an `AsyncSequence`) +
  `Transaction.finish()…Tu`, `VerificationResult`/`PurchaseResult` enums, with their
  `…Ma`/`…Mn` metadata symbols.
- Framework linking: `StoreKit` must be added to the dylib install-name table
  (`src/os/macos/object.rs:616` + mirror `src/os/macos/link/mod.rs:294`) — the same
  whitelist the app survey flagged.
- Package precedent: source packages with a native runtime seam — `datetime::`
  (source package + libc runtime-helper intrinsics, arity-aware
  `implementation_name`), `csv::`. `iap::`'s "native intrinsics" call the 30-D driver
  rather than libc.

## 3. Design Overview

- **Package surface** as a source package (like `datetime::`) declaring the `iap::`
  functions and the `Product`/`Transaction`/`PurchaseOutcome` types.
- **Runtime seam:** each `iap::` function lowers to a native intrinsic that (a) posts
  a request from the worker to the main thread over the outbound queue, (b) the main
  thread runs the corresponding StoreKit call through the 30-D async driver (StoreKit
  is `@MainActor`-friendly there), (c) the verified result is posted back over the
  inbound queue, (d) the worker unblocks and returns an mfb value. The synchronous
  MFBASIC API is this round-trip.
- **Verification** is intrinsic to SK2: unwrap `VerificationResult` — `.verified(t)`
  → `isVerified = TRUE`; `.unverified(t, err)` → surfaced with `isVerified = FALSE`,
  never auto-trusted.

Risk here is integration (correct threading + marshalling of the specific StoreKit
types), not new ABI — 30-D owns the ABI risk. `iap::products` (read-only) is landed
first as the end-to-end proof.

## 4. Detailed Design

### 4.1 Threading / event flow (per call)

Worker calls `iap::purchase(id)` → intrinsic enqueues `{op: purchase, id}` on the
**outbound** queue and blocks (30-D semaphore) → main thread dequeues, runs
`Product.purchase(options:)` via the 30-D async driver, unwraps `PurchaseResult` +
`VerificationResult` → enqueues the marshalled `PurchaseOutcome` on the **inbound**
queue → worker unblocks, returns it. `iap::observe` runs a long-lived task consuming
`Transaction.updates` (the `AsyncSequence` from the oracle), posting each verified
update inbound to the registered handler.

### 4.2 Types (mfb records)

- `Product`: `id`, `displayName`, `description`, `displayPrice` (String), `price`
  (the numeric amount — see Open Decisions on `Money` vs `Float`).
- `Transaction`: `id`, `productId`, `purchaseDate`, `isVerified`.
- `PurchaseOutcome`: a tagged result — `success(Transaction)` | `userCancelled` |
  `pending` | `failed(reason)`.

### 4.3 StoreKit linking

Add `StoreKit` → `/System/Library/Frameworks/StoreKit.framework/StoreKit` to the
simulator dylib table (30-A §4.2 mechanism); reference the SK2 `…Tu`/`…Ma` symbols the
oracle named.

### 4.4 Diagnostics

New `iap::` runtime error codes (network failure, product-not-found, unverified
transaction when the program requires verification, purchase failure) added to
`mfb spec diagnostics` `error-codes` (the build input for `errorCode::`).

## Layout / ABI Impact

Adds `iap::` package types (ordinary mfb records) and `StoreKit` + SK2 symbol imports
on the iOS backend. Documented under `mfb spec package` and `mfb spec diagnostics`.
No change to existing types, copy/transfer, or golden output on other targets.

## Phases

### Phase 1 — StoreKit linking + package skeleton + types

- [ ] Add `StoreKit` to the simulator dylib table (`src/os/macos/object.rs:616`, mirror `src/os/macos/link/mod.rs:294`).
- [ ] Create the `iap::` source package (surface + `Product`/`Transaction`/`PurchaseOutcome` types), mirroring the `datetime::` package structure.
- [ ] Tests: `tests/func_iap_*_valid/**` + `_invalid/**` for the type surface / arities.

Acceptance: an iOS build links `StoreKit` and the `iap::` package compiles with its
types resolvable; function tests for the surface pass.
Commit: —

### Phase 2 — `iap::products` (read-only, end-to-end proof)

- [ ] Lower `iap::products` to the intrinsic → outbound → 30-D driver (`Product.products(for:)`) → inbound → worker path (30-C channel).
- [ ] Provide a `.storekit` StoreKitTest config for the simulator; document the run.
- [ ] Tests: `tests/func_iap_products_valid/**` + `_invalid/**` (bad ids, empty).

Acceptance: on the simulator with a `.storekit` config, an MFBASIC program calls
`iap::products` and prints real product metadata — the whole 30-A…E stack proven
end-to-end.
Commit: —

### Phase 3 — `iap::purchase` + verification + `iap::finish`

- [ ] Lower `iap::purchase` through the driver; unwrap `PurchaseResult`/`VerificationResult`; surface `isVerified`; implement `iap::finish`.
- [ ] Tests: valid/invalid for purchase outcomes (success, userCancelled, pending, failed) via the `.storekit` config.

Acceptance: a program completes a StoreKitTest purchase, receives a **verified**
transaction, finishes it, and the outcomes (incl. cancel) are observable.
Commit: —

### Phase 4 — `iap::observe` + `iap::currentEntitlements` (highest-risk: AsyncSequence)

- [ ] Consume `Transaction.updates` (`AsyncSequence`) in a long-lived task; deliver verified updates to the mfb handler over the inbound queue.
- [ ] Implement `iap::currentEntitlements`.
- [ ] Tests: valid/invalid for the observe handler + entitlements.

Acceptance: a renewal/restore triggered via StoreKitTest is delivered to the
program's `iap::observe` handler, and `iap::currentEntitlements` lists the active
entitlement.
Commit: —

## Validation Plan

- Function tests: `tests/func_iap_<func>_valid/**` and `_invalid/**` for every
  `iap::` function, all overloads.
- Runtime proof: Phases 2–4 — real StoreKit 2 calls on the simulator against a
  `.storekit` StoreKitTest config, with observed product data / verified transaction /
  delivered update (execution proof, not golden output).
- Doc sync: `mfb spec package` (the `iap::` surface + types) and `mfb spec
  diagnostics` (new `iap::` error codes).
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` passes.

## Open Decisions

- **`Product.price` type** — recommend the new **`Money`** type (plan-29) if landed,
  else `Float` + rely on `displayPrice` (String) for exact display. (§4.2)
- **`PurchaseOutcome` shape** — recommend a **union/tagged type** vs. a status enum +
  optional transaction fields. (§4.2)
- **`iap::observe` handler model** — recommend a **callback FUNC** (matches existing
  callback-member support) vs. a polling `iap::nextUpdate()`. (§4.1)

## Non-Goals

- StoreKit 1, receipt blobs, server verification, on-device receipt parsing.
- Real-device sandbox purchasing / App Store Connect product configuration
  (deployment, not this plan).
- Promotional offers, subscription-offer signing, refund UI, `SKAdNetwork`.

## Summary

The IAP surface itself is ordinary package + threading integration; all the ABI risk
was spent in 30-D. Verification is free from StoreKit 2 (`.verified` JWS), so there is
no receipt-server or parser to build. `iap::products` lands first as the end-to-end
proof of the whole plan-30 stack; purchase, finish, observe, and entitlements follow.
The MFBASIC surface is synchronous and blocks the worker; the main thread — and the
UI from 30-C — stays responsive.
