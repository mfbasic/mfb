/// Each registry implementation of `package` as `("member(paramType, …)",
/// declared errors)`, in registration order. The key names the PACKAGE's own
/// member and its parameter types, so the two mirror packages' identically
/// named overloads can never be confused for each other.
fn declared_rows(package: &str) -> Vec<(String, Vec<&'static str>)> {
    let registry = crate::codegen::registry::registry();
    let package = registry
        .resolve_package(package)
        .unwrap_or_else(|| panic!("{package} is a registered package"));
    let mut rows = Vec::new();
    for function in package.functions() {
        for implementation in function.implementations() {
            let params = implementation
                .params
                .iter()
                .map(|param| param.ty.name().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            rows.push((
                format!("{}({params})", function.name),
                implementation.errors.clone(),
            ));
        }
    }
    rows
}

fn assert_rows(package: &str, expected: &[(&str, &[&str])]) {
    let actual = declared_rows(package);
    // On any mismatch, print the whole actual table: a new overload shifts
    // every later row, and the diff of one row is unreadable without it.
    let rendered: Vec<String> = actual
        .iter()
        .map(|(key, errors)| format!("    (\"{key}\", &{errors:?}),"))
        .collect();
    assert_eq!(
        actual.len(),
        expected.len(),
        "{package} overload count changed; actual rows:\n{}",
        rendered.join("\n")
    );
    for (index, ((key, errors), (want_key, want_errors))) in
        actual.iter().zip(expected.iter()).enumerate()
    {
        assert_eq!(
            (key.as_str(), errors.as_slice()),
            (*want_key, *want_errors),
            "{package} row {index} disagrees; actual rows:\n{}",
            rendered.join("\n")
        );
    }
}

#[test]
fn every_tcp_member_declares_the_errors_its_lowering_raises() {
    assert_rows(
        "tcp",
        &[
            // All four `connect` overloads select `lower_net_endpoint_helper`
            // with `listen = false`: a negative `timeoutMs` is
            // ErrInvalidArgument, an unresolvable host ErrAddressNotFound
            // (the `listen` arm of the same branch answers
            // ErrAddressInvalid), the deadline ErrTimeout.
            (
                "connect(String, Integer)",
                &[
                    "ErrAddressInvalid",
                    "ErrAddressNotFound",
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrTimeout",
                ],
            ),
            (
                "connect(String, Integer, Integer)",
                &[
                    "ErrAddressInvalid",
                    "ErrAddressNotFound",
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrTimeout",
                ],
            ),
            (
                "connect(net.Address)",
                &[
                    "ErrAddressInvalid",
                    "ErrAddressNotFound",
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrTimeout",
                ],
            ),
            (
                "connect(net.Address, Integer)",
                &[
                    "ErrAddressInvalid",
                    "ErrAddressNotFound",
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrTimeout",
                ],
            ),
            // `listen = true` in the same helper: no timeout wait is branched
            // to (the label is emitted but unreachable), the backlog is
            // clamped rather than refused, and an unusable bind address is
            // ErrAddressInvalid.
            (
                "listen(String, Integer, Integer)",
                &["ErrAddressInvalid", "ErrNetworkFailed", "ErrOutOfMemory"],
            ),
            (
                "accept(tcp.Listener, Integer)",
                &[
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                    "ErrTimeout",
                ],
            ),
            // NOT ErrEncoding: that raise is inside `if text` and `tcp::read`
            // passes `text = false`. `maxBytes <= 0` is ErrInvalidArgument.
            (
                "read(tcp.Socket, Integer)",
                &[
                    "ErrConnectionClosed",
                    "ErrInvalidArgument",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                    "ErrTimeout",
                ],
            ),
            // The byte form validates the payload block's header (bug-497) —
            // that check, and so ErrInvalidArgument, is `if !text` only.
            (
                "write(tcp.Socket, List OF Byte)",
                &[
                    "ErrConnectionClosed",
                    "ErrInvalidArgument",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                    "ErrTimeout",
                ],
            ),
            (
                "write(tcp.Socket, String)",
                &[
                    "ErrConnectionClosed",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                    "ErrTimeout",
                ],
            ),
            // Readiness is a query: the scalar form answers an expired
            // deadline with FALSE, so it has no ErrTimeout. The list form has
            // no value that could mean "nothing" and raises instead.
            (
                "poll(tcp.Socket, Integer)",
                &[
                    "ErrInvalidArgument",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
            (
                "poll(List OF RES tcp.Socket, Integer)",
                &[
                    "ErrInvalidArgument",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrTimeout",
                ],
            ),
            // Already correct before bug-553, and the standing proof that the
            // parameter-gated reading is the right one: `lower_fs_close_helper`
            // also raises ErrWriteFailed, but only under `flush_on_close`,
            // which `tcp` passes `false`.
            (
                "close(tcp.Socket)",
                &["ErrResourceClosed", "ErrResourceMoved", "ErrCloseFailed"],
            ),
            (
                "close(tcp.Listener)",
                &["ErrResourceClosed", "ErrResourceMoved", "ErrCloseFailed"],
            ),
            (
                "localAddress(tcp.Socket)",
                &[
                    "ErrAddressInvalid",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
            (
                "localAddress(tcp.Listener)",
                &[
                    "ErrAddressInvalid",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
            (
                "remoteAddress(tcp.Socket)",
                &[
                    "ErrAddressInvalid",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
            (
                "setReadTimeout(tcp.Socket, Integer)",
                &[
                    "ErrInvalidArgument",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
            (
                "setWriteTimeout(tcp.Socket, Integer)",
                &[
                    "ErrInvalidArgument",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
        ],
    );
}

/// The `tls` half. Every row is the UNION over the three backends
/// (Network.framework, OpenSSL, Schannel), because the descriptor is
/// platform-independent and a page cannot say "ErrAddressNotFound on Linux".
/// That union is where the pair legitimately diverges: `.ai/net-tls.md`
/// records the backends naming one event three ways, and the union is the only
/// honest single answer.
#[test]
fn every_tls_member_declares_the_errors_its_lowering_raises() {
    assert_rows(
        "tls",
        &[
            (
                "connect(String, Integer, Integer, String, Boolean)",
                &[
                    "ErrAddressInvalid",
                    "ErrAddressNotFound",
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrTimeout",
                    "ErrTlsFailed",
                ],
            ),
            (
                "connect(net.Address, Integer, String, Boolean)",
                &[
                    "ErrAddressInvalid",
                    "ErrAddressNotFound",
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrTimeout",
                    "ErrTlsFailed",
                ],
            ),
            (
                "listen(String, Integer, String, String, Integer)",
                &[
                    "ErrAddressInvalid",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrTlsFailed",
                ],
            ),
            (
                "accept(tls.Listener, Integer)",
                &[
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                    "ErrTimeout",
                    "ErrTlsFailed",
                ],
            ),
            (
                "read(tls.Socket, Integer)",
                &[
                    "ErrConnectionClosed",
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                    "ErrTimeout",
                    "ErrTlsFailed",
                ],
            ),
            // Same `if !text` payload-header check as `tcp::write`.
            (
                "write(tls.Socket, List OF Byte)",
                &[
                    "ErrConnectionClosed",
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                    "ErrTimeout",
                    "ErrTlsFailed",
                ],
            ),
            (
                "write(tls.Socket, String)",
                &[
                    "ErrConnectionClosed",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                    "ErrTimeout",
                    "ErrTlsFailed",
                ],
            ),
            // macOS answers the endpoint query out of the Network.framework
            // connection (ErrNetworkFailed); Linux and Windows share `tcp`'s
            // `getsockname` emitter (ErrAddressInvalid). The union is both.
            (
                "localAddress(tls.Socket)",
                &[
                    "ErrAddressInvalid",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
            (
                "localAddress(tls.Listener)",
                &[
                    "ErrAddressInvalid",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
            (
                "remoteAddress(tls.Socket)",
                &[
                    "ErrAddressInvalid",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
            (
                "setReadTimeout(tls.Socket, Integer)",
                &[
                    "ErrInvalidArgument",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
            (
                "setWriteTimeout(tls.Socket, Integer)",
                &[
                    "ErrInvalidArgument",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
            ),
            // The scalar form answers an expired deadline FALSE, so no
            // ErrTimeout. The list form branch-links the scalar helper and
            // propagates its error, so its set is the scalar's plus the two it
            // raises itself.
            (
                "poll(tls.Socket, Integer)",
                &[
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                    "ErrTlsFailed",
                ],
            ),
            (
                "poll(List OF RES tls.Socket, Integer)",
                &[
                    "ErrInvalidArgument",
                    "ErrNetworkFailed",
                    "ErrOutOfMemory",
                    "ErrResourceClosed",
                    "ErrTimeout",
                    "ErrTlsFailed",
                ],
            ),
            // Already correct before bug-553 (bug-525 made a second close
            // refuse) — pinned so the audit cannot churn a right answer.
            (
                "close(tls.Socket)",
                &["ErrResourceClosed", "ErrResourceMoved", "ErrTlsFailed"],
            ),
            (
                "close(tls.Listener)",
                &["ErrResourceClosed", "ErrResourceMoved", "ErrTlsFailed"],
            ),
        ],
    );
}

/// The reason a WRONG list would have been worse than an empty one, pinned:
/// today these lists are inert for the dead-handler verdict, and this is what
/// makes that true.
///
/// [`super::inline_builtin_is_infallible`] reaches its registry-data branch
/// through [`crate::codegen::registry::native_member_declares_error`], which
/// answers `None` unless the member owns a [`Body::AbiInline`] call-site
/// lowering. Every `tcp`/`tls` member is an `abi_function` (a `bl`'d runtime
/// helper), so no list here — right, wrong, or empty — can prove a raising
/// member infallible and delete a live `TRAP` handler the way bug-486,
/// bug-533 and `strings::left`/`right`/`padLeft`/`padRight` did.
///
/// If a member here ever moves to an inline-lowerable body this test goes red,
/// which is exactly when the tables above start being load-bearing for
/// codegen rather than for the man pages.
#[test]
fn no_tcp_or_tls_error_list_reaches_the_infallibility_verdict() {
    use crate::codegen::registry::{native_bare_target, native_member_declares_error};
    let registry = crate::codegen::registry::registry();
    for package_name in ["tcp", "tls"] {
        let package = registry
            .resolve_package(package_name)
            .expect("a registered package");
        for function in package.functions() {
            let qualified = format!("{package_name}.{}", function.name);
            assert_eq!(
                native_member_declares_error(&qualified),
                None,
                "{qualified} is not an abi_function any more — its `errors` list \
                     now feeds inline_builtin_is_infallible, so it must be exact",
            );
            assert_eq!(
                native_bare_target(&qualified),
                None,
                "{qualified} dequalifies to a bare inline native name",
            );
        }
    }
}
