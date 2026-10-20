//! plan-176-L: running to the next stop — counting, stop commands, settling, the watched
//! writes, and finish/return/restart. The `Breakpoints` table and its editing live in
//! `debugger/breakpoints/mod.rs`.

use super::*;

impl Breakpoints {
    /// Count `stop` against what made it: each breakpoint a hit stopped for,
    /// and each watchpoint whose bytes changed.
    fn count(&mut self, stop: &RunStop) {
        match stop {
            RunStop::Breakpoints(hits) => {
                for (_, ids) in self.matched.iter().filter(|(hit, _)| hits.contains(hit)) {
                    for breakpoint in &mut self.list {
                        if ids.contains(&breakpoint.id) {
                            breakpoint.hits += 1;
                        }
                    }
                }
            }
            RunStop::Changed(changes) => {
                for change in changes {
                    for watch in &mut self.watches {
                        if watch.address == change.address {
                            watch.hits += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// The commands each stop in `stop` runs, by thread, and whether every
    /// stop in it has commands.
    pub(crate) fn stop_commands(&self, stop: &RunStop) -> (Vec<(usize, Vec<String>)>, bool) {
        let mut found = Vec::new();
        let mut every = true;
        let mut add = |thread: usize, commands: Option<&Vec<String>>| match commands {
            Some(commands) if !commands.is_empty() => found.push((thread, commands.clone())),
            _ => every = false,
        };
        match stop {
            // Each breakpoint a hit stopped for runs its commands.
            RunStop::Breakpoints(hits) => {
                for hit in hits {
                    let stopped_for = self
                        .matched
                        .iter()
                        .find(|(matched, _)| matched == hit)
                        .map_or(&[][..], |(_, ids)| ids.as_slice());
                    if stopped_for.is_empty() {
                        add(hit.0, None);
                    }
                    for id in stopped_for {
                        add(
                            hit.0,
                            self.list
                                .iter()
                                .find(|breakpoint| breakpoint.id == *id)
                                .map(|breakpoint| &breakpoint.commands),
                        );
                    }
                }
            }
            RunStop::Changed(changes) => {
                for change in changes {
                    add(
                        change.thread,
                        self.watches
                            .iter()
                            .find(|watch| watch.address == change.address)
                            .map(|watch| &watch.commands),
                    );
                }
            }
            RunStop::Raised { thread, .. } => {
                add(*thread, self.errors.as_ref().map(|watch| &watch.commands))
            }
            _ => every = false,
        }
        (found, every)
    }

    /// The notes made since the session last heard a stop.
    pub(crate) fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// Run the program, handing `hear` each stop the user should see as it
    /// happens, until `hear` says not to run on, the program ends, or it
    /// reaches breakpoints the caller placed itself. Pending breakpoints are
    /// placed as soon as their functions are bound; while any is pending, the
    /// program's entry point holds a breakpoint of the debugger's own, and
    /// stopping there only places them and runs on. A write a watchpoint
    /// covers that leaves its bytes as they were runs on unseen.
    pub(crate) fn run(
        &mut self,
        inferior: &mut dyn Inferior,
        hear: &mut Hear,
    ) -> Result<Ended, String> {
        loop {
            let (stop, own) = self.run_once(inferior)?;
            let Some(stop) = stop else {
                if own.is_empty() {
                    continue;
                }
                self.close_report(inferior, false)?;
                return Ok(Ended::Own(own));
            };
            let exited = matches!(stop, RunStop::Exited(_));
            // The program is the user's again: no `step` runs the report's
            // breakpoints, so a report half written is the program's to end.
            if exited {
                self.report_ended();
            } else {
                self.close_report(inferior, false)?;
                self.place_pending(inferior)?;
            }
            let notes = self.take_notes();
            self.count(&stop);
            let on = hear(inferior, self, notes, &stop)?;
            if !on || exited {
                return Ok(Ended::Heard(stop));
            }
            if !own.is_empty() {
                return Ok(Ended::Own(own));
            }
        }
    }

    /// Run the program once: the stop the user should see, if any, and the
    /// threads at breakpoints neither the user's nor the debugger's own.
    #[allow(clippy::type_complexity)]
    fn run_once(
        &mut self,
        inferior: &mut dyn Inferior,
    ) -> Result<(Option<RunStop>, Vec<(usize, u64)>), String> {
        self.place_pending(inferior)?;
        let entry = inferior.entry()?;
        let temporary = match entry {
            Some(entry)
                if !self.entry_passed
                    && self
                        .list
                        .iter()
                        .any(|breakpoint| breakpoint.address.is_none())
                    && !self
                        .list
                        .iter()
                        .any(|breakpoint| breakpoint.address == Some(entry)) =>
            {
                inferior.insert_breakpoint(entry)?;
                Some(entry)
            }
            _ => None,
        };
        let stop = self.run_inferior(inferior, temporary)?;
        if let (Some(entry), false) = (temporary, matches!(stop, RunStop::Exited(_))) {
            inferior.remove_breakpoint(entry)?;
        }
        self.settle(inferior, stop, entry, temporary)
    }

    /// Sort what the program stopped for: the user's breakpoints are a stop
    /// when one there stops the thread; the debugger's own — the entry point,
    /// the error park, the debug report's — are not, though a watched raise at
    /// the park runs on to the raiser; a watched write is a stop only when it
    /// changed something.
    #[allow(clippy::type_complexity)]
    fn settle(
        &mut self,
        inferior: &mut dyn Inferior,
        stop: RunStop,
        entry: Option<u64>,
        temporary: Option<u64>,
    ) -> Result<(Option<RunStop>, Vec<(usize, u64)>), String> {
        match stop {
            RunStop::Breakpoints(hits) => {
                if hits.iter().any(|(_, address)| Some(*address) == entry) {
                    self.entry_passed = true;
                }
                let hits = hits
                    .into_iter()
                    .filter(|(_, address)| Some(*address) != temporary)
                    .collect::<Vec<_>>();
                let (user, own) = self.sort_hits(inferior, &hits);
                self.report_hits(inferior, &unseen(&hits, &user))?;
                if !user.is_empty() {
                    return Ok((Some(RunStop::Breakpoints(user)), own));
                }
                if !own.is_empty() {
                    return Ok((None, own));
                }
                // A thread at the park that no breakpoint of the user's
                // stopped is the watch's to look at.
                let mut raised = None;
                for &(thread, address) in &hits {
                    if self.is_park(address) {
                        raised = self.watched_raise(inferior, thread)?;
                        if raised.is_some() {
                            break;
                        }
                    }
                }
                match raised {
                    Some(raise) => {
                        let stop = self.run_to_raiser(inferior, raise)?;
                        self.settle(inferior, stop, entry, None)
                    }
                    None => Ok((None, Vec::new())),
                }
            }
            RunStop::Watchpoints(threads) => {
                Ok((self.watched_writes(inferior, &threads, None)?, Vec::new()))
            }
            other => Ok((Some(other), Vec::new())),
        }
    }

    /// Sort breakpoint hits into the user's — a hit where a breakpoint stops
    /// the thread (it has no test, or its test holds for that thread), each
    /// remembered for its commands — and the caller's own: a `ret` a `finish`
    /// waits on, or a breakpoint the session does not hold. A hit only at
    /// breakpoints whose tests fail is neither, and neither is the park's or
    /// the debug report's.
    #[allow(clippy::type_complexity)]
    pub(super) fn sort_hits(
        &mut self,
        inferior: &mut dyn Inferior,
        hits: &[(usize, u64)],
    ) -> (Vec<(usize, u64)>, Vec<(usize, u64)>) {
        self.matched.clear();
        let mut user = Vec::new();
        let mut own = Vec::new();
        for &(thread, address) in hits {
            let here = self
                .list
                .iter()
                .filter(|breakpoint| breakpoint.address == Some(address))
                .collect::<Vec<_>>();
            let held = !here.is_empty();
            let stopping = here
                .into_iter()
                .filter(|breakpoint| {
                    breakpoint
                        .condition
                        .as_ref()
                        .is_none_or(|condition| condition.holds(inferior, thread))
                })
                .map(|breakpoint| breakpoint.id)
                .collect::<Vec<_>>();
            if !stopping.is_empty() {
                user.push((thread, address));
                self.matched.push(((thread, address), stopping));
            }
            if self.finishing.contains(&address)
                || (!held && !self.is_park(address) && !self.report.holds(address))
            {
                own.push((thread, address));
            }
        }
        (user, own)
    }

    /// Run the program, as [`Inferior::run`] does, without passing a watched
    /// write unseen. A thread a step left on a breakpoint is a hit there
    /// before anything runs. With watchpoints set, a thread paused on a
    /// breakpoint (one the session holds, or `own`, one the caller placed for
    /// this run) is first stepped off it here, where its write is seen: the
    /// backend's `run` steps such a thread with watchpoints off.
    pub(super) fn run_inferior(
        &mut self,
        inferior: &mut dyn Inferior,
        own: Option<u64>,
    ) -> Result<RunStop, String> {
        let landed = self.take_landed(inferior)?;
        if !landed.is_empty() {
            return Ok(RunStop::Breakpoints(landed));
        }
        if !self.watches.is_empty() {
            let paused = inferior
                .threads()?
                .iter()
                .enumerate()
                .filter(|(_, facts)| self.armed(facts.pc) || own == Some(facts.pc))
                .map(|(thread, _)| thread)
                .collect::<Vec<_>>();
            if !paused.is_empty() {
                if let Some(stop) = self.watched_writes(inferior, &paused, own)? {
                    return Ok(stop);
                }
                let landed = self.take_landed(inferior)?;
                if !landed.is_empty() {
                    return Ok(RunStop::Breakpoints(landed));
                }
            }
        }
        inferior.run()
    }

    /// The threads a step left on a breakpoint, each still there (a `jump`
    /// since moves one away), with the breakpoint's address.
    fn take_landed(&mut self, inferior: &mut dyn Inferior) -> Result<Vec<(usize, u64)>, String> {
        let landed = std::mem::take(&mut self.landed);
        if landed.is_empty() {
            return Ok(landed);
        }
        let threads = inferior.threads()?;
        Ok(landed
            .into_iter()
            .filter(|(thread, address)| {
                threads
                    .get(*thread)
                    .is_some_and(|facts| facts.pc == *address)
            })
            .collect())
    }

    /// Step each of `threads`, stopped before a watched write, over the write:
    /// a `Changed` stop for every watchpoint whose bytes it changed, or `None`
    /// when it changed none. A step that ends otherwise is that stop. A thread
    /// the step leaves on a breakpoint the session holds, or on `own`, stops
    /// there next.
    pub(super) fn watched_writes(
        &mut self,
        inferior: &mut dyn Inferior,
        threads: &[usize],
        own: Option<u64>,
    ) -> Result<Option<RunStop>, String> {
        let mut changes = Vec::new();
        for &thread in threads {
            let Some(pc) = inferior.threads()?.get(thread).map(|facts| facts.pc) else {
                continue;
            };
            let before = self.watched_values(inferior);
            let step = inferior
                .step(&[thread], 1)?
                .pop()
                .ok_or("the step answered for no thread")?;
            match step.stop {
                StepStop::Finished | StepStop::Waiting => {}
                StepStop::Exception(what) => return Ok(Some(RunStop::Exception { thread, what })),
                // A program that has ended answers how at once.
                StepStop::Exited => return Ok(Some(inferior.run()?)),
            }
            let after = self.watched_values(inferior);
            for ((watch, old), new) in self.watches.iter().zip(before).zip(after) {
                if let (Some(old), Some(new)) = (old, new) {
                    if old != new {
                        changes.push(Change {
                            thread,
                            pc,
                            address: watch.address,
                            size: watch.size,
                            old,
                            new,
                        });
                    }
                }
            }
            if let Some(now) = inferior.threads()?.get(thread).map(|facts| facts.pc) {
                if self.armed(now) || own == Some(now) {
                    self.landed.push((thread, now));
                }
            }
        }
        Ok((!changes.is_empty()).then_some(RunStop::Changed(changes)))
    }

    /// Every watchpoint's bytes now, `None` where they cannot be read.
    fn watched_values(&self, inferior: &mut dyn Inferior) -> Vec<Option<u64>> {
        self.watches
            .iter()
            .map(|watch| watched_value(inferior, watch.address, watch.size).ok())
            .collect()
    }

    /// After a restart: place every breakpoint again by its function and
    /// offset in the new process (a linked function's is pending until its
    /// library loads); one no function held is dropped, and so is every
    /// watchpoint.
    pub(crate) fn restarted(&mut self, inferior: &mut dyn Inferior) -> Result<Vec<String>, String> {
        self.entry_passed = false;
        self.landed.clear();
        self.notes.clear();
        self.matched.clear();
        let mut notes = std::mem::take(&mut self.watches)
            .into_iter()
            .map(|watch| {
                format!(
                    "Dropped the watchpoint at 0x{:016x}: a restarted program's memory is laid \
                     out anew.",
                    watch.address
                )
            })
            .collect::<Vec<_>>();
        // The addresses that hold an instruction again: one per address.
        let mut placed = Vec::new();
        if let Some(watch) = &mut self.errors {
            watch.park = park_address(inferior)?;
            inferior.insert_breakpoint(watch.park)?;
            placed.push(watch.park);
        }
        self.watch_report(inferior, &mut placed)?;
        let functions = inferior.functions().unwrap_or_default();
        let mut kept = Vec::new();
        for mut breakpoint in std::mem::take(&mut self.list) {
            breakpoint.hits = 0;
            let Some((symbol, offset)) = breakpoint.anchor.clone() else {
                notes.push(format!(
                    "Dropped the breakpoint at {}: no function holds it.",
                    place(&breakpoint)
                ));
                continue;
            };
            let function = functions.iter().find(|function| function.name == symbol);
            breakpoint.address = match function {
                Some(function) if !function.linked => function.address.map(|start| start + offset),
                Some(_) => None,
                None => {
                    notes.push(format!(
                        "Dropped the breakpoint on {symbol}: the program no longer has it."
                    ));
                    continue;
                }
            };
            if let Some(address) = breakpoint.address {
                if !placed.contains(&address) {
                    inferior.insert_breakpoint(address)?;
                    placed.push(address);
                }
            }
            kept.push(breakpoint);
        }
        self.list = kept;
        Ok(notes)
    }

    /// Run `thread` until the function it is in returns: breakpoints on every
    /// `ret` of the function, a stop there by `thread` with its frame popped
    /// (stack pointer at or above where it is now, so a recursive call's return
    /// does not count), then one step over the `ret`. Every stop on the way is
    /// handed to `hear`, as `run` does; one it does not run on from ends the
    /// finish early.
    pub(crate) fn finish(
        &mut self,
        inferior: &mut dyn Inferior,
        thread: usize,
        hear: &mut Hear,
    ) -> Result<Finish, String> {
        let threads = inferior.threads()?;
        let facts = threads
            .get(thread)
            .ok_or_else(|| format!("no thread {thread}"))?
            .clone();
        let range = inferior
            .function_at(facts.pc)?
            .ok_or_else(|| format!("cannot finish: no known function holds 0x{:016x}", facts.pc))?;
        let ends = returns(inferior, &range)?;
        if ends.is_empty() {
            return Err(format!(
                "cannot finish: no ret instruction found in the function at 0x{:016x}",
                range.start
            ));
        }
        // A `ret` that holds a breakpoint already needs no instruction of the
        // finish's own, but is still one it waits on.
        let ours = ends
            .iter()
            .copied()
            .filter(|address| !self.armed(*address))
            .collect::<Vec<_>>();
        for address in &ours {
            inferior.insert_breakpoint(*address)?;
        }
        self.finishing = ends;
        let outcome = self.run_to_return(inferior, thread, facts.sp, hear);
        self.finishing.clear();
        let outcome = outcome?;
        if !matches!(outcome, Some(RunStop::Exited(_))) {
            for address in &ours {
                inferior.remove_breakpoint(*address)?;
            }
        }
        if let Some(stop) = outcome {
            return Ok(Finish::Stopped {
                stop,
                from: range.name.clone(),
            });
        }
        let step = inferior
            .step(&[thread], 1)?
            .pop()
            .ok_or("the step answered for no thread")?;
        let stop = match step.stop {
            StepStop::Finished => {
                return Ok(Finish::Returned {
                    from: range.name.clone(),
                })
            }
            StepStop::Exception(what) => RunStop::Exception { thread, what },
            StepStop::Waiting => RunStop::Interrupted,
            StepStop::Exited => RunStop::Exited(super::Exit::Unknown),
        };
        let notes = self.take_notes();
        hear(inferior, self, notes, &stop)?;
        Ok(Finish::Stopped {
            stop,
            from: range.name.clone(),
        })
    }

    /// Run until `thread` reaches a `ret` the finish waits on with its frame
    /// popped (its stack pointer at or above `sp`): `None`; or the stop heard
    /// first, which ends the finish.
    fn run_to_return(
        &mut self,
        inferior: &mut dyn Inferior,
        thread: usize,
        sp: u64,
        hear: &mut Hear,
    ) -> Result<Option<RunStop>, String> {
        loop {
            let hits = match self.run(inferior, hear)? {
                Ended::Heard(stop) => return Ok(Some(stop)),
                Ended::Own(hits) => hits,
            };
            let popped = hits.iter().any(|(hit, _)| {
                *hit == thread
                    && inferior
                        .threads()
                        .ok()
                        .and_then(|threads| threads.get(thread).map(|now| now.sp >= sp))
                        .unwrap_or(false)
            });
            if popped {
                return Ok(None);
            }
        }
    }

    /// Place every pending breakpoint whose function is now bound, with a
    /// note for each.
    fn place_pending(&mut self, inferior: &mut dyn Inferior) -> Result<(), String> {
        if self
            .list
            .iter()
            .all(|breakpoint| breakpoint.address.is_some())
        {
            return Ok(());
        }
        let functions = inferior.functions()?;
        let mut index = 0;
        while index < self.list.len() {
            let pending = &self.list[index];
            let bound = (pending.address.is_none())
                .then(|| {
                    functions
                        .iter()
                        .find(|function| pending.symbol.as_ref() == Some(&function.name))
                        .and_then(|function| function.address)
                })
                .flatten();
            let Some(address) = bound else {
                index += 1;
                continue;
            };
            if self
                .list
                .iter()
                .any(|other| other.address == Some(address) && other.condition == pending.condition)
            {
                // One with the same test is already there; this adds nothing.
                self.notes.push(format!(
                    "The pending breakpoint on {}{} is at 0x{address:016x}, which already has one.",
                    shown(&pending.symbol),
                    tested(pending)
                ));
                self.list.remove(index);
                continue;
            }
            if !self.armed(address) {
                inferior.insert_breakpoint(address)?;
            }
            self.list[index].address = Some(address);
            self.list[index].anchor = self.list[index].symbol.clone().map(|symbol| (symbol, 0));
            self.notes.push(format!(
                "Placed the pending breakpoint on {} at 0x{address:016x}.",
                shown(&self.list[index].symbol)
            ));
            index += 1;
        }
        Ok(())
    }
}

/// How a `finish` ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Finish {
    /// The thread returned from the function named here.
    Returned { from: Option<String> },
    /// Something else stopped the program before the function named here
    /// returned; the stop has been heard.
    Stopped { stop: RunStop, from: Option<String> },
}
