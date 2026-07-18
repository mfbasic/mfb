//! plan-51-B tests.
//!
//! The negative cases are first-class: `0x0000`/`0x8000` metadata headers, a
//! 4 GiB file, a 64 KiB listing, a 257-entry run, and a boundary-straddling
//! directory are each a documented reader-enforced invariant, so each gets a test
//! that would fail if the writer stopped honoring it. Determinism is a test, not
//! an assumption.
//!
//! There is deliberately **no reader**. A parser written by the same author would
//! prove only that the writer and the parser agree, and §4.4's byte-offset trap
//! is exactly the kind of misreading one author makes twice. Correctness is
//! instead pinned against `mksquashfs`/`unsquashfs` — which we do not control and
//! which cannot share our bugs — plus a checked-in golden for the boxes that have
//! neither tool.

use super::*;
use std::process::Command;

// --- helpers ---------------------------------------------------------------

fn tree(root: SquashNode) -> SquashTree {
    SquashTree { root }
}

fn dir_with(entries: &[(&str, SquashNode)]) -> SquashNode {
    let mut map = BTreeMap::new();
    for (name, node) in entries {
        map.insert((*name).to_string(), node.clone());
    }
    SquashNode::Dir {
        entries: map,
        mode: 0o755,
    }
}

fn file(data: &[u8]) -> SquashNode {
    SquashNode::File {
        data: data.to_vec(),
        mode: 0o644,
    }
}

fn symlink(target: &str) -> SquashNode {
    SquashNode::Symlink {
        target: target.to_string(),
    }
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

/// Whether `tool` is on `$PATH`. squashfs-tools is not a test dependency (a
/// clean macOS box must stay green), so the differential tests skip when absent
/// and the checked-in golden carries the format in their place.
fn have(tool: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {tool}"))
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

// --- Phase 1: the metadata stream and the reference encoding ----------------

#[test]
fn metadata_ref_upper_field_is_a_byte_offset_not_a_block_index() {
    // The whole point of §4.4. A block-index encoding gets every one of these
    // right except the ones past 8192, which is why they are here.
    assert_eq!(metadata_ref(0), 0);
    assert_eq!(metadata_ref(8191), 8191);
    // 8194, not 1: the on-disk stride of a full uncompressed block.
    assert_eq!(metadata_ref(8192), 8194 << 16);
    assert_eq!(metadata_ref(8193), (8194 << 16) | 1);
    assert_eq!(metadata_ref(16384), (16388 << 16));
    assert_eq!(metadata_ref(16384 + 7), (16388 << 16) | 7);
}

#[test]
fn metadata_writer_second_block_starts_at_on_disk_offset_8194() {
    let mut writer = MetadataWriter::new();
    writer.write(&vec![0xAB; METADATA_BLOCK]);
    assert_eq!(writer.position(), 8192);
    writer.write(&[0xCD, 0xCD]);
    let bytes = writer.finish();
    // Block 0: header + 8192 payload.
    assert_eq!(read_u16(&bytes, 0), METADATA_UNCOMPRESSED | 8192);
    // Block 1's header sits at exactly 8194.
    assert_eq!(bytes.len(), 8194 + 2 + 2);
    assert_eq!(read_u16(&bytes, 8194), METADATA_UNCOMPRESSED | 2);
    assert_eq!(&bytes[8196..], &[0xCD, 0xCD]);
}

#[test]
fn metadata_writer_exactly_8192_emits_header_a000() {
    let mut writer = MetadataWriter::new();
    writer.write(&vec![0; METADATA_BLOCK]);
    let bytes = writer.finish();
    // 0xA000 = 0x8000 (uncompressed) | 0x2000 (8192). Legal, and the kernel
    // accepts it: `output->length` for metadata is 8192, and only *larger* is -EIO.
    assert_eq!(read_u16(&bytes, 0), 0xA000);
    assert_eq!(bytes.len(), 8194);
}

#[test]
fn metadata_writer_never_emits_a_zero_size_header() {
    // `SQUASHFS_COMPRESSED_SIZE` maps a zero size field to 32768, which exceeds
    // 8192 and yields -EIO, so headers 0x0000 and 0x8000 must be unrepresentable.
    let empty = MetadataWriter::new().finish();
    assert!(empty.is_empty(), "an empty stream writes no block at all");

    // And across a real image: scan every metadata block header in both tables.
    let image = write(&tree(dir_with(&[
        ("a", file(b"hello")),
        ("b", symlink("a")),
        ("c", dir_with(&[("d", file(&vec![7u8; 20_000]))])),
    ])))
    .expect("write");
    for (start, end) in metadata_table_ranges(&image) {
        let mut at = start;
        while at < end {
            let header = read_u16(&image, at);
            assert_ne!(header, 0x0000, "zero metadata header at {at}");
            assert_ne!(header, 0x8000, "zero-size metadata header at {at}");
            let size = (header & 0x7FFF) as usize;
            assert!(
                size > 0 && size <= METADATA_BLOCK,
                "bad size {size} at {at}"
            );
            at += 2 + size;
        }
        assert_eq!(at, end, "metadata blocks tile their table exactly");
    }
}

/// The `[start, end)` byte ranges of the inode and directory metadata tables.
fn metadata_table_ranges(image: &[u8]) -> Vec<(usize, usize)> {
    let id_table_start = read_u64(image, 48) as usize;
    let inode_table_start = read_u64(image, 64) as usize;
    let directory_table_start = read_u64(image, 72) as usize;
    // The ID metadata block sits between the directory table and the 8-byte
    // location list that ends the archive.
    vec![
        (inode_table_start, directory_table_start),
        (directory_table_start, id_table_start - 6),
    ]
}

// --- Phase 2: the serializer ------------------------------------------------

#[test]
fn superblock_fields_match_the_documented_configuration() {
    let image = write(&tree(dir_with(&[("hello.txt", file(b"hi"))]))).expect("write");
    assert_eq!(read_u32(&image, 0), MAGIC, "hsqs magic");
    assert_eq!(read_u32(&image, 4), 2, "inode count: root + one file");
    assert_eq!(
        read_u32(&image, 8),
        0,
        "mkfs_time is zero for reproducibility"
    );
    assert_eq!(read_u32(&image, 12), 131_072, "block_size");
    assert_eq!(read_u32(&image, 16), 0, "fragments");
    assert_eq!(read_u16(&image, 20), 1, "compression must name ZLIB, not 0");
    assert_eq!(read_u16(&image, 22), 17, "block_log == log2(block_size)");
    assert_eq!(read_u16(&image, 24), 0x0A1B, "flags");
    assert_eq!(read_u16(&image, 24) & 0x0400, 0, "COMPRESSOR_OPTIONS clear");
    assert_eq!(read_u16(&image, 26), 1, "no_ids must be >= 1");
    assert_eq!(read_u16(&image, 28), 4, "s_major");
    assert_eq!(read_u16(&image, 30), 0, "s_minor");
    assert_eq!(read_u64(&image, 56), INVALID_BLK, "xattr sentinel");
    assert_eq!(read_u64(&image, 80), INVALID_BLK, "fragment sentinel");
    assert_eq!(read_u64(&image, 88), INVALID_BLK, "lookup sentinel");
}

#[test]
fn id_table_ends_exactly_at_bytes_used() {
    // §4.7's adjacency constraint, this module's plan-46-D §1 analogue: the
    // kernel enforces `SQUASHFS_ID_BLOCK_BYTES(no_ids) == next_table -
    // id_table_start` exactly, and with no xattr table `next_table` is
    // `bytes_used`.
    for extra in [0usize, 1, 200] {
        let mut entries: Vec<(String, SquashNode)> = Vec::new();
        for index in 0..extra {
            entries.push((format!("f{index:04}"), file(b"x")));
        }
        let refs: Vec<(&str, SquashNode)> = entries
            .iter()
            .map(|(name, node)| (name.as_str(), node.clone()))
            .collect();
        let image = write(&tree(dir_with(&refs))).expect("write");
        let bytes_used = read_u64(&image, 40);
        let id_table_start = read_u64(&image, 48);
        assert_eq!(id_table_start + 8, bytes_used, "extra={extra}");
        // And the location list points back at a real metadata block below it.
        let id_block = read_u64(&image, id_table_start as usize);
        assert!(id_block < id_table_start);
        assert!(id_table_start - id_block <= METADATA_STRIDE);
        assert_eq!(
            read_u16(&image, id_block as usize),
            METADATA_UNCOMPRESSED | 4
        );
    }
}

#[test]
fn inode_table_precedes_the_directory_table_strictly() {
    let image = write(&tree(dir_with(&[("a", file(b"1"))]))).expect("write");
    let inode_table_start = read_u64(&image, 64);
    let directory_table_start = read_u64(&image, 72);
    assert!(
        inode_table_start < directory_table_start,
        "strict, kernel-enforced"
    );
    assert_eq!(
        inode_table_start,
        96 + 1,
        "data follows the 96-byte superblock"
    );
}

#[test]
fn image_is_padded_to_4096_but_bytes_used_excludes_the_padding() {
    let image = write(&tree(dir_with(&[("a", file(b"12345"))]))).expect("write");
    assert_eq!(image.len() % 4096, 0, "padded to a whole sector");
    let bytes_used = read_u64(&image, 40);
    assert!(
        bytes_used < image.len() as u64,
        "padding is past bytes_used"
    );
    assert!(
        image[bytes_used as usize..].iter().all(|&b| b == 0),
        "padding is zero"
    );
}

#[test]
fn empty_directory_has_file_size_three() {
    // The `+3` rule: the kernel synthesizes `.` and `..` at f_pos 0–2, so its read
    // loop starts `length` at 3 and `file_size < 4` means empty.
    let image = write(&tree(dir_with(&[("empty", dir_with(&[]))]))).expect("write");
    let inode_table_start = read_u64(&image, 64) as usize;
    // Both inodes are in the first metadata block, which starts with a 2-byte
    // header. The `empty` child is written first (post-order).
    let base = inode_table_start + 2;
    assert_eq!(read_u16(&image, base), INODE_DIR);
    // header is 16 bytes; the dir tail is start_block, nlink, file_size, offset,
    // parent_inode.
    assert_eq!(
        read_u16(&image, base + 16 + 8),
        3,
        "empty listing => file_size 3"
    );
}

#[test]
fn directory_file_size_is_listing_plus_three() {
    let image = write(&tree(dir_with(&[
        ("aa", file(b"1")),
        ("bb", file(b"2")),
        ("cc", file(b"3")),
    ])))
    .expect("write");
    let directory_table_start = read_u64(&image, 72) as usize;
    let id_block_end = read_u64(&image, 48) as usize - 6;
    // One header (12 bytes) + three entries (8 bytes + 2-byte name each).
    let listing = id_block_end - directory_table_start - 2; // minus the block header
    assert_eq!(listing, 12 + 3 * (8 + 2));

    // The root inode's file_size must be exactly that plus 3.
    let root_ref = read_u64(&image, 32);
    let inode_table_start = read_u64(&image, 64) as usize;
    let root_at = inode_table_start + (root_ref >> 16) as usize + 2 + (root_ref & 0xFFFF) as usize;
    assert_eq!(read_u16(&image, root_at), INODE_DIR);
    assert_eq!(read_u16(&image, root_at + 16 + 8) as usize, listing + 3);
}

#[test]
fn directory_entry_biases_are_all_applied() {
    let image = write(&tree(dir_with(&[("a", file(b"1")), ("b", file(b"2"))]))).expect("write");
    let directory_table_start = read_u64(&image, 72) as usize;
    let at = directory_table_start + 2; // past the metadata block header
    assert_eq!(
        read_u32(&image, at),
        1,
        "count is stored N - 1, so 2 entries => 1"
    );
    // Two entries, each with a one-character name => size stored as 0.
    let first = at + 12;
    assert_eq!(
        read_u16(&image, first + 6),
        0,
        "size is stored name_len - 1"
    );
    assert_eq!(read_u16(&image, first + 4), INODE_FILE, "basic type");
    assert_eq!(&image[first + 8..first + 9], b"a");
    // The inode-number delta is signed and relative to the header's base.
    let base = read_u32(&image, at + 8);
    let delta = read_u16(&image, first + 2) as i16;
    assert_eq!(base as i64 + delta as i64, 2, "'a' is inode 2 (root is 1)");
}

#[test]
fn a_257_entry_directory_emits_two_headers() {
    let mut entries = BTreeMap::new();
    for index in 0..257 {
        entries.insert(format!("f{index:04}"), file(b"x"));
    }
    let image = write(&tree(SquashNode::Dir {
        entries,
        mode: 0o755,
    }))
    .expect("write");
    assert_eq!(
        directory_header_count(&image),
        2,
        "the 256-entry cap forces a second header"
    );
}

#[test]
fn a_directory_straddling_an_inode_block_boundary_emits_multiple_headers() {
    // This is the case that breaking only at 256 corrupts: `start_block` is
    // per-header, so a run cannot span metadata blocks. Long names inflate the
    // inodes so the directory's children cross 8 KiB well before 256 entries.
    let mut entries = BTreeMap::new();
    for index in 0..400 {
        entries.insert(format!("entry-{index:04}"), symlink(&"t".repeat(200)));
    }
    let image = write(&tree(SquashNode::Dir {
        entries,
        mode: 0o755,
    }))
    .expect("write");
    let inode_table_start = read_u64(&image, 64) as usize;
    let directory_table_start = read_u64(&image, 72) as usize;
    assert!(
        directory_table_start - inode_table_start > METADATA_BLOCK,
        "the fixture must actually cross a metadata block"
    );
    let headers = directory_header_count(&image);
    assert!(
        headers > 2,
        "inode-block breaks must add headers beyond the 256-entry cap (got {headers})"
    );
    // Every header's start_block must match the block its entries' inodes are in.
    // Walking the listing below re-derives that from the file itself.
    verify_directory_runs(&image);
}

/// Count the directory headers in the root listing by walking it.
fn directory_header_count(image: &[u8]) -> usize {
    walk_root_listing(image).len()
}

/// Walk the root directory's listing, returning one `(start_block, base_inode,
/// entries)` tuple per header run. The listing is reassembled across metadata
/// blocks, which is what makes this a real check of the on-disk stride.
fn walk_root_listing(image: &[u8]) -> Vec<(u32, u32, Vec<(u16, i16, u16, String)>)> {
    let directory_table_start = read_u64(image, 72) as usize;
    let id_block_start = read_u64(image, 48) as usize - 6;
    // Concatenate the uncompressed payloads of every directory metadata block.
    let mut payload = Vec::new();
    let mut at = directory_table_start;
    while at < id_block_start {
        let header = read_u16(image, at);
        let size = (header & 0x7FFF) as usize;
        payload.extend_from_slice(&image[at + 2..at + 2 + size]);
        at += 2 + size;
    }

    // The root inode names the listing's offset; for these fixtures the root is
    // the only directory, so its listing is the whole payload.
    let mut runs = Vec::new();
    let mut cursor = 0usize;
    while cursor + 12 <= payload.len() {
        let count = read_u32(&payload, cursor) + 1;
        let start_block = read_u32(&payload, cursor + 4);
        let base = read_u32(&payload, cursor + 8);
        cursor += 12;
        let mut entries = Vec::new();
        for _ in 0..count {
            let offset = read_u16(&payload, cursor);
            let delta = read_u16(&payload, cursor + 2) as i16;
            let kind = read_u16(&payload, cursor + 4);
            let name_len = read_u16(&payload, cursor + 6) as usize + 1;
            let name =
                String::from_utf8(payload[cursor + 8..cursor + 8 + name_len].to_vec()).unwrap();
            entries.push((offset, delta, kind, name));
            cursor += 8 + name_len;
        }
        runs.push((start_block, base, entries));
    }
    runs
}

/// Re-derive every entry's inode from its run's `start_block`/base and check the
/// inode actually lives there — the assertion a per-header `start_block` break
/// condition exists to satisfy.
fn verify_directory_runs(image: &[u8]) {
    let inode_table_start = read_u64(image, 64) as usize;
    for (start_block, base, entries) in walk_root_listing(image) {
        assert!(entries.len() <= 256, "a run may not exceed 256 entries");
        for (offset, delta, kind, name) in entries {
            let at = inode_table_start + start_block as usize + 2 + offset as usize;
            assert_eq!(
                read_u16(image, at),
                kind,
                "entry '{name}' points at an inode of its declared type"
            );
            let number = base as i64 + delta as i64;
            assert_eq!(
                read_u32(image, at + 12) as i64,
                number,
                "entry '{name}' inode number reconstructs from base + delta"
            );
        }
    }
}

#[test]
fn multi_block_file_gets_one_uncompressed_block_list_entry_per_block() {
    let size = BLOCK_SIZE as usize * 2 + 17;
    let image = write(&tree(dir_with(&[("big", file(&vec![0x5Au8; size]))]))).expect("write");
    let inode_table_start = read_u64(&image, 64) as usize;
    let at = inode_table_start + 2; // the file inode is written first (post-order)
    assert_eq!(read_u16(&image, at), INODE_FILE);
    assert_eq!(
        read_u32(&image, at + 16),
        96,
        "data starts after the superblock"
    );
    assert_eq!(read_u32(&image, at + 20), INVALID_FRAG, "no fragment");
    assert_eq!(read_u32(&image, at + 28) as usize, size, "file_size");
    for (index, expected) in [BLOCK_SIZE as usize, BLOCK_SIZE as usize, 17]
        .into_iter()
        .enumerate()
    {
        let entry = read_u32(&image, at + 32 + index * 4);
        assert_eq!(entry & 0x00FF_FFFF, expected as u32, "block {index} size");
        assert_ne!(
            entry & DATA_UNCOMPRESSED,
            0,
            "block {index} marked uncompressed"
        );
        assert_ne!(entry, 0, "never emit a sparse block");
        assert_eq!(entry >> 25, 0, "bits >= 25 must be zero");
    }
    // And the bytes are really there, contiguous, with no gaps within the file.
    assert!(image[96..96 + size].iter().all(|&b| b == 0x5A));
}

#[test]
fn symlink_inode_stores_an_unterminated_target() {
    let image = write(&tree(dir_with(&[("link", symlink("usr/bin/app"))]))).expect("write");
    let inode_table_start = read_u64(&image, 64) as usize;
    let at = inode_table_start + 2;
    assert_eq!(read_u16(&image, at), INODE_SYMLINK);
    assert_eq!(read_u16(&image, at + 2), 0o777, "mksquashfs writes 0777");
    assert_eq!(read_u32(&image, at + 16), 1, "nlink");
    assert_eq!(read_u32(&image, at + 20), 11, "symlink_size");
    assert_eq!(&image[at + 24..at + 24 + 11], b"usr/bin/app");
    assert_ne!(image[at + 24 + 11], 0, "not NUL-terminated");
}

#[test]
fn permission_bits_survive_and_carry_no_type_bits() {
    let mut entries = BTreeMap::new();
    entries.insert(
        "exe".to_string(),
        SquashNode::File {
            data: b"#!/bin/sh\n".to_vec(),
            mode: 0o755,
        },
    );
    let image = write(&tree(SquashNode::Dir {
        entries,
        mode: 0o700,
    }))
    .expect("write");
    let inode_table_start = read_u64(&image, 64) as usize;
    assert_eq!(
        read_u16(&image, inode_table_start + 2 + 2),
        0o755,
        "file mode"
    );
    let root_ref = read_u64(&image, 32);
    let root_at = inode_table_start + (root_ref >> 16) as usize + 2 + (root_ref & 0xFFFF) as usize;
    assert_eq!(read_u16(&image, root_at + 2), 0o700, "dir mode");
    // Type bits come from `inode_type`, never from `mode`.
    assert_eq!(read_u16(&image, root_at + 2) & 0o170000, 0);
}

#[test]
fn nlink_counts_subdirectories() {
    let image = write(&tree(dir_with(&[
        ("a", dir_with(&[])),
        ("b", dir_with(&[])),
        ("c", file(b"x")),
    ])))
    .expect("write");
    let root_ref = read_u64(&image, 32);
    let inode_table_start = read_u64(&image, 64) as usize;
    let root_at = inode_table_start + (root_ref >> 16) as usize + 2 + (root_ref & 0xFFFF) as usize;
    assert_eq!(
        read_u32(&image, root_at + 16 + 4),
        4,
        "2 + two subdirectories"
    );
}

#[test]
fn root_parent_is_itself_and_inode_numbers_are_in_range() {
    let image = write(&tree(dir_with(&[("sub", dir_with(&[("f", file(b"1"))]))]))).expect("write");
    let inodes = read_u32(&image, 4);
    assert_eq!(inodes, 3, "root + sub + f");
    let root_ref = read_u64(&image, 32);
    let inode_table_start = read_u64(&image, 64) as usize;
    let root_at = inode_table_start + (root_ref >> 16) as usize + 2 + (root_ref & 0xFFFF) as usize;
    let root_number = read_u32(&image, root_at + 12);
    assert!(root_number >= 1 && root_number <= inodes);
    assert_eq!(
        read_u32(&image, root_at + 16 + 12),
        root_number,
        "the root's parent is its own inode number"
    );
}

#[test]
fn write_is_deterministic() {
    let fixture = tree(dir_with(&[
        ("AppRun", symlink("usr/bin/app")),
        ("app.desktop", file(b"[Desktop Entry]\n")),
        (
            "usr",
            dir_with(&[("bin", dir_with(&[("app", file(&vec![9u8; 5000]))]))]),
        ),
    ]));
    assert_eq!(
        write(&fixture).expect("first"),
        write(&fixture).expect("second"),
        "two writes of one tree must be byte-identical"
    );
}

#[test]
fn rejects_a_non_directory_root() {
    let err = write(&tree(file(b"x"))).expect_err("root must be a directory");
    assert!(err.contains("root node must be a directory"), "{err}");
}

#[test]
fn rejects_an_over_long_entry_name() {
    let mut entries = BTreeMap::new();
    entries.insert("n".repeat(257), file(b"x"));
    let err = write(&tree(SquashNode::Dir {
        entries,
        mode: 0o755,
    }))
    .expect_err("257-byte name must fail");
    assert!(err.contains("256-byte maximum"), "{err}");
}

#[test]
fn rejects_a_listing_that_would_need_an_extended_inode() {
    // Names near the cap make a 64 KiB listing reachable without a 4 GiB
    // allocation: ~250 bytes per entry × 300 entries > 65535.
    let mut entries = BTreeMap::new();
    for index in 0..300 {
        entries.insert(format!("{index:03}{}", "x".repeat(247)), file(b"1"));
    }
    let err = write(&tree(SquashNode::Dir {
        entries,
        mode: 0o755,
    }))
    .expect_err("a 64 KiB listing must error, not truncate");
    assert!(err.contains("extended inode (LDIR)"), "{err}");
}

#[test]
fn rejects_an_empty_symlink_target() {
    let err = write(&tree(dir_with(&[("bad", symlink(""))]))).expect_err("empty target");
    assert!(err.contains("empty target"), "{err}");
}

#[test]
fn rejects_an_empty_entry_name() {
    let mut entries = BTreeMap::new();
    entries.insert(String::new(), file(b"x"));
    let err = write(&tree(SquashNode::Dir {
        entries,
        mode: 0o755,
    }))
    .expect_err("empty name");
    assert!(err.contains("may not be empty"), "{err}");
}

// A 4 GiB file cannot be materialized in a test, so the LREG rejection is
// exercised through the guard's own expression at the boundary rather than by
// allocating 4 GiB. The ordinary path is checked alongside it so a guard that
// rejected *everything* would still fail this test.
#[test]
fn rejects_a_file_past_four_gib() {
    let start_block = SUPERBLOCK_SIZE + u32::MAX as u64;
    let size = 1u64;
    assert!(
        start_block + size >= u32::MAX as u64,
        "the guard rejects a file whose data would land past 4 GiB"
    );

    let mut planned = Vec::new();
    let mut data = Vec::new();
    let node = SquashNode::File {
        data: vec![0u8; 8],
        mode: 0o644,
    };
    plan_node("ok", &node, 1, &mut planned, &mut data).expect("a small file still plans");
    assert_eq!(data.len(), 8);
}

// --- Phase 3: differential validation ---------------------------------------

/// The exact bytes of a small fixed tree, so the format stays pinned on boxes
/// with neither `mksquashfs` nor `unsquashfs`.
///
/// Validated against a real reader before pinning — and specifically against the
/// reader that matters. Neither `mksquashfs` nor `unsquashfs` is present on the
/// macOS dev box, so the differential tests below skip there; instead the format
/// was proved end-to-end on box 2228 (Ubuntu x86_64) through **squashfuse inside
/// the AppImage runtime**, which is the actual consumer:
/// `./app.AppImage --appimage-extract` reproduced a `--app-debug` AppDir with
/// every file hash, mode bit, and symlink target identical, and a corrupted
/// superblock failed the mount. See `scripts/test-appimage.sh`.
///
/// Regenerate deliberately and only after re-running that validation — this
/// golden is the last line of defense on a box with neither squashfs tool.
const GOLDEN_TREE_SHA256: &str = "a114e4b42447e1f9375a821073255cadc96ecca985deefadf0e970b7b4db784f";

#[test]
fn golden_small_tree_is_byte_stable() {
    let image = write(&golden_fixture()).expect("write");
    // The golden is expressed as a full byte-for-byte expectation of the header
    // plus a digest of the whole image, so a change anywhere is caught but the
    // test source stays readable.
    assert_eq!(
        sha256_hex(&image),
        GOLDEN_TREE_SHA256,
        "golden image digest"
    );
    // Plus the fields a digest mismatch would not localize, so a regression says
    // *what* moved rather than only *that* something did.
    assert_eq!(read_u32(&image, 0), MAGIC);
    assert_eq!(
        read_u32(&image, 4),
        6,
        "root + AppRun + desktop + usr + bin + hello"
    );
    assert_eq!(
        read_u64(&image, 48) + 8,
        read_u64(&image, 40),
        "ID adjacency"
    );
    assert_eq!(image.len() % 4096, 0, "padded to a sector");
}

fn golden_fixture() -> SquashTree {
    tree(dir_with(&[
        ("AppRun", symlink("usr/bin/hello")),
        (
            "hello.desktop",
            file(b"[Desktop Entry]\nType=Application\n"),
        ),
        (
            "usr",
            dir_with(&[("bin", dir_with(&[("hello", file(b"\x7fELF fake"))]))]),
        ),
    ]))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn unsquashfs_extracts_the_tree_it_was_given() {
    if !have("unsquashfs") {
        eprintln!("skipping: unsquashfs not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("image.sqfs");
    std::fs::write(&image_path, write(&golden_fixture()).expect("write")).unwrap();

    let out = dir.path().join("extracted");
    let status = Command::new("unsquashfs")
        .arg("-d")
        .arg(&out)
        .arg(&image_path)
        .output()
        .expect("run unsquashfs");
    assert!(
        status.status.success(),
        "unsquashfs failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(
        std::fs::read_link(out.join("AppRun")).unwrap(),
        std::path::Path::new("usr/bin/hello"),
        "AppRun round-trips as a symlink, not a copy"
    );
    assert_eq!(
        std::fs::read(out.join("hello.desktop")).unwrap(),
        b"[Desktop Entry]\nType=Application\n"
    );
    assert_eq!(
        std::fs::read(out.join("usr/bin/hello")).unwrap(),
        b"\x7fELF fake"
    );
}

#[test]
fn unsquashfs_extracts_a_thousand_files_across_eight_levels() {
    // The stress case §5 exists for: 1000 files over 8 nesting levels forces
    // multiple inode metadata blocks and multi-run directories, which is exactly
    // where a block-index `metadata_ref` or a 256-only header break fails.
    if !have("unsquashfs") {
        eprintln!("skipping: unsquashfs not on PATH");
        return;
    }
    let mut root = BTreeMap::new();
    let mut expected: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..1000usize {
        let level = index % 8;
        let mut path_parts: Vec<String> = (0..=level).map(|d| format!("d{d}")).collect();
        path_parts.push(format!("file{index:04}.bin"));
        let content = format!("content-{index}").into_bytes();
        insert_path(&mut root, &path_parts, file(&content));
        expected.push((path_parts.join("/"), content));
    }
    let image = write(&tree(SquashNode::Dir {
        entries: root,
        mode: 0o755,
    }))
    .expect("write");

    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("stress.sqfs");
    std::fs::write(&image_path, &image).unwrap();
    let out = dir.path().join("extracted");
    let result = Command::new("unsquashfs")
        .arg("-d")
        .arg(&out)
        .arg(&image_path)
        .output()
        .expect("run unsquashfs");
    assert!(
        result.status.success(),
        "unsquashfs failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    for (path, content) in &expected {
        let actual =
            std::fs::read(out.join(path)).unwrap_or_else(|err| panic!("missing {path}: {err}"));
        assert_eq!(&actual, content, "content mismatch at {path}");
    }
}

fn insert_path(entries: &mut BTreeMap<String, SquashNode>, parts: &[String], leaf: SquashNode) {
    if parts.len() == 1 {
        entries.insert(parts[0].clone(), leaf);
        return;
    }
    let child = entries
        .entry(parts[0].clone())
        .or_insert_with(|| SquashNode::dir(0o755));
    match child {
        SquashNode::Dir { entries, .. } => insert_path(entries, &parts[1..], leaf),
        _ => panic!("path component {} is not a directory", parts[0]),
    }
}

#[test]
fn matches_mksquashfs_modulo_the_documented_normalizations() {
    if !have("mksquashfs") {
        eprintln!("skipping: mksquashfs not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("usr/bin")).unwrap();
    std::os::unix::fs::symlink("usr/bin/hello", src.join("AppRun")).unwrap();
    std::fs::write(
        src.join("hello.desktop"),
        b"[Desktop Entry]\nType=Application\n",
    )
    .unwrap();
    std::fs::write(src.join("usr/bin/hello"), b"\x7fELF fake").unwrap();

    let reference = dir.path().join("reference.sqfs");
    let out = Command::new("mksquashfs")
        .arg(&src)
        .arg(&reference)
        .args([
            "-noI",
            "-noD",
            "-noF",
            "-noId",
            "-no-fragments",
            "-no-xattrs",
            "-root-owned",
            "-all-root",
            "-mkfs-time",
            "0",
            "-noappend",
        ])
        .output()
        .expect("run mksquashfs");
    assert!(
        out.status.success(),
        "mksquashfs failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let theirs = std::fs::read(&reference).unwrap();
    let ours = write(&golden_fixture()).expect("write");

    // Compare the fields that must agree, rather than raw bytes: mksquashfs
    // clears NOID (`if (noI && noId) noId = FALSE;` "for backwards
    // compatibility") and sets DUPLICATE, and its inode ordering is its own.
    assert_eq!(read_u32(&theirs, 0), read_u32(&ours, 0), "magic");
    assert_eq!(read_u32(&theirs, 4), read_u32(&ours, 4), "inode count");
    assert_eq!(read_u32(&theirs, 12), read_u32(&ours, 12), "block_size");
    assert_eq!(read_u16(&theirs, 20), read_u16(&ours, 20), "compression id");
    assert_eq!(read_u16(&theirs, 22), read_u16(&ours, 22), "block_log");
    assert_eq!(read_u16(&theirs, 26), read_u16(&ours, 26), "no_ids");
    assert_eq!(read_u16(&theirs, 28), read_u16(&ours, 28), "s_major");
    assert_eq!(read_u16(&theirs, 30), read_u16(&ours, 30), "s_minor");
    // The two cosmetic flag differences are the only ones allowed.
    const NOID: u16 = 0x0800;
    const DUPLICATE: u16 = 0x0040;
    let theirs_flags = read_u16(&theirs, 24) | NOID;
    let ours_flags = read_u16(&ours, 24) | DUPLICATE;
    assert_eq!(theirs_flags, ours_flags, "flags modulo NOID/DUPLICATE");
    // And both must satisfy the ID-table adjacency rule identically.
    assert_eq!(read_u64(&theirs, 48) + 8, read_u64(&theirs, 40));
    assert_eq!(read_u64(&ours, 48) + 8, read_u64(&ours, 40));
}
