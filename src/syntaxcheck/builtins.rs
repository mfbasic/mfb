use super::helpers::*;
use super::*;

impl<'a> SyntaxChecker<'a> {
    pub(super) fn check_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
    ) -> Type {
        if builtins::encoding::is_encoding_call(callee) {
            return self.check_encoding_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
                expected,
            );
        }
        if builtins::crypto::is_crypto_call(callee) {
            return self.check_crypto_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::general::is_general_call(callee) {
            return self.check_general_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::collections::is_native_member_call(callee) {
            return self.check_collections_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::strings::is_strings_call(callee) {
            return self.check_strings_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::math::is_math_call(callee) {
            return self.check_math_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::bits::is_bits_call(callee) {
            return self.check_bits_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::fs::is_fs_call(callee) {
            return self.check_fs_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::os::is_os_call(callee) {
            return self.check_os_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::net::is_net_call(callee) {
            return self.check_net_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::tls::is_tls_call(callee) {
            return self.check_tls_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::io::is_io_call(callee) {
            return self.check_io_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::term::is_term_call(callee) {
            return self.check_term_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::json::is_json_call(callee) {
            return self.check_json_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::csv::is_csv_call(callee) {
            return self.check_csv_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::regex::is_regex_call(callee) {
            return self.check_regex_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::datetime::is_datetime_call(callee) {
            return self.check_datetime_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::http::is_http_call(callee) {
            return self.check_http_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::thread::is_thread_call(callee) {
            return self.check_thread_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::vector::is_vector_call(callee) {
            return self.check_vector_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }

        for argument in arguments {
            self.infer_expression(file, call_arg_value(argument), locals, line, ExprMode::Read);
        }
        Type::Unknown
    }

    pub(super) fn check_vector_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Borrow);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::vector::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::vector::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::vector::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_fs_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                let mode = if callee == "fs.close" && index == 0 {
                    ExprMode::Transfer
                } else {
                    ExprMode::Borrow
                };
                let type_ = self.infer_expression(file, argument, locals, line, mode);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::fs::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::fs::resolve_call(callee, &arg_types) else {
            let expected = builtins::fs::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_os_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Borrow);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::os::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::os::resolve_call(callee, &arg_types) else {
            let expected = builtins::os::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_net_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                // `net.close` consumes the socket/listener handle it closes.
                let mode = if callee == "net.close" && index == 0 {
                    ExprMode::Transfer
                } else {
                    ExprMode::Borrow
                };
                let type_ = self.infer_expression(file, argument, locals, line, mode);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::net::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::net::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::net::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_tls_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                // `tls.close` consumes the `TlsSocket` it closes.
                let mode = if builtins::tls::consumes_argument(callee, index) {
                    ExprMode::Transfer
                } else {
                    ExprMode::Borrow
                };
                let type_ = self.infer_expression(file, argument, locals, line, mode);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::tls::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::tls::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::tls::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_json_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::json::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::json::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::json::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_csv_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::csv::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::csv::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::csv::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_http_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                // `http.respondFile` consumes (takes ownership of) the `RES File`
                // it serves, closing it by lexical drop (plan-05 §F.5.5).
                let mode = if builtins::http::consumes_argument(callee, index) {
                    ExprMode::Transfer
                } else {
                    ExprMode::Read
                };
                let type_ = self.infer_expression(file, argument, locals, line, mode);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::http::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::http::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::http::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_regex_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::regex::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::regex::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::regex::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_datetime_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::datetime::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::datetime::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::datetime::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_io_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::io::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    if min == 0 {
                        "0".to_string()
                    } else {
                        min.to_string()
                    }
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::io::resolve_call(callee, &arg_types) else {
            let expected = builtins::io::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_term_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);

        if let Some((min, max)) = builtins::term::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                // Still infer the arguments so nested errors are reported.
                for argument in &arguments {
                    self.infer_expression(file, argument, locals, line, ExprMode::Read);
                }
                return self.term_return_type(callee);
            }
        }

        let param_types = builtins::term::param_types(callee).unwrap_or(&[]);
        let arg_types = arguments
            .iter()
            .map(|argument| self.infer_expression(file, argument, locals, line, ExprMode::Read))
            .collect::<Vec<_>>();

        let mut mismatch = false;
        for (index, expected_name) in param_types.iter().enumerate() {
            let expected = self.parse_type(expected_name);
            let actual = &arg_types[index];
            if !self.expression_compatible(&expected, actual, Some(&arguments[index])) {
                mismatch = true;
            }
        }

        if mismatch {
            let expected = builtins::term::expected_arguments(callee)
                .unwrap_or_else(|| "no arguments".to_string());
            let actual = arg_types
                .iter()
                .map(|type_| self.type_name(type_))
                .collect::<Vec<_>>()
                .join(", ");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({actual}), expected {expected}."
                ),
                file,
                line,
            );
        }

        self.term_return_type(callee)
    }

    pub(super) fn term_return_type(&mut self, callee: &str) -> Type {
        match builtins::term::resolve_call(callee) {
            Some(resolved) => self.parse_type(&resolved.return_type),
            None => Type::Unknown,
        }
    }

    pub(super) fn check_thread_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                self.infer_expression(
                    file,
                    argument,
                    locals,
                    line,
                    self.thread_argument_mode(callee, index),
                )
            })
            .collect::<Vec<_>>();
        let arg_type_names = arg_types
            .iter()
            .map(|type_| self.type_name(type_))
            .collect::<Vec<_>>();

        if callee == "thread.start" {
            let valid_entry = match arguments.first() {
                Some(Expression::Identifier(name)) => {
                    let canonical_name = self.canonical_import_name(file, name);
                    self.lookup_visible_function(file, name)
                        .or_else(|| self.lookup_visible_function(file, &canonical_name))
                        .is_some_and(|sig| {
                            sig.imported_package_export
                                && matches!(sig.kind, FunctionKind::Func)
                                && sig.isolated
                        })
                }
                _ => false,
            };
            if !valid_entry {
                self.report(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    "thread.start entry point must be an exported ISOLATED FUNC from an imported package.",
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        if let Some((min, max)) = builtins::thread::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::thread::resolve_call(callee, &arg_type_names) else {
            let expected =
                builtins::thread::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_type_names.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        let return_type = self.parse_type(&resolved.return_type);
        self.check_thread_boundary_sendability(
            file,
            display_callee,
            callee,
            &arg_types,
            &return_type,
            line,
        );
        return_type
    }

    pub(super) fn check_strings_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::strings::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::strings::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::strings::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_math_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::math::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::math::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::math::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_bits_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::bits::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::bits::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::bits::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_crypto_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::crypto::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected_count = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected_count}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::crypto::resolve_call(callee, &arg_types) else {
            let expected_args =
                builtins::crypto::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected_args}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_encoding_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::encoding::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected_count = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected_count}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::encoding::resolve_call(callee, &arg_types) else {
            let expected_args =
                builtins::encoding::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected_args}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        // `utf8Encode` is a return-type overload (List OF Byte | List OF Integer).
        // When the call has an expected (contextual) type of one of the two, adopt
        // it; otherwise fall back to the default (List OF Byte). The hard
        // `TYPE_OVERLOAD_AMBIGUOUS` error for an unannotated call is raised later,
        // in the monomorphizer (plan-01-overload.md §F.2).
        if callee == "encoding.utf8Encode" {
            if let Some(expected) = expected {
                let expected_name = self.type_name(expected);
                if expected_name == "List OF Byte" || expected_name == "List OF Integer" {
                    return expected.clone();
                }
            }
        }

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_general_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);

        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                self.infer_expression(
                    file,
                    argument,
                    locals,
                    line,
                    self.general_argument_mode(callee, index),
                )
            })
            .collect::<Vec<_>>();
        let arg_type_names = arg_types
            .iter()
            .map(|type_| self.type_name(type_))
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::general::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::general::resolve_call(callee, &arg_type_names) else {
            // The built-in rejected these argument types, so an override may fill
            // the gap (plan-01-overload.md §A.3.2). A *user* override has already
            // been rewritten to its mangled concrete symbol by the monomorphizer
            // (§B.1 / Phase 5), so it never reaches this bare-name path; only a
            // *package*-provided override (the registry, §B.2) is resolved here —
            // e.g. `toString(net::Url)` routes to the package's internal renderer
            // and yields the built-in's conventional result type.
            if builtins::general::is_overridable(callee)
                && arg_type_names.len() == 1
                && builtins::general_override_target(callee, &arg_type_names[0]).is_some()
            {
                return self.parse_type(
                    builtins::general::override_result_type(callee).unwrap_or("Unknown"),
                );
            }
            let expected =
                builtins::general::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_type_names.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.check_general_builtin_comparability(file, display_callee, callee, &arg_types, line);

        self.parse_type(&resolved.return_type)
    }

    /// Typechecks a migrated `collections::` native member call (plan-01 §5).
    /// Mirrors `check_general_builtin_call` but resolves through the `collections`
    /// helper set; `callee` is the canonical `collections.<member>` form.
    pub(super) fn check_collections_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let member = builtins::collections::native_member_bare(callee).unwrap_or(callee);
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        if callee == "collections.filter" && arguments.len() == 2 {
            if let Expression::Identifier(predicate) = &arguments[1] {
                if builtins::general::builtin_function_id(predicate).is_some() {
                    let collection_type =
                        self.infer_expression(file, &arguments[0], locals, line, ExprMode::Read);
                    let collection_type_name = self.type_name(&collection_type);
                    let predicate_type =
                        collection_type_name
                            .strip_prefix("List OF ")
                            .and_then(|element| {
                                builtins::general::filter_predicate_type(predicate, element)
                            });

                    let Some(predicate_type) = predicate_type else {
                        self.report(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            &format!(
                                "Call to `{display_callee}` has argument type(s) ({collection_type_name}, {predicate}), expected {}.",
                                builtins::collections::expected_arguments(callee)
                                    .unwrap_or("supported overload")
                            ),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    };

                    let arg_types = vec![collection_type_name, predicate_type];
                    let Some(resolved) = builtins::collections::resolve_call(callee, &arg_types)
                    else {
                        self.report(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            &format!(
                                "Call to `{display_callee}` has argument type(s) ({}), expected {}.",
                                arg_types.join(", "),
                                builtins::collections::expected_arguments(callee)
                                    .unwrap_or("supported overload")
                            ),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    };

                    return self.parse_type(&resolved.return_type);
                }
            }
        }

        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                // License a `MUT` borrow for a lambda in a non-escaping callback
                // position (e.g. `forEach`'s action). `infer_lambda` consumes it;
                // reset afterward so a non-lambda argument never carries it.
                self.nonescaping_callback = builtins::is_nonescaping_callback_arg(member, index);
                let arg_type = self.infer_expression(
                    file,
                    argument,
                    locals,
                    line,
                    self.general_argument_mode(member, index),
                );
                self.nonescaping_callback = false;
                arg_type
            })
            .collect::<Vec<_>>();
        let arg_type_names = arg_types
            .iter()
            .map(|type_| self.type_name(type_))
            .collect::<Vec<_>>();

        if matches!(member, "append" | "prepend" | "insert" | "set") {
            for (index, (argument, arg_type)) in arguments.iter().zip(arg_types.iter()).enumerate()
            {
                if index == 0 {
                    continue;
                }
                self.check_collection_resource_element(
                    file, line, "element", argument, arg_type, locals,
                );
            }
        }

        if let Some((min, max)) = builtins::collections::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::collections::resolve_call(callee, &arg_type_names) else {
            let expected =
                builtins::collections::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_type_names.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.check_general_builtin_comparability(file, display_callee, member, &arg_types, line);

        self.parse_type(&resolved.return_type)
    }

    pub(super) fn check_general_builtin_comparability(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arg_types: &[Type],
        line: usize,
    ) {
        match callee {
            "contains" | "replace" => {
                let Some(Type::List(element)) = arg_types.first() else {
                    return;
                };
                self.require_comparable_type(
                    file,
                    line,
                    &format!("Call to `{display_callee}`"),
                    element,
                );
            }
            "find" => {
                let Some(first) = arg_types.first() else {
                    return;
                };
                if let Type::List(element) = first {
                    self.require_comparable_type(
                        file,
                        line,
                        &format!("Call to `{display_callee}`"),
                        element,
                    );
                }
            }
            _ => {}
        }
    }

    pub(super) fn normalize_builtin_call_arguments(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        line: usize,
    ) -> Vec<Expression> {
        if !arguments
            .iter()
            .any(|argument| matches!(argument, CallArg::Named { .. }))
        {
            return arguments
                .iter()
                .map(|argument| call_arg_value(argument).clone())
                .collect();
        }
        if let Some(overloads) = builtins::call_param_name_overloads(callee) {
            return self.normalize_overloaded_builtin_call_arguments(
                file,
                display_callee,
                overloads,
                arguments,
                line,
            );
        }
        let Some(param_names) = builtins::call_param_names(callee) else {
            return arguments
                .iter()
                .map(|argument| call_arg_value(argument).clone())
                .collect();
        };
        let mut ordered = vec![None; param_names.len()];
        let mut next_positional = 0usize;
        let mut extras = Vec::new();
        let mut saw_unknown_named = false;
        for argument in arguments {
            match argument {
                CallArg::Positional(value) => {
                    while next_positional < ordered.len() && ordered[next_positional].is_some() {
                        next_positional += 1;
                    }
                    if next_positional < ordered.len() {
                        ordered[next_positional] = Some(value.clone());
                        next_positional += 1;
                    } else {
                        extras.push(value.clone());
                    }
                }
                CallArg::Named { name, value, line } => {
                    let Some(index) = param_names
                        .iter()
                        .position(|aliases| aliases.iter().any(|alias| alias == name))
                    else {
                        self.report(
                            "TYPE_UNKNOWN_ARGUMENT_NAME",
                            &format!(
                                "Call to `{display_callee}` does not have a parameter named `{name}`."
                            ),
                            file,
                            *line,
                        );
                        saw_unknown_named = true;
                        continue;
                    };
                    if ordered[index].is_some() {
                        self.report(
                            "TYPE_DUPLICATE_ARGUMENT_NAME",
                            &format!(
                                "Call to `{display_callee}` supplies parameter `{}` more than once.",
                                param_names[index][0]
                            ),
                            file,
                            *line,
                        );
                        continue;
                    }
                    ordered[index] = Some(value.clone());
                }
            }
        }
        if !saw_unknown_named {
            for (index, aliases) in param_names.iter().enumerate() {
                if ordered[index].is_none()
                    && ordered
                        .iter()
                        .skip(index + 1)
                        .any(|argument| argument.is_some())
                {
                    self.report(
                        "TYPE_CALL_ARITY_MISMATCH",
                        &format!(
                            "Call to `{display_callee}` omits parameter `{}` before a later supplied argument.",
                            aliases[0]
                        ),
                        file,
                        line,
                    );
                    break;
                }
            }
        }
        let mut normalized = ordered.into_iter().flatten().collect::<Vec<_>>();
        normalized.extend(extras);
        normalized
    }

    /// Normalize a call to a builtin whose overloads place the same parameter name
    /// at different positions (`net::connectTcp`'s `timeoutMs` is param 1 of the
    /// `Address` forms and param 2 of the host/port forms).
    ///
    /// A merged per-position alias table cannot express that: the first group
    /// containing a name wins, so `timeoutMs` would always bind to `port`. Select
    /// the overload first — the one whose parameter names cover every supplied name
    /// with no positional collision and no missing argument — then bind names
    /// within it.
    fn normalize_overloaded_builtin_call_arguments(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        overloads: &[&[&str]],
        arguments: &[CallArg],
        line: usize,
    ) -> Vec<Expression> {
        let positionals: Vec<&Expression> = arguments
            .iter()
            .filter_map(|argument| match argument {
                CallArg::Positional(value) => Some(value),
                CallArg::Named { .. } => None,
            })
            .collect();
        let named: Vec<(&String, &Expression, usize)> = arguments
            .iter()
            .filter_map(|argument| match argument {
                CallArg::Named { name, value, line } => Some((name, value, *line)),
                CallArg::Positional(_) => None,
            })
            .collect();

        let fallback = || {
            arguments
                .iter()
                .map(|argument| call_arg_value(argument).clone())
                .collect::<Vec<_>>()
        };

        for (index, (name, _, named_line)) in named.iter().enumerate() {
            if named[..index].iter().any(|(earlier, _, _)| earlier == name) {
                self.report(
                    "TYPE_DUPLICATE_ARGUMENT_NAME",
                    &format!(
                        "Call to `{display_callee}` supplies parameter `{name}` more than once."
                    ),
                    file,
                    *named_line,
                );
                return fallback();
            }
        }
        if let Some((name, _, named_line)) = named
            .iter()
            .find(|(name, _, _)| !overloads.iter().any(|params| params.contains(&name.as_str())))
        {
            self.report(
                "TYPE_UNKNOWN_ARGUMENT_NAME",
                &format!("Call to `{display_callee}` does not have a parameter named `{name}`."),
                file,
                *named_line,
            );
            return fallback();
        }

        let supplied_names: Vec<&str> = named.iter().map(|(name, _, _)| name.as_str()).collect();
        if let Some(params) =
            builtins::select_param_name_overload(overloads, positionals.len(), &supplied_names)
        {
            let mut ordered: Vec<Option<&Expression>> = vec![None; params.len()];
            for (index, value) in positionals.iter().enumerate() {
                ordered[index] = Some(value);
            }
            for (name, value, _) in &named {
                let index = params
                    .iter()
                    .position(|param| param == name)
                    .expect("the selected overload names every supplied argument");
                ordered[index] = Some(value);
            }
            return ordered.into_iter().flatten().cloned().collect();
        }

        // Every supplied name exists, but no overload's arity and layout accept
        // this combination: report the first parameter left unsupplied by the
        // smallest overload that names them all (`connectTcp(host:, timeoutMs:)`
        // omits `port`).
        let covering = overloads
            .iter()
            .filter(|params| {
                named
                    .iter()
                    .all(|(name, _, _)| params.contains(&name.as_str()))
            })
            .collect::<Vec<_>>();
        if let Some(params) = covering.iter().min_by_key(|params| params.len()) {
            let missing = params.iter().enumerate().find(|(index, param)| {
                *index >= positionals.len() && !named.iter().any(|(name, _, _)| name == *param)
            });
            if let Some((_, missing)) = missing {
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` omits parameter `{missing}` before a later supplied argument."
                    ),
                    file,
                    line,
                );
                return fallback();
            }
        }
        self.report(
            "TYPE_CALL_ARITY_MISMATCH",
            &format!("Call to `{display_callee}` has no overload taking these arguments."),
            file,
            line,
        );
        fallback()
    }

    pub(super) fn normalize_named_arguments(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[CallArg],
        params: &[ParamSig],
        line: usize,
        allow_trailing_omission: bool,
    ) -> Vec<Option<Expression>> {
        let mut ordered = vec![None; params.len()];
        let mut next_positional = 0usize;
        let mut supplied = 0usize;
        let mut arity_error = false;

        for argument in arguments {
            match argument {
                CallArg::Positional(value) => {
                    while next_positional < ordered.len() && ordered[next_positional].is_some() {
                        next_positional += 1;
                    }
                    if next_positional >= ordered.len() {
                        arity_error = true;
                        continue;
                    }
                    ordered[next_positional] = Some(value.clone());
                    next_positional += 1;
                    supplied += 1;
                }
                CallArg::Named { name, value, line } => {
                    let Some(index) = params.iter().position(|param| param.name == *name) else {
                        self.report(
                            "TYPE_UNKNOWN_ARGUMENT_NAME",
                            &format!(
                                "Call to `{callee}` does not have a parameter named `{name}`."
                            ),
                            file,
                            *line,
                        );
                        continue;
                    };
                    if ordered[index].is_some() {
                        self.report(
                            "TYPE_DUPLICATE_ARGUMENT_NAME",
                            &format!(
                                "Call to `{callee}` supplies parameter `{name}` more than once."
                            ),
                            file,
                            *line,
                        );
                        continue;
                    }
                    ordered[index] = Some(value.clone());
                    supplied += 1;
                }
            }
        }

        let required = params.iter().filter(|param| !param.has_default).count();
        let missing_required = ordered
            .iter()
            .zip(params.iter())
            .any(|(argument, param)| argument.is_none() && !param.has_default);
        let max_supplied = ordered
            .iter()
            .rposition(Option::is_some)
            .map(|index| index + 1)
            .unwrap_or(0);
        let has_internal_gap = allow_trailing_omission
            && ordered
                .iter()
                .zip(params.iter())
                .take(max_supplied)
                .any(|(argument, param)| argument.is_none() && !param.has_default);

        if arity_error
            || supplied < required
            || supplied > params.len()
            || missing_required
            || has_internal_gap
        {
            self.report(
                "TYPE_CALL_ARITY_MISMATCH",
                &format!(
                    "Call to `{callee}` has {} argument(s), expected {} to {}.",
                    supplied,
                    required,
                    params.len()
                ),
                file,
                line,
            );
        }

        ordered
    }
}

#[cfg(test)]
mod builtins_tests {
    use crate::syntaxcheck::testutil::*;

    // ---- per-package accept paths (resolved return type) -------------------

    #[test]
    fn bits_valid() {
        assert!(accepts(
            "IMPORT io\nIMPORT bits\nFUNC main AS Integer\n  io::print(toString(bits::band(255, 15)))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn math_valid() {
        assert!(accepts(
            "IMPORT io\nIMPORT math\nFUNC main AS Integer\n  io::print(toString(math::abs(-7)))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn strings_valid() {
        assert!(accepts(
            "IMPORT io\nIMPORT strings\nFUNC main AS Integer\n  LET b = strings::toBytes(\"hi\")\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn encoding_valid() {
        assert!(accepts(
            "IMPORT io\nIMPORT encoding\nFUNC main AS Integer\n  io::print(encoding::base64Encode([toByte(102)]))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn term_valid() {
        assert!(accepts(
            "IMPORT term\nFUNC main AS Integer\n  term::clear()\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn datetime_valid() {
        assert!(accepts(
            "IMPORT io\nIMPORT datetime\nFUNC main AS Integer\n  LET i = datetime::instant(100)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn regex_valid() {
        assert!(accepts(
            "IMPORT regex\nFUNC main AS Integer\n  LET hits AS List OF Integer = regex::findAll(\"a\", \"abc\")\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn vector_valid() {
        assert!(accepts(
            "IMPORT vector\nIMPORT io\nFUNC main AS Integer\n  io::print(toString(vector::abs(vector::Integer2[-3, 4])))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn net_valid() {
        assert!(accepts(
            "IMPORT net\nIMPORT io\nFUNC main AS Integer\n  RES server = net::listenTcp(\"127.0.0.1\", 0)\n  net::close(server)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn tls_valid() {
        assert!(accepts(
            "IMPORT tls\nIMPORT io\nFUNC main AS Integer\n  RES c = tls::connect(\"example.com\", 443)\n  tls::close(c)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn fs_valid() {
        assert!(accepts(
            "IMPORT fs\nIMPORT io\nFUNC main AS Integer\n  fs::writeText(\"t.txt\", \"hi\")\n  io::print(fs::readText(\"t.txt\"))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn crypto_valid() {
        assert!(accepts(
            "IMPORT crypto\nFUNC main AS Integer\n  LET kp = crypto::generateP256()\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn json_valid() {
        assert!(accepts(
            "IMPORT io\nIMPORT json\nFUNC main AS Integer\n  io::print(json::stringify(json::parse(\"null\")))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- arity mismatch on each dispatch family ----------------------------

    #[test]
    fn bits_arity_mismatch() {
        assert!(rejects_with(
            "IMPORT io\nIMPORT bits\nFUNC main AS Integer\n  io::print(toString(bits::band(1)))\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn math_argument_mismatch() {
        assert!(rejects_with(
            "IMPORT io\nIMPORT math\nFUNC main AS Integer\n  io::print(toString(math::abs(\"x\")))\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn strings_argument_mismatch() {
        assert!(rejects_with(
            "IMPORT io\nIMPORT strings\nFUNC main AS Integer\n  io::print(strings::toUpper(42))\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn fs_argument_mismatch() {
        assert!(rejects_with(
            "IMPORT fs\nFUNC main AS Integer\n  fs::writeText(42, 7)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn vector_arity_mismatch() {
        assert!(rejects_with(
            "IMPORT vector\nFUNC main AS Integer\n  LET x = vector::abs()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn vector_argument_mismatch() {
        assert!(rejects_with(
            "IMPORT vector\nFUNC main AS Integer\n  LET x = vector::abs(\"nope\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn encoding_argument_mismatch() {
        assert!(rejects_with(
            "IMPORT encoding\nFUNC main AS Integer\n  LET x = encoding::base64Encode(\"str\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- general / collections builtins ------------------------------------

    #[test]
    fn general_len_valid() {
        assert!(accepts(
            "FUNC main AS Integer\n  LET xs AS List OF Integer = [1, 2, 3]\n  LET n = len(xs)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn general_argument_mismatch() {
        assert!(rejects_with(
            "FUNC main AS Integer\n  LET n = toByte(\"nope\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn collections_contains_valid() {
        assert!(accepts(
            "IMPORT collections\nIMPORT io\nFUNC main AS Integer\n  LET xs AS List OF Integer = [1, 2, 3]\n  io::print(toString(collections::contains(xs, 2)))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn collections_filter_builtin_predicate_valid() {
        // Exercises the `filter` + builtin-predicate branch (collections path).
        assert!(accepts(
            "IMPORT collections\nFUNC main AS Integer\n  LET xs AS List OF Integer = [1, 2, 3]\n  LET ys AS List OF Integer = collections::filter(xs, isPositive)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn collections_filter_predicate_type_mismatch() {
        // A builtin predicate that cannot resolve for the element type walks the
        // `predicate_type` None arm.
        let src = "IMPORT collections\nFUNC main AS Integer\n  LET xs AS List OF String = [\"a\"]\n  LET ys AS List OF String = collections::filter(xs, isEven)\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_CALL_ARGUMENT_MISMATCH"));
    }

    #[test]
    fn general_filter_builtin_predicate() {
        // `filter` with a builtin function id as predicate (general path).
        let src = "FUNC main AS Integer\n  LET xs AS List OF Integer = [1, 2, 3]\n  LET ys AS List OF Integer = filter(xs, isPositive)\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn collections_arity_mismatch() {
        assert!(rejects_with(
            "IMPORT collections\nFUNC main AS Integer\n  LET xs AS List OF Integer = [1, 2, 3]\n  LET y = collections::get(xs)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn collections_argument_mismatch() {
        assert!(rejects_with(
            "IMPORT collections\nFUNC main AS Integer\n  LET y = collections::get(42, 0)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- named argument normalization --------------------------------------

    #[test]
    fn named_argument_valid() {
        // A builtin with named args that resolve to its parameter names.
        assert!(accepts(
            "IMPORT json\nIMPORT io\nFUNC main AS Integer\n  io::print(json::stringify(json::parse(value := \"null\")))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn named_argument_unknown_name() {
        assert!(rejects_with(
            "IMPORT json\nFUNC main AS Integer\n  LET x = json::parse(nope := \"null\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_UNKNOWN_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn named_argument_duplicate() {
        assert!(rejects_with(
            "IMPORT json\nFUNC main AS Integer\n  LET x = json::parse(\"a\", value := \"b\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_DUPLICATE_ARGUMENT_NAME"
        ));
    }

    // ---- general comparability (contains/find on non-comparable element) ---

    #[test]
    fn contains_valid_on_comparable() {
        assert!(accepts(
            "IMPORT collections\nFUNC main AS Integer\n  LET xs AS List OF String = [\"a\"]\n  LET b = collections::contains(xs, \"a\")\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn find_valid_on_list() {
        assert!(accepts(
            "IMPORT collections\nFUNC main AS Integer\n  LET xs AS List OF Integer = [1]\n  LET i = collections::find(xs, 1)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- io / term valid + reject ------------------------------------------

    #[test]
    fn io_valid_and_rejects() {
        assert!(accepts(
            "IMPORT io\nFUNC main AS Integer\n  io::print(\"a\")\n  io::printError(\"b\")\n  RETURN 0\nEND FUNC\n"
        ));
        assert!(rejects_with(
            "IMPORT io\nFUNC main AS Integer\n  io::print()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
        assert!(rejects_with(
            "IMPORT io\nFUNC main AS Integer\n  io::print(42)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn term_valid_and_rejects() {
        assert!(accepts(
            "IMPORT term\nFUNC main AS Integer\n  term::moveTo(1, 1)\n  RETURN 0\nEND FUNC\n"
        ));
        assert!(rejects_with(
            "IMPORT term\nFUNC main AS Integer\n  term::moveTo(1)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
        assert!(rejects_with(
            "IMPORT term\nFUNC main AS Integer\n  term::moveTo(\"a\", \"b\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- json / csv / http reject paths ------------------------------------

    #[test]
    fn json_rejects() {
        assert!(rejects_with(
            "IMPORT json\nFUNC main AS Integer\n  LET x = json::parse()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
        assert!(rejects_with(
            "IMPORT json\nFUNC main AS Integer\n  LET x = json::parse(42)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn csv_valid_and_rejects() {
        assert!(accepts(
            "IMPORT csv\nFUNC main AS Integer\n  LET doc AS List OF List OF String = csv::parse(\"a,b\")\n  RETURN 0\nEND FUNC\n"
        ));
        assert!(rejects_with(
            "IMPORT csv\nFUNC main AS Integer\n  LET doc AS List OF List OF String = csv::parse()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
        assert!(rejects_with(
            "IMPORT csv\nFUNC main AS Integer\n  LET doc AS List OF List OF String = csv::parse(42)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- net / tls reject paths --------------------------------------------

    #[test]
    fn net_rejects() {
        assert!(rejects_with(
            "IMPORT net\nFUNC main AS Integer\n  RES s = net::listenTcp(\"127.0.0.1\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
        assert!(rejects_with(
            "IMPORT net\nFUNC main AS Integer\n  RES s = net::listenTcp(1, \"x\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn tls_rejects() {
        assert!(rejects_with(
            "IMPORT tls\nFUNC main AS Integer\n  RES c = tls::connect(\"h\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
        assert!(rejects_with(
            "IMPORT tls\nFUNC main AS Integer\n  RES c = tls::connect(1, \"x\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- regex / datetime reject paths -------------------------------------

    #[test]
    fn regex_rejects() {
        assert!(rejects_with(
            "IMPORT regex\nFUNC main AS Integer\n  LET x AS List OF Integer = regex::findAll()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
        assert!(rejects_with(
            "IMPORT regex\nFUNC main AS Integer\n  LET x AS List OF Integer = regex::findAll(1, 2)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn datetime_rejects() {
        assert!(rejects_with(
            "IMPORT datetime\nFUNC main AS Integer\n  LET i = datetime::instant(\"nope\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- bits / crypto / strings reject paths ------------------------------

    #[test]
    fn bits_argument_mismatch() {
        assert!(rejects_with(
            "IMPORT bits\nFUNC main AS Integer\n  LET x = bits::band(\"a\", \"b\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn crypto_rejects() {
        assert!(rejects_with(
            "IMPORT crypto\nFUNC main AS Integer\n  LET kp = crypto::generateP256(\"extra\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn strings_arity_mismatch() {
        assert!(rejects_with(
            "IMPORT strings\nFUNC main AS Integer\n  LET b = strings::toBytes()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn math_arity_mismatch() {
        assert!(rejects_with(
            "IMPORT math\nFUNC main AS Integer\n  LET x = math::abs()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn encoding_arity_mismatch() {
        assert!(rejects_with(
            "IMPORT encoding\nFUNC main AS Integer\n  LET x = encoding::base64Encode()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    // ---- unknown / non-builtin dotted call falls through -------------------

    #[test]
    fn unknown_dotted_call_infers_unknown() {
        // A dotted call with no matching builtin/user function walks the
        // fall-through arm in check_builtin_call's dispatch tail.
        let _ = check_src(
            "FUNC main AS Integer\n  LET x = mystery::doThing(1, 2)\n  RETURN 0\nEND FUNC\n",
        );
    }

    // ---- thread arity / argument mismatch ----------------------------------

    #[test]
    fn thread_start_bad_entry_rejected() {
        // thread.start whose first arg is not an exported ISOLATED FUNC.
        assert!(rejects_with(
            "IMPORT thread\nFUNC main AS Integer\n  LET t = thread::start(main, \"x\", 1, 1)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- general override target (toString on a package/builtin type) ------

    #[test]
    fn tostring_override_on_net_url() {
        // toString(net::Url) resolves via the package override registry branch.
        assert!(accepts(
            "IMPORT net\nIMPORT io\nFUNC main AS Integer\n  LET u AS net::Url = net::toUrl(\"http://x/\")\n  io::print(toString(u))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- http builtin call -------------------------------------------------

    #[test]
    fn http_read_valid() {
        assert!(accepts(
            "IMPORT http\nIMPORT net\nFUNC main AS Integer\n  LET u AS net::Url = net::toUrl(\"http://x/\")\n  LET r AS http::Response = http::read(u)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn http_read_rejects() {
        assert!(rejects_with(
            "IMPORT http\nFUNC main AS Integer\n  LET r = http::read()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
        assert!(rejects_with(
            "IMPORT http\nFUNC main AS Integer\n  LET r = http::read(42)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- regex builtin call reject interior --------------------------------

    #[test]
    fn regex_match_and_replace_valid() {
        assert!(accepts(
            "IMPORT regex\nIMPORT io\nFUNC main AS Integer\n  LET out AS String = regex::replace(\"a\", \"b\", \"c\")\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- range-arity ({min} to {max}) messages -----------------------------

    #[test]
    fn fs_range_arity_too_few() {
        assert!(rejects_with(
            "IMPORT fs\nFUNC main AS Integer\n  RES f AS File = fs::openFileNoFollow()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn io_range_arity_too_many() {
        assert!(rejects_with(
            "IMPORT io\nFUNC main AS Integer\n  LET x = io::pollInput(1, 2)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn strings_range_arity_too_few() {
        assert!(rejects_with(
            "IMPORT strings\nFUNC main AS Integer\n  LET i = strings::find(\"a\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn net_range_arity_too_few() {
        assert!(rejects_with(
            "IMPORT net\nFUNC main AS Integer\n  LET a = net::lookup()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn datetime_range_arity_too_many() {
        assert!(rejects_with(
            "IMPORT datetime\nFUNC main AS Integer\n  LET i = datetime::instant(1, 2, 3, 4, 5, 6, 7, 8)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn thread_range_arity_via_start() {
        // thread.start arity (2,4): calling with 5 args walks the thread checker
        // (the bad-entry check fires first, but the checker body runs).
        let _ = check_src(
            "IMPORT thread\nFUNC main AS Integer\n  LET t = thread::start(main, \"x\", 1, 1, 1)\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn regex_range_arity_too_many() {
        assert!(rejects_with(
            "IMPORT regex\nFUNC main AS Integer\n  LET x AS List OF Integer = regex::findAll(\"a\", \"b\", 0, 1, 2)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    // ---- unknown builtin-namespace member falls through --------------------

    #[test]
    fn unknown_math_member_falls_through() {
        // `math::nonexistent` is not a known builtin; it walks the dotted-call
        // fall-through in the inference dispatcher.
        let _ = check_src(
            "IMPORT math\nFUNC main AS Integer\n  LET x = math::nonexistent(1)\n  RETURN 0\nEND FUNC\n",
        );
    }

    // ---- collection resource-element check on append -----------------------

    #[test]
    fn append_resource_binding_valid() {
        // Appending a RES binding to a `List OF RES File` stores a borrow (valid).
        let src = "IMPORT collections\nIMPORT fs\nFUNC main AS Integer\n  RES f AS File = fs::openFile(\"x\")\n  MUT xs AS List OF RES File = []\n  xs = collections::append(xs, f)\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- fixed-arity ({min}) arity messages per package --------------------

    #[test]
    fn fs_fixed_arity_message() {
        assert!(rejects_with(
            "IMPORT fs\nFUNC main AS Integer\n  fs::setCurrentDirectory()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn net_fixed_arity_message() {
        assert!(rejects_with(
            "IMPORT net\nFUNC main AS Integer\n  LET x = net::receiveTextFrom()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn tls_fixed_arity_message() {
        assert!(rejects_with(
            "IMPORT tls\nFUNC main AS Integer\n  tls::writeText()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn regex_fixed_arity_message() {
        assert!(rejects_with(
            "IMPORT regex\nFUNC main AS Integer\n  LET b = regex::match()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    // ---- general builtin arity / argument mismatch (check_general_*) -------

    #[test]
    fn general_len_arity_fixed() {
        assert!(rejects_with(
            "FUNC main AS Integer\n  LET n = len()\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn general_tostring_range_arity() {
        // toString has arity (1,2); calling with 3 args hits the range message.
        assert!(rejects_with(
            "FUNC main AS Integer\n  LET s = toString(1, 2, 3)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn general_toint_argument_mismatch() {
        assert!(rejects_with(
            "FUNC main AS Integer\n  LET n = toInt(TRUE)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- thread checker arity / argument mismatch (non-start) --------------

    #[test]
    fn thread_receive_arity_mismatch() {
        // thread.receive on a worker with too many args hits the thread arity arm.
        assert!(rejects_with(
            "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  LET m AS String = thread::receive(t, 1, 2, 3)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn thread_argument_mismatch() {
        // thread.send with a wrong-typed message hits the thread resolve-None arm.
        assert!(rejects_with(
            "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  thread::send(t, 42)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- strings / encoding resolve-None argument mismatch -----------------

    #[test]
    fn strings_startswith_argument_mismatch() {
        assert!(rejects_with(
            "IMPORT strings\nFUNC main AS Integer\n  LET b = strings::startsWith(42, 7)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn strings_startswith_valid() {
        assert!(accepts(
            "IMPORT strings\nFUNC main AS Integer\n  LET b = strings::startsWith(\"abc\", \"a\")\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn encoding_base64_decode_argument_mismatch() {
        assert!(rejects_with(
            "IMPORT encoding\nFUNC main AS Integer\n  LET x AS List OF Byte = encoding::base64Decode(42)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- encoding utf8Encode return-type overload by expected type ---------

    #[test]
    fn encoding_utf8encode_return_overload_by_expected() {
        // An expected `List OF Integer` selects that overload of utf8Encode.
        let src = "IMPORT encoding\nFUNC main AS Integer\n  LET x AS List OF Integer = encoding::utf8Encode(\"hi\")\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn collections_find_range_arity() {
        // collections::find has arity (2,3); calling with 4 hits the range arm.
        assert!(rejects_with(
            "IMPORT collections\nFUNC main AS Integer\n  LET xs AS List OF Integer = [1]\n  LET i = collections::find(xs, 1, 0, 9)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn io_zero_arity_message() {
        // io::isErrorTerminal has arity (0,0); supplying an argument hits the "0"
        // expected-count message.
        assert!(rejects_with(
            "IMPORT io\nFUNC main AS Integer\n  LET b = io::isErrorTerminal(1)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn term_no_arguments_message() {
        // term::clear expects no arguments; supplying one hits the term argument
        // mismatch "no arguments" fallback.
        assert!(rejects_with(
            "IMPORT term\nFUNC main AS Integer\n  term::clear(1)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn term_argument_type_mismatch() {
        // A term call with a wrong-typed argument walks the term argument-type
        // compatibility check + "no arguments" / expected-args formatting.
        assert!(rejects_with(
            "IMPORT term\nFUNC main AS Integer\n  term::moveTo(\"a\", \"b\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn crypto_sign_argument_mismatch() {
        // p256Sign with wrong-typed arguments passes arity but fails resolve.
        assert!(rejects_with(
            "IMPORT crypto\nFUNC main AS Integer\n  LET s = crypto::p256Sign(1, 2)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    // ---- named-argument normalization on builtins --------------------------

    #[test]
    fn builtin_named_argument_without_param_names() {
        // math::abs has no call_param_names; a named call walks the fallback
        // that returns the arguments in source order.
        let _ = check_src(
            "IMPORT math\nFUNC main AS Integer\n  LET x = math::abs(value := -1)\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn builtin_named_then_positional_reorders() {
        // strings::startsWith(prefix := "a", "abc"): a named arg fills a later
        // slot, then a positional fills the earlier one — walks the positional
        // slot-skipping loop.
        assert!(accepts(
            "IMPORT strings\nFUNC main AS Integer\n  LET b = strings::startsWith(prefix := \"a\", \"abc\")\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn builtin_named_with_extra_positional() {
        // A named call supplying more positionals than parameters pushes to the
        // `extras` overflow branch.
        let _ = check_src(
            "IMPORT strings\nFUNC main AS Integer\n  LET b = strings::startsWith(value := \"abc\", \"a\", \"extra\")\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn thread_fixed_arity_message() {
        // thread.waitFor has fixed arity; supplying extra args hits the thread
        // arity min==max message.
        let _ = check_src(
            "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  LET r = thread::isRunning(t, 1)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn datetime_fixed_arity_message() {
        // datetime::date has fixed arity (3,3); a wrong count hits the min==max
        // arity message.
        assert!(rejects_with(
            "IMPORT datetime\nFUNC main AS Integer\n  LET d = datetime::date(1)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    // ---- collections source-generic (falls through builtin dispatch) -------

    #[test]
    fn collections_source_generic_falls_through() {
        // `collections::sort` is a source-generic function (is_builtin_call true
        // but no native-member sub-checker), so it reaches check_builtin_call's
        // fall-through arm that infers each argument and yields Unknown.
        let _ = check_src(
            "IMPORT collections\nFUNC main AS Integer\n  LET xs AS List OF Integer = [3, 1, 2]\n  LET ys = collections::sort(xs)\n  RETURN 0\nEND FUNC\n",
        );
    }
}
