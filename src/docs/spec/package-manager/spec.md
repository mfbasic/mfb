# Package Manager and Registry

The registry protocol and signing/trust workflow behind `mfb repo register`,
`mfb repo auth`, `mfb repo publish`, and `mfb build --sign`, together with
registry reads, machine linking, key
rotation, ownership operations, release states, and signed metadata. This is
the contract a compatible registry server or a reimplemented client must honor:
the HTTP endpoints and wire shapes, the local key/session store, the Ed25519
signing model and its domain strings, and the owner-name grammar. (`mfb pkg add`
accepts either a `file://…​.mfp` URL — copied into `packages/` locally with no
protocol — or an `<owner>#<package>[@version]` registry ident, which is resolved
and installed over this protocol; see `./mfb spec tooling cli-reference`.)[[src/cli/pkg.rs:add_package]][[src/manifest/url.rs:package_file_url_path]]

This package owns the *protocol and crypto workflow*. It is distinct from the `.mfp` byte format and its
signature header (`./mfb spec package container-format`), the CLI command surface
(`./mfb spec tooling cli-reference`), and the manifest fields that record an
owner's fingerprints (`./mfb spec tooling project-manifest`).

## Reading order

- `repository-protocol` — the client↔registry HTTP protocol: endpoints, JSON
  request/response bodies, the challenge-response auth flow, the session token,
  and the validate-then-publish sequence.
- `key-store` — the local `~/.mfb` key and session storage layout and permissions.
- `signing` — the Ed25519 ident-key vs signing-key model, fingerprints, the
  signing-domain strings, and how `build --sign` matches a local key.
- `owner-names` — the owner-name grammar and validation rules.

## See Also

* ./mfb spec package container-format — the `.mfp` ed25519 signature header
* ./mfb spec tooling cli-reference — the `repo`/`pkg` commands and their exit codes
* ./mfb spec tooling project-manifest — `ident`/`signingFingerprint` manifest fields
