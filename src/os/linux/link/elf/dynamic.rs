//! plan-176-O: the ELF writer's dynamic payload — `.dynamic`/`.dynstr`, the runpath and the
//! dynamic prefix size. The static/dynamic image encoders and the header helpers live in
//! `linux/link/elf/mod.rs`.

use super::*;

pub(super) struct DynamicPayload {
    pub(super) bytes: Vec<u8>,
    /// Offsets are relative to the payload start (`image.data.len()`).
    pub(super) dynamic_offset: usize,
    pub(super) dynamic_size: usize,
    pub(super) dynstr_offset: usize,
    pub(super) dynstr_size: usize,
}

impl DynamicPayload {
    pub(super) fn build(
        arch: &str,
        flavor: LinuxFlavor,
        image: &EncodedImage,
    ) -> Result<Self, String> {
        let payload_start = image.data.len();
        // Page-align the dynamic payload (dynstr/dynsym/.rela/GOT/.dynamic) so it
        // starts on a fresh page after the mutable arena global. This isolates the
        // GOT/.dynamic onto their own pages, letting PT_GNU_RELRO freeze them
        // read-only after startup binding without also freezing the writable arena
        // global (bug-263 / LNK-03). Every downstream offset derives from this
        // base, so the shift propagates through the GOT/relocation/DT vmaddrs with
        // no other change; the leading gap becomes zero padding in `bytes`.
        let data_base_offset = align(image.data.len(), PAGE_SIZE);
        let mut libraries = Vec::<String>::new();
        for import in &image.imports {
            if !libraries.contains(&import.library) {
                libraries.push(import.library.clone());
            }
        }
        let mut dynstr = vec![0];
        let mut library_offsets = Vec::new();
        for library in &libraries {
            library_offsets.push(dynstr.len());
            dynstr.extend_from_slice(library.as_bytes());
            dynstr.push(0);
        }
        let mut symbol_name_offsets = Vec::new();
        for import in &image.imports {
            symbol_name_offsets.push(dynstr.len());
            dynstr.extend_from_slice(import.symbol.as_bytes());
            dynstr.push(0);
        }

        // Symbol versioning (plan-linker.md §6.2). Each distinct (library, version)
        // pair becomes one Vernaux with a unique version index (>= 2); each
        // imported symbol's `.gnu.version` entry names its index (1 = unversioned
        // global). Driven by OpenSSL 3's `@@OPENSSL_3.0.0` exports, validated here
        // against glibc's `GLIBC_2.17` aarch64 baseline.
        let versioned = image.imports.iter().any(|import| import.version.is_some());
        let mut version_needs: Vec<(usize, String)> = Vec::new();
        let mut import_versym: Vec<u16> = Vec::with_capacity(image.imports.len());
        for import in &image.imports {
            match &import.version {
                Some(version) => {
                    let library_index = libraries
                        .iter()
                        .position(|library| library == &import.library)
                        .expect("import library is in the library list");
                    let index = version_needs
                        .iter()
                        .position(|(lib, ver)| *lib == library_index && ver == version)
                        .unwrap_or_else(|| {
                            version_needs.push((library_index, version.clone()));
                            version_needs.len() - 1
                        });
                    import_versym.push((index + 2) as u16);
                }
                None => import_versym.push(1),
            }
        }
        let mut version_string_offsets: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (_, version) in &version_needs {
            if !version_string_offsets.contains_key(version) {
                version_string_offsets.insert(version.clone(), dynstr.len());
                dynstr.extend_from_slice(version.as_bytes());
                dynstr.push(0);
            }
        }

        // DT_RUNPATH (plan-46-D §4.2): the loader search path for `dlopen`ing a
        // vendored native library. Appended last so its offset is stable, and only
        // when the build actually vendors something — a build with no vendor
        // libraries grows `dynstr` by nothing and emits no tag, staying
        // byte-identical.
        //
        // `DT_STRSZ` is computed as `dynstr.len()` and `dynsym_offset` derives from
        // `dynstr_offset + dynstr.len()`, so growing `dynstr` propagates through
        // the layout with no manual fixups.
        // `$ORIGIN` is written verbatim — the loader expands it, not the build.
        let runpath_offset = runpath_string(image).map(|path| {
            let offset = dynstr.len();
            dynstr.extend_from_slice(path.as_bytes());
            dynstr.push(0);
            offset
        });

        let dynstr_offset = data_base_offset;
        let dynsym_offset = align(dynstr_offset + dynstr.len(), 8);
        let dynsym_size = (image.imports.len() + 1) * 24;
        let hash_offset = align(dynsym_offset + dynsym_size, 8);
        let hash_size = (2 + 1 + image.imports.len() + 1) * 4;
        let rela_offset = align(hash_offset + hash_size, 8);
        // .rela.dyn holds the import GLOB_DAT/JUMP_SLOT bindings first, then one
        // R_*_RELATIVE per DT_INIT_ARRAY entry that biases its absolute function
        // pointer into the PIE (bug-186). DT_RELA/DT_RELASZ cover the whole table;
        // DT_JMPREL/DT_PLTRELSZ cover only the leading import portion.
        let import_rela_size = image.imports.len() * 24;
        let rela_size = import_rela_size + image.initializers.len() * 24;
        let got_offset = align(rela_offset + rela_size, 8);
        let got_size = image.imports.len() * 8;
        // `.gnu.version` (one Elf64_Half per dynsym) and `.gnu.version_r` follow
        // the GOT, before the dynamic table, only when versioning is active.
        let versym_offset = align(got_offset + got_size, 8);
        let versym_size = if versioned {
            (image.imports.len() + 1) * 2
        } else {
            0
        };
        // Group needs by library to size the Verneed/Vernaux records.
        let mut needs_by_library: Vec<(usize, Vec<(String, usize)>)> = Vec::new();
        for (global, (library_index, version)) in version_needs.iter().enumerate() {
            let entry = needs_by_library
                .iter_mut()
                .find(|(lib, _)| lib == library_index);
            match entry {
                Some((_, versions)) => versions.push((version.clone(), global + 2)),
                None => {
                    needs_by_library.push((*library_index, vec![(version.clone(), global + 2)]))
                }
            }
        }
        let verneed_offset = align(versym_offset + versym_size, 8);
        let verneed_size = if versioned {
            needs_by_library.len() * 16 + version_needs.len() * 16
        } else {
            0
        };
        // Load-time initializers (plan-linker.md §5.3/§6.4): an array of absolute
        // text addresses the loader runs after relocation and before the entry.
        let init_array_offset = align(verneed_offset + verneed_size, 8);
        let init_array_size = image.initializers.len() * 8;
        let dynamic_offset = align(init_array_offset + init_array_size, 8);
        let dynamic_count = libraries.len()
            + 14
            + if versioned { 3 } else { 0 }
            + if image.initializers.is_empty() { 0 } else { 2 }
            // plan-46-D §4.2: one DT_RUNPATH tag for a vendoring build. Counted
            // here as well as emitted below — the `expected_dynamic_size` check
            // after the tag block is what keeps the two in lockstep.
            + usize::from(runpath_offset.is_some());
        let dynamic_size = dynamic_count * 16;
        let data_offset = align(
            TEXT_FILE_OFFSET + image.text.len() + image.imports.len() * 12,
            PAGE_SIZE,
        );
        // PIE: file-relative (base 0); the loader biases GOT/.rela/DT_* itself.
        let data_vmaddr = DYN_IMAGE_BASE + data_offset as u64;

        let mut bytes = vec![0; dynstr_offset - payload_start];
        bytes.extend_from_slice(&dynstr);
        bytes.resize(dynsym_offset - payload_start, 0);
        bytes.resize(bytes.len() + 24, 0);
        for (index, import) in image.imports.iter().enumerate() {
            put_u32(&mut bytes, symbol_name_offsets[index] as u32);
            // st_info: GLOBAL OBJECT (0x11) for a data global, GLOBAL FUNC (0x12)
            // for a function.
            bytes.push(match import.kind {
                ImportKind::Data => 0x11,
                ImportKind::Function => 0x12,
            });
            bytes.push(0);
            put_u16(&mut bytes, 0);
            put_u64(&mut bytes, 0);
            put_u64(&mut bytes, 0);
        }

        // SysV `DT_HASH`: nbucket, nchain, one bucket, then `nchain` chain words.
        // Symbol 0 is the null symbol, so `chain[0]` is unused and must be 0; the
        // bucket starts the list at symbol 1 and each `chain[i]` links to the next
        // symbol, with 0 (STN_UNDEF) terminating it. Writing the first link into
        // `chain[0]` shifted every entry down one slot, so a by-name lookup of the
        // second and later symbols walked past its own entry.
        bytes.resize(hash_offset - payload_start, 0);
        put_u32(&mut bytes, 1);
        put_u32(&mut bytes, (image.imports.len() + 1) as u32);
        put_u32(&mut bytes, if image.imports.is_empty() { 0 } else { 1 });
        put_u32(&mut bytes, 0); // chain[0] — the null symbol
        for index in 1..=image.imports.len() {
            let next = if index == image.imports.len() {
                0
            } else {
                index + 1
            };
            put_u32(&mut bytes, next as u32);
        }

        bytes.resize(rela_offset - payload_start, 0);
        for (index, import) in image.imports.iter().enumerate() {
            let symbol_index = index + 1;
            // GLOB_DAT binds a data global's GOT slot to the symbol's address;
            // JUMP_SLOT binds a function's GOT slot for its call stub
            // (plan-linker.md §6.1).
            let reloc_type = match (arch, import.kind) {
                ("x86_64", ImportKind::Data) => R_X86_64_GLOB_DAT,
                ("x86_64", ImportKind::Function) => R_X86_64_JUMP_SLOT,
                ("riscv64", ImportKind::Data) => R_RISCV_64,
                ("riscv64", ImportKind::Function) => R_RISCV_JUMP_SLOT,
                (_, ImportKind::Data) => R_AARCH64_GLOB_DAT,
                (_, ImportKind::Function) => R_AARCH64_JUMP_SLOT,
            };
            put_u64(
                &mut bytes,
                data_vmaddr + got_offset as u64 + (index * 8) as u64,
            );
            put_u64(
                &mut bytes,
                ((symbol_index as u64) << 32) | reloc_type as u64,
            );
            put_u64(&mut bytes, 0);
        }
        // One R_*_RELATIVE per DT_INIT_ARRAY slot: *(base + r_offset) = base +
        // r_addend, biasing the absolute initializer pointer for the PIE slide
        // (bug-186). Symbol index 0 (RELATIVE has no symbol). Emitted after the
        // import bindings; DT_JMPREL's size stays the import-only prefix so the
        // PLT pass never sees these.
        let relative_type = match arch {
            "x86_64" => R_X86_64_RELATIVE,
            "riscv64" => R_RISCV_RELATIVE,
            _ => R_AARCH64_RELATIVE,
        };
        // Each initializer's load-relative address, resolved once: the RELATIVE
        // relocs here and the `.init_array` slots below both carry it.
        let initializer_addends = image
            .initializers
            .iter()
            .map(|name| {
                image
                    .symbols
                    .iter()
                    .find(|symbol| symbol.name == *name && symbol.section == EncodedSection::Text)
                    .map(|symbol| DYN_IMAGE_BASE + TEXT_FILE_OFFSET as u64 + symbol.offset as u64)
                    .ok_or_else(|| {
                        format!("initializer '{name}' does not resolve to a text symbol")
                    })
            })
            .collect::<Result<Vec<u64>, String>>()?;
        for (index, addend) in initializer_addends.iter().enumerate() {
            put_u64(
                &mut bytes,
                data_vmaddr + init_array_offset as u64 + (index * 8) as u64,
            );
            put_u64(&mut bytes, relative_type as u64);
            put_u64(&mut bytes, *addend);
        }

        bytes.resize(got_offset - payload_start, 0);
        bytes.resize(bytes.len() + got_size, 0);

        if versioned {
            // .gnu.version: index 0 (null sym) then one per imported symbol.
            bytes.resize(versym_offset - payload_start, 0);
            put_u16(&mut bytes, 0);
            for value in &import_versym {
                put_u16(&mut bytes, *value);
            }
            // .gnu.version_r: one Verneed per library, one Vernaux per version.
            bytes.resize(verneed_offset - payload_start, 0);
            for (need_index, (library_index, versions)) in needs_by_library.iter().enumerate() {
                let last_need = need_index + 1 == needs_by_library.len();
                put_u16(&mut bytes, 1); // vn_version
                put_u16(&mut bytes, versions.len() as u16); // vn_cnt
                put_u32(&mut bytes, library_offsets[*library_index] as u32); // vn_file
                put_u32(&mut bytes, 16); // vn_aux: first Vernaux follows
                put_u32(
                    &mut bytes,
                    if last_need {
                        0
                    } else {
                        (16 + versions.len() * 16) as u32
                    },
                ); // vn_next
                for (aux_index, (version, version_index)) in versions.iter().enumerate() {
                    let last_aux = aux_index + 1 == versions.len();
                    put_u32(&mut bytes, elf_hash(version.as_bytes())); // vna_hash
                    put_u16(&mut bytes, 0); // vna_flags
                    put_u16(&mut bytes, *version_index as u16); // vna_other
                    put_u32(&mut bytes, version_string_offsets[version] as u32); // vna_name
                    put_u32(&mut bytes, if last_aux { 0 } else { 16 }); // vna_next
                }
            }
        }

        if !image.initializers.is_empty() {
            bytes.resize(init_array_offset - payload_start, 0);
            for addend in &initializer_addends {
                // PIE: store the file-relative addend; the R_*_RELATIVE reloc
                // emitted in the .rela table biases it to base + addend at load
                // time (bug-186). The value here is also the addend the loader
                // reads, so pre-filling it keeps REL/RELA implementations that
                // add-in-place correct too.
                put_u64(&mut bytes, *addend);
            }
        }

        bytes.resize(dynamic_offset - payload_start, 0);
        for offset in library_offsets {
            put_dynamic(&mut bytes, 1, offset as u64);
        }
        // DT_RUNPATH is tag 29, not DT_RPATH (15): RPATH is deprecated and,
        // unlike RUNPATH, cannot be overridden by LD_LIBRARY_PATH. Emitted only
        // for a build that vendors something (plan-46-D §4.2).
        if let Some(offset) = runpath_offset {
            put_dynamic(&mut bytes, 29, offset as u64);
        }
        put_dynamic(&mut bytes, 4, data_vmaddr + hash_offset as u64);
        put_dynamic(&mut bytes, 5, data_vmaddr + dynstr_offset as u64);
        put_dynamic(&mut bytes, 6, data_vmaddr + dynsym_offset as u64);
        put_dynamic(&mut bytes, 10, dynstr.len() as u64);
        put_dynamic(&mut bytes, 11, 24);
        put_dynamic(&mut bytes, 3, data_vmaddr + got_offset as u64);
        put_dynamic(&mut bytes, 7, data_vmaddr + rela_offset as u64);
        put_dynamic(&mut bytes, 8, rela_size as u64);
        put_dynamic(&mut bytes, 9, 24);
        put_dynamic(&mut bytes, 20, 7);
        put_dynamic(&mut bytes, 23, data_vmaddr + rela_offset as u64);
        // DT_PLTRELSZ is the import (PLT/GOT) prefix only; the R_*_RELATIVE tail is
        // covered by DT_RELASZ above, not the JMPREL pass (bug-186).
        put_dynamic(&mut bytes, 2, import_rela_size as u64);
        put_dynamic(&mut bytes, 30, 8);
        if versioned {
            // DT_VERSYM, DT_VERNEED, DT_VERNEEDNUM.
            put_dynamic(&mut bytes, 0x6fff_fff0, data_vmaddr + versym_offset as u64);
            put_dynamic(&mut bytes, 0x6fff_fffe, data_vmaddr + verneed_offset as u64);
            put_dynamic(&mut bytes, 0x6fff_ffff, needs_by_library.len() as u64);
        }
        if !image.initializers.is_empty() {
            // DT_INIT_ARRAY, DT_INIT_ARRAYSZ.
            put_dynamic(&mut bytes, 25, data_vmaddr + init_array_offset as u64);
            put_dynamic(&mut bytes, 27, init_array_size as u64);
        }
        put_dynamic(&mut bytes, 0, 0);

        let expected_dynamic_size = bytes.len() - (dynamic_offset - payload_start);
        if expected_dynamic_size != dynamic_size {
            return Err(format!(
                "internal Linux {} dynamic table size mismatch: expected {dynamic_size}, got {expected_dynamic_size}",
                flavor.suffix()
            ));
        }

        Ok(Self {
            bytes,
            dynamic_offset: dynamic_offset - payload_start,
            dynamic_size,
            dynstr_offset: dynstr_offset - payload_start,
            dynstr_size: dynstr.len(),
        })
    }
}

/// The `DT_RUNPATH` string this image contributes to `.dynstr`, or `None` when
/// the build vendors nothing (plan-46-D §4.2).
///
/// **The single owner of this string.** `.dynstr`'s contents and
/// [`dynamic_prefix_size`] are two independent computations of the same layout —
/// the latter derives the GOT offset baked into every import stub, so if it
/// under-counts `.dynstr` by even one byte, every stub jumps through the wrong
/// GOT slot and the program segfaults on its first imported call. Both call this.
///
/// Multiple entries are one colon-separated string, per the ELF convention.
fn runpath_string(image: &EncodedImage) -> Option<String> {
    if image.rpaths.is_empty() {
        None
    } else {
        Some(image.rpaths.join(":"))
    }
}

/// The `.dynstr` bytes a `DT_RUNPATH` occupies: the string plus its NUL.
fn runpath_dynstr_len(image: &EncodedImage) -> usize {
    runpath_string(image).map_or(0, |path| path.len() + 1)
}

pub(in crate::os::linux::link) fn dynamic_prefix_size(image: &EncodedImage) -> usize {
    let mut libraries = Vec::<&str>::new();
    for import in &image.imports {
        if !libraries.contains(&import.library.as_str()) {
            libraries.push(import.library.as_str());
        }
    }
    // Distinct version strings also live in `.dynstr` (plan-linker.md §6.2); they
    // must be counted here so the GOT offset baked into each stub matches the
    // offset `DynamicPayload::build` computes after appending them.
    let mut version_strings = Vec::<&str>::new();
    for import in &image.imports {
        if let Some(version) = &import.version {
            if !version_strings.contains(&version.as_str()) {
                version_strings.push(version.as_str());
            }
        }
    }
    let dynstr_len = 1
        + libraries
            .iter()
            .map(|library| library.len() + 1)
            .sum::<usize>()
        + image
            .imports
            .iter()
            .map(|import| import.symbol.len() + 1)
            .sum::<usize>()
        + version_strings
            .iter()
            .map(|version| version.len() + 1)
            .sum::<usize>()
        // The DT_RUNPATH string is in `.dynstr` too (plan-46-D §4.2). Omitting it
        // here under-counts `.dynstr`, which shifts every offset derived from it
        // — including the GOT offset baked into each import stub — and the program
        // segfaults on its first imported call, long before it reaches the
        // `dlopen` the runpath exists for.
        + runpath_dynstr_len(image);
    // Must match DynamicPayload::build's page-aligned payload base (bug-263): the
    // GOT offset baked into each import stub has to land where the GOT actually
    // sits, or the first imported call jumps through the wrong address.
    let dynstr_offset = align(image.data.len(), PAGE_SIZE);
    let dynsym_offset = align(dynstr_offset + dynstr_len, 8);
    let dynsym_size = (image.imports.len() + 1) * 24;
    let hash_offset = align(dynsym_offset + dynsym_size, 8);
    let hash_size = (2 + 1 + image.imports.len() + 1) * 4;
    let rela_offset = align(hash_offset + hash_size, 8);
    // Must match DynamicPayload::build: imports + one R_*_RELATIVE per initializer
    // (bug-186), so the GOT offset baked into each stub stays correct.
    let rela_size = (image.imports.len() + image.initializers.len()) * 24;
    align(rela_offset + rela_size, 8)
}
