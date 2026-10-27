### 1. Overview never mentions six of the package's 26 functions, and its framing excludes them
UNIT:      man-pkg:os
PAGE:      package-wide (overview Description)
CATEGORY:  overview-mismatch
CLAIM:     "The introspection calls are all nullary and read-only." and the opening paragraph's list "(command-line arguments, process id, executable path, OS family, CPU architecture, host and user names, and CPU count)"
VERDICT:   missing
EVIDENCE:  `sed -n 69,121p src/codegen/builtins/os/mod.rs | grep -o 'os::[a-zA-Z]*' | sort -u` lists 20 names; the function table has 26. Absent from the Description: `os::exit`, `os::sleep`, `os::uptime`, `os::version`, `os::isAdmin`, `os::getConfigOr`. Three of the six contradict the overview's own framing: `os::exit` and `os::sleep` are not read-only and `os::getConfigOr` is neither nullary nor runtime introspection (it is a build-time literal substitution). A developer arriving with "how do I pause / exit / read a build setting" gets no pointer from the prose.
SUGGESTED: Add a short paragraph after the introspection paragraph: "Three calls act rather than report: `os::sleep` blocks the calling thread (a cancellation point inside a worker), `os::exit` ends the process with a status code, and `os::getConfigOr` reads a setting from `project.json` fixed at build time. `os::version`, `os::uptime` and `os::isAdmin` report the host OS version, its uptime in seconds, and whether the process is elevated." Also add version/uptime/isAdmin to the parenthetical list in paragraph 1 and drop "all" from "all nullary and read-only" or scope it to the listed calls.

### 2. `os::name` documents two values; the package targets three
UNIT:      man-pkg:os
PAGE:      os::name and the overview
CATEGORY:  consistency
CLAIM:     overview: `os::name and os::arch are compile-time constants selected by the build target ("macos"/"linux"; ...)`; name page: `returns the operating-system family of the build target: "macos" or "linux"`
VERDICT:   inconsistent
EVIDENCE:  `grep -n '"windows"' src/codegen/builtins/os/*.rs` → `gen_shared.rs:109: PlatformFamily::Windows => "windows"` and `func_name.rs:9` doc comment lists `"macos"/"linux"/"windows"`. Every sibling path page in this unit has a Windows row (`appCachePath`, `appDataPath`, `appResourcePath` "windows-x86_64 included", `executablePath` "on Windows it calls GetModuleFileNameW"), so the unit tells the reader Windows is a target and then tells them `os::name` cannot say so. A developer writing `IF os::name() = "windows"` has no documented value to compare against.
SUGGESTED: Both places: `"macos"`, `"linux"` or `"windows"`. Same fix on `os::arch` if a Windows build reports `x86_64` (it does per the appResourcePath page), so no change needed there.

### 3. The overview states a name rule that four of the five environment calls do not enforce
UNIT:      man-pkg:os
PAGE:      package-wide (overview paragraph 4; getEnv, getEnvOr, hasEnv, unsetEnv parameter descriptions)
CATEGORY:  consistency
CLAIM:     "A name must be non-empty and, like a value, may not contain an embedded NUL byte or, for a name, an = — the host requires NUL-terminated strings and uses = to separate a name from its value." and each read page's "Must be non-empty and free of embedded NUL bytes."
VERDICT:   misleading
EVIDENCE:  Probe `/tmp/plan-125-scratch/B-recheck/man-pkg-os/proj` (built with `mfb build proj`, ran `./proj/build/proj.out`) after `os::setEnv("ZZ", "a=b=c")`:
```
empty getEnv raised 77050004      (ErrNotFound, not ErrInvalidArgument)
empty hasEnv=FALSE
empty unsetEnv ok
eq getEnv=a=b=c                   (os::getEnv("ZZ=a") returns ZZ's value)
eq hasEnv=TRUE
eq unsetEnv ok
after unsetEnv(ZZ=a): hasEnv ZZ=TRUE   (silently did nothing)
hasEnv(HOME=)=TRUE
empty setEnv raised 77050002
eq setEnv raised 77050002
```
Only `os::setEnv` declares and raises `ErrInvalidArgument` (`func_set_env.rs:174`); `func_unset_env.rs:122`, `func_get_env.rs:85`, `func_has_env.rs:150` declare no such error, and the comment at `func_unset_env.rs:37` says "unsetenv cannot fail", so the host's EINVAL is discarded. The overview presents one rule for the package; the reader cannot tell from it that a bad name raises on write, reads as "not found" or as a prefix match on read, and is a silent no-op on unset.
SUGGESTED: Overview: "`os::setEnv` rejects an empty name or one containing `=` with ErrInvalidArgument. The read calls and `os::unsetEnv` do not validate the name: an empty or `=`-bearing name is simply never found by `os::getEnv`/`os::getEnvOr`/`os::hasEnv` on most hosts (on macOS and glibc a name such as `ZZ=a` can match `ZZ`'s entry), and `os::unsetEnv` of such a name removes nothing." Mirror one sentence on each of the four pages. NUL handling is UNVERIFIED: I found no way to build a String containing U+0000 in a probe (no `chr`; `strings::fromScalars` needs a Scalar literal I could not produce), so I did not test the NUL clause.

### 4. The Windows example paths on the sibling directory pages do not agree with their own tables
UNIT:      man-pkg:os
PAGE:      os::appCachePath, os::userDocumentsPath
CATEGORY:  consistency
CLAIM:     appCachePath: "so on Windows it reads C:\Users\me\AppData\Roaming/MyApp/save.dat"; userDocumentsPath: "so on Windows it reads like C:\Users\me\\Documents/notes.txt"
VERDICT:   inconsistent
EVIDENCE:  `grep -n "Roaming/MyApp" src/codegen/builtins/os/*.rs` → the identical sentence in `func_app_data_path.rs:52` and `func_app_cache_path.rs:53`. The appCachePath table two paragraphs above says `<Local AppData>\<name>` (`AppData\Local`), so the page's own example lands in the appData directory. `func_user_documents_path.rs:69` has a literal `me\\Documents` which renders as a doubled backslash; the sibling `userHomePath` renders `C:\Users\me/notes.txt` with a single one.
SUGGESTED: appCachePath: `C:\Users\me\AppData\Local\MyApp/thumbs/a.png` (matching its parameter example). userDocumentsPath: `C:\Users\me\Documents/notes.txt`.

### 5. `os::prog` lists ErrInvalidArgument with no condition anywhere in the unit
UNIT:      man-pkg:os
PAGE:      os::prog
CATEGORY:  coverage
CLAIM:     Errors table row `77050002 ErrInvalidArgument` with Description text that names only ErrEncoding and ErrOutOfMemory
VERDICT:   missing
EVIDENCE:  `sed -n 80,95p src/codegen/builtins/os/func_prog.rs`: the `empty` label raises `ErrInvalidArgument`, i.e. the host supplied an empty argv[0]. The sibling `os::args` page, which reads the same argv, declares no such error, so a reader comparing the two pages cannot infer the cause. Probe above printed `prog=./proj/build/proj.out`, confirming the normal path.
SUGGESTED: Add to the Description: "If the host supplied an empty program name (argv[0] is ""), os::prog raises ErrInvalidArgument."
