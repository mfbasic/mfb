    use super::*;

    #[test]
    fn free_list_bump_then_reuse() {
        // One 4064-byte block (4096 mapped − 32 header). Allocate a few chunks;
        // with no frees the list is one shrinking trailing entry (bump behavior).
        let mut sim = FreeListSim::default();
        sim.insert_free(0x1020, 4064);
        let a = sim.alloc(24, 8).unwrap(); // → rounded 32
        let b = sim.alloc(16, 8).unwrap();
        let c = sim.alloc(100, 8).unwrap(); // → rounded 112
        assert_eq!(a, 0x1020);
        assert_eq!(b, 0x1040);
        assert_eq!(c, 0x1050);
        assert_eq!(sim.nodes.len(), 1, "bump leaves one trailing free entry");
        sim.assert_invariants();
        // Free the middle chunk; first-fit reuses that 16-byte hole next.
        sim.free(b, 16);
        sim.assert_invariants();
        let d = sim.alloc(16, 8).unwrap();
        assert_eq!(d, b, "low-address hole is reused before the trailing entry");
    }

    #[test]
    fn free_list_coalesces_neighbors() {
        let mut sim = FreeListSim::default();
        sim.insert_free(0x1000, 0x1000);
        let a = sim.alloc(64, 16).unwrap();
        let b = sim.alloc(64, 16).unwrap();
        let c = sim.alloc(64, 16).unwrap();
        let free_before = sim.free_bytes();
        // Free a and c (non-adjacent) → two holes; then b merges all three.
        sim.free(a, 64);
        sim.free(c, 64);
        sim.assert_invariants();
        sim.free(b, 64);
        sim.assert_invariants();
        assert_eq!(sim.free_bytes(), free_before + 3 * 64);
        // After full coalescing the block is whole again: a single entry.
        assert_eq!(sim.nodes.len(), 1);
        assert_eq!(sim.nodes[0].0, a);
    }

    #[test]
    fn free_list_same_shape_churn_stays_short() {
        // A loop that allocs/frees the same shape each pass must not grow the
        // list: the freed chunk coalesces straight back into its neighbor.
        let mut sim = FreeListSim::default();
        sim.insert_free(0x2000, 0x4000);
        for _ in 0..1000 {
            let p = sim.alloc(48, 16).unwrap();
            sim.free(p, 48);
            assert!(sim.nodes.len() <= 1, "churn must keep the list ~1 entry");
        }
        sim.assert_invariants();
        assert_eq!(sim.free_bytes(), 0x4000);
    }

    #[test]
    fn free_list_never_merges_across_blocks() {
        // Two separate blocks (header gap between them). Freeing the last chunk
        // of the low block must not merge into the high block.
        let mut sim = FreeListSim::default();
        sim.insert_free(0x1020, 4064); // block A usable
        sim.insert_free(0x3020, 4064); // block B usable (non-contiguous)
        let a = sim.alloc(4064, 16).unwrap(); // consume all of A
        assert_eq!(a, 0x1020);
        assert_eq!(sim.nodes.len(), 1, "only B remains free");
        sim.free(a, 4064);
        sim.assert_invariants();
        assert_eq!(sim.nodes.len(), 2, "A and B stay distinct (header gap)");
    }

    #[test]
    fn free_list_over_aligns_to_16_with_front_split() {
        let mut sim = FreeListSim::default();
        sim.insert_free(0x1010, 0x1000); // start 16-aligned but not 64-aligned
        let p = sim.alloc(32, 64).unwrap();
        assert_eq!(p % 64, 0);
        assert!(p > 0x1010, "front padding split into its own free chunk");
        sim.assert_invariants();
        // Freeing reconstitutes by merging with the front padding chunk.
        let before = sim.free_bytes();
        sim.free(p, 32);
        sim.assert_invariants();
        assert_eq!(sim.free_bytes(), before + 32);
    }

    #[test]
    fn arena_rejects_invalid_alignment() {
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, 128, 8, 0),
            Err(77050002)
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, 128, 8, 3),
            Err(77050002)
        );
    }

    #[test]
    fn arena_handles_zero_size_allocations() {
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, 128, 0, 8),
            Ok((0x1020, 1))
        );
    }

    #[test]
    fn arena_checks_alignment_rounding_and_capacity() {
        assert_eq!(
            checked_arena_used_after_alloc(0x1003, 5, 128, 8, 16),
            Ok((0x1030, 21))
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 120, 128, 16, 16),
            Err(77010001)
        );
    }

    #[test]
    fn arena_checks_arithmetic_overflow() {
        assert_eq!(
            checked_arena_used_after_alloc(u64::MAX - 8, 0, 128, 8, 8),
            Err(77010001)
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, u64::MAX, u64::MAX, 8),
            Err(77010001)
        );
    }
