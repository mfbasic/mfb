//! plan-66-J message-loop ↔ worker spike (the app-mode unproven premise).
//!
//! Before the full Win32 app-mode floor is built, this hand-assembled PE proves
//! the one thing plan-66 flags as unproven: that a `RegisterClassExW` /
//! `CreateWindowExW` window with a `GetMessageW`/`DispatchMessageW` loop owning
//! the main thread integrates with a `CreateThread` worker that cross-thread
//! `PostMessageW`s a message the `WndProc` handles — the AppKit
//! `performSelectorOnMainThread` / GTK `g_idle_add` analog.
//!
//! The program: `_start` gets the module handle, registers a window class whose
//! `WndProc` is emitted here, creates a visible overlapped window, spawns a worker
//! thread (handed the `HWND`), then runs the message loop. The worker posts a
//! `WM_APP` message to the window and returns. The main thread's loop dispatches
//! that message to `WndProc` — **on the UI thread** — which writes a proof-of-life
//! file (`mfb_spike_proof.txt`, bytes `SPIKE_OK`) and calls `PostQuitMessage(0)`,
//! ending the loop; `_start` then `ExitProcess(0)`s. If the file appears on the
//! box, the cross-thread marshaling premise holds.
//!
//! It is linked GUI-subsystem (Subsystem=2, plan-66-I) so no console attaches.
//! Not a CI assertion of GUI behavior (headless CI can't open a window); the
//! `links_and_is_gui_subsystem` test only proves it *builds* and carries the right
//! header, and `writes_spike_exe_when_env_set` writes it for a box run.

#![cfg(test)]

use crate::arch::image::{
    EncodedImage, EncodedImport, EncodedRelocation, EncodedSection, EncodedSymbol, ImportKind,
};
use std::collections::HashMap;

const KERNEL32: &str = "kernel32.dll";
const USER32: &str = "user32.dll";

/// A tiny byte-level x86-64 assembler that records internal/data/external
/// relocations against the emitted `.text` and resolves local (short) jumps.
struct Asm {
    code: Vec<u8>,
    relocs: Vec<EncodedRelocation>,
    symbols: Vec<EncodedSymbol>,
    /// Local label offsets within `.text` (function-internal jump targets).
    locals: HashMap<String, usize>,
    /// Pending short-jump fixups: (byte offset of the rel8 field, target label).
    fixups: Vec<(usize, String)>,
}

impl Asm {
    fn new() -> Self {
        Asm {
            code: Vec::new(),
            relocs: Vec::new(),
            symbols: Vec::new(),
            locals: HashMap::new(),
            fixups: Vec::new(),
        }
    }

    /// Define a public text symbol at the current offset.
    fn sym(&mut self, name: &str) {
        self.symbols.push(EncodedSymbol {
            name: name.to_string(),
            section: EncodedSection::Text,
            offset: self.code.len(),
        });
    }

    /// Define a function-local jump label at the current offset.
    fn label(&mut self, name: &str) {
        self.locals.insert(name.to_string(), self.code.len());
    }

    fn b(&mut self, bytes: &[u8]) {
        self.code.extend_from_slice(bytes);
    }

    /// `E8 rel32` call to an imported function (resolved to its IAT thunk).
    fn call_import(&mut self, symbol: &str, library: &str) {
        self.b(&[0xE8]);
        let off = self.code.len();
        self.b(&[0, 0, 0, 0]);
        self.relocs.push(EncodedRelocation {
            offset: off,
            target: symbol.to_string(),
            kind: "call_pc32".to_string(),
            binding: "external".to_string(),
            library: Some(library.to_string()),
        });
    }

    /// A RIP-relative `lea`/`mov` whose 4-byte displacement addresses an internal
    /// text symbol (e.g. `lea r8, [rip+worker]`). `opcode` is the instruction up
    /// to (not including) the disp32 field.
    fn rip_internal(&mut self, opcode: &[u8], symbol: &str) {
        self.b(opcode);
        let off = self.code.len();
        self.b(&[0, 0, 0, 0]);
        self.relocs.push(EncodedRelocation {
            offset: off,
            target: symbol.to_string(),
            kind: "call_pc32".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
    }

    /// A RIP-relative `lea` whose disp32 addresses a read-only data symbol.
    fn rip_data(&mut self, opcode: &[u8], symbol: &str) {
        self.b(opcode);
        let off = self.code.len();
        self.b(&[0, 0, 0, 0]);
        self.relocs.push(EncodedRelocation {
            offset: off,
            target: symbol.to_string(),
            kind: "data_pc32".to_string(),
            binding: "data".to_string(),
            library: None,
        });
    }

    /// Short `jmp`/`jcc` (`opcode` = the 1-byte opcode) to a local label.
    fn jshort(&mut self, opcode: u8, label: &str) {
        self.b(&[opcode]);
        let off = self.code.len();
        self.b(&[0]); // rel8 placeholder
        self.fixups.push((off, label.to_string()));
    }

    /// Resolve every short-jump fixup; panics if a target is unknown or a
    /// displacement overflows an `i8` (the spike is small enough that it never
    /// does — a panic would mean a genuine encoding bug).
    fn resolve(&mut self) {
        for (off, label) in &self.fixups {
            let target = *self
                .locals
                .get(label)
                .unwrap_or_else(|| panic!("spike: undefined local label '{label}'"));
            let rel = target as i64 - (*off as i64 + 1);
            let rel = i8::try_from(rel)
                .unwrap_or_else(|_| panic!("spike: rel8 to '{label}' overflows ({rel})"));
            self.code[*off] = rel as u8;
        }
    }
}

/// Build the spike's `EncodedImage`. All strings are read-only data.
fn spike_image() -> EncodedImage {
    // ---- read-only data: UTF-16LE strings + the ASCII proof bytes ----
    fn utf16z(s: &str) -> Vec<u8> {
        let mut v = Vec::new();
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v.extend_from_slice(&0u16.to_le_bytes()); // NUL terminator
        v
    }
    let mut data = Vec::new();
    let mut data_syms: Vec<EncodedSymbol> = Vec::new();
    let mut push_data = |data: &mut Vec<u8>, syms: &mut Vec<EncodedSymbol>, name: &str, bytes: &[u8]| {
        syms.push(EncodedSymbol {
            name: name.to_string(),
            section: EncodedSection::Data,
            offset: data.len(),
        });
        data.extend_from_slice(bytes);
    };
    push_data(&mut data, &mut data_syms, "spike_class", &utf16z("MFBSpike"));
    push_data(&mut data, &mut data_syms, "spike_title", &utf16z("MFB Spike"));
    push_data(
        &mut data,
        &mut data_syms,
        "spike_path",
        &utf16z("mfb_spike_proof.txt"),
    );
    let proof = b"SPIKE_OK";
    push_data(&mut data, &mut data_syms, "spike_proof", proof);
    let proof_len = proof.len() as u32;

    let mut a = Asm::new();

    // ===== _start (PE entry; sp % 16 == 8 on arrival, like a normal call) =====
    a.sym("_start");
    // sub rsp, 0xF8  (0xF8 % 16 == 8 -> restores 16-alignment before a call)
    a.b(&[0x48, 0x81, 0xEC, 0xF8, 0x00, 0x00, 0x00]);
    // Frame slots: shadow [0x00..0x20], outgoing stack args [0x20..0x60],
    // WNDCLASSEXW [0x60..0xB0], MSG [0xB0..0xE0], hInstance [0xE0], hwnd [0xE8].

    // hInstance = GetModuleHandleW(NULL)
    a.b(&[0x48, 0x31, 0xC9]); // xor rcx, rcx
    a.call_import("GetModuleHandleW", KERNEL32);
    a.b(&[0x48, 0x89, 0x84, 0x24, 0xE0, 0x00, 0x00, 0x00]); // mov [rsp+0xE0], rax

    // Zero the 80-byte WNDCLASSEXW: lea rdi,[rsp+0x60]; xor eax,eax; mov ecx,10; rep stosq
    a.b(&[0x48, 0x8D, 0xBC, 0x24, 0x60, 0x00, 0x00, 0x00]); // lea rdi, [rsp+0x60]
    a.b(&[0x31, 0xC0]); // xor eax, eax
    a.b(&[0xB9, 0x0A, 0x00, 0x00, 0x00]); // mov ecx, 10
    a.b(&[0xF3, 0x48, 0xAB]); // rep stosq

    // cbSize = 80
    a.b(&[0xC7, 0x84, 0x24, 0x60, 0x00, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00]); // mov dword [rsp+0x60], 80
    // lpfnWndProc = &WndProc  (offset 0x68)
    a.rip_internal(&[0x48, 0x8D, 0x05], "WndProc"); // lea rax, [rip+WndProc]
    a.b(&[0x48, 0x89, 0x84, 0x24, 0x68, 0x00, 0x00, 0x00]); // mov [rsp+0x68], rax
    // hInstance (offset 0x78)
    a.b(&[0x48, 0x8B, 0x84, 0x24, 0xE0, 0x00, 0x00, 0x00]); // mov rax, [rsp+0xE0]
    a.b(&[0x48, 0x89, 0x84, 0x24, 0x78, 0x00, 0x00, 0x00]); // mov [rsp+0x78], rax
    // lpszClassName = &spike_class  (offset 0xA0)
    a.rip_data(&[0x48, 0x8D, 0x05], "spike_class"); // lea rax, [rip+spike_class]
    a.b(&[0x48, 0x89, 0x84, 0x24, 0xA0, 0x00, 0x00, 0x00]); // mov [rsp+0xA0], rax

    // RegisterClassExW(&wndclass)
    a.b(&[0x48, 0x8D, 0x8C, 0x24, 0x60, 0x00, 0x00, 0x00]); // lea rcx, [rsp+0x60]
    a.call_import("RegisterClassExW", USER32);

    // CreateWindowExW(0, &class, &title, WS_OVERLAPPEDWINDOW|WS_VISIBLE, CW_USEDEFAULT,
    //                 CW_USEDEFAULT, 400, 300, NULL, NULL, hInstance, NULL)
    a.b(&[0x31, 0xC9]); // xor ecx, ecx  (dwExStyle)
    a.rip_data(&[0x48, 0x8D, 0x15], "spike_class"); // lea rdx, [rip+spike_class]
    a.rip_data(&[0x4C, 0x8D, 0x05], "spike_title"); // lea r8, [rip+spike_title]
    a.b(&[0x41, 0xB9, 0x00, 0x00, 0xCF, 0x10]); // mov r9d, 0x10CF0000
    a.b(&[0xC7, 0x44, 0x24, 0x20, 0x00, 0x00, 0x00, 0x80]); // mov dword [rsp+0x20], CW_USEDEFAULT
    a.b(&[0xC7, 0x44, 0x24, 0x28, 0x00, 0x00, 0x00, 0x80]); // mov dword [rsp+0x28], CW_USEDEFAULT
    a.b(&[0xC7, 0x44, 0x24, 0x30, 0x90, 0x01, 0x00, 0x00]); // mov dword [rsp+0x30], 400
    a.b(&[0xC7, 0x44, 0x24, 0x38, 0x2C, 0x01, 0x00, 0x00]); // mov dword [rsp+0x38], 300
    a.b(&[0x48, 0xC7, 0x44, 0x24, 0x40, 0x00, 0x00, 0x00, 0x00]); // mov qword [rsp+0x40], 0 (parent)
    a.b(&[0x48, 0xC7, 0x44, 0x24, 0x48, 0x00, 0x00, 0x00, 0x00]); // mov qword [rsp+0x48], 0 (menu)
    a.b(&[0x48, 0x8B, 0x84, 0x24, 0xE0, 0x00, 0x00, 0x00]); // mov rax, [rsp+0xE0]
    a.b(&[0x48, 0x89, 0x44, 0x24, 0x50]); // mov [rsp+0x50], rax (hInstance)
    a.b(&[0x48, 0xC7, 0x44, 0x24, 0x58, 0x00, 0x00, 0x00, 0x00]); // mov qword [rsp+0x58], 0 (lpParam)
    a.call_import("CreateWindowExW", USER32);
    a.b(&[0x48, 0x89, 0x84, 0x24, 0xE8, 0x00, 0x00, 0x00]); // mov [rsp+0xE8], rax (hwnd)

    // CreateThread(NULL, 0, &worker, hwnd, 0, NULL)
    a.b(&[0x31, 0xC9]); // xor ecx, ecx
    a.b(&[0x31, 0xD2]); // xor edx, edx
    a.rip_internal(&[0x4C, 0x8D, 0x05], "worker"); // lea r8, [rip+worker]
    a.b(&[0x4C, 0x8B, 0x8C, 0x24, 0xE8, 0x00, 0x00, 0x00]); // mov r9, [rsp+0xE8] (hwnd)
    a.b(&[0x48, 0xC7, 0x44, 0x24, 0x20, 0x00, 0x00, 0x00, 0x00]); // mov qword [rsp+0x20], 0
    a.b(&[0x48, 0xC7, 0x44, 0x24, 0x28, 0x00, 0x00, 0x00, 0x00]); // mov qword [rsp+0x28], 0
    a.call_import("CreateThread", KERNEL32);

    // Message loop.
    a.label("loop");
    a.b(&[0x48, 0x8D, 0x8C, 0x24, 0xB0, 0x00, 0x00, 0x00]); // lea rcx, [rsp+0xB0] (&msg)
    a.b(&[0x31, 0xD2]); // xor edx, edx (hWnd = NULL)
    a.b(&[0x45, 0x31, 0xC0]); // xor r8d, r8d
    a.b(&[0x45, 0x31, 0xC9]); // xor r9d, r9d
    a.call_import("GetMessageW", USER32);
    a.b(&[0x85, 0xC0]); // test eax, eax
    a.jshort(0x7E, "done"); // jle done  (0 = WM_QUIT, -1 = error)
    a.b(&[0x48, 0x8D, 0x8C, 0x24, 0xB0, 0x00, 0x00, 0x00]); // lea rcx, [rsp+0xB0]
    a.call_import("TranslateMessage", USER32);
    a.b(&[0x48, 0x8D, 0x8C, 0x24, 0xB0, 0x00, 0x00, 0x00]); // lea rcx, [rsp+0xB0]
    a.call_import("DispatchMessageW", USER32);
    a.jshort(0xEB, "loop"); // jmp loop
    a.label("done");
    a.b(&[0x31, 0xC9]); // xor ecx, ecx (exit code 0)
    a.call_import("ExitProcess", KERNEL32);
    a.b(&[0xCC]); // int3 (never reached)

    // ===== worker(LPVOID hwnd in rcx) =====
    a.sym("worker");
    a.b(&[0x48, 0x83, 0xEC, 0x28]); // sub rsp, 0x28 (shadow + align)
    // PostMessageW(hwnd, WM_APP, 0, 0) — rcx already holds hwnd.
    a.b(&[0xBA, 0x00, 0x80, 0x00, 0x00]); // mov edx, 0x8000 (WM_APP)
    a.b(&[0x45, 0x31, 0xC0]); // xor r8d, r8d
    a.b(&[0x45, 0x31, 0xC9]); // xor r9d, r9d
    a.call_import("PostMessageW", USER32);
    a.b(&[0x31, 0xC0]); // xor eax, eax (return 0)
    a.b(&[0x48, 0x83, 0xC4, 0x28]); // add rsp, 0x28
    a.b(&[0xC3]); // ret

    // ===== WndProc(hwnd rcx, msg rdx, wParam r8, lParam r9) =====
    a.sym("WndProc");
    a.b(&[0x48, 0x83, 0xEC, 0x48]); // sub rsp, 0x48
    a.b(&[0x81, 0xFA, 0x00, 0x80, 0x00, 0x00]); // cmp edx, 0x8000 (WM_APP)
    a.jshort(0x74, "wnd_app"); // je wnd_app
    a.b(&[0x81, 0xFA, 0x02, 0x00, 0x00, 0x00]); // cmp edx, 0x0002 (WM_DESTROY)
    a.jshort(0x74, "wnd_destroy"); // je wnd_destroy
    // default: DefWindowProcW(hwnd, msg, wParam, lParam) — args untouched.
    a.call_import("DefWindowProcW", USER32);
    a.b(&[0x48, 0x83, 0xC4, 0x48]); // add rsp, 0x48
    a.b(&[0xC3]); // ret

    a.label("wnd_app");
    // CreateFileW(&path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL)
    a.rip_data(&[0x48, 0x8D, 0x0D], "spike_path"); // lea rcx, [rip+spike_path]
    a.b(&[0xBA, 0x00, 0x00, 0x00, 0x40]); // mov edx, 0x40000000 (GENERIC_WRITE)
    a.b(&[0x45, 0x31, 0xC0]); // xor r8d, r8d (share 0)
    a.b(&[0x45, 0x31, 0xC9]); // xor r9d, r9d (lpSecurityAttributes NULL)
    a.b(&[0xC7, 0x44, 0x24, 0x20, 0x02, 0x00, 0x00, 0x00]); // mov dword [rsp+0x20], 2 (CREATE_ALWAYS)
    a.b(&[0xC7, 0x44, 0x24, 0x28, 0x80, 0x00, 0x00, 0x00]); // mov dword [rsp+0x28], 0x80 (NORMAL)
    a.b(&[0x48, 0xC7, 0x44, 0x24, 0x30, 0x00, 0x00, 0x00, 0x00]); // mov qword [rsp+0x30], 0 (hTemplate)
    a.call_import("CreateFileW", KERNEL32);
    a.b(&[0x48, 0x89, 0x44, 0x24, 0x38]); // mov [rsp+0x38], rax (handle)
    // WriteFile(handle, &proof, len, &written, NULL)
    a.b(&[0x48, 0x89, 0xC1]); // mov rcx, rax
    a.rip_data(&[0x48, 0x8D, 0x15], "spike_proof"); // lea rdx, [rip+spike_proof]
    a.b(&[0x41, 0xB8]); // mov r8d, imm32
    a.b(&proof_len.to_le_bytes());
    a.b(&[0x4C, 0x8D, 0x8C, 0x24, 0x40, 0x00, 0x00, 0x00]); // lea r9, [rsp+0x40] (&written)
    a.b(&[0x48, 0xC7, 0x44, 0x24, 0x20, 0x00, 0x00, 0x00, 0x00]); // mov qword [rsp+0x20], 0 (overlapped)
    a.call_import("WriteFile", KERNEL32);
    // CloseHandle(handle)
    a.b(&[0x48, 0x8B, 0x4C, 0x24, 0x38]); // mov rcx, [rsp+0x38]
    a.call_import("CloseHandle", KERNEL32);
    // PostQuitMessage(0)
    a.b(&[0x31, 0xC9]); // xor ecx, ecx
    a.call_import("PostQuitMessage", USER32);
    a.b(&[0x31, 0xC0]); // xor eax, eax
    a.b(&[0x48, 0x83, 0xC4, 0x48]); // add rsp, 0x48
    a.b(&[0xC3]); // ret

    a.label("wnd_destroy");
    a.b(&[0x31, 0xC9]); // xor ecx, ecx
    a.call_import("PostQuitMessage", USER32);
    a.b(&[0x31, 0xC0]); // xor eax, eax
    a.b(&[0x48, 0x83, 0xC4, 0x48]); // add rsp, 0x48
    a.b(&[0xC3]); // ret

    a.resolve();

    let mut symbols = a.symbols;
    symbols.extend(data_syms);

    let imports: Vec<(&str, &str)> = vec![
        (KERNEL32, "GetModuleHandleW"),
        (KERNEL32, "CreateThread"),
        (KERNEL32, "ExitProcess"),
        (KERNEL32, "CreateFileW"),
        (KERNEL32, "WriteFile"),
        (KERNEL32, "CloseHandle"),
        (USER32, "RegisterClassExW"),
        (USER32, "CreateWindowExW"),
        (USER32, "GetMessageW"),
        (USER32, "TranslateMessage"),
        (USER32, "DispatchMessageW"),
        (USER32, "PostMessageW"),
        (USER32, "PostQuitMessage"),
        (USER32, "DefWindowProcW"),
    ];

    EncodedImage {
        text: a.code,
        rodata_size: data.len(),
        data,
        symbols,
        relocations: a.relocs,
        imports: imports
            .into_iter()
            .map(|(lib, sym)| EncodedImport {
                library: lib.to_string(),
                symbol: sym.to_string(),
                kind: ImportKind::Function,
                version: None,
            })
            .collect(),
        entry: "_start".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    }
}

fn le_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn le_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

#[test]
fn links_and_is_gui_subsystem() {
    // gui = true -> Subsystem=2, the plan-66-I app-mode toggle.
    let bytes = super::write_executable(&spike_image(), true, None, None).expect("link spike");
    assert_eq!(&bytes[0..2], b"MZ");
    let e_lfanew = le_u32(&bytes, 0x3C) as usize;
    let opt = e_lfanew + 4 + 20;
    assert_eq!(le_u16(&bytes, opt), 0x020B, "PE32+");
    assert_eq!(le_u16(&bytes, opt + 68), 2, "Subsystem = WINDOWS_GUI");
    // Two import DLLs (kernel32 + user32) => an .idata section is present.
    let n = le_u16(&bytes, e_lfanew + 6) as usize;
    let sect_table = e_lfanew + 4 + 20 + 240;
    let mut names: Vec<String> = Vec::new();
    for i in 0..n {
        let s = sect_table + i * 40;
        let name: String = bytes[s..s + 8]
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as char)
            .collect();
        names.push(name);
    }
    assert!(names.iter().any(|n| n == ".idata"), "imports present: {names:?}");
    assert!(names.iter().any(|n| n == ".text"));
    assert!(names.iter().any(|n| n == ".rdata"), "strings present: {names:?}");
}

/// Dev harness (not a CI assertion): with `MFB_SPIKE_OUT` set, write the GUI
/// spike `.exe` there for a real Windows-box run (plan-66-J premise proof).
#[test]
fn writes_spike_exe_when_env_set() {
    let Ok(path) = std::env::var("MFB_SPIKE_OUT") else {
        return;
    };
    let bytes = super::write_executable(&spike_image(), true, None, None).expect("link spike");
    std::fs::write(&path, &bytes).expect("write spike.exe");
    eprintln!("wrote {} bytes to {path}", bytes.len());
}
