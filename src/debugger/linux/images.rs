//! plan-176-L: the Linux backend's memory and loaded images — reading and writing the program,
//! its mappings and load bias, and the shared libraries' functions. The `Tracee` handle lives in
//! `debugger/linux/mod.rs`.

use super::*;

impl Tracee {
    pub(super) fn new(
        pid: c_int,
        args: Option<Vec<OsString>>,
        program: &Path,
        tids: Vec<c_int>,
    ) -> Result<Self, String> {
        let memory = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/proc/{pid}/mem"))
            .map_err(|error| format!("cannot open the memory of process {pid}: {error}"))?;
        let action = SigAction {
            handler: on_interrupt as *const () as usize,
            mask: [0; 16],
            flags: 0,
            restorer: 0,
        };
        let mut previous_interrupt = action;
        // SAFETY: `on_interrupt` only stores to atomics and makes
        // async-signal-safe system calls; without `SA_RESTART` a wait it
        // interrupts returns `EINTR`, so a `run` sees Ctrl-C. The previous
        // action is put back in `end`.
        unsafe {
            SESSION_THREAD.store(syscall(SYS_GETTID) as c_int, Ordering::SeqCst);
            sigaction(SIGINT, &action, &mut previous_interrupt);
        }
        let launched = args.is_some();
        Ok(Tracee {
            pid,
            launched,
            args: args.unwrap_or_default(),
            program: program.to_path_buf(),
            ended: false,
            memory,
            threads: tids
                .into_iter()
                .map(|tid| Thread {
                    tid,
                    stopped: launched,
                    pending: 0,
                    fresh: false,
                })
                .collect(),
            image: None,
            bias: None,
            breakpoints: BTreeMap::new(),
            watchpoints: Vec::new(),
            exited: None,
            previous_interrupt,
            libraries: Vec::new(),
            maps: String::new(),
            files: HashMap::new(),
            pthread_gap: None,
        })
    }

    pub(super) fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let mut buffer = vec![0u8; length];
        let failure = |detail: String| {
            format!(
                "cannot read process {}'s memory at 0x{address:016x}: {detail}",
                self.pid
            )
        };
        match self.memory.read_at(&mut buffer, address) {
            Ok(read) if read == length => Ok(buffer),
            Ok(_) => Err(failure("the memory ends".to_string())),
            Err(error) => Err(failure(error.to_string())),
        }
    }

    pub(super) fn write(&self, address: u64, bytes: &[u8]) -> Result<(), String> {
        match self.memory.write_at(bytes, address) {
            Ok(written) if written == bytes.len() => Ok(()),
            Ok(_) => Err(format!(
                "cannot change the instruction at 0x{address:016x}: the memory ends"
            )),
            Err(error) => Err(format!(
                "cannot change the instruction at 0x{address:016x}: {error}"
            )),
        }
    }

    pub(super) fn image(&mut self) -> Result<&elf::Executable, String> {
        if self.image.is_none() {
            let shown = super::shown(&self.program);
            let bytes = std::fs::read(&self.program)
                .map_err(|err| format!("cannot read '{shown}': {err}"))?;
            let image = elf::executable(&bytes)
                .ok_or_else(|| format!("cannot read the symbol table of '{shown}'"))?;
            self.image = Some(image);
        }
        Ok(self.image.as_ref().expect("read above"))
    }

    /// `/proc/<pid>/maps`: its text, and each mapping in it.
    pub(super) fn mappings(&self) -> Result<(String, Vec<Mapping>), String> {
        let text =
            std::fs::read_to_string(format!("/proc/{}/maps", self.pid)).map_err(|error| {
                format!(
                    "cannot read the memory map of process {}: {error}",
                    self.pid
                )
            })?;
        let mut mappings = Vec::new();
        for line in text.lines() {
            let mut fields = line.splitn(6, ' ');
            let (Some(span), _, Some(offset), _, _) = (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            ) else {
                continue;
            };
            let path = fields.next().unwrap_or("").trim_start().to_string();
            let Some((start, end)) = span.split_once('-') else {
                continue;
            };
            let (Ok(start), Ok(end), Ok(offset)) = (
                u64::from_str_radix(start, 16),
                u64::from_str_radix(end, 16),
                u64::from_str_radix(offset, 16),
            ) else {
                continue;
            };
            mappings.push(Mapping {
                start,
                end,
                offset,
                path,
            });
        }
        Ok((text, mappings))
    }

    /// The executable's load bias: where its first loaded page is, less that
    /// page's link-time address.
    pub(super) fn bias(&mut self) -> Result<u64, String> {
        if let Some(bias) = self.bias {
            return Ok(bias);
        }
        let first = self
            .image()?
            .elf
            .segments
            .first()
            .copied()
            .ok_or("the executable maps no segment")?;
        let executable = std::fs::read_link(format!("/proc/{}/exe", self.pid))
            .unwrap_or_else(|_| self.program.clone());
        let (_, mappings) = self.mappings()?;
        let page = first.offset & !0xfff;
        let start = mappings
            .iter()
            .find(|mapping| mapping.offset == page && Path::new(&mapping.path) == executable)
            .map(|mapping| mapping.start)
            .ok_or_else(|| {
                format!(
                    "cannot find the executable in the memory map of process {}",
                    self.pid
                )
            })?;
        let bias = start.wrapping_sub(first.vaddr & !0xfff);
        self.bias = Some(bias);
        Ok(bias)
    }

    /// Re-read the libraries when the memory map has changed.
    pub(super) fn refresh_libraries(&mut self) -> Result<(), String> {
        let (text, mappings) = self.mappings()?;
        if text == self.maps {
            return Ok(());
        }
        let executable = std::fs::read_link(format!("/proc/{}/exe", self.pid))
            .unwrap_or_else(|_| self.program.clone());
        let mut libraries: Vec<Library> = Vec::new();
        for Mapping {
            start,
            offset,
            path,
            ..
        } in &mappings
        {
            if !path.starts_with('/') || Path::new(path) == executable || *offset != 0 {
                continue;
            }
            let path = PathBuf::from(path);
            if libraries.iter().any(|library| library.path == path) {
                continue;
            }
            let Some(elf) = self.library_file(&path) else {
                continue;
            };
            let Some(first) = elf.segments.first() else {
                continue;
            };
            let bias = start.wrapping_sub(first.vaddr & !0xfff);
            let code = elf
                .code()
                .into_iter()
                .map(|(begin, end)| (begin.wrapping_add(bias), end.wrapping_add(bias)))
                .collect();
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            libraries.push(Library {
                name,
                path,
                bias,
                code,
                loaded: false,
            });
        }
        // Every library the executable's needs reach was loaded with it, and
        // so was the loader; any other was loaded while the program ran.
        let interpreter = self.image()?.elf.interpreter.clone();
        let mut reached = self
            .image()?
            .elf
            .needed
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        loop {
            let before = reached.len();
            for library in &libraries {
                let Some(Some(elf)) = self.files.get(&library.path) else {
                    continue;
                };
                let soname = elf.soname.clone().unwrap_or_else(|| library.name.clone());
                if reached.contains(&soname) {
                    reached.extend(elf.needed.iter().cloned());
                }
            }
            if reached.len() == before {
                break;
            }
        }
        for library in &mut libraries {
            let soname = self
                .files
                .get(&library.path)
                .and_then(|file| file.as_ref()?.soname.clone())
                .unwrap_or_else(|| library.name.clone());
            let is_loader = interpreter.as_deref().is_some_and(|loader| {
                Path::new(loader).file_name() == library.path.file_name()
                    || Path::new(loader) == library.path
            });
            library.loaded = !reached.contains(&soname) && !is_loader;
        }
        self.libraries = libraries;
        self.maps = text;
        Ok(())
    }

    /// A library's file, read on first use (through the program's own root
    /// when the path does not resolve here).
    fn library_file(&mut self, path: &Path) -> Option<elf::Elf> {
        if let Some(file) = self.files.get(path) {
            return file.clone();
        }
        let bytes = std::fs::read(path).ok().or_else(|| {
            std::fs::read(
                Path::new(&format!("/proc/{}/root", self.pid)).join(path.strip_prefix("/").ok()?),
            )
            .ok()
        });
        let elf = bytes.as_deref().and_then(elf::read);
        self.files.insert(path.to_path_buf(), elf.clone());
        elf
    }

    /// The functions the libraries the program loaded while running export,
    /// by address.
    pub(super) fn loaded_functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        self.refresh_libraries()?;
        let mut functions = Vec::new();
        for library in &self.libraries {
            if !library.loaded {
                continue;
            }
            let Some(Some(elf)) = self.files.get(&library.path) else {
                continue;
            };
            functions.extend(
                elf.defined_functions()
                    .into_iter()
                    .filter(|(.., exported)| *exported)
                    .map(|(name, address, ..)| FunctionSymbol {
                        name,
                        linked: true,
                        loaded: true,
                        address: Some(address.wrapping_add(library.bias)),
                        size: None,
                    }),
            );
        }
        functions.sort_by_key(|function| function.address);
        Ok(functions)
    }

    /// The function of library `index` holding the live `address`.
    pub(super) fn library_function_at(&self, index: usize, address: u64) -> Option<FunctionRange> {
        let library = &self.libraries[index];
        let elf = self.files.get(&library.path)?.as_ref()?;
        let at = address.wrapping_sub(library.bias);
        if let Some((start, name)) = elf.plt_entry(at) {
            return Some(FunctionRange {
                image: library.name.clone(),
                name: Some(format!("symbol stub for: {name}")),
                start: start.wrapping_add(library.bias),
                size: Some(16),
            });
        }
        let functions = elf.defined_functions();
        let holder = functions
            .iter()
            .filter(|(_, start, size, _)| *start <= at && at < start.saturating_add(*size))
            .max_by_key(|(_, start, ..)| *start)
            .or_else(|| {
                // A symbol without a size runs up to the next one.
                let before = functions
                    .iter()
                    .filter(|(_, start, ..)| *start <= at)
                    .max_by_key(|(_, start, ..)| *start)?;
                (before.2 == 0).then_some(before)
            })?;
        Some(FunctionRange {
            image: library.name.clone(),
            name: Some(holder.0.clone()),
            start: holder.1.wrapping_add(library.bias),
            size: (holder.2 > 0).then_some(holder.2),
        })
    }
}
