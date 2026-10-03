# Auditability

`mfb audit` validates the project front end and collects a report without
executing its code. The report combines source-level analysis with manifest
and installed-package metadata. Its output schema, finding catalogue and
fallibility tables are specified by `./mfb spec tooling audit-format`.
[[src/audit/mod.rs:run]] [[src/audit/collect/mod.rs:collect]]

## Source analysis

Collection uses the original parsed AST, after the augmented and monomorphized
project passes resolution, entry and shape validation. It reports recognized
fallible calls, their propagation context, function-level `TRAP` classifications,
recognized resource acquisitions, native declarations and host capabilities.
This is a syntactic analysis with explicit builtin tables, not a complete
control-flow or resource-liveness analysis. It does not enumerate lexical
cleanup edges for every exit path. [[src/audit/mod.rs:run]]
[[src/audit/collect/source.rs:collect_source]]
[[src/audit/collect/source.rs:collect_resources]]

## Package and manifest analysis

The collector reports dependency status, installed package header/info decoding,
export/import/cleanup counts, exported mutable globals and secondary-close
metadata. It does not reconstruct imported function bodies into the source-flow
table. A package's `verifier: "ok"` means its header and representation info
were readable; it is not a cryptographic trust-chain verdict. Use the package
verification commands for that contract.
[[src/audit/collect/dependencies.rs:collect_packages]]
[[src/audit/collect/findings.rs:package_findings]]

Manifest library locators and resource-copy entries appear independently of
source call analysis. Lockfile findings compare the declared dependency hash;
the audit does not install dependencies or contact a registry.
[[src/audit/collect/project.rs:collect_libraries]]
[[src/audit/collect/lockfile.rs:collect_lockfile]]

## See Also

* ./mfb spec tooling audit-format — schema, analysis rules and findings
* ./mfb spec tooling cli-reference — audit and package verification commands
* ./mfb spec tooling lockfile — lockfile interpretation
* ./mfb spec language error-model — language failure propagation
* ./mfb spec language resource-management — resource cleanup semantics
