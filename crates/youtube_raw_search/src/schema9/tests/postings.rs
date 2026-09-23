use super::*;
use crate::schema9::sets::{GenerationId, SegmentId};

fn universe(len: u64) -> Universe<DocId> {
    Universe::new(GenerationId(11), SegmentId(4), len).unwrap()
}

fn ids(list: &PostingList) -> Vec<(u32, u32)> {
    let mut cursor = list.cursor();
    let mut output = Vec::new();
    while let Some(posting) = cursor.next().unwrap() {
        output.push((posting.doc.raw(), posting.frequency));
    }
    output
}

#[test]
fn round_trip_and_seek_all_skip_boundaries() {
    let postings = (0..16_400_u32)
        .map(|ordinal| (ordinal * 3, 1 + ordinal % 131))
        .collect::<Vec<_>>();
    let list = PostingList::encode(universe(50_000), &postings).unwrap();
    assert_eq!(list.len(), postings.len() as u64);
    assert!(list.encoded_bytes() > 0);
    assert_eq!(ids(&list), postings);
    assert_eq!(
        list.directories.each_ref().map(Vec::len),
        [5, 33, 257, 2050]
    );

    for step in LEVELS {
        for ordinal in [step - 1, step, step + 1, step * 2 - 1, step * 2] {
            let target = postings[ordinal].0;
            let mut cursor = list.cursor();
            assert_eq!(
                cursor
                    .advance(DocId::from_raw(target))
                    .unwrap()
                    .unwrap()
                    .doc
                    .raw(),
                target
            );
            assert_eq!(cursor.current().unwrap().unwrap().doc.raw(), target);
            assert_eq!(cursor.decoded_counts(), (1, BLOCK_SIZE as u64));
            assert_eq!(cursor.next().unwrap().unwrap().doc.raw(), target);
            assert_eq!(
                cursor
                    .advance(DocId::from_raw(target - 1))
                    .unwrap()
                    .unwrap()
                    .doc
                    .raw(),
                target + 3
            );
        }
    }
}

#[test]
fn seek_near_end_decodes_only_one_base_block() {
    let postings = (0..100_000_u32).map(|id| (id * 37, 1)).collect::<Vec<_>>();
    let list = PostingList::encode(universe(4_000_000), &postings).unwrap();
    let mut cursor = list.cursor();
    let target = postings[99_999].0;
    assert_eq!(
        cursor
            .advance(DocId::from_raw(target))
            .unwrap()
            .unwrap()
            .doc
            .raw(),
        target
    );
    assert_eq!(cursor.decoded_counts(), (1, BLOCK_SIZE as u64));
    assert_eq!(cursor.next().unwrap().unwrap().doc.raw(), target);
    assert_eq!(cursor.advance(DocId::from_raw(0)).unwrap(), None);
}

#[test]
fn seek_gap_and_full_u32_boundary() {
    let list = PostingList::encode(
        universe(u32::MAX as u64 + 1),
        &[(0, 1), (8, 300), (u32::MAX, u32::MAX)],
    )
    .unwrap();
    let mut cursor = list.cursor();
    assert_eq!(
        cursor
            .advance(DocId::from_raw(1))
            .unwrap()
            .unwrap()
            .doc
            .raw(),
        8
    );
    assert_eq!(
        cursor
            .advance(DocId::from_raw(0))
            .unwrap()
            .unwrap()
            .doc
            .raw(),
        8
    );
    assert_eq!(
        cursor
            .advance(DocId::from_raw(9))
            .unwrap()
            .unwrap()
            .doc
            .raw(),
        u32::MAX
    );
    assert_eq!(cursor.next().unwrap().unwrap().frequency, u32::MAX);
    assert_eq!(cursor.next().unwrap(), None);
    assert_eq!(cursor.advance(DocId::from_raw(0)).unwrap(), None);
}

#[test]
fn rejects_invalid_input_and_malformed_block_bytes() {
    let u = universe(100);
    assert!(matches!(
        PostingList::encode(u, &[(1, 1), (1, 2)]),
        Err(PostingError::Unsorted(1))
    ));
    assert!(matches!(
        PostingList::encode(u, &[(2, 1), (1, 2)]),
        Err(PostingError::Unsorted(1))
    ));
    assert!(matches!(
        PostingList::encode(u, &[(100, 1)]),
        Err(PostingError::OutOfRange { .. })
    ));
    assert!(matches!(
        PostingList::encode(u, &[(1, 0)]),
        Err(PostingError::ZeroFrequency(0))
    ));
    let empty = PostingList::encode(u, &[]).unwrap();
    assert_eq!(empty.cursor().current().unwrap(), None);
    let mut list = PostingList::encode(u, &[(1, 1), (2, 1)]).unwrap();
    list.bytes.pop();
    assert!(matches!(
        list.cursor().current(),
        Err(PostingError::Malformed)
    ));
    let mut list = PostingList::encode(u, &[(1, 1)]).unwrap();
    list.bytes[5] = 0;
    assert!(matches!(
        list.cursor().current(),
        Err(PostingError::Malformed)
    ));
    let mut list = PostingList::encode(u, &[(1, 1)]).unwrap();
    list.bytes.splice(5..6, [0x81, 0x00]);
    assert!(matches!(
        list.cursor().current(),
        Err(PostingError::Malformed)
    ));
    let mut list = PostingList::encode(u, &[(1, 1), (2, 1)]).unwrap();
    list.bytes[6] = 0;
    assert!(matches!(
        list.cursor().current(),
        Err(PostingError::Malformed)
    ));
    let mut list = PostingList::encode(u, &[(1, 1), (2, 1)]).unwrap();
    list.bytes[0] = 3;
    assert!(matches!(
        list.cursor().current(),
        Err(PostingError::Malformed)
    ));
}

#[test]
fn randomized_seeks_match_a_sorted_vector_oracle() {
    let postings = (0..300_000_u32)
        .filter(|id| id % 7 == 0 || id % 113 == 0)
        .map(|id| (id, id % 17 + 1))
        .collect::<Vec<_>>();
    let list = PostingList::encode(universe(300_000), &postings).unwrap();
    let mut seed = 0x734c_5d91_u32;
    for _ in 0..1000 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let target = seed % 300_001;
        let expected = postings
            .get(postings.partition_point(|item| item.0 < target))
            .copied();
        let mut cursor = list.cursor();
        let actual = cursor
            .advance(DocId::from_raw(target))
            .unwrap()
            .map(|posting| (posting.doc.raw(), posting.frequency));
        assert_eq!(actual, expected, "target={target}");
        assert!(cursor.decoded_counts().0 <= 1);
        assert!(cursor.decoded_counts().1 <= BLOCK_SIZE as u64);
    }
}

#[test]
fn malformed_later_block_poison_cursor_without_panicking() {
    let postings = (0..9_u32).map(|doc| (doc, 1)).collect::<Vec<_>>();
    let mut list = PostingList::encode(universe(9), &postings).unwrap();
    let second_block = list.directories[3][1].byte_offset as usize;
    list.bytes[second_block + 5] = 0;
    let mut cursor = list.cursor();
    for doc in 0..7 {
        assert_eq!(cursor.next().unwrap().unwrap().doc.raw(), doc);
    }
    assert!(matches!(cursor.next(), Err(PostingError::Malformed)));
    assert!(matches!(cursor.current(), Err(PostingError::Malformed)));
    assert!(matches!(cursor.next(), Err(PostingError::Malformed)));
    assert!(matches!(
        cursor.advance(DocId::from_raw(8)),
        Err(PostingError::Malformed)
    ));
    let mut direct_seek = list.cursor();
    assert!(matches!(
        direct_seek.advance(DocId::from_raw(8)),
        Err(PostingError::Malformed)
    ));
    assert!(matches!(
        direct_seek.current(),
        Err(PostingError::Malformed)
    ));
}
