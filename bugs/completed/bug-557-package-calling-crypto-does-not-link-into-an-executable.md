# bug-557: an executable cannot link a package that calls `crypto::hash`, `seal`/`open`, `sign`, `verify` or `generate`

Last updated: 2026-09-06
Effort: small (one merge-time pass; no format change)
Severity: HIGH (a package using `crypto` is unusable by any consumer that does not itself import `crypto`)
Class: Package consumption / symbol namespacing

Status: Fixed — in the commit that carries this file.

## The finding

A package whose source called `crypto::hash`, `crypto::seal`, `crypto::open`,
`crypto::sign`, `crypto::verify` or `crypto::generate` built fine on its own and
produced a valid `.mfp`. An executable that imported that package failed to
build:

```text
error: native code internal relocation target
       '_mfb_ifn_crypto_5FgenerateEd25519' is not defined
```

The executable had to `IMPORT crypto` itself for the build to succeed — which is
why nothing in the suite caught it. Every existing fixture that consumes a
package either does not use `crypto`, or imports `crypto` in the consumer too.

## Reproduction

    mkdir -p /tmp/b557/pkg/src /tmp/b557/app/src /tmp/b557/app/packages

`/tmp/b557/pkg/project.json`

    {"name":"cryptouser","version":"0.1.0","mfb":"1.0","kind":"package","description":"m",
     "sources":[{"root":"src","role":"package","include":["**/*.mfb"]}]}

`/tmp/b557/pkg/src/lib.mfb`

    IMPORT crypto

    EXPORT FUNC signSomething() AS List OF Byte
      LET pair AS crypto::KeyPair = crypto::generate(crypto::Certificate.Ed25519)
      RETURN crypto::sign(crypto::Certificate.Ed25519, pair.privateKey, [])
    END FUNC

`/tmp/b557/app/project.json`

    {"name":"cryptoapp","version":"0.1.0","mfb":"1.0","kind":"executable",
     "sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
     "packages":[{"name":"cryptouser","version":"=0.1.0","source":"file:packages/cryptouser.mfp"}],
     "entry":"main","targets":["native"]}

`/tmp/b557/app/src/main.mfb` — note that it does NOT import `crypto`:

    IMPORT cryptouser
    IMPORT io

    FUNC main() AS Integer
      io::print(toString(len(cryptouser::signSomething())))
      RETURN 0
    END FUNC

Then:

    mfb build /tmp/b557/pkg
    cp /tmp/b557/pkg/cryptouser.mfp /tmp/b557/app/packages/
    mfb build /tmp/b557/app          # error: ... '_mfb_ifn_crypto_5FgenerateEd25519' is not defined

Adding `IMPORT crypto` to `main.mfb` made it build and run — that was the
workaround, and the shape of the workaround is the shape of the cause.

## Mechanism

Six `crypto` members dispatch on an enum ordinal at run time and hand the chosen
branch to an MFBASIC helper rather than to native code: `hash` to a SHA core,
`seal`/`open` to an AEAD core, and `sign`/`verify`/`generate` to a software-curve
core. The dispatch is an `abi_function` lowering, which is emitted **once per
compilation unit** as a standalone body with no calling function in scope
(`function_lowering.rs`, "a runtime helper is standalone: no user
functions/globals/strings are in scope"). The symbol it branches to is therefore
fixed — `_mfb_ifn_crypto_generateEd25519` and friends — and cannot vary with
which package the call came from.

`ir::prefix_package_symbols` namespaces every function a decoded `.mfp` carries
into `<identity>.<package>.<name>`, so that two packages holding same-named
functions stay distinct after merge. The registry-injected `#crypto_…` helpers
are functions like any other in the package's IR, so they were renamed too. When
the program itself imported `crypto`, its own injection defined the bare name and
the branch resolved by luck; when it did not, the only definition in the merged
IR was the identity-prefixed one and the branch dangled.

## Two fixes that do not work, and why

Recorded because both look right.

**Resolve the symbol at the call site, from the enclosing function's package
prefix.** Impossible: the `abi_function` body is emitted once per unit with no
enclosing function, so there is no prefix to read. This is also why the fix
cannot live in `CodeBuilder`.

**Stop prefixing every `#`-sigil name at merge, since the namespace is reserved
and its bodies come from the registry.** This links, and then miscompiles.
`mangle_private` also puts file-scoped PRIVATE user names into the sigil space,
and lambda lifting produces sigil names whose bodies differ between packages, so
`merge_package`'s dedup-by-name silently collapses two different functions into
one. The observed symptom was
`PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: closure capture index 0 appears in a
function that is not a closure body`.

## The fix

`RegistryHelper::always_natively_called` marks the helpers a native lowering
branches to by their bare reserved symbol — the 30 `crypto` cores listed by
`registry::natively_called_helpers()`. After `merge_packages` has merged and
identity-prefixed every package, `define_natively_called_helpers` binds each
marked name that is not already defined to the first merged copy of it.

The copy is *aliased*, not rewritten: the package's own calls keep reaching their
own prefixed helpers, and the alias's body still calls those, so nothing about an
existing package changes and only the entry point is duplicated. Only the marked
helpers are aliased, which is what keeps this away from the second failed fix
above — a lambda-lifted body is never marked, so it is never collapsed.

Marking is declared on the helper, in the same package directory as the
`func_*.rs` that emits the branch. Forgetting it on a future natively-called
helper is not silent: the consumer's build fails with the same
`internal relocation target … is not defined` message this bug was found by.

Regression test: `tests/rt_package_calls_software_curve_builtin.rs` builds a
package that reaches all three curve members and an executable that consumes it
**without importing `crypto`**, then runs it and asserts the RFC 8032 signature
sizes (64 for Ed25519, 114 for Ed448) — so a link that succeeds but calls the
wrong body still fails.

## Found by

Building `packages/jwt/oracle/probe`, which imports `packages/jwt` and not
`crypto`.
