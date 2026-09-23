//! Local paired measurements on a checksum-verified active schema-6 corpus.
//! This is P01 representation evidence, not a production reader benchmark.

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs::File,
    hint::black_box,
    io::{self, Read},
    path::Path,
    time::Instant,
};

use memmap2::{Mmap, MmapOptions};
use serde_json::{Value, json};

const PAIRS: [(&str, &str); 9] = [
    ("the", "and"),
    ("in", "the"),
    ("this", "is"),
    ("is", "the"),
    ("coding", "dead"),
    ("css", "dead"),
    ("create", "effect"),
    ("black", "hole"),
    ("one", "fire"),
];
const ROUNDS: usize = 31;
const WARMUP: usize = 5;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn u16_at(data: &[u8], offset: usize) -> Result<u16, Box<dyn Error>> {
    Ok(u16::from_le_bytes(
        data.get(offset..offset + 2)
            .ok_or_else(|| invalid("truncated u16"))?
            .try_into()?,
    ))
}

fn u32_at(data: &[u8], offset: usize) -> Result<u32, Box<dyn Error>> {
    Ok(u32::from_le_bytes(
        data.get(offset..offset + 4)
            .ok_or_else(|| invalid("truncated u32"))?
            .try_into()?,
    ))
}

fn u64_at(data: &[u8], offset: usize) -> Result<u64, Box<dyn Error>> {
    Ok(u64::from_le_bytes(
        data.get(offset..offset + 8)
            .ok_or_else(|| invalid("truncated u64"))?
            .try_into()?,
    ))
}

fn checked_mmap(path: &Path, expected_digest: &str) -> Result<Mmap, Box<dyn Error>> {
    let file = File::open(path)?;
    // SAFETY: the retained local input is immutable for this read-only run.
    let map = unsafe { MmapOptions::new().map(&file)? };
    if blake3::hash(&map).to_hex().as_str() != expected_digest {
        return Err(invalid("input BLAKE3 differs from the active manifest").into());
    }
    Ok(map)
}

fn term_descriptor(lexicon: &[u8], wanted: &str) -> Result<(usize, usize, usize), Box<dyn Error>> {
    if lexicon.get(..8) != Some(b"RAWLEX01") || u32_at(lexicon, 12)? != 1 {
        return Err(invalid("invalid lexicon header").into());
    }
    let count = u32_at(lexicon, 8)? as usize;
    let entries_start = 16 + (count + 1) * 8;
    if entries_start > lexicon.len()
        || entries_start + u64_at(lexicon, 16 + count * 8)? as usize != lexicon.len()
    {
        return Err(invalid("invalid lexicon directory").into());
    }
    let mut low = 0;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        let start = entries_start + u64_at(lexicon, 16 + middle * 8)? as usize;
        let end = entries_start + u64_at(lexicon, 16 + (middle + 1) * 8)? as usize;
        let term_len = u16_at(lexicon, start)? as usize;
        let fields = start + 2 + term_len;
        if fields + 32 != end || end > lexicon.len() {
            return Err(invalid("invalid lexicon entry").into());
        }
        match lexicon[start + 2..fields].cmp(wanted.as_bytes()) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => {
                return Ok((
                    u64_at(lexicon, fields)? as usize,
                    u32_at(lexicon, fields + 8)? as usize,
                    u32_at(lexicon, fields + 12)? as usize,
                ));
            }
        }
    }
    Err(invalid("required benchmark term is absent").into())
}

fn varint(bytes: &[u8], cursor: &mut usize) -> Result<u32, Box<dyn Error>> {
    let mut value = 0_u32;
    for shift in (0..=28).step_by(7) {
        let byte = *bytes
            .get(*cursor)
            .ok_or_else(|| invalid("truncated varint"))?;
        *cursor += 1;
        if shift == 28 && byte > 0x0f {
            return Err(invalid("varint overflow").into());
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(invalid("varint exceeds five bytes").into())
}

fn decode(
    postings: &[u8],
    descriptor: (usize, usize, usize),
    documents: u32,
) -> Result<Vec<u32>, Box<dyn Error>> {
    let (offset, length, count) = descriptor;
    let bytes = postings
        .get(offset..offset + length)
        .ok_or_else(|| invalid("posting range is outside file"))?;
    let mut output = Vec::with_capacity(count);
    let mut cursor = 0;
    let mut previous = 0_u32;
    while cursor < bytes.len() {
        let delta = varint(bytes, &mut cursor)?;
        let frequency = varint(bytes, &mut cursor)?;
        if frequency == 0 || (!output.is_empty() && delta == 0) {
            return Err(invalid("invalid posting frequency or delta").into());
        }
        previous = previous
            .checked_add(delta)
            .ok_or_else(|| invalid("posting ID overflow"))?;
        if previous >= documents || output.len() >= count {
            return Err(invalid("posting ID or count exceeds the lexicon").into());
        }
        output.push(previous);
    }
    if output.len() != count {
        return Err(invalid("posting count differs from lexicon DF").into());
    }
    Ok(output)
}

fn bitmap(ids: &[u32], words: usize) -> Vec<u64> {
    let mut bitmap = vec![0_u64; words];
    for &id in ids {
        bitmap[id as usize / 64] |= 1_u64 << (id % 64);
    }
    bitmap
}

fn bitmap_count(left: &[u64], right: &[u64]) -> u64 {
    left.iter()
        .zip(right)
        .map(|(a, b)| u64::from((a & b).count_ones()))
        .sum()
}

fn sparse_linear_count(left: &[u32], right: &[u32]) -> u64 {
    let (mut i, mut j, mut count) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        match left[i].cmp(&right[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                count += 1;
                i += 1;
                j += 1;
            }
        }
    }
    count
}

fn sparse_seek_count(left: &[u32], right: &[u32]) -> u64 {
    let (short, long) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let mut long_offset = 0;
    let mut count = 0;
    for &id in short {
        long_offset += long[long_offset..].partition_point(|&candidate| candidate < id);
        if long.get(long_offset) == Some(&id) {
            count += 1;
            long_offset += 1;
        }
        if long_offset == long.len() {
            break;
        }
    }
    count
}

fn sample<T>(action: impl FnOnce() -> T) -> (T, u128) {
    let start = Instant::now();
    let result = black_box(action());
    (result, start.elapsed().as_nanos())
}

fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 9 {
        return Err(invalid("usage: active_set_measure LEXICON POSTINGS OVERLAP_JSON DOCUMENTS LEXICON_BLAKE3 POSTINGS_BLAKE3 SOURCE_COMMIT OUTPUT_JSON").into());
    }
    let documents = args[4].parse::<u32>()?;
    let lexicon = checked_mmap(Path::new(&args[1]), &args[5])?;
    let postings = checked_mmap(Path::new(&args[2]), &args[6])?;
    if postings.get(..8) != Some(b"RAWPOS01") || u32_at(&postings, 12)? != 1 {
        return Err(invalid("invalid postings header").into());
    }
    if u32_at(&lexicon, 8)? != u32_at(&postings, 8)? {
        return Err(invalid("posting and lexicon term counts differ").into());
    }
    let mut oracle_source = String::new();
    File::open(&args[3])?.read_to_string(&mut oracle_source)?;
    let oracle: Value = serde_json::from_str(&oracle_source)?;
    let expected_pairs = oracle["analysis"]["pairs"]
        .as_array()
        .ok_or_else(|| invalid("overlap receipt has no pair array"))?;
    let mut ids = BTreeMap::<String, Vec<u32>>::new();
    let mut bitmaps = BTreeMap::<String, Vec<u64>>::new();
    let mut construction = Vec::new();
    let bitmap_words = (usize::try_from(documents)?.div_ceil(64)).max(1);
    for term in PAIRS.iter().flat_map(|(a, b)| [*a, *b]) {
        if ids.contains_key(term) {
            continue;
        }
        let descriptor = term_descriptor(&lexicon, term)?;
        let (decoded, decode_ns) = sample(|| decode(&postings, descriptor, documents));
        let decoded = decoded?;
        let (dense, bitmap_ns) = sample(|| bitmap(&decoded, bitmap_words));
        construction.push(json!({
            "term": term,
            "df": decoded.len(),
            "posting_bytes": descriptor.1,
            "sparse_capacity_bytes": decoded.capacity() * std::mem::size_of::<u32>(),
            "bitmap_capacity_bytes": dense.capacity() * std::mem::size_of::<u64>(),
            "decode_ns": decode_ns,
            "bitmap_build_ns": bitmap_ns,
        }));
        ids.insert(term.to_owned(), decoded);
        bitmaps.insert(term.to_owned(), dense);
    }
    let mut cases = Vec::new();
    for (left, right) in PAIRS {
        let a = &ids[left];
        let b = &ids[right];
        let dense_a = &bitmaps[left];
        let dense_b = &bitmaps[right];
        let oracle_count = expected_pairs
            .iter()
            .find(|item| item["left"] == left && item["right"] == right)
            .and_then(|item| item["text"]["intersection"].as_u64())
            .ok_or_else(|| invalid("missing exact overlap oracle"))?;
        if sparse_linear_count(a, b) != oracle_count
            || sparse_seek_count(a, b) != oracle_count
            || bitmap_count(dense_a, dense_b) != oracle_count
        {
            return Err(invalid("candidate count differs from independent overlap receipt").into());
        }
        for _ in 0..WARMUP {
            black_box(sparse_linear_count(black_box(a), black_box(b)));
            black_box(sparse_seek_count(black_box(a), black_box(b)));
            black_box(bitmap_count(black_box(dense_a), black_box(dense_b)));
        }
        let mut linear_samples = Vec::with_capacity(ROUNDS);
        let mut seek_samples = Vec::with_capacity(ROUNDS);
        let mut bitmap_samples = Vec::with_capacity(ROUNDS);
        for round in 0..ROUNDS {
            if round % 2 == 0 {
                let (count, ns) = sample(|| bitmap_count(black_box(dense_a), black_box(dense_b)));
                assert_eq!(count, oracle_count);
                bitmap_samples.push(ns);
            }
            if round % 2 == 0 {
                let (count, ns) = sample(|| sparse_linear_count(black_box(a), black_box(b)));
                assert_eq!(count, oracle_count);
                linear_samples.push(ns);
                let (count, ns) = sample(|| sparse_seek_count(black_box(a), black_box(b)));
                assert_eq!(count, oracle_count);
                seek_samples.push(ns);
            } else {
                let (count, ns) = sample(|| sparse_seek_count(black_box(a), black_box(b)));
                assert_eq!(count, oracle_count);
                seek_samples.push(ns);
                let (count, ns) = sample(|| sparse_linear_count(black_box(a), black_box(b)));
                assert_eq!(count, oracle_count);
                linear_samples.push(ns);
            }
            if round % 2 != 0 {
                let (count, ns) = sample(|| bitmap_count(black_box(dense_a), black_box(dense_b)));
                assert_eq!(count, oracle_count);
                bitmap_samples.push(ns);
            }
        }
        let mut sorted_linear = linear_samples.clone();
        let mut sorted_seek = seek_samples.clone();
        let mut sorted_bitmap = bitmap_samples.clone();
        sorted_linear.sort_unstable();
        sorted_seek.sort_unstable();
        sorted_bitmap.sort_unstable();
        let linear_median = sorted_linear[ROUNDS / 2];
        let seek_median = sorted_seek[ROUNDS / 2];
        cases.push(json!({
            "left": left,
            "right": right,
            "exact_intersection": oracle_count,
            "sparse_linear_samples_ns": linear_samples,
            "sparse_seek_samples_ns": seek_samples,
            "bitmap_and_samples_ns": bitmap_samples,
            "sparse_linear_median_ns": linear_median,
            "sparse_seek_median_ns": seek_median,
            "best_sparse_median_ns": linear_median.min(seek_median),
            "bitmap_median_ns": sorted_bitmap[ROUNDS / 2],
            "best_sparse_method": if linear_median <= seek_median { "linear_merge" } else { "ordered_binary_seek" },
        }));
    }
    let report = json!({
        "schema_version": 1,
        "harness_source_commit": args[7],
        "input": {
            "documents": documents,
            "lexicon_blake3": args[5],
            "postings_blake3": args[6],
            "overlap_receipt_postings_sha256": oracle["source"]["postings_sha256"],
        },
        "rounds": ROUNDS,
        "warmup": WARMUP,
        "sparse_methods": ["linear merge", "ordered binary seeks in the longer list"],
        "construction": construction,
        "cases": cases,
        "peak_rss_bytes": peak_rss_bytes(),
        "limits": [
            "Warm local in-process count kernels on schema-6 postings, not the Fly 2 GiB reader.",
            "Bitmaps are already constructed during query timing; build time is reported separately.",
            "No phrase, ranking, exact video totals, moment summary, or publication overlap timing."
        ]
    });
    let output = Path::new(&args[8]);
    if output.exists() {
        return Err(invalid("refusing to overwrite output").into());
    }
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{bitmap, bitmap_count, sparse_linear_count, sparse_seek_count};

    #[test]
    fn sparse_and_bitmap_intersections_agree_across_size_ratios() {
        for (left, right) in [
            (vec![], vec![]),
            (vec![1, 3, 5], vec![2, 3, 4]),
            (vec![2, 100], (0..1000).collect()),
            ((0..1000).collect(), vec![2, 100]),
        ] {
            let expected = left.iter().filter(|id| right.contains(id)).count() as u64;
            assert_eq!(sparse_linear_count(&left, &right), expected);
            assert_eq!(sparse_seek_count(&left, &right), expected);
            assert_eq!(
                bitmap_count(&bitmap(&left, 16), &bitmap(&right, 16)),
                expected
            );
        }
    }
}
