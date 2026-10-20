//! plan-176-L: the macOS backend's loaded images — dyld's image list, each library's symbols,
//! function starts and stubs, and the executable's own header and slide. The `Task` handle lives
//! in `debugger/macos/mod.rs`.

use super::*;

impl Task {
    /// Refresh the list of loaded libraries when the loader's has changed.
    pub(super) fn refresh_libraries(&mut self) -> Result<(), String> {
        let mut info = [0i32; 8];
        let mut count = TASK_DYLD_INFO_COUNT;
        // SAFETY: `info` holds more than `count` ints.
        let status = unsafe { task_info(self.task, TASK_DYLD_INFO, info.as_mut_ptr(), &mut count) };
        if status != KERN_SUCCESS {
            return Err(format!(
                "cannot read the loaded libraries of process {}: task_info: {}",
                self.pid,
                mach_error(status)
            ));
        }
        let all_images = u64::from(info[0] as u32) | (u64::from(info[1] as u32) << 32);
        if all_images == 0 {
            return Ok(());
        }
        let count = self.read(all_images + ALL_IMAGES_COUNT, 4)?;
        let count = u32::from_le_bytes(count[..].try_into().expect("4 bytes"));
        if count == self.library_count {
            return Ok(());
        }
        let array = self.read_u64(all_images + ALL_IMAGES_ARRAY)?;
        if array == 0 {
            return Ok(());
        }
        let executable = self.slide()?.wrapping_add(self.image()?.text_address);
        let mut libraries = Vec::new();
        for index in 0..u64::from(count) {
            let entry = array + index * IMAGE_INFO_SIZE;
            let header = self.read_u64(entry)?;
            if header == 0 || header == executable {
                continue;
            }
            let path = self.read_u64(entry + 8)?;
            let name = self.read_c_string(path).unwrap_or_default();
            let name = name.rsplit('/').next().unwrap_or(&name).to_string();
            if let Ok(library) = self.read_library(header, name) {
                libraries.push(library);
            }
        }
        // Every library the executable's dependencies reach was loaded with
        // it; any other was loaded while the program ran.
        let mut reached = self
            .executable_dependencies()
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        loop {
            let before = reached.len();
            for library in &libraries {
                if library
                    .install_name
                    .as_ref()
                    .is_some_and(|name| reached.contains(name))
                {
                    reached.extend(library.dependencies.iter().cloned());
                }
            }
            if reached.len() == before {
                break;
            }
        }
        for library in &mut libraries {
            library.loaded = !library
                .install_name
                .as_ref()
                .is_some_and(|name| reached.contains(name));
        }
        self.libraries = libraries;
        self.library_count = count;
        Ok(())
    }

    /// The install names of the libraries the executable is linked against.
    fn executable_dependencies(&mut self) -> Vec<String> {
        if self.dependencies.is_none() {
            let libraries = std::fs::read(&self.program)
                .ok()
                .and_then(|bytes| crate::os::inspect::inspect(&bytes))
                .map(|binary| binary.libraries)
                .unwrap_or_default();
            self.dependencies = Some(libraries);
        }
        self.dependencies.clone().unwrap_or_default()
    }

    /// The NUL-terminated string at `address`, read a page at a time.
    fn read_c_string(&self, address: u64) -> Result<String, String> {
        let mut bytes = Vec::new();
        let mut at = address;
        while bytes.len() < MAX_PATH {
            let chunk = 64 - (at % 64) as usize;
            let read = self.read(at, chunk)?;
            if let Some(end) = read.iter().position(|byte| *byte == 0) {
                bytes.extend_from_slice(&read[..end]);
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            bytes.extend_from_slice(&read);
            at += chunk as u64;
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// A library's segments and table locations, from its header in memory.
    fn read_library(&self, header: u64, name: String) -> Result<Library, String> {
        let fixed = self.read(header, 32)?;
        let word = |bytes: &[u8], at: usize| {
            u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
        };
        let wide = |bytes: &[u8], at: usize| {
            u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
        };
        if word(&fixed, 0) != crate::os::inspect::MH_MAGIC_64 {
            return Err(format!("no Mach-O header at 0x{header:016x}"));
        }
        let commands = self.read(header + 32, word(&fixed, 20) as usize)?;
        let (mut text, mut linkedit, mut symtab, mut function_starts) = (None, None, None, None);
        let mut indirect = None;
        let mut sections = Vec::new();
        let (mut install_name, mut dependencies) = (None, Vec::new());
        let dylib_name = |command: &[u8]| {
            let offset = word(command, 8) as usize;
            command
                .get(offset..)
                .map(|name| String::from_utf8_lossy(crate::os::inspect::c_bytes(name)).into_owned())
        };
        let mut at = 0usize;
        for _ in 0..word(&fixed, 16) {
            if at + 8 > commands.len() {
                break;
            }
            let size = word(&commands, at + 4) as usize;
            if size < 8 || at + size > commands.len() {
                break;
            }
            let command = &commands[at..at + size];
            match word(command, 0) {
                LC_SEGMENT_64 if size >= 72 => {
                    let segment = crate::os::inspect::c_bytes(&command[8..24]);
                    if segment == b"__TEXT" {
                        text = Some((wide(command, 24), wide(command, 32)));
                        let count = word(command, 64) as usize;
                        for record in command[72..].chunks_exact(80).take(count) {
                            sections.push((
                                wide(record, 32),
                                wide(record, 40),
                                word(record, 64),
                                word(record, 68),
                                word(record, 72),
                            ));
                        }
                    } else if segment == b"__LINKEDIT" {
                        linkedit = Some((wide(command, 24), wide(command, 40)));
                    }
                }
                LC_SYMTAB if size >= 24 => {
                    symtab = Some((
                        word(command, 8),
                        word(command, 12),
                        word(command, 16),
                        word(command, 20),
                    ))
                }
                LC_FUNCTION_STARTS if size >= 16 => {
                    function_starts = Some((word(command, 8), word(command, 12)))
                }
                LC_DYSYMTAB if size >= 64 => {
                    indirect = Some((word(command, 56), word(command, 60)))
                }
                LC_ID_DYLIB if size >= 24 => install_name = dylib_name(command),
                LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB | LC_LOAD_UPWARD_DYLIB
                    if size >= 24 =>
                {
                    dependencies.extend(dylib_name(command))
                }
                _ => {}
            }
            at += size;
        }
        let (text_address, text_size) = text.ok_or("a library without __TEXT")?;
        let slide = header.wrapping_sub(text_address);
        let linkedit = linkedit
            .map(|(address, offset)| address.wrapping_add(slide).wrapping_sub(offset))
            .unwrap_or(0);
        Ok(Library {
            name,
            install_name,
            dependencies,
            loaded: false,
            text: (header, header.wrapping_add(text_size)),
            linkedit,
            slide,
            symtab: if linkedit == 0 { None } else { symtab },
            function_starts: if linkedit == 0 { None } else { function_starts },
            indirect: if linkedit == 0 { None } else { indirect },
            sections: sections
                .into_iter()
                .map(|(address, size, flags, reserved1, reserved2)| {
                    let start = address.wrapping_add(slide);
                    (start, start.wrapping_add(size), flags, reserved1, reserved2)
                })
                .collect(),
            starts: None,
            symbols: None,
        })
    }

    /// Library `index`'s function starts, read on first use.
    pub(super) fn library_starts(&mut self, index: usize) -> Vec<u64> {
        if let Some(starts) = &self.libraries[index].starts {
            return starts.clone();
        }
        let library = &self.libraries[index];
        let mut starts = Vec::new();
        if let Some((offset, size)) = library.function_starts {
            if let Ok(bytes) = self.read(library.linkedit + u64::from(offset), size as usize) {
                let mut address = library.text.0;
                let mut at = 0;
                while at < bytes.len() {
                    let (delta, length) = uleb128(&bytes[at..]);
                    if delta == 0 || length == 0 {
                        break;
                    }
                    at += length;
                    address = address.wrapping_add(delta);
                    starts.push(address);
                }
            }
        }
        self.libraries[index].starts = Some(starts.clone());
        starts
    }

    /// Library `index`'s defined symbols, read on first use.
    pub(super) fn library_symbols(&mut self, index: usize) -> Vec<(u64, String, bool)> {
        if let Some(symbols) = &self.libraries[index].symbols {
            return symbols.clone();
        }
        let library = &self.libraries[index];
        let mut symbols = Vec::new();
        if let Some((symoff, nsyms, stroff, _)) = library.symtab {
            if let Ok(table) = self.read(library.linkedit + u64::from(symoff), nsyms as usize * 16)
            {
                let entries = table
                    .chunks_exact(16)
                    .filter(|entry| entry[4] & 0xe0 == 0 && entry[4] & 0x0e == 0x0e)
                    .map(|entry| {
                        (
                            u32::from_le_bytes(entry[0..4].try_into().expect("4 bytes")),
                            u64::from_le_bytes(entry[8..16].try_into().expect("8 bytes")),
                            entry[4] & N_EXT != 0,
                        )
                    })
                    .collect::<Vec<_>>();
                // A shared-cache library's names lie scattered through one
                // string pool shared by every library, so the names are read
                // in runs of nearby offsets rather than as one span.
                let mut offsets = entries.iter().map(|(strx, ..)| *strx).collect::<Vec<_>>();
                offsets.sort_unstable();
                offsets.dedup();
                let mut names = std::collections::HashMap::new();
                let mut at = 0;
                while at < offsets.len() {
                    let first = offsets[at];
                    let mut last = first;
                    let mut next = at + 1;
                    while next < offsets.len()
                        && offsets[next] - last < STRING_GAP
                        && offsets[next] - first < STRING_RUN
                    {
                        last = offsets[next];
                        next += 1;
                    }
                    let base = library.linkedit + u64::from(stroff) + u64::from(first);
                    let length = (last - first) as usize + MAX_NAME;
                    if let Ok(strings) = self.read(base, length) {
                        for offset in &offsets[at..next] {
                            let name =
                                crate::os::inspect::c_bytes(&strings[(offset - first) as usize..]);
                            names.insert(*offset, String::from_utf8_lossy(name).into_owned());
                        }
                    }
                    at = next;
                }
                for (strx, value, external) in entries {
                    if let Some(name) = names.get(&strx).filter(|name| !name.is_empty()) {
                        symbols.push((value.wrapping_add(library.slide), name.clone(), external));
                    }
                }
            }
        }
        symbols.sort();
        self.libraries[index].symbols = Some(symbols.clone());
        symbols
    }

    /// The stub holding `address` in library `index`, named by the symbol its
    /// indirect-symbol entry selects (`symbol stub for: <symbol>`).
    pub(super) fn library_stub(&self, index: usize, address: u64) -> Option<FunctionRange> {
        let library = &self.libraries[index];
        let &(begin, _, _, first, size) =
            library.sections.iter().find(|(begin, end, flags, ..)| {
                flags & 0xff == S_SYMBOL_STUBS && *begin <= address && address < *end
            })?;
        if size == 0 {
            return None;
        }
        let stub = (address - begin) / u64::from(size);
        let start = begin + stub * u64::from(size);
        let (indirect, _) = library.indirect?;
        let (symoff, _, stroff, _) = library.symtab?;
        let entry = self
            .read(
                library.linkedit + u64::from(indirect) + (u64::from(first) + stub) * 4,
                4,
            )
            .ok()?;
        let entry = u32::from_le_bytes(entry[..].try_into().ok()?);
        let name = if entry & 0xc000_0000 == 0 {
            let nlist = self
                .read(
                    library.linkedit + u64::from(symoff) + u64::from(entry) * 16,
                    4,
                )
                .ok()?;
            let strx = u32::from_le_bytes(nlist[..].try_into().ok()?);
            self.read_c_string(library.linkedit + u64::from(stroff) + u64::from(strx))
                .ok()
                .map(|symbol| format!("symbol stub for: {symbol}"))
        } else {
            None
        };
        Some(FunctionRange {
            image: library.name.clone(),
            name,
            start,
            size: Some(u64::from(size)),
        })
    }

    /// The range of the function at `start` in library `index`: up to the next
    /// function's start, and never past the end of its section.
    pub(super) fn library_function(&mut self, index: usize, start: u64) -> FunctionRange {
        let starts = self.library_starts(index);
        let section_end = self.libraries[index]
            .sections
            .iter()
            .find(|(begin, end, ..)| *begin <= start && start < *end)
            .map_or(self.libraries[index].text.1, |(_, end, ..)| *end);
        let end = starts
            .iter()
            .copied()
            .find(|next| *next > start)
            .unwrap_or(section_end)
            .min(section_end);
        let symbols = self.library_symbols(index);
        let name = symbols
            .iter()
            .find(|(address, ..)| *address == start)
            .map(|(_, name, _)| name.clone());
        FunctionRange {
            image: self.libraries[index].name.clone(),
            name,
            start,
            size: starts.contains(&start).then(|| end - start),
        }
    }

    /// The functions the libraries the program loaded while running export,
    /// by address.
    pub(super) fn loaded_functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        self.refresh_libraries()?;
        let mut functions = Vec::new();
        for index in 0..self.libraries.len() {
            if !self.libraries[index].loaded {
                continue;
            }
            let symbols = self.library_symbols(index);
            let library = &self.libraries[index];
            let code = |address: u64| {
                library.sections.iter().any(|(begin, end, flags, ..)| {
                    flags & S_ATTR_INSTRUCTIONS != 0 && *begin <= address && address < *end
                })
            };
            functions.extend(
                symbols
                    .into_iter()
                    .filter(|(address, _, external)| *external && code(*address))
                    .map(|(address, name, _)| FunctionSymbol {
                        name,
                        linked: true,
                        loaded: true,
                        address: Some(address),
                        size: None,
                    }),
            );
        }
        functions.sort_by_key(|function| function.address);
        Ok(functions)
    }

    pub(super) fn image(&mut self) -> Result<&macho::Image, String> {
        if self.image.is_none() {
            let shown = super::shown(&self.program);
            let bytes = std::fs::read(&self.program)
                .map_err(|err| format!("cannot read '{shown}': {err}"))?;
            let image = macho::read(&bytes)
                .ok_or_else(|| format!("cannot read the symbol table of '{shown}'"))?;
            self.image = Some(image);
        }
        Ok(self.image.as_ref().expect("read above"))
    }

    /// How far the executable was loaded from the address it asks for.
    pub(super) fn slide(&mut self) -> Result<u64, String> {
        if let Some(slide) = self.slide {
            return Ok(slide);
        }
        let header = self.executable_header()?;
        let slide = header.wrapping_sub(self.image()?.text_address);
        self.slide = Some(slide);
        Ok(slide)
    }

    /// The live address of the main executable's Mach-O header: the first
    /// readable region that starts with a 64-bit `MH_EXECUTE` header. The
    /// kernel maps the executable before `dyld` runs, so this holds from the
    /// first instruction on.
    fn executable_header(&self) -> Result<u64, String> {
        let mut address = 0u64;
        for _ in 0..MAX_REGIONS {
            let mut size = 0u64;
            let mut info = [0i32; VM_REGION_BASIC_INFO_COUNT_64 as usize];
            let mut count = VM_REGION_BASIC_INFO_COUNT_64;
            let mut object: MachPort = 0;
            // SAFETY: every out-parameter is a valid local; `info` holds
            // `count` ints, the size of `vm_region_basic_info_data_64_t`.
            let status = unsafe {
                mach_vm_region(
                    self.task,
                    &mut address,
                    &mut size,
                    VM_REGION_BASIC_INFO_64,
                    info.as_mut_ptr(),
                    &mut count,
                    &mut object,
                )
            };
            if status == KERN_INVALID_ADDRESS {
                break;
            }
            if status != KERN_SUCCESS {
                return Err(format!(
                    "cannot read the memory map of process {}: mach_vm_region: {}",
                    self.pid,
                    mach_error(status)
                ));
            }
            // `protection` is the first field of the region info.
            if info[0] & VM_PROT_READ != 0 {
                if let Ok(header) = self.read(address, 16) {
                    let magic = u32::from_le_bytes(header[0..4].try_into().expect("16 bytes"));
                    let kind = u32::from_le_bytes(header[12..16].try_into().expect("16 bytes"));
                    if magic == crate::os::inspect::MH_MAGIC_64 && kind == MH_EXECUTE {
                        return Ok(address);
                    }
                }
            }
            address = match address.checked_add(size) {
                Some(next) if size > 0 => next,
                _ => break,
            };
        }
        Err(format!(
            "cannot find the executable's header in process {}",
            self.pid
        ))
    }
}

/// A ULEB128 value and how many bytes it took.
fn uleb128(bytes: &[u8]) -> (u64, usize) {
    let mut value = 0u64;
    for (index, byte) in bytes.iter().enumerate().take(10) {
        value |= u64::from(byte & 0x7f) << (7 * index);
        if byte & 0x80 == 0 {
            return (value, index + 1);
        }
    }
    (value, 0)
}
