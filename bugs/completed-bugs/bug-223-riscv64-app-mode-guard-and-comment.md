# bug-223: riscv64 build guard admits LinuxApp (→ unimplemented! panic) + variadic-call comment names wrong arch

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun / docs

Status: Fixed (2026-07-15) — lower_validated_module now admits only NativeBuildMode::Console for riscv64 (a LinuxApp module returns a clean error instead of reaching code.rs's unimplemented! panic), and emit_variadic_call's comment references the RISC-V lp64d ABI (a0–a7) instead of the copied AArch64 text.

Two low items on the linux-riscv64 target:

- `lower_validated_module` (`src/target/linux_riscv64/mod.rs:427`) permits
  `NativeBuildMode::LinuxApp` even though `supports_app_mode()` is false, so a
  `LinuxApp` module reaching this backend panics via `unimplemented!("rv64 app
  mode not ported")` in `code.rs` instead of returning a clean error. Not
  reachable via the CLI (which rejects `-app` for unsupported targets first), but
  any non-CLI/test/API caller that constructs `write_executable(..., LinuxApp)`
  aborts the process. Fix: restrict the accepted set to `Console` for riscv64.
- `emit_variadic_call` (`src/target/linux_riscv64/code.rs:409`) comment claims
  "The Linux AArch64 ABI passes variadic GP arguments in registers" — a verbatim
  copy of the aarch64 comment inside the riscv64 backend. Behavior is correct
  (lp64d also passes variadic GP args in a0–a7); only the named arch is wrong.
  Fix: reword to reference the RISC-V lp64d ABI.
