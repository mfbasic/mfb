# plan-30-C: UIKit app runtime backend

Last updated: 2026-09-20 (validity review: design still holds; anchors refreshed —
the app seam is now trait methods with a `uses_mouse` parameter, `AppEntrySpec` is
gone, and the macOS backend has since grown mouse/canvas/presentation surfaces that
this plan explicitly does NOT mirror in v1)
Effort: medium (1h–2h)

This sub-plan gives the iOS target a real **UIKit app runtime**, the sibling of the
macOS AppKit backend at `src/target/macos_aarch64/app/`. `mfb build -app` for the
iOS-simulator target boots through `UIApplicationMain` on the main thread; a
**runtime-registered** app-delegate class builds a `UIWindow` + root
`UIViewController` + scrolling `UITextView`, spawns the MFBASIC program on a worker
pthread, and marshals its output back to the transcript on the main thread via
`performSelectorOnMainThread:` — the same worker/main split the macOS backend uses.
All UIKit interaction is hand-emitted `objc_msgSend`/`sel_registerName`; **no Swift,
no static Objective-C.**

The single behavioral outcome: an iOS app that, launched on the simulator, shows the
running MFBASIC program's output in an on-screen transcript, with the program
executing on its own thread.

It complements:

- `mfb spec app spec` / `mfb spec app macos-runtime` (`src/docs/spec/app/**` — the macOS runtime this mirrors)

## 1. Goal

- `main` → `UIApplicationMain(argc, argv, nil, "MFBAppDelegate")` on the process main
  thread.
- A UIKit app delegate class **registered at runtime** (`objc_allocateClassPair` /
  `class_addMethod` / `objc_registerClassPair`) whose
  `application:didFinishLaunchingWithOptions:` builds the window/controller/textview,
  spawns the worker pthread, and returns.
- Worker thread runs `code::` program entry (a `_mfb_iosapp_worker` mirroring
  `_mfb_macapp_worker`); output hops to the main thread and appends to the
  `UITextView` transcript.
- A headless env-var hook (mirroring macOS `MFB_MACAPP_HEADLESS`) runs the worker
  without UI for automated runtime tests.

### Non-goals (explicit constraints)

- No Swift, no `swiftc`, no static Objective-C source — UIKit reached purely via the
  Objective-C runtime, hand-emitted, exactly as the macOS backend does.
- No `app::` widget package (plan-13) — this is the transcript runtime, not widgets.
- No input/keyboard editing surface beyond what the macOS backend already models
  (read paths follow the macOS backend's approach; no new interaction model here).
- No StoreKit (30-E). This sub-plan does wire the worker↔main event channel
  (§4.4) that 30-E rides on, but adds no IAP surface.
- No mouse/touch events, no canvas, no presentation mode on iOS in v1 — the macOS
  backend has grown these since this plan was written (plan-94 mouse, plan-98
  canvas/Metal, presentation mode); the iOS backend passes `uses_mouse = false`
  and mirrors the **transcript** runtime only. Touch/canvas are follow-up plans.

## 2. Current State

- macOS app backend: `src/target/macos_aarch64/app/` —
  - `mod.rs` (`_main` bootstrap, selectors/classes as read-only data, GOT-referenced
    `_OBJC_CLASS_$_*`);
  - `bootstrap.rs` (`_mfb_macapp_worker` at ~1814; `pthread_create` of it at ~2197,
    calling the program entry; headless `MFB_MACAPP_HEADLESS` check at ~163 — the
    file has grown ~4× since this plan was written);
  - `term_view.rs`, `app_io.rs` (transcript view + I/O redirect via
    `performSelectorOnMainThread:withObject:waitUntilDone:`);
  - **grown since this plan was written:** `mouse_view.rs` (plan-94 mouse events)
    and `metal.rs` (plan-98 canvas/Metal renderer); the app spec now also covers
    presentation mode and canvas (`src/docs/spec/app/05_presentation-mode.md`,
    `06_canvas.md`).
- Backend seam: `AppEntrySpec` no longer exists. The seam is trait methods on the
  plan backend — notably `app_mode_imports(&self, uses_mouse: bool)`
  (`src/target/shared/plan/mod.rs:216`; the `uses_mouse` gate is plan-94, and it is
  golden-visible). Three app toolkits already prove the seam: AppKit
  (`macos_aarch64`), GTK4 (`src/target/linux_gtk/`), and Win32 GDI
  (`src/target/win_x86_64/`, plan-66-J) — `NativeBuildMode` is
  `Console | MacApp | LinuxApp | WindowsApp` (`src/target.rs:38`), so the iOS
  runtime adds a fourth toolkit through an established, repeatedly-exercised seam.
- **Key iOS difference:** AppKit lets you build a window and call `[NSApp run]`
  directly. UIKit does **not** — you must go through `UIApplicationMain` with an app
  delegate. Since we have no static ObjC, the delegate class is **synthesized at
  runtime** via the Objective-C runtime.

## 3. Design Overview

New backend under `src/target/ios_sim_aarch64/app/` (or a shared `uikit/` module if
a device target later shares it), wired through the same app-seam trait methods
(`app_mode_imports(uses_mouse)` etc.) the AppKit/GTK/Win32 backends use. Three layers:

1. **Bootstrap**: register the delegate class at process start, call
   `UIApplicationMain`.
2. **Delegate `didFinishLaunching`**: build window/controller/textview, spawn worker.
3. **I/O + event bridge**: worker→UI output via `performSelectorOnMainThread:`;
   worker↔main event channel over the existing inbound/outbound resource queues
   (for 30-E).

Correctness risk concentrates in the runtime class registration + `UIApplicationMain`
handoff (getting the delegate method signatures / type encodings right) — proven
first in headless mode before any pixels.

## 4. Detailed Design

### 4.1 Runtime delegate class registration

At process start (before `UIApplicationMain`):
`objc_allocateClassPair(UIResponder, "MFBAppDelegate", 0)`; add methods with
`class_addMethod` for `application:didFinishLaunchingWithOptions:` (type encoding
`c@:@@`) pointing at a hand-emitted IMP; `objc_registerClassPair`. Adopt
`UIApplicationDelegate` (informal — responding to the selector suffices).

### 4.2 `UIApplicationMain` handoff

`UIApplicationMain(argc, argv, nil, @"MFBAppDelegate")`. This never returns; it owns
the main run loop. The `nil` principal class means default `UIApplication`.

### 4.3 didFinishLaunching: window + transcript + worker

- `UIWindow` `initWithFrame:` `[UIScreen mainScreen].bounds`.
- `rootViewController` = a plain `UIViewController`; its `view` hosts a `UITextView`
  (`editable = NO`, scrollable) sized to the view bounds with autoresizing — the
  transcript surface, mirroring `term_view.rs`.
- `[window makeKeyAndVisible]`.
- `pthread_create(_mfb_iosapp_worker)` — mirrors `bootstrap.rs` (~2197).
- Return `YES`.

### 4.4 I/O + worker↔main event bridge

- **Worker → UI (output):** append to `textView.textStorage` via
  `performSelectorOnMainThread:withObject:waitUntilDone:` — identical to macOS
  `app_io.rs`.
- **Worker ↔ main (events, for 30-E):** reuse the resource channel —
  `THREAD_OFFSET_RESOURCE_INBOUND_QUEUE` (104) / `OUTBOUND` (112)
  (`src/codegen/runtime/thread/runtime_helpers.rs:32`) and the thread
  transfer/accept helpers (`src/codegen/runtime/thread/`). This sub-plan wires the plumbing so 30-E can post a request from the worker
  and receive a StoreKit result back without adding IAP surface.

### 4.5 Headless test hook

Env var (e.g. `MFB_IOSAPP_HEADLESS`) mirroring macOS: skip window/`UIApplicationMain`,
run the worker directly, so runtime tests exercise the same construction/worker code
without a GUI.

## Layout / ABI Impact

New backend imports (UIKit classes, `objc_allocateClassPair`/`class_addMethod`/
`objc_registerClassPair`, `UIApplicationMain`) added to the iOS backend's
`app_mode_imports()`. `UIKit` added to the simulator dylib table (30-A §4.2). No
`mfb`-language layout, copy/transfer, or golden change; existing backends untouched.

## Phases

### Phase 1 — Delegate class registration + `UIApplicationMain` (headless-provable)

- [ ] New `src/target/ios_sim_aarch64/app/` backend + wiring through the app-seam trait methods, incl. `app_mode_imports(uses_mouse)` (`src/target/shared/plan/mod.rs:216`; iOS passes `uses_mouse = false` in v1), and an app-mode selection for iOS (`NativeBuildMode` variant or target-derived, `src/target.rs:38`).
- [ ] Hand-emit runtime class registration (`objc_allocateClassPair`/`class_addMethod`/`objc_registerClassPair`) and the `UIApplicationMain` call; add `UIKit` to the simulator dylib table.
- [ ] Headless hook (`MFB_IOSAPP_HEADLESS`) runs the worker without UI.

Acceptance: headless run on the simulator executes the MFBASIC program on the worker
and exits cleanly (delegate constructed, worker ran) — proven without pixels.
Commit: —

### Phase 2 — Window + transcript + worker output

- [ ] `didFinishLaunching:` builds `UIWindow`/`UIViewController`/`UITextView`, spawns `_mfb_iosapp_worker`, `makeKeyAndVisible` (`app/*` in the new backend).
- [ ] Worker→UI output via `performSelectorOnMainThread:` appending to the transcript.
- [ ] Wire the worker↔main resource-channel plumbing (offsets 104/112) for 30-E (no IAP surface yet).

Acceptance: launched on the simulator, the app shows program output in the on-screen
transcript; the program runs on its own thread (verified by a program that emits over
time).
Commit: —

### Phase 3 — On-simulator visual proof (highest-risk: real UIKit boot)

- [ ] Capture a simulator screenshot (`xcrun simctl io booted screenshot`) of a known program's transcript; record the reproducible sequence.

Acceptance: screenshot shows the expected transcript text; sequence recorded.
Commit: —

## Validation Plan

- Function tests: none new (no `mfb` surface) — coverage is the headless runtime run
  (Phase 1) + visual proof (Phase 3).
- Runtime proof: Phases 2–3 — a real UIKit app on the simulator rendering worker
  output.
- Doc sync: add `src/docs/spec/app/` iOS-runtime topic paralleling `01_macos-runtime.md`.
- Acceptance: `scripts/test-accept.sh` unaffected.

## Open Decisions

- **Backend location** — recommend `src/target/ios_sim_aarch64/app/` now, refactor to
  a shared `uikit/` module only if/when a device target arrives, vs. building the
  shared module up front. (§3)
- **Input surface** — recommend matching the macOS backend's existing read model
  (no new keyboard/interaction design in this plan) vs. adding a `UITextField`
  input row. (Non-goals)

## Summary

The technique is proven — hand-emitted `objc_msgSend` is exactly how the macOS
backend already ships. The one genuinely new thing is UIKit's mandatory
`UIApplicationMain` + a **runtime-synthesized** delegate class, de-risked by proving
it headless before any UI. The worker/main threading model and output-marshalling are
lifted from the macOS backend; the event channel is the existing one. No Swift, no
static ObjC, nothing in the language layer.
