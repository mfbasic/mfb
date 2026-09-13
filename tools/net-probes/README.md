# tools/net-probes

Measuring probes behind the `net::ping` native backends (plan-110-A). No gate runs
them: each prints facts from the running OS and its headers, so the values committed
in codegen are transcribed from a measurement rather than recalled. Re-run them on
every supported POSIX target to re-derive those values.

- **icmp-capability-probe.c** — per-OS ICMP socket behavior: unprivileged
  `SOCK_DGRAM`/`SOCK_RAW`, reply shape (IPv4 header attached or not), identifier
  rewriting, reply demux, maximum payload, Time Exceeded / Destination Unreachable
  delivery. It justifies the backend split in `src/codegen/builtins/net/gen_ping.rs`.

      cc -O0 -w -o /tmp/icmp-probe tools/net-probes/icmp-capability-probe.c
      /tmp/icmp-probe                 # local + default-route probes
      /tmp/icmp-probe 1.1.1.1         # override the off-link target

- **icmp-constants-probe.c** — every socket-option value, clock id and
  `msghdr`/`cmsghdr` field offset the backends hardcode into emitted machine code,
  as transcribed in `src/target/linux_common/code.rs` and
  `src/target/macos_aarch64/code.rs` (plan-110-A §C5).

      cc -O0 -o /tmp/icmp-consts tools/net-probes/icmp-constants-probe.c && /tmp/icmp-consts
