use super::*;
use super::helpers::*;

impl<'a> TypeChecker<'a> {
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
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
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
        if callee == "filter" && arguments.len() == 2 {
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
                                builtins::general::expected_arguments(callee)
                                    .unwrap_or("supported overload")
                            ),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    };

                    let arg_types = vec![collection_type_name, predicate_type];
                    let Some(resolved) = builtins::general::resolve_call(callee, &arg_types) else {
                        self.report(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            &format!(
                                "Call to `{display_callee}` has argument type(s) ({}), expected {}.",
                                arg_types.join(", "),
                                builtins::general::expected_arguments(callee)
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

        // A resource added to a collection through an update builtin must be a
        // `RES` binding (the owner); its slot holds a borrow (§15.6). The op
        // arrives qualified as `collections.append` after the §5 migration.
        if matches!(
            crate::builtins::collections::native_member_bare(callee),
            Some("append" | "prepend" | "insert" | "set")
        ) {
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
