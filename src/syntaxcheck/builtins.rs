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
        // Legacy pre-migration paths. `check_general_builtin_call` is only entered
        // for bare general callees (`len`, `toString`, `isOdd`, …); `filter` and
        // the `collections.*` update members are no longer general calls
        // (`is_general_call` excludes them), so the `filter` block and the
        // `collections.*` resource-element block below are unreachable here. Their
        // live equivalents run in `check_collections_builtin_call`.
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

#[cfg(test)]
mod tests {
    use crate::testutil::*;

    /// Wrap a builtin-call expression inside a runnable `main` with the given
    /// import lines, binding the result so the call is actually type-checked.
    fn prog(imports: &[&str], body: &str) -> String {
        let mut src = String::new();
        for import in imports {
            src.push_str("IMPORT ");
            src.push_str(import);
            src.push('\n');
        }
        src.push_str("FUNC main AS Integer\n");
        src.push_str(body);
        src.push_str("\n  RETURN 0\nEND FUNC\n");
        src
    }

    const ARITY: &str = "TYPE_CALL_ARITY_MISMATCH";
    const ARGTYPE: &str = "TYPE_CALL_ARGUMENT_MISMATCH";
    const DUP_NAME: &str = "TYPE_DUPLICATE_ARGUMENT_NAME";
    const UNKNOWN_NAME: &str = "TYPE_UNKNOWN_ARGUMENT_NAME";

    // ---- vector ---------------------------------------------------------

    #[test]
    fn vector_valid() {
        assert!(accepts(&prog(
            &["vector"],
            "  LET d AS Float = vector::dot(vector::Float2[3.0, 4.0], vector::Float2[1.0, 2.0])",
        )));
    }

    #[test]
    fn vector_wrong_arity() {
        assert!(rejects_with(
            &prog(
                &["vector"],
                "  LET d AS Float = vector::dot(vector::Float2[3.0, 4.0])",
            ),
            ARITY,
        ));
    }

    #[test]
    fn vector_wrong_argtype() {
        assert!(rejects_with(
            &prog(
                &["vector"],
                "  LET d AS Float = vector::dot(vector::Float2[3.0, 4.0], vector::Float3[1.0, 2.0, 3.0])",
            ),
            ARGTYPE,
        ));
    }

    // ---- fs --------------------------------------------------------------

    #[test]
    fn fs_valid() {
        assert!(accepts(&prog(
            &["fs"],
            "  LET ok AS Boolean = fs::exists(\"path\")",
        )));
    }

    #[test]
    fn fs_wrong_arity() {
        assert!(rejects_with(
            &prog(&["fs"], "  LET ok AS Boolean = fs::exists()"),
            ARITY,
        ));
    }

    #[test]
    fn fs_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["fs"], "  LET ok AS Boolean = fs::exists(1)"),
            ARGTYPE,
        ));
    }

    #[test]
    fn fs_close_transfers_handle() {
        // Exercises the fs.close Transfer-mode branch on argument 0.
        assert!(accepts(&prog(
            &["fs"],
            "  RES f AS File = fs::openFile(\"path\")\n  fs::close(f)",
        )));
    }

    // ---- net -------------------------------------------------------------

    #[test]
    fn net_valid() {
        assert!(accepts(&prog(
            &["net"],
            "  RES s AS UdpSocket = net::bindUdp(\"127.0.0.1\", 0)",
        )));
    }

    #[test]
    fn net_wrong_arity() {
        assert!(rejects_with(
            &prog(&["net"], "  RES s AS UdpSocket = net::bindUdp()"),
            ARITY,
        ));
    }

    #[test]
    fn net_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["net"], "  RES s AS UdpSocket = net::bindUdp(1, 2)"),
            ARGTYPE,
        ));
    }

    // ---- tls -------------------------------------------------------------

    #[test]
    fn tls_wrong_arity() {
        assert!(rejects_with(&prog(&["tls"], "  tls::writeText()"), ARITY));
    }

    #[test]
    fn tls_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["tls"], "  tls::writeText(1, 2)"),
            ARGTYPE,
        ));
    }

    // ---- json ------------------------------------------------------------

    #[test]
    fn json_valid() {
        assert!(accepts(&prog(
            &["json"],
            "  LET j AS Json = json::parse(\"{}\")",
        )));
    }

    #[test]
    fn json_wrong_arity() {
        assert!(rejects_with(
            &prog(&["json"], "  LET j AS Json = json::parse()"),
            ARITY,
        ));
    }

    #[test]
    fn json_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["json"], "  LET j AS Json = json::parse(TRUE)"),
            ARGTYPE,
        ));
    }

    // ---- csv -------------------------------------------------------------

    #[test]
    fn csv_valid() {
        assert!(accepts(&prog(
            &["csv"],
            "  LET rows AS List OF List OF String = csv::parse(\"a,b\")",
        )));
    }

    #[test]
    fn csv_wrong_arity() {
        assert!(rejects_with(
            &prog(
                &["csv"],
                "  LET rows AS List OF List OF String = csv::parse()",
            ),
            ARITY,
        ));
    }

    #[test]
    fn csv_wrong_argtype() {
        assert!(rejects_with(
            &prog(
                &["csv"],
                "  LET rows AS List OF List OF String = csv::parse(TRUE)",
            ),
            ARGTYPE,
        ));
    }

    // ---- http ------------------------------------------------------------

    #[test]
    fn http_valid() {
        assert!(accepts(&prog(
            &["http", "net"],
            "  LET u AS Url = net::toUrl(\"http://x/\")\n  LET r AS http::Response = http::read(u)",
        )));
    }

    #[test]
    fn http_wrong_arity() {
        assert!(rejects_with(
            &prog(&["http"], "  LET r AS http::Response = http::read()"),
            ARITY,
        ));
    }

    #[test]
    fn http_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["http"], "  LET r AS http::Response = http::read(123)"),
            ARGTYPE,
        ));
    }

    // ---- regex -----------------------------------------------------------

    #[test]
    fn regex_valid() {
        assert!(accepts(&prog(
            &["regex"],
            "  LET m AS Integer = regex::find(\"hello\", \"l\")",
        )));
    }

    #[test]
    fn regex_wrong_arity() {
        assert!(rejects_with(
            &prog(&["regex"], "  LET m AS Integer = regex::find(\"a\")"),
            ARITY,
        ));
    }

    #[test]
    fn regex_wrong_argtype() {
        assert!(rejects_with(
            &prog(
                &["regex"],
                "  LET m AS Integer = regex::find(\"a\", \"b\", \"c\")",
            ),
            ARGTYPE,
        ));
    }

    // ---- datetime --------------------------------------------------------

    #[test]
    fn datetime_valid() {
        assert!(accepts(&prog(
            &["datetime"],
            "  LET i AS Instant = datetime::instant(100)",
        )));
    }

    #[test]
    fn datetime_wrong_arity() {
        assert!(rejects_with(
            &prog(&["datetime"], "  LET i AS Instant = datetime::instant()"),
            ARITY,
        ));
    }

    #[test]
    fn datetime_wrong_argtype() {
        assert!(rejects_with(
            &prog(
                &["datetime"],
                "  LET i AS Instant = datetime::instant(\"100\")",
            ),
            ARGTYPE,
        ));
    }

    // ---- io --------------------------------------------------------------

    #[test]
    fn io_valid() {
        assert!(accepts(&prog(&["io"], "  io::print(\"hello\")")));
    }

    #[test]
    fn io_wrong_arity() {
        assert!(rejects_with(&prog(&["io"], "  io::print()"), ARITY));
    }

    #[test]
    fn io_wrong_argtype() {
        assert!(rejects_with(&prog(&["io"], "  io::print(1)"), ARGTYPE));
    }

    // ---- term ------------------------------------------------------------

    #[test]
    fn term_valid_zero_args() {
        assert!(accepts(&prog(&["term"], "  term::clear()")));
    }

    #[test]
    fn term_valid_typed_arg() {
        assert!(accepts(&prog(&["term"], "  term::setUnderline(FALSE)")));
    }

    #[test]
    fn term_wrong_arity() {
        assert!(rejects_with(&prog(&["term"], "  term::clear(1)"), ARITY));
    }

    #[test]
    fn term_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["term"], "  term::setUnderline(\"x\")"),
            ARGTYPE,
        ));
    }

    // ---- thread ----------------------------------------------------------

    #[test]
    fn thread_start_bad_entry() {
        // A non-identifier entry point fails the `valid_entry` check and is
        // rejected with the dedicated entry-point message. (A valid thread.start
        // requires an imported ISOLATED-FUNC package, which the single-file test
        // harness cannot provide.)
        assert!(rejects_with(
            &prog(
                &["thread"],
                "  LET t AS Thread OF String TO Integer = thread::start(42, \"a\")",
            ),
            ARGTYPE,
        ));
    }

    #[test]
    fn thread_wrong_arity() {
        // thread.isRunning has arity (1, 1); zero args is an arity mismatch.
        assert!(rejects_with(
            &prog(&["thread"], "  LET r AS Boolean = thread::isRunning()",),
            ARITY,
        ));
    }

    #[test]
    fn thread_wrong_argtype() {
        // An Integer is not a Thread, so resolve_call rejects the argument type.
        assert!(rejects_with(
            &prog(&["thread"], "  LET r AS Boolean = thread::isRunning(42)",),
            ARGTYPE,
        ));
    }

    // ---- strings ---------------------------------------------------------

    #[test]
    fn strings_valid() {
        assert!(accepts(&prog(
            &["strings"],
            "  LET s AS String = strings::trim(\"  hi  \")",
        )));
    }

    #[test]
    fn strings_wrong_arity() {
        assert!(rejects_with(
            &prog(&["strings"], "  LET s AS String = strings::trim()"),
            ARITY,
        ));
    }

    #[test]
    fn strings_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["strings"], "  LET s AS String = strings::trim(42)"),
            ARGTYPE,
        ));
    }

    // ---- math ------------------------------------------------------------

    #[test]
    fn math_valid() {
        assert!(accepts(&prog(
            &["math"],
            "  LET n AS Integer = math::abs(-7)",
        )));
    }

    #[test]
    fn math_wrong_arity() {
        assert!(rejects_with(
            &prog(&["math"], "  LET n AS Integer = math::floor()"),
            ARITY,
        ));
    }

    #[test]
    fn math_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["math"], "  LET n AS Integer = math::floor(\"x\")"),
            ARGTYPE,
        ));
    }

    // ---- bits ------------------------------------------------------------

    #[test]
    fn bits_valid() {
        assert!(accepts(&prog(
            &["bits"],
            "  LET n AS Integer = bits::band(65280, 4080)",
        )));
    }

    #[test]
    fn bits_wrong_arity() {
        assert!(rejects_with(
            &prog(&["bits"], "  LET n AS Integer = bits::band(1)"),
            ARITY,
        ));
    }

    #[test]
    fn bits_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["bits"], "  LET n AS Integer = bits::band(1.5, 2)"),
            ARGTYPE,
        ));
    }

    // ---- crypto ----------------------------------------------------------

    #[test]
    fn crypto_valid() {
        assert!(accepts(&prog(
            &["crypto"],
            "  LET h AS List OF Byte = crypto::sha256(\"data\")",
        )));
    }

    #[test]
    fn crypto_wrong_arity() {
        // crypto.uuid4 takes 0 args; passing one is an arity mismatch.
        assert!(rejects_with(
            &prog(&["crypto"], "  LET s AS String = crypto::uuid4(42)"),
            ARITY,
        ));
    }

    #[test]
    fn crypto_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["crypto"], "  LET h AS List OF Byte = crypto::sha256(42)"),
            ARGTYPE,
        ));
    }

    // ---- encoding --------------------------------------------------------

    #[test]
    fn encoding_valid() {
        assert!(accepts(&prog(
            &["encoding"],
            "  LET h AS String = encoding::hexEncode(encoding::utf8Encode(\"hi\"))",
        )));
    }

    #[test]
    fn encoding_wrong_arity() {
        assert!(rejects_with(
            &prog(&["encoding"], "  LET h AS String = encoding::hexEncode()"),
            ARITY,
        ));
    }

    #[test]
    fn encoding_wrong_argtype() {
        assert!(rejects_with(
            &prog(&["encoding"], "  LET h AS String = encoding::hexEncode(42)"),
            ARGTYPE,
        ));
    }

    #[test]
    fn encoding_utf8encode_adopts_expected_bytes() {
        // Expected type `List OF Byte` is adopted by the return-type overload.
        assert!(accepts(&prog(
            &["encoding"],
            "  LET b AS List OF Byte = encoding::utf8Encode(\"hi\")",
        )));
    }

    #[test]
    fn encoding_utf8encode_adopts_expected_ints() {
        // Expected type `List OF Integer` is adopted by the return-type overload.
        assert!(accepts(&prog(
            &["encoding"],
            "  LET b AS List OF Integer = encoding::utf8Encode(\"hi\")",
        )));
    }

    // ---- general ---------------------------------------------------------

    #[test]
    fn general_valid() {
        assert!(accepts(&prog(&[], "  LET n AS Integer = len(\"hello\")")));
    }

    #[test]
    fn general_wrong_arity() {
        assert!(rejects_with(
            &prog(&[], "  LET n AS Integer = len()"),
            ARITY
        ));
    }

    #[test]
    fn general_wrong_argtype() {
        assert!(rejects_with(
            &prog(&[], "  LET n AS Integer = len(42)"),
            ARGTYPE,
        ));
    }

    #[test]
    fn general_to_string_valid() {
        assert!(accepts(&prog(&[], "  LET s AS String = toString(42)")));
    }

    #[test]
    fn general_to_string_package_override() {
        // `toString(net::Url)` is not a built-in overload; it resolves through the
        // package-provided override registry (the `is_overridable` + override-target
        // path), yielding the built-in's conventional String result.
        assert!(accepts(&prog(
            &["net"],
            "  LET u AS Url = net::toUrl(\"http://x/\")\n  LET s AS String = toString(u)",
        )));
    }

    // ---- collections (native member calls) -------------------------------

    #[test]
    fn collections_valid() {
        assert!(accepts(&prog(
            &["collections"],
            "  LET xs AS List OF Integer = [1, 2, 3]\n  LET x AS Integer = collections::get(xs, 0)",
        )));
    }

    #[test]
    fn collections_wrong_arity() {
        assert!(rejects_with(
            &prog(
                &["collections"],
                "  LET xs AS List OF Integer = [1, 2, 3]\n  LET x AS Integer = collections::get(xs)",
            ),
            ARITY,
        ));
    }

    #[test]
    fn collections_wrong_argtype() {
        assert!(rejects_with(
            &prog(
                &["collections"],
                "  LET xs AS List OF Integer = [1, 2, 3]\n  LET x AS Integer = collections::get(xs, \"k\")",
            ),
            ARGTYPE,
        ));
    }

    #[test]
    fn collections_filter_builtin_predicate_valid() {
        assert!(accepts(&prog(
            &["collections"],
            "  LET xs AS List OF Integer = [1, 2, 3]\n  LET odds AS List OF Integer = collections::filter(xs, isOdd)",
        )));
    }

    #[test]
    fn collections_filter_builtin_predicate_type_mismatch() {
        assert!(rejects_with(
            &prog(
                &["collections"],
                "  LET xs AS List OF String = [\"a\"]\n  LET r AS List OF String = collections::filter(xs, isOdd)",
            ),
            ARGTYPE,
        ));
    }

    #[test]
    fn collections_append_valid() {
        // Exercises the append/prepend/insert/set resource-element check path.
        assert!(accepts(&prog(
            &["collections"],
            "  MUT xs AS List OF Integer = [1, 2]\n  collections::append(xs, 3)",
        )));
    }

    #[test]
    fn collections_contains_comparable_valid() {
        // Exercises check_general_builtin_comparability via the `contains` member.
        assert!(accepts(&prog(
            &["collections"],
            "  LET xs AS List OF Integer = [1, 2, 3]\n  LET yes AS Boolean = collections::contains(xs, 2)",
        )));
    }

    #[test]
    fn collections_find_comparable_valid() {
        assert!(accepts(&prog(
            &["collections"],
            "  LET xs AS List OF Integer = [1, 2, 3]\n  LET i AS Integer = collections::find(xs, 2)",
        )));
    }

    #[test]
    fn collections_replace_comparable_valid() {
        assert!(accepts(&prog(
            &["collections"],
            "  MUT xs AS List OF Integer = [1, 2, 3]\n  collections::replace(xs, 2, 9)",
        )));
    }

    // ---- named-argument normalization -----------------------------------

    #[test]
    fn named_args_valid() {
        // strings.split has named params value / delimiter.
        assert!(accepts(&prog(
            &["strings"],
            "  LET parts AS List OF String = strings::split(value := \"a,b\", delimiter := \",\")",
        )));
    }

    #[test]
    fn named_args_duplicate_name() {
        assert!(rejects_with(
            &prog(
                &["strings"],
                "  LET parts AS List OF String = strings::split(value := \"a\", value := \"b\")",
            ),
            DUP_NAME,
        ));
    }

    #[test]
    fn named_args_unknown_name() {
        assert!(rejects_with(
            &prog(
                &["strings"],
                "  LET parts AS List OF String = strings::split(value := \"a\", nope := \",\")",
            ),
            UNKNOWN_NAME,
        ));
    }

    #[test]
    fn named_args_mixed_positional_and_named() {
        // First positional fills `value`; named `delimiter` fills the rest.
        assert!(accepts(&prog(
            &["strings"],
            "  LET parts AS List OF String = strings::split(\"a,b\", delimiter := \",\")",
        )));
    }

    #[test]
    fn named_args_omit_before_later_supplied() {
        // Omitting an earlier parameter while supplying a later named one hits
        // the "omits parameter before a later supplied argument" arity branch.
        assert!(rejects_with(
            &prog(
                &["strings"],
                "  LET s AS String = strings::padLeft(value := \"a\", padChar := \"x\")",
            ),
            ARITY,
        ));
    }

    #[test]
    fn named_args_general_to_string_duplicate() {
        assert!(rejects_with(
            &prog(
                &[],
                "  LET s AS String = toString(value := 42, value := 43)"
            ),
            DUP_NAME,
        ));
    }

    #[test]
    fn named_args_unknown_on_general() {
        assert!(rejects_with(
            &prog(&[], "  LET s AS String = toString(bogus := 42)"),
            UNKNOWN_NAME,
        ));
    }

    #[test]
    fn named_args_no_param_names_fallback() {
        // A named argument on a callee without registered param names falls
        // through to positional passthrough (no crash, no duplicate-name error).
        let codes = check_src(&prog(&["io"], "  io::print(msg := \"hi\")"));
        assert!(!codes.contains(&DUP_NAME.to_string()));
    }

    // ---- normalize_named_arguments (user FUNC/SUB call path) --------------

    /// Build a program that declares a user `greet(name, greeting = "Hello")`
    /// FUNC and calls it in `main` with the given argument list text.
    fn greet_call(call: &str) -> String {
        format!(
            "FUNC greet(name AS String, greeting AS String = \"Hello\") AS String\n  RETURN greeting & \", \" & name\nEND FUNC\nFUNC main AS Integer\n  LET s AS String = {call}\n  RETURN len(s)\nEND FUNC\n"
        )
    }

    #[test]
    fn user_named_args_valid() {
        assert!(accepts(&greet_call(
            "greet(greeting := \"Hi\", name := \"Ada\")"
        )));
    }

    #[test]
    fn user_named_args_mixed_positional() {
        assert!(accepts(&greet_call(
            "greet(\"Grace\", greeting := \"Welcome\")"
        )));
    }

    #[test]
    fn user_named_args_all_defaults_omitted() {
        // Only the required parameter supplied; the defaulted one is omitted.
        assert!(accepts(&greet_call("greet(\"Ada\")")));
    }

    #[test]
    fn user_named_args_duplicate() {
        assert!(rejects_with(
            &greet_call("greet(name := \"Ada\", name := \"Grace\")"),
            DUP_NAME,
        ));
    }

    #[test]
    fn user_named_args_unknown() {
        // Unknown name plus a missing required parameter: exercises the
        // unknown-name report and the missing-required arity branch.
        let codes = check_src(&greet_call("greet(person := \"Ada\")"));
        assert!(codes.iter().any(|c| c == UNKNOWN_NAME));
        assert!(codes.iter().any(|c| c == ARITY));
    }

    #[test]
    fn user_named_args_too_many_positional() {
        // More positional arguments than parameters trips the positional-overflow
        // arity path (next_positional >= ordered.len()).
        assert!(rejects_with(
            &greet_call("greet(\"Ada\", \"Hi\", \"extra\")"),
            ARITY,
        ));
    }

    #[test]
    fn user_named_args_missing_required() {
        // Supplying only the defaulted parameter leaves the required one unset.
        assert!(rejects_with(
            &greet_call("greet(greeting := \"Hi\")"),
            ARITY,
        ));
    }

    #[test]
    fn user_named_args_named_then_positional() {
        // A named arg fills slot 0, then a positional must skip that filled slot
        // (the `while ordered[next_positional].is_some()` advance).
        assert!(accepts(&greet_call("greet(name := \"Ada\", \"Hi\")")));
    }

    // ---- range-form arity messages (`expected M to N`) -------------------
    //
    // Each module's arity report formats the expectation as `min to max` when
    // `min != max`. The per-module valid/wrong-arity tests above already exercise
    // the `min == max` branch; these hit the range branch on a variadic builtin.

    #[test]
    fn regex_arity_range_message() {
        // regex.find has arity (2, 3); zero args reports "expected 2 to 3".
        assert!(rejects_with(
            &prog(&["regex"], "  LET m AS Integer = regex::find()"),
            ARITY,
        ));
    }

    #[test]
    fn datetime_arity_range_message() {
        // datetime.instant has arity (1, 5).
        assert!(rejects_with(
            &prog(
                &["datetime"],
                "  LET i AS Instant = datetime::instant(1, 2, 3, 4, 5, 6)",
            ),
            ARITY,
        ));
    }

    #[test]
    fn http_arity_range_message() {
        // http.read has arity (1, 3).
        assert!(rejects_with(
            &prog(&["http"], "  LET r AS http::Response = http::read()"),
            ARITY,
        ));
    }

    #[test]
    fn net_arity_range_message() {
        // net.lookup has arity (1, 2); passing three args reports "expected 1 to 2".
        assert!(rejects_with(
            &prog(
                &["net"],
                "  LET a AS List OF Address = net::lookup(\"h\", 1, 2)",
            ),
            ARITY,
        ));
    }

    #[test]
    fn thread_arity_range_message() {
        // thread.send has arity (2, 3); too many args reports "expected 2 to 3".
        assert!(rejects_with(
            &prog(
                &["thread"],
                "  LET ok AS Boolean = thread::send(1, 2, 3, 4)",
            ),
            ARITY,
        ));
    }

    #[test]
    fn collections_arity_range_message() {
        // collections.find has arity (2, 3); a single arg reports "expected 2 to 3".
        assert!(rejects_with(
            &prog(
                &["collections"],
                "  LET xs AS List OF Integer = [1, 2]\n  LET i AS Integer = collections::find(xs)",
            ),
            ARITY,
        ));
    }

    #[test]
    fn fs_arity_range_message() {
        // fs.openFile has arity (1, 2); zero args reports "expected 1 to 2".
        assert!(rejects_with(
            &prog(&["fs"], "  RES f AS File = fs::openFile()"),
            ARITY,
        ));
    }

    #[test]
    fn io_arity_range_message() {
        // io.input has arity (0, 1); two args reports "expected 0 to 1".
        assert!(rejects_with(
            &prog(&["io"], "  LET s AS String = io::input(\"a\", \"b\")"),
            ARITY,
        ));
    }

    #[test]
    fn strings_arity_range_message() {
        // strings.padLeft has arity (2, 3); one arg reports "expected 2 to 3".
        assert!(rejects_with(
            &prog(&["strings"], "  LET s AS String = strings::padLeft(\"a\")"),
            ARITY,
        ));
    }

    #[test]
    fn tls_arity_range_message() {
        // tls.connect has arity (2, 4); zero args reports "expected 2 to 4".
        assert!(rejects_with(&prog(&["tls"], "  tls::connect()"), ARITY));
    }

    // ---- named-arg normalization corners --------------------------------

    #[test]
    fn named_args_positional_skips_filled_slot() {
        // A named arg fills slot 0 of `strings.split`, then a positional must skip
        // that filled slot (the `while ordered[next_positional].is_some()` advance
        // inside normalize_builtin_call_arguments).
        assert!(accepts(&prog(
            &["strings"],
            "  LET parts AS List OF String = strings::split(value := \"a,b\", \",\")",
        )));
    }

    #[test]
    fn named_args_extra_positional_overflow() {
        // A named arg plus more positionals than parameters pushes the surplus
        // onto the `extras` path in normalize_builtin_call_arguments.
        let codes = check_src(&prog(
            &["strings"],
            "  LET parts AS List OF String = strings::split(value := \"a\", \",\", \"x\")",
        ));
        // Surplus arguments still surface as an arity mismatch downstream; the
        // point is that normalization does not panic on the extras branch.
        assert!(codes.iter().any(|c| c == ARITY));
    }
}
