//! bug-876: a port outside 0..65535 is `ErrAddressInvalid` on every host/port entry
//! point, never silently reduced to its low 16 bits.
//!
//! The rule (`mfb spec stdlib transports`, error registry row `7-707-0001`): a port
//! is an unsigned 16-bit number. `tcp::connect(host, p + 65536)` used to connect to
//! port `p`, `tcp::listen(host, 70000)` bound 4464 and `udp::bind(host, -1)` bound
//! 65535. The programs print the raw error code so a differently-shaped failure
//! (`ErrNetworkFailed`, 77070003) cannot satisfy the assertion.

#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

use common::{build_project, run_bounded, temp_project};

/// `errorCode::ErrAddressInvalid` — `mfb spec diagnostics error-codes` row `7-707-0001`.
const ADDRESS_INVALID: &str = "77070001";

fn run(name: &str, source: &str) -> String {
    let project = temp_project(name, source);
    let executable = build_project(&project);
    let (status, stdout) = run_bounded(&executable, Duration::from_secs(60), name);
    assert!(
        status.success(),
        "{name} exited {:?}\nstdout:\n{stdout}",
        status.code()
    );
    stdout
}

/// One probe function per call: `TRAP` sits at the bottom of a function, and the
/// probe returns 0 when the call succeeded, else the error code.
fn probe(name: &str, body: &str) -> String {
    format!(
        "FUNC {name}(p AS Integer) AS Integer\n  {body}\n  RETURN 0\n  TRAP(e)\n    RETURN e.code\n  END TRAP\nEND FUNC\n\n"
    )
}

fn program(cases: &[(&str, &str)], main_body: &str) -> String {
    let mut src = String::from("IMPORT io\nIMPORT net\nIMPORT tcp\nIMPORT tls\nIMPORT udp\n\n");
    for (name, body) in cases {
        src.push_str(&probe(name, body));
    }
    src.push_str("FUNC main AS Integer\n");
    src.push_str(main_body);
    src.push_str("  RETURN 0\nEND FUNC\n");
    src
}

const BAD_PORTS: [&str; 4] = ["70000", "65536", "-1", "4294967296"];

#[test]
fn out_of_range_port_is_address_invalid_on_tcp_and_udp() {
    let cases = [
        ("tcpConnect", "RES c = tcp::connect(\"127.0.0.1\", p)\n  tcp::close(c)"),
        ("tcpConnectTimeout", "RES c = tcp::connect(\"127.0.0.1\", p, 100)\n  tcp::close(c)"),
        ("tcpListen", "RES l = tcp::listen(\"127.0.0.1\", p)\n  tcp::close(l)"),
        ("tcpListenAll", "RES l = tcp::listen(\"\", p)\n  tcp::close(l)"),
        ("udpBind", "RES u = udp::bind(\"127.0.0.1\", p)\n  udp::close(u)"),
        ("udpBindAll", "RES u = udp::bind(\"\", p)\n  udp::close(u)"),
        ("lookupPort", "LET a = net::lookup(\"127.0.0.1\", p)"),
        ("tlsConnect", "RES c = tls::connect(\"127.0.0.1\", p, 100)\n  tls::close(c)"),
        ("tlsListen", "RES l = tls::listen(\"127.0.0.1\", p, \"no-cert.pem\", \"no-key.pem\")\n  tls::close(l)"),
    ];
    let mut main_body = String::new();
    for (name, _) in &cases {
        for port in BAD_PORTS {
            main_body.push_str(&format!(
                "  io::print(\"{name} {port}=\" & toString({name}({port})))\n"
            ));
        }
    }
    let stdout = run("port_range_bad", &program(&cases, &main_body));
    for (name, _) in &cases {
        for port in BAD_PORTS {
            assert!(
                stdout.contains(&format!("{name} {port}={ADDRESS_INVALID}\n")),
                "{name}({port}) must raise ErrAddressInvalid ({ADDRESS_INVALID}), not wrap \
                 to the low 16 bits (bug-876).\nstdout:\n{stdout}"
            );
        }
    }
}

#[test]
fn in_range_ports_still_work() {
    let cases = [
        (
            "listenZero",
            "RES l = tcp::listen(\"127.0.0.1\", p)\n  tcp::close(l)",
        ),
        (
            "bindZero",
            "RES u = udp::bind(\"127.0.0.1\", p)\n  udp::close(u)",
        ),
    ];
    let stdout = run(
        "port_range_ok",
        &program(
            &cases,
            "  io::print(\"listen0=\" & toString(listenZero(0)))\n  io::print(\"bind0=\" & toString(bindZero(0)))\n",
        ),
    );
    assert!(
        stdout.contains("listen0=0\n"),
        "port 0 must still bind: {stdout}"
    );
    assert!(
        stdout.contains("bind0=0\n"),
        "port 0 must still bind: {stdout}"
    );
}
