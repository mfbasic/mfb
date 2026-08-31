//! The built-in `fs` package (plan-72 migration).
//!
//! `fs` is filesystem access: whole-file reads/writes, an opaque owned `File`
//! handle for streaming I/O, purely syntactic path-string manipulation, directory
//! creation/listing, and existence tests. It is a **native, plain-syscall**
//! package — the second native package (after `process`) migrated onto the
//! clean-room registry (`crate::codegen::registry`).
//!
//! Every syscall member (36 of them) is `Body::abi_function` (crypto/io's
//! clean-room shape): each `func_*.rs` owns its `lower_<name>` body, which calls its
//! own per-member `lower_fs_*_helper` emitter in the `gen_*` backends (each branching
//! on `platform.family()` internally) and finalizes. The five `path*` string members are
//! `Body::abi_inline` (the self-lowering successor to the former `common` slot),
//! lowering at the call site through the relocated `impl CodeBuilder` path emitters.
//! Unlike `os.resourcePath`, `fs` needs no build context.
//!
//! The opaque `File` handle is the one owned resource (`add_resource`); its close
//! op is the public `fs.close`. Runtime specs and the resource close op are
//! **derived** by `registry::runtime_specs()` — there is no `fs/specs.rs`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, Registry, RegistryFunction, RegistryPackage,
    RegistryResource,
};

pub(crate) mod gen_atomic_write;
pub(crate) mod gen_canonical;
pub(crate) mod gen_directory;
pub(crate) mod gen_exists;
pub(crate) mod gen_handle;
pub(crate) mod gen_open;
pub(crate) mod gen_path_builder;
pub(crate) mod gen_read_write;
pub(crate) mod gen_shared;
pub(crate) mod gen_temp_file;

mod func_append_bytes;
mod func_append_text;
mod func_canonical_path;
mod func_close;
mod func_create_directories;
mod func_create_directory;
mod func_create_temp_file;
mod func_current_directory;
mod func_delete_directory;
mod func_delete_file;
mod func_directory_exists;
mod func_eof;
mod func_exists;
mod func_file_exists;
mod func_flush;
mod func_is_buffered;
mod func_is_within;
mod func_list_directory;
mod func_open;
mod func_open_file;
mod func_open_file_no_follow;
mod func_open_within;
mod func_path_base_name;
mod func_path_dir_name;
mod func_path_extension;
mod func_path_join;
mod func_path_normalize;
mod func_read_all;
mod func_read_all_bytes;
mod func_read_bytes;
mod func_read_line;
mod func_read_text;
mod func_set_buffered;
mod func_set_current_directory;
mod func_temp_directory;
mod func_write_all;
mod func_write_all_bytes;
mod func_write_bytes;
mod func_write_bytes_atomic;
mod func_write_text;
mod func_write_text_atomic;

/// The opaque `File` resource handle's bare type name — its identity *within* the
/// `fs` package (the `RegistryResource` name, the `type` half of the qualified id).
pub(crate) const FILE_TYPE: &str = "File";

/// The `File` resource's **package-qualified type identity** (`fs.File`, plan-97 /
/// bug-441): the string every `RES` binding of an open file, every `File`
/// parameter/return, and the `ResourceRegistry` key carry.
pub(crate) const FILE_TYPE_ID: &str = "fs.File";

/// `File`'s registered resource close op — the public `fs.close` (flush-then-close).
/// A `File` is otherwise released automatically by lexical scope drop, which runs
/// this same op.
pub(crate) const CLOSE: &str = "fs.close";

const MODULE_INTRO: &str = r#"Filesystem path, file, and directory operations"#;
const MODULE_DESC: &str = r#"The `fs` package provides filesystem access: one-shot whole-file reads and
writes, an open `File` handle for streaming I/O, purely syntactic path-string
manipulation, directory creation and listing, and existence tests. `fs` is a
built-in package: `IMPORT fs` needs no manifest dependency.

Paths are UTF-8 `String` values, interpreted as bytes and passed to the host
filesystem, so they may carry Unicode characters where the host accepts such
names. A path may be absolute or relative to the process current working
directory; relative paths resolve against `fs::currentDirectory()`. Every path
argument must be non-empty and free of embedded NUL bytes, because the host call
requires a NUL-terminated path. The path-syntax functions — `fs::pathJoin`,
`fs::pathNormalize`, `fs::pathDirName`, `fs::pathBaseName`, and
`fs::pathExtension` — are byte-oriented and never touch the filesystem, while
`fs::canonicalPath` and `fs::isWithin` consult the disk to resolve `.`, `..`,
and symlinks. Where a path names a symlink, the final component is followed (so
reads and writes act on the target) except in `fs::openFileNoFollow`, which
refuses a symlinked final component, and `fs::deleteFile`, which removes the link
itself.

Whole-file functions operate directly on a path. `fs::readText` and
`fs::readBytes` read the entire file in one call; `fs::writeText` and
`fs::writeBytes` replace its contents; `fs::appendText` and `fs::appendBytes` add
to it; and the `fs::writeTextAtomic` and `fs::writeBytesAtomic` variants stage
the new contents in a temporary file and swap it in with an OS rename so readers
never observe a partial write. Text functions require and produce well-formed
UTF-8; byte functions transfer a `List OF Byte` verbatim, with no encoding or
newline translation, and so suit binary data.

Handle functions work through the opaque `File` resource type. `fs::open`,
`fs::openFile`, `fs::openFileNoFollow`, and `fs::createTempFile` return a `File`;
`fs::readLine`, `fs::readAll`, `fs::readAllBytes`, `fs::writeAll`,
`fs::writeAllBytes`, and `fs::eof` act on one. Portable open modes are
`"read"`/`"r"`, `"write"`/`"w"`, `"readWrite"`/`"rw"`, and `"append"`/`"a"`. A `File` is a handle that closes itself when its binding goes out of scope; call `fs::close` only to release it earlier. Using a
`File` after it is closed fails.

Each `File` handle can independently opt in to output buffering. It is off by
default, so `fs::writeAll`/`fs::writeAllBytes` reach the OS immediately;
`fs::setBuffered(file, TRUE)` instead holds incremental writes in a per-handle
buffer that is drained on `fs::flush(file)`, when it fills, and — mandatorily — on
close — `fs::close`, or the binding going out of scope — so buffered on-disk
data is never stranded.
`fs::setBuffered(file, FALSE)` drains and disables it, and `fs::isBuffered(file)`
reports the current mode. Only incremental handle writes are buffered; whole-file
and atomic writes already issue one write and ignore the setting. A hard crash may
lose buffered bytes not yet flushed — flush or close for durability.

Directory functions create (`fs::createDirectory`, `fs::createDirectories`),
remove (`fs::deleteDirectory`), and inspect (`fs::listDirectory`) directories,
read or change the working directory (`fs::currentDirectory`,
`fs::setCurrentDirectory`), and report the host temporary directory
(`fs::tempDirectory`), which is also the default location `fs::createTempFile`
uses when called without one. `fs::listDirectory` returns entry names only,
excluding `.` and `..`, sorted in ascending byte-wise order for deterministic
results. The existence predicates `fs::exists`, `fs::fileExists`, and
`fs::directoryExists` return a `Boolean` and report a missing or unreadable path
as `FALSE` rather than raising; only an internal running out of memory can raise from them."#;

/// Register the `fs` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("fs", MODULE_INTRO, MODULE_DESC);

    // The one opaque owned resource. Semantic-only (no injectable source): it makes
    // `registry().qualified_builtin_type("fs.File")` and
    // `registry::resource_close_function("File")` answer generically. A `File` is
    // thread-sendable and its `close` op (a real host `close`) may fail.
    pkg.add_resource(RegistryResource {
        name: FILE_TYPE,
        export: true,
        description: "An opaque handle to an open file, closed automatically \
                      when its binding goes out of scope.",
        close_function: CLOSE,
        sendable: true,
        close_may_fail: true,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });

    func_file_exists::register(&mut pkg);
    func_directory_exists::register(&mut pkg);
    func_exists::register(&mut pkg);
    func_read_bytes::register(&mut pkg);
    func_read_text::register(&mut pkg);
    func_write_bytes::register(&mut pkg);
    func_write_text::register(&mut pkg);
    func_write_bytes_atomic::register(&mut pkg);
    func_write_text_atomic::register(&mut pkg);
    func_append_bytes::register(&mut pkg);
    func_append_text::register(&mut pkg);
    func_open::register(&mut pkg);
    func_open_file::register(&mut pkg);
    func_open_file_no_follow::register(&mut pkg);
    func_open_within::register(&mut pkg);
    func_create_temp_file::register(&mut pkg);
    func_temp_directory::register(&mut pkg);
    func_read_line::register(&mut pkg);
    func_read_all::register(&mut pkg);
    func_read_all_bytes::register(&mut pkg);
    func_write_all::register(&mut pkg);
    func_write_all_bytes::register(&mut pkg);
    func_set_buffered::register(&mut pkg);
    func_is_buffered::register(&mut pkg);
    func_flush::register(&mut pkg);
    func_close::register(&mut pkg);
    func_eof::register(&mut pkg);
    func_canonical_path::register(&mut pkg);
    func_is_within::register(&mut pkg);
    func_path_join::register(&mut pkg);
    func_path_dir_name::register(&mut pkg);
    func_path_base_name::register(&mut pkg);
    func_path_extension::register(&mut pkg);
    func_path_normalize::register(&mut pkg);
    func_delete_file::register(&mut pkg);
    func_create_directory::register(&mut pkg);
    func_create_directories::register(&mut pkg);
    func_delete_directory::register(&mut pkg);
    func_list_directory::register(&mut pkg);
    func_current_directory::register(&mut pkg);
    func_set_current_directory::register(&mut pkg);

    r.add_package(pkg);
}
