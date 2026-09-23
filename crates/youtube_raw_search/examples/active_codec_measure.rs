//! Read-only P01 ID-codec comparison on a checksum-pinned schema-6 generation.
//! Candidate blocks use bit-packed deltas and a fixed-size skip directory.

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs::File,
    hint::black_box,
    io::{self, Write},
    path::Path,
    time::Instant,
};

use memmap2::{Mmap, MmapOptions};
use serde_json::{Value, json};

const BLOCKS: [usize; 4] = [128, 64, 128, 256];
const ROUNDS: usize = 7;
const PROBES: usize = 256;

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

fn checked_mmap(path: &Path, digest: &str) -> Result<Mmap, Box<dyn Error>> {
    let file = File::open(path)?;
    // SAFETY: the retained local corpus is immutable during this read-only run.
    let map = unsafe { MmapOptions::new().map(&file)? };
    if blake3::hash(&map).to_hex().as_str() != digest {
        return Err(invalid("input BLAKE3 differs from the active manifest").into());
    }
    Ok(map)
}

fn descriptor(lex: &[u8], wanted: &str) -> Result<(usize, usize, usize), Box<dyn Error>> {
    if lex.get(..8) != Some(b"RAWLEX01") || u32_at(lex, 12)? != 1 {
        return Err(invalid("invalid lexicon header").into());
    }
    let count = u32_at(lex, 8)? as usize;
    let table = 16_usize
        .checked_add(
            (count + 1)
                .checked_mul(8)
                .ok_or_else(|| invalid("lexicon table overflow"))?,
        )
        .ok_or_else(|| invalid("lexicon table overflow"))?;
    if table.checked_add(u64_at(lex, 16 + count * 8)? as usize) != Some(lex.len()) {
        return Err(invalid("invalid lexicon table").into());
    }
    let (mut low, mut high) = (0, count);
    while low < high {
        let middle = low + (high - low) / 2;
        let start = table + u64_at(lex, 16 + middle * 8)? as usize;
        let end = table + u64_at(lex, 16 + (middle + 1) * 8)? as usize;
        let term_length = u16_at(lex, start)? as usize;
        let fields = start + 2 + term_length;
        if fields + 32 != end || end > lex.len() {
            return Err(invalid("invalid lexicon entry").into());
        }
        match lex[start + 2..fields].cmp(wanted.as_bytes()) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => {
                return Ok((
                    u64_at(lex, fields)? as usize,
                    u32_at(lex, fields + 8)? as usize,
                    u32_at(lex, fields + 12)? as usize,
                ));
            }
        }
    }
    Err(invalid("benchmark term absent").into())
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u32, Box<dyn Error>> {
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
            if shift != 0 && byte == 0 {
                return Err(invalid("noncanonical varint").into());
            }
            return Ok(value);
        }
    }
    Err(invalid("varint exceeds five bytes").into())
}

fn source_ids(
    postings: &[u8],
    (offset, length, count): (usize, usize, usize),
    universe: u32,
) -> Result<Vec<u32>, Box<dyn Error>> {
    let bytes = postings
        .get(offset..offset + length)
        .ok_or_else(|| invalid("posting range outside file"))?;
    let mut ids = Vec::with_capacity(count);
    let (mut cursor, mut previous) = (0, 0_u32);
    while cursor < bytes.len() {
        let delta = read_varint(bytes, &mut cursor)?;
        let freq = read_varint(bytes, &mut cursor)?;
        if freq == 0 || (!ids.is_empty() && delta == 0) {
            return Err(invalid("invalid source posting").into());
        }
        previous = previous
            .checked_add(delta)
            .ok_or_else(|| invalid("posting ID overflow"))?;
        if previous >= universe || ids.len() >= count {
            return Err(invalid("posting ID/count exceeds lexicon").into());
        }
        ids.push(previous);
    }
    if ids.len() != count {
        return Err(invalid("posting count differs from lexicon DF").into());
    }
    Ok(ids)
}

fn append_varint(mut value: u32, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

#[derive(Clone, Copy)]
struct Entry {
    max: u32,
    offset: u32,
    previous: u32,
    count: u16,
    width: u8,
}

struct Encoded {
    payload: Vec<u8>,
    entries: Vec<Entry>,
    block: usize,
    packed: bool,
}

impl Encoded {
    fn encoded_bytes(&self) -> usize {
        self.payload.len() + self.entries.len() * if self.packed { 16 } else { 12 }
    }
    fn retained_bytes(&self) -> usize {
        self.payload.capacity() + self.entries.capacity() * std::mem::size_of::<Entry>()
    }
}

fn encode(ids: &[u32], block: usize, packed: bool) -> Encoded {
    let mut output = Encoded {
        payload: Vec::new(),
        entries: Vec::with_capacity(ids.len().div_ceil(block)),
        block,
        packed,
    };
    let mut previous = 0_u32;
    for chunk in ids.chunks(block) {
        let offset = output.payload.len() as u32;
        let base = previous;
        let mut deltas = Vec::with_capacity(chunk.len());
        for &id in chunk {
            deltas.push(id - previous);
            previous = id;
        }
        let width = if packed {
            32 - deltas.iter().copied().max().unwrap_or(0).leading_zeros() as u8
        } else {
            0
        };
        output.entries.push(Entry {
            max: previous,
            offset,
            previous: base,
            count: chunk.len() as u16,
            width,
        });
        if packed {
            let bits = usize::from(width) * deltas.len();
            output
                .payload
                .resize(output.payload.len() + bits.div_ceil(8), 0);
            let slice = &mut output.payload[offset as usize..];
            for (index, &delta) in deltas.iter().enumerate() {
                let mut value = delta;
                let bit = index * usize::from(width);
                let byte = bit / 8;
                let shift = bit % 8;
                let mut consumed = 0;
                while consumed < usize::from(width) {
                    slice[byte + (shift + consumed) / 8] |=
                        (value as u8) << ((shift + consumed) % 8);
                    let take = 8 - (shift + consumed) % 8;
                    value >>= take.min(32);
                    consumed += take;
                }
            }
        } else {
            for delta in deltas {
                append_varint(delta, &mut output.payload);
            }
        }
    }
    output
}

fn packed_delta(payload: &[u8], entry: Entry, index: usize) -> u32 {
    let bit = index * usize::from(entry.width);
    let byte = entry.offset as usize + bit / 8;
    let shift = bit % 8;
    let mut word = 0_u64;
    for j in 0..5 {
        if let Some(&part) = payload.get(byte + j) {
            word |= u64::from(part) << (j * 8);
        }
    }
    let mask = if entry.width == 32 {
        u32::MAX as u64
    } else {
        (1_u64 << entry.width) - 1
    };
    ((word >> shift) & mask) as u32
}

fn decode(encoded: &Encoded) -> Result<Vec<u32>, Box<dyn Error>> {
    let mut ids = Vec::with_capacity(encoded.entries.len() * encoded.block);
    for (i, &entry) in encoded.entries.iter().enumerate() {
        let end = encoded
            .entries
            .get(i + 1)
            .map_or(encoded.payload.len(), |next| next.offset as usize);
        let mut previous = entry.previous;
        let mut cursor = entry.offset as usize;
        for j in 0..usize::from(entry.count) {
            let delta = if encoded.packed {
                packed_delta(&encoded.payload, entry, j)
            } else {
                read_varint(&encoded.payload[..end], &mut cursor)?
            };
            previous = previous
                .checked_add(delta)
                .ok_or_else(|| invalid("candidate posting overflow"))?;
            ids.push(previous);
        }
        if previous != entry.max || (!encoded.packed && cursor != end) {
            return Err(invalid("candidate block differs from directory").into());
        }
    }
    Ok(ids)
}

fn seek(encoded: &Encoded, target: u32) -> Result<Option<u32>, Box<dyn Error>> {
    let block = encoded.entries.partition_point(|entry| entry.max < target);
    let Some(&entry) = encoded.entries.get(block) else {
        return Ok(None);
    };
    let end = encoded
        .entries
        .get(block + 1)
        .map_or(encoded.payload.len(), |next| next.offset as usize);
    let mut previous = entry.previous;
    let mut cursor = entry.offset as usize;
    for j in 0..usize::from(entry.count) {
        let delta = if encoded.packed {
            packed_delta(&encoded.payload, entry, j)
        } else {
            read_varint(&encoded.payload[..end], &mut cursor)?
        };
        previous += delta;
        if previous >= target {
            return Ok(Some(previous));
        }
    }
    Err(invalid("candidate skip directory is inconsistent").into())
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, u128) {
    let start = Instant::now();
    let result = black_box(f());
    (result, start.elapsed().as_nanos())
}

fn median(samples: &[u128]) -> u128 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

fn probes(universe: u32) -> Vec<u32> {
    let mut output = (0..PROBES)
        .map(|i| ((u64::from(universe) * i as u64) / PROBES as u64) as u32)
        .collect::<Vec<_>>();
    output.push(universe);
    output
}

fn overlap(a: &[u32], b: &[u32]) -> u64 {
    let (mut i, mut j, mut count) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
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

fn main() -> Result<(), Box<dyn Error>> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 9 {
        return Err(invalid("usage: active_codec_measure LEXICON POSTINGS OVERLAP_JSON DOCUMENTS LEXICON_BLAKE3 POSTINGS_BLAKE3 SOURCE_COMMIT OUTPUT_JSON").into());
    }
    if args[7].len() != 40 || !args[7].bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("source commit must be a full SHA").into());
    }
    let universe = args[4].parse::<u32>()?;
    let lex = checked_mmap(Path::new(&args[1]), &args[5])?;
    let postings = checked_mmap(Path::new(&args[2]), &args[6])?;
    if postings.get(..8) != Some(b"RAWPOS01")
        || u32_at(&postings, 12)? != 1
        || u32_at(&lex, 8)? != u32_at(&postings, 8)?
    {
        return Err(invalid("invalid posting header or term count").into());
    }
    let oracle: Value = serde_json::from_reader(File::open(&args[3])?)?;
    let expected_terms = oracle["analysis"]["terms"]
        .as_array()
        .ok_or_else(|| invalid("oracle terms missing"))?;
    let expected_pairs = oracle["analysis"]["pairs"]
        .as_array()
        .ok_or_else(|| invalid("oracle pairs missing"))?;
    if oracle["source"]["generation"] != 2950
        || oracle["source"]["documents"] != universe
        || oracle["source"]["lexicon_sha256"]
            != "7641ce43647ae24bba7706115bf2d22e5c1280f4292e3657bb985568841b9c9e"
        || oracle["source"]["postings_sha256"]
            != "15f698ac9ea5641032727120cd40fadb9711c4ad0c1592df5b54bbc8b3004c1e"
    {
        return Err(invalid("oracle source identity differs from pinned active generation").into());
    }
    let probes = probes(universe);
    let mut original = BTreeMap::new();
    let mut terms = Vec::new();
    for row in expected_terms {
        let name = row["term"].as_str().ok_or_else(|| invalid("term absent"))?;
        let source = descriptor(&lex, name)?;
        if row["text"]["df"] != source.2 || row["text"]["encoded_bytes"] != source.1 {
            return Err(invalid("source differs from independent term receipt").into());
        }
        let ids = source_ids(&postings, source, universe)?;
        let mut codecs = Vec::new();
        for (kind, &block) in BLOCKS.iter().enumerate() {
            let packed = kind != 0;
            let mut encode_ns = Vec::new();
            let mut decode_ns = Vec::new();
            let mut seek_ns = Vec::new();
            let mut encoded = encode(&ids, block, packed);
            if decode(&encoded)? != ids {
                return Err(invalid("candidate decode differs from source posting IDs").into());
            }
            for &probe in &probes {
                let expected = ids.get(ids.partition_point(|&id| id < probe)).copied();
                if seek(&encoded, probe)? != expected {
                    return Err(invalid("candidate seek differs from source posting IDs").into());
                }
            }
            for round in 0..ROUNDS {
                let (candidate, elapsed) = timed(|| encode(black_box(&ids), block, packed));
                encode_ns.push(elapsed);
                if round % 2 == 0 {
                    let (decoded, elapsed) = timed(|| decode(black_box(&candidate)));
                    if decoded? != ids {
                        return Err(invalid("timed decode differs from source").into());
                    }
                    decode_ns.push(elapsed);
                }
                let (result, elapsed) = timed(|| -> Result<u64, Box<dyn Error>> {
                    let mut sum = 0_u64;
                    for &probe in &probes {
                        sum +=
                            u64::from(seek(black_box(&candidate), black_box(probe))?.unwrap_or(0));
                    }
                    Ok(sum)
                });
                black_box(result?);
                seek_ns.push(elapsed);
                if round % 2 != 0 {
                    let (decoded, elapsed) = timed(|| decode(black_box(&candidate)));
                    if decoded? != ids {
                        return Err(invalid("timed decode differs from source").into());
                    }
                    decode_ns.push(elapsed);
                }
                encoded = candidate;
            }
            codecs.push(json!({
                "codec": if packed { "bitpacked_delta" } else { "delta_varint" },
                "block": block,
                "serialized_id_bytes": encoded.encoded_bytes(),
                "payload_bytes": encoded.payload.len(),
                "directory_entries": encoded.entries.len(),
                "directory_serialized_bytes": encoded.entries.len() * if packed { 16 } else { 12 },
                "retained_capacity_bytes": encoded.retained_bytes(),
                "encode_samples_ns": encode_ns,
                "encode_median_ns": median(&encode_ns),
                "decode_samples_ns": decode_ns,
                "decode_median_ns": median(&decode_ns),
                "seek_257_probes_samples_ns": seek_ns,
                "seek_257_probes_median_ns": median(&seek_ns),
            }));
        }
        terms.push(json!({ "term": name, "df": ids.len(), "schema6_source_posting_bytes_with_frequency": source.1, "codecs": codecs }));
        original.insert(name.to_owned(), ids);
    }
    for row in expected_pairs {
        let left = row["left"]
            .as_str()
            .ok_or_else(|| invalid("pair left absent"))?;
        let right = row["right"]
            .as_str()
            .ok_or_else(|| invalid("pair right absent"))?;
        let expected = row["text"]["intersection"]
            .as_u64()
            .ok_or_else(|| invalid("pair intersection absent"))?;
        if overlap(&original[left], &original[right]) != expected {
            return Err(invalid("independent pair overlap differs").into());
        }
    }
    let report = json!({
        "schema_version": 1,
        "harness_source_commit": args[7],
        "input": {"generation": 2950, "documents": universe, "lexicon_blake3": args[5], "postings_blake3": args[6], "overlap_receipt_source_commit": oracle["analyzer_source_commit"]},
        "rounds": ROUNDS,
        "probes": "256 evenly spaced universe targets plus universe sentinel; same targets for all codecs and terms",
        "terms": terms,
        "limits": [
            "Local release-build ID-only codec prototype; positions, frequencies, checksums, file footer, and schema-9 writer/reader are excluded.",
            "Candidate directory serialized sizes are projected fixed layouts; in-memory capacity uses Rust Vec capacity and Entry size.",
            "Warm sequential decode and point-seek kernels are not full query latency, publication, or a 2 GiB Fly memory measurement.",
            "Named terms from one schema-6 generation only; no corpus-wide codec size or workload distribution claim."
        ]
    });
    let output = Path::new(&args[8]);
    let mut file = File::options().write(true).create_new(true).open(output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    writeln!(file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BLOCKS, decode, encode, seek};

    #[test]
    fn codecs_round_trip_and_seek_block_edges() {
        for ids in [
            vec![],
            vec![0],
            vec![1, 128, 129, u32::MAX - 1],
            (0..600).map(|i| i * 3).collect(),
            (0..600).map(|i| i * 127).collect(),
        ] {
            for (kind, &block) in BLOCKS.iter().enumerate() {
                let encoded = encode(&ids, block, kind != 0);
                assert_eq!(decode(&encoded).unwrap(), ids);
                for target in [0, 1, 127, 128, 129, 255, 256, 1024, u32::MAX] {
                    let expected = ids.get(ids.partition_point(|&id| id < target)).copied();
                    assert_eq!(seek(&encoded, target).unwrap(), expected);
                }
            }
        }
    }
}
