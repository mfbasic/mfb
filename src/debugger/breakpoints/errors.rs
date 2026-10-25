//! plan-176-L: `breakerror` — watching for a raised error code and running to its raiser.
//! The `Breakpoints` table lives in `debugger/breakpoints/mod.rs`.

use super::*;

/// The runtime function every newly raised error goes through: it parks the
/// `Error` block (code in `x1`, origin `ErrorLoc` in `x3`) and returns to the
/// function that raised it.
pub(crate) const PARK_ERROR: &str = "_mfb_rt_park_error";

/// Which errors `breakerror` stops on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ErrorCodes {
    Any,
    /// These codes, in the order they were asked for.
    Codes(Vec<i64>),
}

/// `breakerror`'s watch: the errors, and the live address of [`PARK_ERROR`],
/// where the debugger keeps a breakpoint of its own.
#[derive(Debug)]
pub(super) struct ErrorWatch {
    codes: ErrorCodes,
    pub(super) park: u64,
    /// The commands a stop for a watched error runs.
    pub(super) commands: Vec<String>,
}

/// A raise `breakerror` stops for, caught as it entered the park.
pub(super) struct Raise {
    thread: usize,
    code: i64,
    origin: Option<String>,
    /// Where the park returns to: in the function that raised the error.
    back: u64,
}

/// The hits of `hits` that stopped for none of the user's breakpoints.
pub(super) fn unseen(hits: &[(usize, u64)], user: &[(usize, u64)]) -> Vec<(usize, u64)> {
    hits.iter()
        .copied()
        .filter(|hit| !user.contains(hit))
        .collect()
}

/// The live address of [`PARK_ERROR`].
pub(super) fn park_address(inferior: &mut dyn Inferior) -> Result<u64, String> {
    inferior
        .functions()?
        .iter()
        .find(|function| function.name == PARK_ERROR && !function.linked)
        .and_then(|function| function.address)
        .ok_or_else(|| {
            "'breakerror' stops in the runtime's error function, which only a debug build \
             names; rebuild the program with: mfb build --debug -O0 <project>"
                .to_string()
        })
}

/// An error code as a sentence names it: `77050010 (errorCode::ErrOverflow)`.
pub(crate) fn code_shown(code: i64) -> String {
    match super::runtime::error_code_name(code) {
        Some(name) => format!("{code} ({name})"),
        None => code.to_string(),
    }
}

impl ErrorCodes {
    fn shown(&self) -> String {
        match self {
            ErrorCodes::Any => "any error".to_string(),
            ErrorCodes::Codes(codes) => codes
                .iter()
                .map(|code| code_shown(*code))
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    fn matches(&self, code: i64) -> bool {
        match self {
            ErrorCodes::Any => true,
            ErrorCodes::Codes(codes) => codes.contains(&code),
        }
    }
}

impl Breakpoints {
    /// Thread `thread` has just entered the park: the error it raises when the
    /// watch takes its code (its argument 1, `x1` on AArch64), with where it
    /// returns (the link register) and the error's origin (the `ErrorLoc`, its
    /// argument 3).
    pub(super) fn watched_raise(
        &self,
        inferior: &mut dyn Inferior,
        thread: usize,
    ) -> Result<Option<Raise>, String> {
        let registers = inferior.registers(thread)?;
        let value = |name: &str| {
            registers
                .iter()
                .find(|register| register.name == name)
                .and_then(|register| match register.value {
                    super::RegisterValue::Bits64(value) => Some(value),
                    super::RegisterValue::Bits128(_) => None,
                })
                .ok_or_else(|| format!("thread {thread} has no register {name}"))
        };
        let arch = inferior.arch();
        let code = value(arch.arguments[1])? as i64;
        let watch = self.errors.as_ref().expect("a watch holds the park");
        if !watch.codes.matches(code) {
            return Ok(None);
        }
        let origin = match value(arch.arguments[3])? {
            0 => None,
            location => super::runtime::error_origin(inferior, location).ok(),
        };
        Ok(Some(Raise {
            thread,
            code,
            origin,
            back: value(arch.link)?,
        }))
    }

    /// Run until the thread that raised `raise` is back in the function that
    /// raised it, the error parked: the `Raised` stop. Another stop first — a
    /// breakpoint, an exception, the end — is answered instead; a raise by
    /// another thread meanwhile is let go.
    pub(super) fn run_to_raiser(
        &mut self,
        inferior: &mut dyn Inferior,
        raise: Raise,
    ) -> Result<RunStop, String> {
        let ours = !self.armed(raise.back);
        if ours {
            inferior.insert_breakpoint(raise.back)?;
        }
        let back = ours.then_some(raise.back);
        let stop = loop {
            let stop = match self.run_inferior(inferior, back)? {
                RunStop::Watchpoints(threads) => {
                    match self.watched_writes(inferior, &threads, back)? {
                        Some(stop) => stop,
                        None => continue,
                    }
                }
                stop => stop,
            };
            let RunStop::Breakpoints(hits) = &stop else {
                break stop;
            };
            if hits.contains(&(raise.thread, raise.back)) {
                break RunStop::Raised {
                    thread: raise.thread,
                    code: raise.code,
                    origin: raise.origin.clone(),
                };
            }
            // Another thread at the raiser's place, or at a breakpoint that
            // does not stop it, runs on; so does a raise at the park.
            let hits = hits
                .iter()
                .copied()
                .filter(|(_, address)| !(ours && *address == raise.back))
                .collect::<Vec<_>>();
            let (user, own) = self.sort_hits(inferior, &hits);
            self.report_hits(inferior, &unseen(&hits, &user))?;
            if !user.is_empty() || !own.is_empty() {
                break RunStop::Breakpoints(
                    hits.into_iter()
                        .filter(|hit| user.contains(hit) || own.contains(hit))
                        .collect(),
                );
            }
        };
        if ours && !matches!(stop, RunStop::Exited(_)) {
            inferior.remove_breakpoint(raise.back)?;
        }
        Ok(stop)
    }

    /// `breakerror <codes> [<commands>]`: stop when a thread raises one of
    /// them, running `commands` when there are any (else keeping the ones the
    /// watch has). The watch keeps a breakpoint of the debugger's own on the
    /// runtime's error park, which only a `--debug` build names.
    pub(crate) fn watch_errors(
        &mut self,
        inferior: &mut dyn Inferior,
        codes: ErrorCodes,
        commands: &[String],
    ) -> Result<Vec<String>, String> {
        let codes = match (self.errors.as_ref().map(|watch| &watch.codes), codes) {
            (Some(ErrorCodes::Any), _) | (_, ErrorCodes::Any) => ErrorCodes::Any,
            (Some(ErrorCodes::Codes(old)), ErrorCodes::Codes(new)) => {
                let mut all = old.clone();
                all.extend(new.into_iter().filter(|code| !old.contains(code)));
                ErrorCodes::Codes(all)
            }
            (None, codes) => codes,
        };
        if let Some(watch) = &mut self.errors {
            watch.codes = codes;
            if !commands.is_empty() {
                watch.commands = commands.to_vec();
            }
        } else {
            let park = park_address(inferior)?;
            if !self.armed(park) {
                inferior.insert_breakpoint(park)?;
            }
            self.errors = Some(ErrorWatch {
                codes,
                park,
                commands: commands.to_vec(),
            });
        }
        Ok(vec![self.watch_line().expect("a watch was just set")])
    }

    /// `breakerror clear`: stop watching for errors.
    pub(crate) fn clear_errors(
        &mut self,
        inferior: &mut dyn Inferior,
    ) -> Result<Vec<String>, String> {
        let Some(watch) = self.errors.take() else {
            return Err("no errors are being watched for".to_string());
        };
        if !self.armed(watch.park) {
            inferior.remove_breakpoint(watch.park)?;
        }
        Ok(vec!["Stopped watching for errors.".to_string()])
    }

    /// `Stopping when a thread raises <codes>.`, for a watch, and what it
    /// runs when it has commands.
    pub(crate) fn watch_line(&self) -> Option<String> {
        let watch = self.errors.as_ref()?;
        Some(format!(
            "Stopping when a thread raises {}.{}",
            watch.codes.shown(),
            runs(&watch.commands)
        ))
    }
}
