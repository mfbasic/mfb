//! A SquashFS 4.0 image writer (plan-51-B).
//!
//! An AppImage is `[runtime ELF][squashfs image]` concatenated (plan-51-C), and
//! this module produces the second half: [`write`] serializes an in-memory
//! [`SquashTree`] of directories, regular files, and symlinks to bytes that the
//! Linux kernel loop-mounts, that the AppImage runtime's bundled squashfuse
//! mounts via FUSE, and that `unsquashfs` extracts byte-for-byte.
//!
//! The image is **fully uncompressed** and this module contains **zero
//! compressor code**. That is a supported configuration rather than a shortcut:
//! decompression is decided per block by a flag bit in each block's own header,
//! so an uncompressed block never reaches a decompressor. The one catch (§4.2) is
//! that the superblock must still *name* a compressor the reader has compiled in,
//! even though it is never invoked — hence [`COMPRESSION_ZLIB`], which is
//! deliberately not `0`.
//!
//! The house precedent is `crate::os::linux::link::elf`: a from-scratch binary
//! format writer, little-endian field packing into a `Vec<u8>`, no encoder crate,
//! offsets computed and then asserted.

use std::collections::BTreeMap;

// --- Superblock constants (plan-51-B §4.2) ---------------------------------

/// `hsqs`, little-endian.
const MAGIC: u32 = 0x7371_7368;
/// Data block size. **Must not be 4096**: the kernel rejects a mount where
/// `PAGE_SIZE > block_size`, so a 4 KiB-block image fails on a 16 KiB-page
/// (Asahi) or 64 KiB-page arm64 kernel. 131072 is `mksquashfs`'s default and
/// clears every page size in use.
const BLOCK_SIZE: u32 = 131_072;
/// `log2(BLOCK_SIZE)`. The kernel checks this equality explicitly.
const BLOCK_LOG: u16 = 17;
/// The compressor id written into the superblock.
///
/// ⚠️ **Must be `1` (ZLIB), not `0`,** and this is the single most
/// counter-intuitive requirement in the format — the one most likely to be
/// "fixed" by a future reader of this code. Both readers validate the compressor
/// id *unconditionally, before any flag is consulted and before any block is
/// read*:
///
/// ```c
/// /* squashfuse fs.c — the AppImage runtime's reader */
/// if (!(fs->decompressor = sqfs_decompressor_get(fs->sb.compression))) return SQFS_BADCOMP;
/// /* linux fs/squashfs/super.c — runs before any table read, no flag check */
/// msblk->decompressor = supported_squashfs_filesystem(fc, major, minor, le16_to_cpu(sblk->compression));
/// if (msblk->decompressor == NULL) goto failed_mount;
/// ```
///
/// We declare gzip and never emit a compressed block, so the decompressor is
/// resolved, allocated, and never called. ZLIB is the safest of the six ids:
/// `CONFIG_SQUASHFS_ZLIB` is universally enabled and it is `mksquashfs`'s own
/// default.
const COMPRESSION_ZLIB: u16 = 1;
/// `NOI | NOD | NOF | NO_FRAG | NO_XATTR | NOID` (plan-51-B §4.3).
///
/// ⚠️ Bit 2 is `SQUASHFS_CHECK`, unused since 4.0 — it is **not**
/// `UNCOMPRESSED_FRAGMENTS`. Every flag from bit 3 up is one position higher than
/// a naive reading suggests, which matters because it puts `COMPRESSOR_OPTIONS`
/// — the only flag with teeth — at `0x0400`. Set that bit and the reader tries to
/// parse a compressor-options block after the superblock, finds our data, and
/// fails the mount.
const FLAGS: u16 = 0x0A1B;
/// `SQUASHFS_COMPRESSOR_OPTIONS`; must stay clear in [`FLAGS`].
const FLAG_COMPRESSOR_OPTIONS: u16 = 0x0400;
/// `SQUASHFS_INVALID_BLK` (`-1LL`), the sentinel that omits the xattr and export
/// tables (plan-51-B §4.8).
const INVALID_BLK: u64 = 0xFFFF_FFFF_FFFF_FFFF;
/// `SQUASHFS_INVALID_FRAG`: no fragment, so the block count is
/// `ceil(file_size / BLOCK_SIZE)` with the tail written as a short data block.
const INVALID_FRAG: u32 = 0xFFFF_FFFF;
/// Uncompressed metadata payload size; also the metadata block header's size
/// mask ceiling.
const METADATA_BLOCK: usize = 8192;
/// On-disk stride of a full uncompressed metadata block: 2-byte header + payload.
const METADATA_STRIDE: u64 = 8194;
/// `SQUASHFS_COMPRESSED_BIT`. The sense is **inverted**: bit 15 set means the
/// block is *uncompressed*.
const METADATA_UNCOMPRESSED: u16 = 0x8000;
/// Block-list entry bit 24: this data block is stored uncompressed.
const DATA_UNCOMPRESSED: u32 = 0x0100_0000;
/// The superblock is 96 bytes and data starts immediately after it.
const SUPERBLOCK_SIZE: u64 = 96;
/// `mksquashfs` pads the image to this boundary unless `-nopad`, and so do we:
/// squashfs is `FS_REQUIRES_DEV`, and a loop device rounds the backing file
/// **down** to whole sectors, which can put an unpadded ID location list past the
/// end of the device.
const PAD_TO: u64 = 4096;
/// Max entries under one directory header (`SQUASHFS_DIR_COUNT`).
const DIR_COUNT_MAX: usize = 256;
/// Max directory entry name length, from the `size = stored + 1` bias on a `u16`
/// field the kernel caps at `SQUASHFS_NAME_LEN`.
const NAME_MAX: usize = 256;

// Inode type ids. Only the three *basic* forms are emitted; the extended forms
// (LDIR = 8, LREG = 9) are rejected rather than silently truncated to (§4.9).
const INODE_DIR: u16 = 1;
const INODE_FILE: u16 = 2;
const INODE_SYMLINK: u16 = 3;

// --- The input tree (plan-51-B §4.1) ---------------------------------------

/// A node in the tree a SquashFS image is built from (plan-51-B §4.1).
///
/// Deliberately cannot represent a hard link, device node, FIFO, or socket: an
/// AppDir has none, and a type that cannot express them cannot emit them wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SquashNode {
    /// `BTreeMap` is not incidental: §4.6 requires directory entries sorted
    /// ASCIIbetically because readers binary-search the listing, and a
    /// `BTreeMap<String, _>` iterates in exactly that order. Sorting is therefore
    /// a property of the type rather than a step that can be forgotten.
    Dir {
        entries: BTreeMap<String, SquashNode>,
        mode: u16,
    },
    File {
        data: Vec<u8>,
        mode: u16,
    },
    /// Symlinks carry no mode: SquashFS stores one, but every reader ignores it
    /// and `mksquashfs` writes `0777`. We match that.
    Symlink {
        target: String,
    },
}

/// The tree [`write`] serializes. `root` must be a [`SquashNode::Dir`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SquashTree {
    pub root: SquashNode,
}

impl SquashNode {
    /// A new empty directory with `mode` permission bits.
    ///
    /// Only the tests build a tree node-by-node; the shipping path constructs
    /// directories in `tree_from_dir` while walking the real AppDir, so this has
    /// no non-test caller (bug-326-A29).
    #[cfg(test)]
    pub(crate) fn dir(mode: u16) -> SquashNode {
        SquashNode::Dir {
            entries: BTreeMap::new(),
            mode,
        }
    }
}

// --- Metadata stream (plan-51-B §4.3/§4.4) ---------------------------------

/// An append-only stream that chunks into 8 KiB metadata blocks.
///
/// Owns the block-header format so nothing else can get the inverted compressed
/// bit wrong, and owns [`metadata_ref`] so the byte-offset-vs-block-index trap
/// has exactly one place to be wrong in. Callers see only a logical stream
/// offset via [`MetadataWriter::position`] and never a block boundary.
struct MetadataWriter {
    out: Vec<u8>,
    pending: Vec<u8>,
    stream: u64,
}

impl MetadataWriter {
    fn new() -> MetadataWriter {
        MetadataWriter {
            out: Vec::new(),
            pending: Vec::with_capacity(METADATA_BLOCK),
            stream: 0,
        }
    }

    /// The logical (uncompressed) stream offset the next byte written will land
    /// at. Feed this to [`metadata_ref`] to address whatever is written next.
    fn position(&self) -> u64 {
        self.stream
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.pending.push(byte);
            self.stream += 1;
            if self.pending.len() == METADATA_BLOCK {
                self.flush();
            }
        }
    }

    /// Emit `pending` as one on-disk metadata block.
    ///
    /// Never called with an empty `pending`: `SQUASHFS_COMPRESSED_SIZE` maps a
    /// zero size field to 32768, which exceeds the 8192 the kernel allows for a
    /// metadata block and yields `-EIO`. A zero-length metadata block is
    /// unrepresentable, so header `0x0000`/`0x8000` must never be written.
    fn flush(&mut self) {
        debug_assert!(!self.pending.is_empty(), "zero-length metadata block");
        debug_assert!(self.pending.len() <= METADATA_BLOCK);
        let header = METADATA_UNCOMPRESSED | self.pending.len() as u16;
        self.out.extend_from_slice(&header.to_le_bytes());
        self.out.extend_from_slice(&self.pending);
        self.pending.clear();
    }

    fn finish(mut self) -> Vec<u8> {
        if !self.pending.is_empty() {
            self.flush();
        }
        self.out
    }
}

/// Encode a metadata reference (plan-51-B §4.4) from a logical stream offset.
///
/// The upper 48 bits are the **on-disk byte offset** of the containing block's
/// 2-byte header relative to the table start — NOT a block index. The two are
/// identical for any stream that fits in one 8 KiB block, which is every small
/// tree and every hand-written test fixture, so a block-index encoding passes
/// every cheap test and then fails to mount the moment the stream crosses 8 KiB.
/// One owner; both `root_inode` and every directory-header `start_block` derive
/// from it.
fn metadata_ref(stream_offset: u64) -> u64 {
    let block = (stream_offset / METADATA_BLOCK as u64) * METADATA_STRIDE;
    let offset = stream_offset % METADATA_BLOCK as u64;
    (block << 16) | offset
}

/// The block half of a metadata reference: the on-disk byte offset of the
/// containing block's header, relative to the table start.
fn ref_block(stream_offset: u64) -> u64 {
    metadata_ref(stream_offset) >> 16
}

/// The offset half of a metadata reference: the position inside the
/// **uncompressed** 8 KiB payload, 0…8191.
fn ref_offset(stream_offset: u64) -> u16 {
    (stream_offset % METADATA_BLOCK as u64) as u16
}

// --- The flattened tree ----------------------------------------------------

/// One node, flattened out of the recursive [`SquashNode`] so that inode numbers
/// (and therefore each directory's `parent_inode`) are known before any inode is
/// serialized.
struct Planned {
    name: String,
    inode_number: u32,
    parent_inode: u32,
    kind: PlannedKind,
}

enum PlannedKind {
    Dir {
        mode: u16,
        /// Indices into the flat vector, in ASCIIbetical name order.
        children: Vec<usize>,
    },
    File {
        mode: u16,
        size: u64,
        /// Absolute byte offset of this file's data in the image.
        start_block: u64,
        /// One entry per data block: bit 24 set (uncompressed) | on-disk size.
        block_list: Vec<u32>,
    },
    Symlink {
        target: String,
    },
}

/// Flatten `node` into `planned` in **pre-order**, assigning inode numbers as we
/// descend so a directory's number is known before its children need it as their
/// `parent_inode`. File data is appended to `data` here, because the format puts
/// every data block ahead of the inode table (§4.10).
///
/// Returns this node's index in `planned`.
fn plan_node(
    name: &str,
    node: &SquashNode,
    parent_inode: u32,
    planned: &mut Vec<Planned>,
    data: &mut Vec<u8>,
) -> Result<usize, String> {
    let inode_number = planned.len() as u32 + 1;
    let index = planned.len();
    // Reserve the slot; the recursion below appends after it.
    planned.push(Planned {
        name: name.to_string(),
        inode_number,
        parent_inode,
        kind: PlannedKind::Symlink {
            target: String::new(),
        },
    });

    let kind = match node {
        SquashNode::Dir { entries, mode } => {
            let mut children = Vec::with_capacity(entries.len());
            for (child_name, child) in entries {
                if child_name.is_empty() {
                    return Err("squashfs: a directory entry name may not be empty".to_string());
                }
                if child_name.len() > NAME_MAX {
                    return Err(format!(
                        "squashfs: directory entry name '{child_name}' is {} bytes, over the \
                         {NAME_MAX}-byte maximum",
                        child_name.len()
                    ));
                }
                children.push(plan_node(child_name, child, inode_number, planned, data)?);
            }
            PlannedKind::Dir {
                mode: *mode,
                children,
            }
        }
        SquashNode::File { data: bytes, mode } => {
            // A basic file (type 2) stores `start_block` and `file_size` as u32,
            // so anything past 4 GiB needs an extended inode (LREG, type 9). We
            // reject rather than silently truncate (§4.9).
            let start_block = SUPERBLOCK_SIZE + data.len() as u64;
            let size = bytes.len() as u64;
            if start_block + size >= u32::MAX as u64 || size >= u32::MAX as u64 {
                return Err(format!(
                    "squashfs: file '{name}' would place data past 4 GiB, which needs an \
                     extended inode this writer does not emit"
                ));
            }
            let mut block_list = Vec::new();
            for chunk in bytes.chunks(BLOCK_SIZE as usize) {
                // Bit 24 set = uncompressed; bits 0–23 are the on-disk size. An
                // entry of 0 would mean a *sparse* block (a whole block of zeros
                // with no I/O) — we never emit one, so a file of zeros
                // round-trips as itself.
                block_list.push(DATA_UNCOMPRESSED | chunk.len() as u32);
                data.extend_from_slice(chunk);
            }
            PlannedKind::File {
                mode: *mode,
                size,
                start_block,
                block_list,
            }
        }
        SquashNode::Symlink { target } => {
            if target.is_empty() {
                return Err(format!("squashfs: symlink '{name}' has an empty target"));
            }
            PlannedKind::Symlink {
                target: target.clone(),
            }
        }
    };
    planned[index].kind = kind;
    Ok(index)
}

// --- Serialization ---------------------------------------------------------

/// Serialize `tree` to a SquashFS 4.0 image (plan-51-B §4.10).
///
/// Deterministic: `mkfs_time` and every inode `mtime` are `0` and directory
/// entries come out of a `BTreeMap`, so two calls with the same tree produce
/// identical bytes — the standing property of every other artifact this compiler
/// emits.
pub(crate) fn write(tree: &SquashTree) -> Result<Vec<u8>, String> {
    if !matches!(tree.root, SquashNode::Dir { .. }) {
        return Err("squashfs: the root node must be a directory".to_string());
    }

    let mut planned: Vec<Planned> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    // The root's parent is its own inode number, which pre-order numbering makes
    // 1 before we descend.
    plan_node("", &tree.root, 1, &mut planned, &mut data)?;
    let inode_count = planned.len() as u32;

    let mut inode_table = MetadataWriter::new();
    let mut dir_table = MetadataWriter::new();
    // Stream offset of each node's inode, filled in post-order.
    let mut inode_at: Vec<u64> = vec![0; planned.len()];
    write_inodes(0, &planned, &mut inode_table, &mut dir_table, &mut inode_at)?;
    let root_ref = metadata_ref(inode_at[0]);
    if ref_offset(inode_at[0]) as usize > METADATA_BLOCK {
        return Err("squashfs: root inode offset exceeds the metadata block size".to_string());
    }

    let inode_bytes = inode_table.finish();
    let dir_bytes = dir_table.finish();

    // §4.10's layout. The kernel walks the tables backwards validating
    // adjacency, which pins this order; `mksquashfs` writes exactly the same.
    let inode_table_start = SUPERBLOCK_SIZE + data.len() as u64;
    let directory_table_start = inode_table_start + inode_bytes.len() as u64;
    let id_block_start = directory_table_start + dir_bytes.len() as u64;
    // The ID table is mandatory: `no_ids == 0` fails the mount with "Bad id count
    // in super block". Our minimum is one metadata block holding a single u32
    // zero (uid 0 / gid 0), addressed by a one-entry u64 location list.
    let id_table_start = id_block_start + 6;
    let bytes_used = id_table_start + 8;

    // §4.9: every invariant a real reader enforces, asserted here so a layout bug
    // is a build error naming the cause rather than a mount failure with no
    // diagnostic.
    if BLOCK_SIZE != 1 << BLOCK_LOG {
        return Err("squashfs: block_size must equal 1 << block_log".to_string());
    }
    if COMPRESSION_ZLIB == 0 {
        return Err("squashfs: the superblock must name a real compressor id".to_string());
    }
    if FLAGS & FLAG_COMPRESSOR_OPTIONS != 0 {
        return Err("squashfs: COMPRESSOR_OPTIONS must be clear".to_string());
    }
    // Strict, and kernel-enforced. Always holds because the root inode alone
    // makes the inode table non-empty, but assert rather than assume.
    if inode_table_start >= directory_table_start {
        return Err("squashfs: inode_table_start must be strictly below \
                    directory_table_start"
            .to_string());
    }
    if directory_table_start > id_block_start {
        return Err("squashfs: directory_table_start must not pass the ID table".to_string());
    }
    // ⚠️ The adjacency constraint, and this module's plan-46-D §1 analogue. With
    // no xattr table `next_table == bytes_used`, and the kernel enforces
    // `SQUASHFS_ID_BLOCK_BYTES(no_ids) == next_table - id_table_start` exactly —
    // two separately-computed numbers that must agree to the byte.
    if id_table_start + 8 != bytes_used {
        return Err(format!(
            "squashfs: the ID location list must end exactly at bytes_used \
             ({id_table_start} + 8 != {bytes_used})"
        ));
    }
    if id_block_start >= id_table_start || id_table_start - id_block_start > METADATA_STRIDE {
        return Err(
            "squashfs: the ID metadata block must sit just below id_table_start".to_string(),
        );
    }

    let mut out: Vec<u8> = Vec::with_capacity(bytes_used as usize + PAD_TO as usize);
    // Superblock (96 bytes, all little-endian).
    put_u32(&mut out, MAGIC);
    put_u32(&mut out, inode_count);
    put_u32(&mut out, 0); // mkfs_time — zero for reproducibility
    put_u32(&mut out, BLOCK_SIZE);
    put_u32(&mut out, 0); // fragments
    put_u16(&mut out, COMPRESSION_ZLIB);
    put_u16(&mut out, BLOCK_LOG);
    put_u16(&mut out, FLAGS);
    put_u16(&mut out, 1); // no_ids
    put_u16(&mut out, 4); // s_major
    put_u16(&mut out, 0); // s_minor
    put_u64(&mut out, root_ref);
    put_u64(&mut out, bytes_used);
    put_u64(&mut out, id_table_start);
    put_u64(&mut out, INVALID_BLK); // xattr_id_table_start
    put_u64(&mut out, inode_table_start);
    put_u64(&mut out, directory_table_start);
    put_u64(&mut out, INVALID_BLK); // fragment_table_start
    put_u64(&mut out, INVALID_BLK); // lookup_table_start
    debug_assert_eq!(out.len() as u64, SUPERBLOCK_SIZE);

    out.extend_from_slice(&data);
    out.extend_from_slice(&inode_bytes);
    out.extend_from_slice(&dir_bytes);
    // The ID metadata block: header (uncompressed, 4 bytes) + one u32 id `0`.
    put_u16(&mut out, METADATA_UNCOMPRESSED | 4);
    put_u32(&mut out, 0);
    // The ID location list, which must be the last thing in the archive.
    put_u64(&mut out, id_block_start);

    if out.len() as u64 != bytes_used {
        return Err(format!(
            "squashfs: wrote {} bytes but computed bytes_used {bytes_used}",
            out.len()
        ));
    }
    // Pad to a whole 4 KiB sector; `bytes_used` deliberately excludes the padding.
    while !(out.len() as u64).is_multiple_of(PAD_TO) {
        out.push(0);
    }
    Ok(out)
}

/// Post-order: serialize `index`'s children, then its own listing (if it is a
/// directory), then its inode. A directory's listing needs its children's inode
/// references, so the children must be written first.
fn write_inodes(
    index: usize,
    planned: &[Planned],
    inode_table: &mut MetadataWriter,
    dir_table: &mut MetadataWriter,
    inode_at: &mut [u64],
) -> Result<(), String> {
    let node = &planned[index];
    match &node.kind {
        PlannedKind::Dir { mode, children } => {
            for &child in children {
                write_inodes(child, planned, inode_table, dir_table, inode_at)?;
            }
            let listing_start = dir_table.position();
            let listing_len = write_directory_listing(children, planned, inode_at, dir_table)?;
            // The `+3` rule: the kernel synthesizes `.` and `..` at f_pos 0–2, so
            // its read loop starts `length` at 3 and a directory inode's
            // `file_size` is always `listing + 3`. `mksquashfs` literally calls
            // `create_inode(..., dir_size + 3, ...)`. An empty directory therefore
            // has `file_size == 3`, and `file_size < 4` means empty.
            let file_size = listing_len + 3;
            if file_size >= 65536 {
                return Err(format!(
                    "squashfs: directory '{}' has a {listing_len}-byte listing, which needs an \
                     extended inode (LDIR) this writer does not emit",
                    node.name
                ));
            }
            let subdir_count = children
                .iter()
                .filter(|&&child| matches!(planned[child].kind, PlannedKind::Dir { .. }))
                .count() as u32;
            inode_at[index] = inode_table.position();
            write_inode_header(inode_table, INODE_DIR, *mode, node.inode_number);
            let mut tail = Vec::with_capacity(16);
            put_u32(&mut tail, ref_block(listing_start) as u32);
            put_u32(&mut tail, 2 + subdir_count); // nlink
            put_u16(&mut tail, file_size as u16);
            put_u16(&mut tail, ref_offset(listing_start));
            put_u32(&mut tail, node.parent_inode);
            inode_table.write(&tail);
        }
        PlannedKind::File {
            mode,
            size,
            start_block,
            block_list,
        } => {
            inode_at[index] = inode_table.position();
            write_inode_header(inode_table, INODE_FILE, *mode, node.inode_number);
            let mut tail = Vec::with_capacity(16 + block_list.len() * 4);
            put_u32(&mut tail, *start_block as u32);
            put_u32(&mut tail, INVALID_FRAG);
            put_u32(&mut tail, 0); // offset — meaningless without a fragment
            put_u32(&mut tail, *size as u32);
            for &entry in block_list {
                put_u32(&mut tail, entry);
            }
            inode_table.write(&tail);
        }
        PlannedKind::Symlink { target } => {
            inode_at[index] = inode_table.position();
            // Every reader ignores a symlink's stored mode; `mksquashfs` writes
            // 0777 and so do we.
            write_inode_header(inode_table, INODE_SYMLINK, 0o777, node.inode_number);
            let mut tail = Vec::with_capacity(8 + target.len());
            put_u32(&mut tail, 1); // nlink
            put_u32(&mut tail, target.len() as u32);
            // Not NUL-terminated.
            tail.extend_from_slice(target.as_bytes());
            inode_table.write(&tail);
        }
    }
    Ok(())
}

/// The 16-byte common inode header.
///
/// `mode` is **permission bits only** — the type comes from `inode_type` and
/// readers OR it back in themselves. `uid`/`guid` are *indices into the ID
/// table*, not raw ids; both are `0`, and `squashfs_get_id` rejects
/// `index >= no_ids`. `mtime` is `0` for the same reproducibility reason as
/// `mkfs_time`.
fn write_inode_header(writer: &mut MetadataWriter, inode_type: u16, mode: u16, inode_number: u32) {
    let mut header = Vec::with_capacity(16);
    put_u16(&mut header, inode_type);
    put_u16(&mut header, mode & 0o7777);
    put_u16(&mut header, 0); // uid index
    put_u16(&mut header, 0); // guid index
    put_u32(&mut header, 0); // mtime
    put_u32(&mut header, inode_number);
    writer.write(&header);
}

/// Write one directory's listing into the directory table, returning its length
/// in bytes.
///
/// ⚠️ Three off-by-one biases, all mandatory (§4.6): `header.count` is stored
/// `N - 1`, `entry.size` is stored `name_len - 1`, and `entry.inode_number` is a
/// **signed 16-bit delta** from the header's base despite its `__le16` type.
///
/// A new header must start when any of: 256 entries have been written under the
/// current one; the next entry's inode lands in a different metadata block
/// (`start_block` is per-header, so a run cannot span blocks); or the
/// inode-number delta would overflow `i16`. Breaking only at 256 corrupts any
/// directory whose inodes straddle an 8 KiB boundary.
fn write_directory_listing(
    children: &[usize],
    planned: &[Planned],
    inode_at: &[u64],
    dir_table: &mut MetadataWriter,
) -> Result<u64, String> {
    if children.is_empty() {
        return Ok(0);
    }
    let start = dir_table.position();

    let mut run: Vec<usize> = Vec::new();
    let mut run_block: u64 = 0;
    let mut run_base: u32 = 0;

    let flush = |run: &mut Vec<usize>,
                 run_block: u64,
                 run_base: u32,
                 dir_table: &mut MetadataWriter|
     -> Result<(), String> {
        if run.is_empty() {
            return Ok(());
        }
        let mut bytes = Vec::new();
        put_u32(&mut bytes, (run.len() - 1) as u32); // count, stored N - 1
        put_u32(&mut bytes, run_block as u32);
        put_u32(&mut bytes, run_base);
        for &child in run.iter() {
            let entry = &planned[child];
            let delta = entry.inode_number as i64 - run_base as i64;
            let delta = i16::try_from(delta)
                .map_err(|_| "squashfs: directory entry inode delta overflows i16".to_string())?;
            put_u16(&mut bytes, ref_offset(inode_at[child]));
            put_u16(&mut bytes, delta as u16);
            put_u16(&mut bytes, entry_type_of(entry));
            put_u16(&mut bytes, (entry.name.len() - 1) as u16); // size, stored len - 1
            bytes.extend_from_slice(entry.name.as_bytes());
        }
        dir_table.write(&bytes);
        run.clear();
        Ok(())
    };

    for &child in children {
        let block = ref_block(inode_at[child]);
        let number = planned[child].inode_number;
        let must_break = run.is_empty()
            || run.len() == DIR_COUNT_MAX
            || block != run_block
            || i16::try_from(number as i64 - run_base as i64).is_err();
        if must_break {
            flush(&mut run, run_block, run_base, dir_table)?;
            run_block = block;
            run_base = number;
        }
        run.push(child);
    }
    flush(&mut run, run_block, run_base, dir_table)?;

    Ok(dir_table.position() - start)
}

fn entry_type_of(entry: &Planned) -> u16 {
    match entry.kind {
        PlannedKind::Dir { .. } => INODE_DIR,
        PlannedKind::File { .. } => INODE_FILE,
        PlannedKind::Symlink { .. } => INODE_SYMLINK,
    }
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests;
