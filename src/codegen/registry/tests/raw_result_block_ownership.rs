//! bug-566: the per-helper ownership audit behind
//! [`CodeBuilder::runtime_result_is_caller_owned`](crate::codegen::engine::builder::CodeBuilder::runtime_result_is_caller_owned).
//!
//! A runtime helper called under an inline `TRAP` has its result copied into a
//! `Result` block, after which the helper's ORIGINAL block is dead. Freeing it
//! needs an answer to one question per helper — is that block a fresh
//! allocation in the CALLING thread's arena? — and getting it wrong is a
//! cross-arena wild free, not a missed optimisation: `x19` is per-thread.
//!
//! The emitter does not consult a name list; it asks the same gate a `Bind` of
//! the same call asks (`is_freeable_flat_value` + not foreign-arena). These
//! tests are the audit that the gate's ANSWERS are the intended ones, and that
//! the vocabulary they cover is the whole catalog: a new runtime helper that
//! returns a block, or a new one whose result type is generic, reds a test and
//! forces the decision rather than inheriting one.

use super::runtime_specs;
use crate::codegen::engine::builder::CodeBuilder;
use crate::types::ParameterType;

/// Runtime calls whose result block is allocated by ANOTHER thread.
///
/// Every one is in the `thread` family, and every one's declared return type is
/// a type VARIABLE — `thread::waitFor` is `Out`, `thread::receive` is `Msg` —
/// so the concrete type is only known at the instantiation, and it can perfectly
/// well be a `String`. The block behind it lives in the thread's block arena
/// (plan-156-A); `materialize_current_result`'s `worker_error_source` flag
/// exists for exactly that reason on the error side. Freeing one from the
/// calling thread writes this thread's free list into another thread's heap.
///
/// This is the counter-example that makes the runtime-helper question an audit
/// and not a blanket rule.
const FOREIGN_ARENA_RESULTS: &[&str] = &[
    "thread.accept",
    "thread.acceptResource",
    "thread.read",
    "thread.readResource",
    "thread.receive",
    "thread.start",
    "thread.waitFor",
];

/// Every runtime call with a CONCRETE block-carrying return type. Each one's
/// block is a fresh `_mfb_arena_alloc` in the caller's arena — which is not a
/// new claim: `LET x AS T = <this call>` has always registered a scope-drop
/// `arena_free` for it (`owns_freeable_value` excludes only the runtime-managed
/// ones), so a helper that broke this would already be a wild free at every
/// binding. bug-566 asks that same question at the site the `TRAP` desugar
/// bypassed.
const CALLER_ARENA_BLOCK_RESULTS: &[&str] = &[
    "app.getMode",       // app.Mode
    "app.getTitle",      // String
    "audio.devices",     // List OF audio.AudioDevice
    "audio.read",        // List OF Byte
    "audio.readTimeout", // List OF Byte
    // plan-127: every `big` result is made by `emit_alloc_magnitude` (or, for the text
    // members, `emit_alloc`; for `toBytes`, `emit_build_byte_list`; for `divMod`, the
    // record marshaller) on this thread's arena and published once — no member hands
    // back an argument or a constant.
    "big.abs",                  // big.Int
    "big.add",                  // big.Int
    "big.divMod",               // big.DivResult
    "big.divide",               // big.Int
    "big.factorial",            // big.Int
    "big.fromBytes",            // big.Int
    "big.fromInteger",          // big.Int
    "big.gcd",                  // big.Int
    "big.modPow",               // big.Int
    "big.multiply",             // big.Int
    "big.negate",               // big.Int
    "big.parse",                // big.Int
    "big.pow",                  // big.Int
    "big.product",              // big.Int
    "big.remainder",            // big.Int
    "big.shiftLeft",            // big.Int
    "big.shiftRight",           // big.Int
    "big.subtract",             // big.Int
    "big.sum",                  // big.Int
    "big.toBytes",              // List OF Byte
    "big.toRadixString",        // String
    "big.toString",             // String
    "canvas.carriedHashes",     // List OF Integer (bug-686)
    "canvas.fontBlobUnchecked", // List OF Byte
    "canvas.frameCounters",     // List OF Integer (plan-155-B)
    "canvas.geoBuild",          // List OF Float (bug-686)
    "canvas.getBytes",          // List OF Byte
    "canvas.getSize",           // canvas.Size
    "canvas.groupItems",        // List OF canvas.DrawItem
    "canvas.installedHashes",   // List OF Integer
    "canvas.installedItems",    // List OF canvas.DrawItem
    "canvas.installedLayers",   // List OF canvas.DrawLayer
    "canvas.newSurface",        // List OF Byte
    "canvas.pointerSnapshot",   // List OF Integer (plan-172-J)
    "canvas.pollMouseRaw",      // canvas.MouseEvent (plan-94-A, plan-172-I)
    "canvas.retiredItems",      // List OF canvas.DrawItem
    "canvas.sceneHashes",       // List OF Integer (bug-686)
    "canvas.systemFontTable",   // String
    "crypto.generate",          // crypto.KeyPair
    "crypto.hash",              // List OF Byte
    "crypto.open",              // List OF Byte
    "crypto.randomBytes",       // List OF Byte
    "crypto.seal",              // crypto.Sealed
    "crypto.sign",              // List OF Byte
    "fs.canonicalPath",         // String
    "fs.currentDirectory",      // String
    "fs.listDirectory",         // List OF String
    "fs.readAll",               // String
    "fs.readAllBytes",          // List OF Byte
    "fs.readBytes",             // List OF Byte
    "fs.readBytesAt",           // List OF Byte
    "fs.readLine",              // String
    "fs.readLineRaw",           // String (bug-809; "" at end of input is a fresh block too)
    "fs.readText",              // String
    "fs.tempDirectory",         // String
    "io.input",                 // String
    "io.readGrapheme",          // String
    "io.readKey",               // io.Key
    "io.readLine",              // String
    "io.readScalar",            // Scalar (a nominal, so classed here like app.Mode; no block)
    "net.lookup",               // List OF net.Address
    "net.ping",                 // net.PingResult
    "net.pingAddr",             // net.PingResult
    "os.appCachePath",          // String
    "os.appDataPath",           // String
    "os.appResourcePath",       // String
    "os.arch",                  // String
    "os.args",                  // List OF String
    "os.environ",               // Map OF String TO String
    "os.executablePath",        // String
    "os.getEnv",                // String
    "os.getEnvOr",              // String
    "os.hostName",              // String
    "os.name",                  // String
    "os.prog",                  // String
    "os.userDocumentsPath",     // String
    "os.userHomePath",          // String
    "os.userName",              // String
    "os.version",               // String
    "process.didSignal",        // process.Signal
    "process.receive",          // String
    "process.receiveBytes",     // List OF Byte
    "process.receiveBytesFrom", // List OF Byte
    "process.receiveFrom",      // String
    "tcp.localAddress",         // net.Address
    "tcp.read",                 // List OF Byte
    "tcp.remoteAddress",        // net.Address
    "term.getBackground",       // color.Color
    "term.getForeground",       // color.Color
    "term.pollMouse",           // term.MouseEvent (plan-94-A)
    "term.terminalSize",        // term.TermSize
    "tls.localAddress",         // net.Address
    "tls.localAddressListener", // net.Address
    "tls.read",                 // List OF Byte
    "tls.remoteAddress",        // net.Address
    "udp.localAddress",         // net.Address
    "udp.receive",              // udp.Datagram
];

/// Whether `ty` is a concrete type whose values are carried by an arena block —
/// the shapes `is_freeable_flat_value` admits, asked without a builder (which
/// needs a program's type model). A record/union NAME is included: it is
/// block-carrying in every program that can call the helper at all.
fn carries_a_block(ty: &ParameterType) -> bool {
    // A resource HANDLE is a `Named` type too, but it is not a flat value
    // block: `is_freeable_flat_value` excludes it, and its lifetime is the
    // §15 close obligation, not an `arena_free`. `fs.openFile` and
    // `tcp.accept` are therefore in neither list.
    if crate::codegen::builtins::is_resource_type(ty) {
        return false;
    }
    matches!(
        ty,
        ParameterType::String | ParameterType::ResultOf(_) | ParameterType::Named(_)
    ) || crate::codegen::engine::types::typed_is_collection_type(ty)
}

/// Whether `ty` is not yet a concrete type — a generic parameter or a thread
/// handle carrying them. The emitter sees the SUBSTITUTED type, so these say
/// nothing about whether a block is involved, which is why they are classified
/// by call name instead.
fn is_generic(ty: &ParameterType) -> bool {
    matches!(
        ty,
        ParameterType::Var(_) | ParameterType::ThreadHandle { .. }
    )
}

/// A package companion may not call a `collections` member whose body is MFBASIC
/// source (`sort`, `distinct`, `sortBy`, …) — plan-148-B found this the hard way.
///
/// The `collections` source reaches a build only when the *program* imports
/// `collections` (`codegen::builtins::collections::augmented_project`, run from
/// `parse_project` because the monomorphizer needs it). Companions are injected
/// later, by the resolver's augmentation chain, so a companion's call to a
/// source-generic member compiles when the program happens to import `collections`
/// and otherwise fails at link with "NIR call target '#collections_sort' does not
/// resolve" — a failure that depends on the user's imports, not on the companion.
/// The native members (`append`, `getOr`, `mid`, …) have no such dependency.
#[test]
fn no_companion_calls_a_source_generic_collections_member() {
    let reg = crate::codegen::registry::registry();
    let collections = reg
        .resolve_package("collections")
        .expect("collections is registered");
    let generic: Vec<&str> = collections
        .functions()
        .iter()
        .filter(|f| {
            f.implementations
                .iter()
                .any(|i| matches!(i.body, crate::codegen::registry::Body::Mfb { .. }))
        })
        .map(|f| f.name)
        .collect();
    assert!(
        generic.contains(&"sort"),
        "the census of source-generic members found no `sort`: {generic:?}",
    );
    let mut offenders = Vec::new();
    for package in reg.packages() {
        if package.import_name() == "collections" {
            continue;
        }
        let mut sources = vec![package.get_mfb()];
        sources.extend(
            package
                .helpers()
                .iter()
                .filter_map(|h| h.body.map(str::to_string)),
        );
        for source in &sources {
            for name in &generic {
                if source.contains(&format!("collections::{name}(")) {
                    offenders.push(format!("{}: collections::{name}", package.import_name()));
                }
            }
        }
    }
    offenders.sort();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "companion source calls a source-generic collections member, which links \
             only when the program itself imports collections: {offenders:#?}",
    );
}

#[test]
fn every_block_returning_runtime_helper_is_classified() {
    let mut generic: Vec<&str> = Vec::new();
    let mut concrete_block: Vec<&str> = Vec::new();
    let mut no_block: Vec<&str> = Vec::new();
    for call in runtime_specs() {
        if is_generic(&call.return_type) {
            generic.push(call.name);
        } else if carries_a_block(&call.return_type) {
            concrete_block.push(call.name);
        } else {
            no_block.push(call.name);
        }
    }
    generic.sort();
    generic.dedup();
    concrete_block.sort();
    concrete_block.dedup();
    no_block.sort();
    no_block.dedup();

    assert_eq!(
        generic, FOREIGN_ARENA_RESULTS,
        "the set of runtime calls with a GENERIC result type changed. The \
             emitter sees the substituted type, so it cannot tell from the type \
             whether a block is involved — it declines these by call name. Every \
             one must therefore be a call whose result really is another arena's, \
             which today means the `thread` family. A NON-thread helper appearing \
             here is the dangerous case: it would be freed on the strength of a \
             concrete type the catalog never declared"
    );
    assert_eq!(
        concrete_block, CALLER_ARENA_BLOCK_RESULTS,
        "the set of runtime calls returning a concrete block-carrying type \
             changed. Each new one is licensed to have its block freed by the \
             caller after an inline `TRAP` copies it into the `Result` — the same \
             licence its `LET` binding already exercises. Add it here only after \
             confirming the helper allocates the result in the CALLER's arena \
             (`_mfb_arena_alloc` on this thread) and hands back the only pointer"
    );

    for name in FOREIGN_ARENA_RESULTS {
        assert!(
            CodeBuilder::runtime_call_result_is_foreign_arena(name),
            "{name} is catalogued as another arena's but the emitter would \
                 free its block"
        );
    }
    for name in CALLER_ARENA_BLOCK_RESULTS {
        assert!(
            !CodeBuilder::runtime_call_result_is_foreign_arena(name),
            "{name} is catalogued as caller-owned but the emitter declines it, \
                 so its block keeps leaking under an inline `TRAP`"
        );
    }
    // The third class is the complement: a scalar / `Nothing` / resource-handle
    // result has no block at all, so `result_payload_is_block` is false and
    // nothing is emitted for it. Asserted disjoint rather than assumed, so a
    // call cannot be read as both "no block" and "freed".
    assert!(
        no_block
            .iter()
            .all(|name| !FOREIGN_ARENA_RESULTS.contains(name)
                && !CALLER_ARENA_BLOCK_RESULTS.contains(name)),
        "a runtime call is in two classes at once"
    );
    assert_eq!(
        generic.len() + concrete_block.len() + no_block.len(),
        runtime_specs()
            .iter()
            .map(|call| call.name)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        "the three classes must cover the whole runtime-call catalog"
    );
}

/// bug-576: the `String` slice of `CALLER_ARENA_BLOCK_RESULTS` — the helpers
/// whose result needs the bug-536 shape B freshness MARK before an UNBOUND
/// result is freed at statement scope.
///
/// A composite result (`List OF Byte`, `net.Address`, …) is freed by
/// `register_pending_temp` with no provenance at all, so only these rows
/// changed behaviour: before bug-576 `LET s AS String = os::arch()` was flat
/// while `len(os::arch())` leaked its block for the life of the process.
///
/// It is a derived list, not a second source of truth — the test below scrapes
/// the catalog and asserts this IS the scrape — so a new `String`-returning
/// helper reds here and forces the same caller's-arena confirmation
/// `CALLER_ARENA_BLOCK_RESULTS` demands.
const STRING_RESULT_HELPERS: &[&str] = &[
    "app.getTitle",
    "big.toRadixString",
    "big.toString",
    "canvas.systemFontTable",
    "fs.canonicalPath",
    "fs.currentDirectory",
    "fs.readAll",
    "fs.readLine",
    "fs.readLineRaw",
    "fs.readText",
    "fs.tempDirectory",
    "io.input",
    "io.readGrapheme",
    "io.readLine",
    "os.appCachePath",
    "os.appDataPath",
    "os.appResourcePath",
    "os.arch",
    "os.executablePath",
    "os.getEnv",
    "os.getEnvOr",
    "os.hostName",
    "os.name",
    "os.prog",
    "os.userDocumentsPath",
    "os.userHomePath",
    "os.userName",
    "os.version",
    "process.receive",
    "process.receiveFrom",
];

/// bug-576: every `String`-returning runtime helper is one the calling thread
/// may free, and every one is already catalogued as caller-arena.
///
/// The emitter consults no list — `mark_runtime_helper_result_fresh` asks
/// `runtime_result_is_caller_owned`, the `Bind` gate — so this is the audit that
/// the ANSWERS are the intended ones for the type the mark exists for. A new
/// `String` helper whose block is NOT this thread's would be a wild free at
/// every unbound call site (SIGBUS on rodata, free-list corruption on a view),
/// so it must red here rather than inherit the verdict.
#[test]
fn every_string_returning_runtime_helper_is_marked_fresh() {
    let mut string_results: Vec<&str> = runtime_specs()
        .iter()
        .filter(|call| call.return_type == ParameterType::String)
        .map(|call| call.name)
        .collect();
    string_results.sort();
    string_results.dedup();
    assert_eq!(
        string_results, STRING_RESULT_HELPERS,
        "the set of runtime calls returning a bare `String` changed. Each one \
             has its result block freed at statement scope when nothing binds it \
             (bug-576). Add it here only after confirming the helper allocates the \
             result with `_mfb_arena_alloc` in the CALLER's arena and hands back the \
             only pointer — a rodata pointer or a view into an argument freed here \
             is SIGBUS or free-list corruption, not a leak"
    );
    for name in STRING_RESULT_HELPERS {
        assert!(
            CALLER_ARENA_BLOCK_RESULTS.contains(name),
            "{name} returns a `String` but is not catalogued as caller-arena, \
                 so the two audits disagree about the same block"
        );
        assert!(
            !CodeBuilder::runtime_call_result_is_foreign_arena(name),
            "{name} is catalogued as caller-owned but the emitter declines it, \
                 so its block keeps leaking at every unbound call site"
        );
    }
}

/// The whole `thread` family is declined, not just the seven above: a thread
/// member that today returns `Nothing` and tomorrow returns a `String` must not
/// be freeable by the calling thread the moment its signature changes.
#[test]
fn no_thread_family_result_is_ever_freed_by_the_calling_thread() {
    let mut thread_calls: Vec<&str> = runtime_specs()
        .iter()
        .map(|call| call.name)
        .filter(|name| name.starts_with("thread."))
        .collect();
    thread_calls.sort();
    thread_calls.dedup();
    assert!(
        thread_calls.len() >= 15,
        "the thread-family scrape found only {} calls, so the assertion below \
             means nothing: {thread_calls:?}",
        thread_calls.len()
    );
    for name in &thread_calls {
        assert!(
            CodeBuilder::runtime_call_result_is_foreign_arena(name),
            "{name} would have its result block freed by the CALLING thread. \
                 A `thread` member marshals across the arena boundary; its block \
                 belongs to the worker"
        );
    }
}
