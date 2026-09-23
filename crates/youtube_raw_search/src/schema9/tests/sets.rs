use super::*;

fn docs(len: u64) -> Universe<DocId> {
    Universe::new(GenerationId(7), SegmentId(2), len).unwrap()
}

fn bits(len: u64, ids: &[u32]) -> Vec<u64> {
    let mut words = vec![0_u64; dense_word_count(len) as usize];
    for &id in ids {
        words[id as usize / 64] |= 1_u64 << (id % 64);
    }
    words
}

fn fixture(universe: Universe<DocId>, ids: &[u32], dense: bool, memory: &Arc<SetMemory>) -> DocSet {
    if dense {
        DocSet::from_dense_words(universe, &bits(universe.len(), ids), memory).unwrap()
    } else {
        DocSet::from_sorted_raw(universe, ids, memory).unwrap()
    }
}

fn collect(expr: &SetExpr<'_, DocId>, memory: &Arc<SetMemory>) -> Vec<u32> {
    let mut cursor = expr.cursor(memory).unwrap();
    let mut ids = Vec::new();
    while let Some(id) = cursor.next() {
        ids.push(id.raw());
    }
    ids
}

#[test]
fn exhaustive_boolean_algebra_matches_a_simple_set_oracle() {
    for len in 0..=5_u64 {
        let universe = docs(len);
        for left_mask in 0..(1_u32 << len) {
            for right_mask in 0..(1_u32 << len) {
                let left_ids = (0..len as u32)
                    .filter(|id| left_mask & (1 << id) != 0)
                    .collect::<Vec<_>>();
                let right_ids = (0..len as u32)
                    .filter(|id| right_mask & (1 << id) != 0)
                    .collect::<Vec<_>>();
                for left_dense in [false, true] {
                    for right_dense in [false, true] {
                        let memory = SetMemory::new(1 << 20);
                        let left = fixture(universe, &left_ids, left_dense, &memory);
                        let right = fixture(universe, &right_ids, right_dense, &memory);
                        let left_expr = SetExpr::leaf(&left);
                        let right_expr = SetExpr::leaf(&right);
                        let expected = [
                            (
                                SetExpr::and(&left_expr, &right_expr).unwrap(),
                                left_mask & right_mask,
                            ),
                            (
                                SetExpr::or(&left_expr, &right_expr).unwrap(),
                                left_mask | right_mask,
                            ),
                            (
                                SetExpr::subtract(&left_expr, &right_expr).unwrap(),
                                left_mask & !right_mask,
                            ),
                        ];
                        for (expr, expected_mask) in expected {
                            let ids = (0..len as u32)
                                .filter(|id| expected_mask & (1 << id) != 0)
                                .collect::<Vec<_>>();
                            assert_eq!(collect(&expr, &memory), ids);
                            assert_eq!(expr.count(&memory).unwrap(), ids.len() as u64);
                            let result = expr.materialize(&memory).unwrap();
                            assert_eq!(result.len(), ids.len() as u64);
                            for id in 0..len as u32 {
                                assert_eq!(result.contains_raw(id), ids.contains(&id));
                            }
                            assert_eq!(collect(&SetExpr::leaf(&result), &memory), ids);
                        }
                        let union = SetExpr::or(&left_expr, &right_expr).unwrap();
                        let nested = SetExpr::subtract(&union, &left_expr).unwrap();
                        let right_only = right_ids
                            .iter()
                            .copied()
                            .filter(|id| !left_ids.contains(id))
                            .collect::<Vec<_>>();
                        assert_eq!(collect(&nested, &memory), right_only);
                        assert_eq!(nested.count(&memory).unwrap(), right_only.len() as u64);
                        assert_eq!(
                            memory.live_bytes(),
                            left.owned_bytes() + right.owned_bytes()
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn rejects_bad_inputs_and_equal_size_wrong_universes() {
    let memory = SetMemory::new(1 << 20);
    let universe = docs(65);
    assert!(matches!(
        DocSet::from_sorted_raw(universe, &[1, 1], &memory),
        Err(SetError::Duplicate(1))
    ));
    assert!(matches!(
        DocSet::from_sorted_raw(universe, &[2, 1], &memory),
        Err(SetError::Unsorted(1))
    ));
    assert!(matches!(
        DocSet::from_sorted_raw(universe, &[65], &memory),
        Err(SetError::OutOfRange { id: 65, len: 65 })
    ));
    assert!(matches!(
        DocSet::from_dense_words(universe, &[0], &memory),
        Err(SetError::WrongWordCount {
            expected: 2,
            actual: 1
        })
    ));
    assert!(matches!(
        DocSet::from_dense_words(universe, &[0, 2], &memory),
        Err(SetError::DirtyTailBits)
    ));
    assert!(matches!(
        Universe::<DocId>::new(GenerationId(7), SegmentId(2), MAX_UNIVERSE_LEN + 1),
        Err(SetError::InvalidUniverseLength(_))
    ));
    let empty = DocSet::from_sorted_raw(universe, &[], &memory).unwrap();
    let other_generation = DocSet::from_sorted_raw(
        Universe::new(GenerationId(8), SegmentId(2), 65).unwrap(),
        &[],
        &memory,
    )
    .unwrap();
    let other_segment = DocSet::from_sorted_raw(
        Universe::new(GenerationId(7), SegmentId(3), 65).unwrap(),
        &[],
        &memory,
    )
    .unwrap();
    for wrong in [&other_generation, &other_segment] {
        assert!(matches!(
            SetExpr::or(&SetExpr::leaf(&empty), &SetExpr::leaf(wrong)),
            Err(SetError::UniverseMismatch { .. })
        ));
    }
}

#[test]
fn full_u32_id_space_and_dense_tail_boundaries() {
    let memory = SetMemory::new(1 << 20);
    let full = docs(MAX_UNIVERSE_LEN);
    let last = DocSet::from_sorted_raw(full, &[u32::MAX], &memory).unwrap();
    assert!(last.contains_raw(u32::MAX));
    assert_eq!(collect(&SetExpr::leaf(&last), &memory), vec![u32::MAX]);
    let short = docs(u32::MAX as u64);
    assert!(matches!(
        DocSet::from_sorted_raw(short, &[u32::MAX], &memory),
        Err(SetError::OutOfRange { .. })
    ));
    for len in [0, 1, 63, 64, 65] {
        let empty_bits = vec![0; dense_word_count(len) as usize];
        let empty = DocSet::from_dense_words(docs(len), &empty_bits, &memory).unwrap();
        assert_eq!(empty.len(), 0);
        assert!(collect(&SetExpr::leaf(&empty), &memory).is_empty());
    }
}

#[test]
fn large_mixed_sets_preserve_exact_counts_and_sorted_iteration() {
    let universe = docs(100_003);
    let memory = SetMemory::new(1 << 20);
    let left_ids = (0..universe.len() as u32)
        .filter(|id| id % 3 == 0)
        .collect::<Vec<_>>();
    let right_ids = (0..universe.len() as u32)
        .filter(|id| id % 7 == 0 || *id == 100_002)
        .collect::<Vec<_>>();
    let left = fixture(universe, &left_ids, true, &memory);
    let right = fixture(universe, &right_ids, false, &memory);
    let a = SetExpr::leaf(&left);
    let b = SetExpr::leaf(&right);
    for (expr, predicate) in [
        (
            SetExpr::and(&a, &b).unwrap(),
            (|id: u32| id.is_multiple_of(3) && (id.is_multiple_of(7) || id == 100_002))
                as fn(u32) -> bool,
        ),
        (
            SetExpr::or(&a, &b).unwrap(),
            (|id: u32| id.is_multiple_of(3) || id.is_multiple_of(7) || id == 100_002)
                as fn(u32) -> bool,
        ),
        (
            SetExpr::subtract(&a, &b).unwrap(),
            (|id: u32| id.is_multiple_of(3) && !id.is_multiple_of(7) && id != 100_002)
                as fn(u32) -> bool,
        ),
    ] {
        let expected = (0..universe.len() as u32)
            .filter(|id| predicate(*id))
            .collect::<Vec<_>>();
        assert_eq!(expr.count(&memory).unwrap(), expected.len() as u64);
        assert_eq!(collect(&expr, &memory), expected);
        let frozen = expr.materialize(&memory).unwrap();
        assert_eq!(frozen.len(), expected.len() as u64);
        assert_eq!(collect(&SetExpr::leaf(&frozen), &memory), expected);
    }
}

#[test]
fn cursor_observes_without_consuming_and_never_rewinds() {
    let memory = SetMemory::new(1 << 20);
    let sparse = fixture(docs(130), &[1, 63, 65, 129], false, &memory);
    let dense = fixture(docs(130), &[0, 63, 64, 129], true, &memory);
    let left = SetExpr::leaf(&sparse);
    let right = SetExpr::leaf(&dense);
    let expr = SetExpr::or(&left, &right).unwrap();
    let mut cursor = expr.cursor(&memory).unwrap();
    assert_eq!(cursor.current().map(SetId::raw), Some(0));
    assert_eq!(cursor.current().map(SetId::raw), Some(0));
    assert_eq!(cursor.advance(DocId(63)).map(SetId::raw), Some(63));
    assert_eq!(cursor.advance(DocId(1)).map(SetId::raw), Some(63));
    assert_eq!(cursor.next().map(SetId::raw), Some(63));
    assert_eq!(cursor.next().map(SetId::raw), Some(64));
    assert_eq!(cursor.advance(DocId(129)).map(SetId::raw), Some(129));
    assert_eq!(cursor.next().map(SetId::raw), Some(129));
    assert_eq!(cursor.current(), None);
    assert_eq!(cursor.advance(DocId(0)), None);
    assert_eq!(cursor.next(), None);
}

#[test]
fn cursor_seeks_across_large_sparse_and_dense_gaps() {
    let memory = SetMemory::new(1 << 22);
    let universe = docs(MAX_UNIVERSE_LEN);
    let sparse_ids = (0..100_000_u32).chain([u32::MAX]).collect::<Vec<_>>();
    let sparse = fixture(universe, &sparse_ids, false, &memory);
    let expr = SetExpr::leaf(&sparse);
    let mut cursor = expr.cursor(&memory).unwrap();
    assert_eq!(cursor.advance(DocId(u32::MAX)), Some(DocId(u32::MAX)));
    assert!(matches!(
        cursor.node,
        CursorNode::Sparse {
            offset: 100_000,
            ..
        }
    ));
    assert_eq!(cursor.advance(DocId(0)), Some(DocId(u32::MAX)));
    assert_eq!(cursor.next(), Some(DocId(u32::MAX)));
    assert_eq!(cursor.advance(DocId(0)), None);

    let dense = fixture(docs(257), &[0, 63, 64, 65, 255, 256], true, &memory);
    let expr = SetExpr::leaf(&dense);
    let mut cursor = expr.cursor(&memory).unwrap();
    for target in [63, 64, 65, 66, 256] {
        let expected = match target {
            66 => 255,
            _ => target,
        };
        assert_eq!(cursor.advance(DocId(target)), Some(DocId(expected)));
    }
    assert!(matches!(
        cursor.node,
        CursorNode::Dense { word_index: 4, .. }
    ));
    assert_eq!(cursor.next(), Some(DocId(256)));
    assert_eq!(cursor.advance(DocId(0)), None);
}

#[test]
fn compound_cursors_seek_monotonically_across_gaps() {
    let memory = SetMemory::new(1 << 20);
    let universe = docs(100_001);
    let a = fixture(universe, &[1, 50_000, 100_000], false, &memory);
    let b = fixture(universe, &[1, 75_000, 100_000], false, &memory);
    let left = SetExpr::leaf(&a);
    let right = SetExpr::leaf(&b);
    for (expr, expected) in [
        (SetExpr::and(&left, &right).unwrap(), 100_000),
        (SetExpr::or(&left, &right).unwrap(), 75_000),
        (SetExpr::subtract(&right, &left).unwrap(), 75_000),
    ] {
        let mut cursor = expr.cursor(&memory).unwrap();
        assert_eq!(cursor.advance(DocId(50_001)), Some(DocId(expected)));
        assert_eq!(cursor.advance(DocId(1)), Some(DocId(expected)));
    }
}

#[test]
fn expression_depth_is_bounded_before_cursor_allocation() {
    fn descend<'a>(
        parent: &'a SetExpr<'a, DocId>,
        leaf: &'a SetExpr<'a, DocId>,
        remaining: u32,
        memory: &Arc<SetMemory>,
    ) {
        let expr = SetExpr::and(parent, leaf).unwrap();
        if remaining == 0 {
            assert!(matches!(
                expr.cursor(memory),
                Err(SetError::ExpressionTooDeep)
            ));
        } else {
            descend(&expr, leaf, remaining - 1, memory);
        }
    }
    let memory = SetMemory::new(1 << 20);
    let set = fixture(docs(1), &[0], false, &memory);
    let leaf = SetExpr::leaf(&set);
    descend(&leaf, &leaf, MAX_EXPRESSION_DEPTH, &memory);
    assert_eq!(memory.live_bytes(), set.owned_bytes());
}

#[test]
fn shared_expression_graphs_are_bounded_by_expanded_node_count() {
    fn double<'a>(parent: &'a SetExpr<'a, DocId>, remaining: u32, memory: &Arc<SetMemory>) {
        let expr = SetExpr::or(parent, parent).unwrap();
        if remaining == 0 {
            assert!(matches!(
                expr.cursor(memory),
                Err(SetError::ExpressionTooLarge)
            ));
        } else {
            double(&expr, remaining - 1, memory);
        }
    }
    let memory = SetMemory::new(1 << 20);
    let set = fixture(docs(1), &[0], false, &memory);
    let leaf = SetExpr::leaf(&set);
    double(&leaf, 11, &memory);
    assert_eq!(memory.live_bytes(), set.owned_bytes());
}

#[test]
fn budget_charges_live_sets_and_releases_failed_or_dropped_outputs() {
    let cursor_bytes = 3 * size_of::<CursorNode<'_>>() as u64;
    let memory = SetMemory::new(32 + cursor_bytes + 15);
    let left = DocSet::from_sorted_raw(docs(128), &[1, 2, 3, 4], &memory).unwrap();
    assert_eq!(memory.live_bytes(), left.owned_bytes());
    let right = DocSet::from_sorted_raw(docs(128), &[5, 6, 7, 8], &memory).unwrap();
    let a = SetExpr::leaf(&left);
    let b = SetExpr::leaf(&right);
    let expr = SetExpr::or(&a, &b).unwrap();
    assert_eq!(expr.count(&memory).unwrap(), 8);
    assert!(matches!(
        expr.materialize(&memory),
        Err(SetError::BudgetExceeded { .. })
    ));
    assert_eq!(
        memory.live_bytes(),
        left.owned_bytes() + right.owned_bytes()
    );
    drop(right);
    drop(left);
    assert_eq!(memory.live_bytes(), 0);
    assert!(memory.peak_bytes() <= memory.limit);

    let tiny = SetMemory::new(15);
    assert!(matches!(
        DocSet::from_sorted_raw(docs(128), &[1, 2, 3, 4], &tiny),
        Err(SetError::BudgetExceeded { .. })
    ));
    assert_eq!(tiny.live_bytes(), 0);
}
