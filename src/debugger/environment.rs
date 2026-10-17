//! The environment `mfb dbg` starts a launched program with, and the `env`
//! commands that change it for the next `restart`.
//!
//! A launched program starts with the debugger's own environment. The `env`
//! commands change what the next `restart` gives it and never touch the running
//! program, whose environment is its C library's, kept in its own memory. A
//! program the debugger attached to was started by someone else and cannot
//! restart, so the session holds no environment for it.

use std::ffi::OsString;

use crate::terminal_safe::safe;

/// A program's environment: each variable's name and value, in the order the
/// program gets them.
pub(crate) type Environment = Vec<(OsString, OsString)>;

/// What the `env` commands do to the environment the next restart gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    /// `env s <name> <value>`: give the variable this value.
    Set { name: String, value: String },
    /// `env r <name>`: take the variable away.
    Remove(String),
    /// `env clear`: take every variable away.
    Clear,
    /// `env reset`: go back to the environment the session started with.
    Reset,
}

/// A launched program's environment, as the session knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Session {
    /// What the session first launched the program with: `env reset`'s.
    started: Environment,
    /// What the running program was launched with.
    running: Environment,
    /// What the next restart gives it.
    next: Environment,
}

impl Session {
    /// The session of a program just launched with `environment`.
    pub(crate) fn new(environment: Environment) -> Self {
        Session {
            started: environment.clone(),
            running: environment.clone(),
            next: environment,
        }
    }

    /// The environment the next restart gives the program.
    pub(crate) fn next(&self) -> &[(OsString, OsString)] {
        &self.next
    }

    /// The program has restarted with [`Session::next`].
    pub(crate) fn restarted(&mut self) {
        self.running = self.next.clone();
    }

    /// Make `change` to the environment the next restart gives; the lines
    /// that say what it did.
    pub(crate) fn change(&mut self, change: Change) -> Result<Vec<String>, String> {
        let line = match change {
            Change::Set { name, value } => {
                let key = OsString::from(&name);
                let mut found = false;
                self.next.retain_mut(|(other, old)| {
                    if *other != key {
                        return true;
                    }
                    // The first keeps its place; any repeat goes.
                    if found {
                        return false;
                    }
                    found = true;
                    *old = OsString::from(&value);
                    true
                });
                if !found {
                    self.next.push((key, OsString::from(&value)));
                }
                format!(
                    "Set {}={}; the program gets it when it restarts.",
                    safe(&name),
                    safe(&value)
                )
            }
            Change::Remove(name) => {
                let key = OsString::from(&name);
                let before = self.next.len();
                self.next.retain(|(other, _)| *other != key);
                if self.next.len() == before {
                    return Err(format!(
                        "no variable '{}' is in the environment; type 'list env' for it",
                        safe(&name)
                    ));
                }
                format!(
                    "Removed {}; the program goes without it when it restarts.",
                    safe(&name)
                )
            }
            Change::Clear => {
                self.next.clear();
                "Cleared the environment; the program starts with none when it restarts."
                    .to_string()
            }
            Change::Reset => {
                self.next = self.started.clone();
                "Reset the environment to the one the session started the program with; the \
                 program gets it when it restarts."
                    .to_string()
            }
        };
        Ok(vec![line])
    }

    /// The `list env` lines: the environment the next restart gives, as
    /// `<name> | <value>` in name order under a header, with a note when it
    /// is not the one the running program has.
    pub(crate) fn lines(&self) -> Vec<String> {
        let mut variables = self.next.iter().collect::<Vec<_>>();
        variables.sort_by(|one, other| one.0.cmp(&other.0));
        let rows = variables
            .into_iter()
            .map(|(name, value)| {
                vec![
                    safe(&name.to_string_lossy()).into_owned(),
                    safe(&value.to_string_lossy()).into_owned(),
                ]
            })
            .collect::<Vec<_>>();
        let mut lines = super::prompt::table(&["Name", "Value"], rows);
        if lines.is_empty() {
            lines.push("no environment variables".to_string());
        }
        if self.next != self.running {
            lines.push(String::new());
            lines.push(
                "Changed since the program started: it gets these when it restarts.".to_string(),
            );
        }
        lines
    }
}

/// Refuse a name no environment variable can have: an empty one, or one
/// holding `=` or a NUL.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('=') || name.contains('\0') {
        return Err(format!(
            "'{}' is not a variable name: a name holds no '=' and no NUL",
            safe(name)
        ));
    }
    Ok(())
}

/// Refuse a value no environment variable can hold: one with a NUL.
pub(crate) fn check_value(value: &str) -> Result<(), String> {
    if value.contains('\0') {
        return Err("a variable's value cannot hold a NUL".to_string());
    }
    Ok(())
}

/// The environment as a program's `envp` holds it: one `<name>=<value>` C
/// string per variable, in order.
#[cfg(any(
    target_os = "macos",
    all(target_os = "linux", target_arch = "aarch64"),
    test
))]
pub(crate) fn entries(
    environment: &[(OsString, OsString)],
) -> Result<Vec<std::ffi::CString>, String> {
    environment
        .iter()
        .map(|(name, value)| {
            let mut entry = name.as_encoded_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_encoded_bytes());
            std::ffi::CString::new(entry).map_err(|_| {
                format!(
                    "the environment variable '{}' holds a NUL byte",
                    safe(&name.to_string_lossy())
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(pairs: &[(&str, &str)]) -> Environment {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }

    fn set(name: &str, value: &str) -> Change {
        Change::Set {
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn a_new_session_lists_what_the_program_was_launched_with_in_name_order() {
        let session = Session::new(environment(&[("PATH", "/bin"), ("HOME", "/home/me")]));
        assert_eq!(
            session.lines(),
            ["Name | Value", "HOME | /home/me", "PATH | /bin"]
        );
        assert_eq!(
            session.next(),
            environment(&[("PATH", "/bin"), ("HOME", "/home/me")])
        );
    }

    #[test]
    fn set_replaces_a_variable_in_its_place_or_adds_it_last() {
        let mut session = Session::new(environment(&[("A", "1"), ("B", "2"), ("A", "3")]));
        assert_eq!(
            session.change(set("A", "x y")).unwrap(),
            ["Set A=x y; the program gets it when it restarts."]
        );
        assert_eq!(session.change(set("C", "")).unwrap().len(), 1);
        assert_eq!(
            session.next(),
            environment(&[("A", "x y"), ("B", "2"), ("C", "")])
        );
        assert_eq!(
            session.lines(),
            [
                "Name | Value",
                "A    | x y",
                "B    | 2",
                "C    | ",
                "",
                "Changed since the program started: it gets these when it restarts.",
            ]
        );
        // Once restarted, the program has them: no note.
        session.restarted();
        assert_eq!(session.lines().last().unwrap(), "C    | ");
    }

    #[test]
    fn remove_takes_every_copy_and_refuses_a_missing_name() {
        let mut session = Session::new(environment(&[("A", "1"), ("B", "2"), ("A", "3")]));
        assert_eq!(
            session.change(Change::Remove("A".to_string())).unwrap(),
            ["Removed A; the program goes without it when it restarts."]
        );
        assert_eq!(session.next(), environment(&[("B", "2")]));
        assert_eq!(
            session.change(Change::Remove("A".to_string())),
            Err("no variable 'A' is in the environment; type 'list env' for it".to_string())
        );
    }

    #[test]
    fn clear_empties_it_and_reset_brings_back_the_first() {
        let mut session = Session::new(environment(&[("A", "1")]));
        session.change(set("B", "2")).unwrap();
        session.restarted();
        assert_eq!(
            session.change(Change::Clear).unwrap(),
            ["Cleared the environment; the program starts with none when it restarts."]
        );
        assert_eq!(
            session.lines(),
            [
                "no environment variables",
                "",
                "Changed since the program started: it gets these when it restarts.",
            ]
        );
        assert_eq!(
            session.change(Change::Reset).unwrap(),
            [
                "Reset the environment to the one the session started the program with; the \
                 program gets it when it restarts."
            ]
        );
        // Back to the first, which is not what the program restarted with.
        assert_eq!(session.next(), environment(&[("A", "1")]));
        assert_eq!(session.lines().len(), 4);
    }

    #[test]
    fn a_name_holds_no_equals_and_nothing_holds_a_nul() {
        assert_eq!(check_name("PATH"), Ok(()));
        assert_eq!(
            check_name("A=B"),
            Err("'A=B' is not a variable name: a name holds no '=' and no NUL".to_string())
        );
        assert!(check_name("A\0").is_err());
        assert_eq!(check_value("a = b"), Ok(()));
        assert_eq!(
            check_value("a\0"),
            Err("a variable's value cannot hold a NUL".to_string())
        );
    }

    #[test]
    fn entries_are_name_equals_value_in_order() {
        let made = entries(&environment(&[("B", "2"), ("A", "x=y")])).unwrap();
        assert_eq!(
            made.iter()
                .map(|entry| entry.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["B=2", "A=x=y"]
        );
        let bad = vec![(OsString::from("A"), OsString::from("a\0b"))];
        assert_eq!(
            entries(&bad),
            Err("the environment variable 'A' holds a NUL byte".to_string())
        );
    }
}
