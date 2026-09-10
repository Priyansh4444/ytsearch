use super::*;
use std::borrow::Cow;

#[derive(Clone, Copy)]
pub(super) enum PayloadEncoding {
    Raw,
    Zstd,
}

pub(super) fn encode(payload: &[u8]) -> Result<Vec<u8>> {
    let compressed = zstd::bulk::compress(payload, 1)?;
    let mut output = Vec::with_capacity(8 + compressed.len());
    output.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    output.extend_from_slice(&compressed);
    Ok(output)
}

pub(super) fn decode(
    payload: &[u8],
    encoding: PayloadEncoding,
    maximum_bytes: usize,
) -> Result<Cow<'_, [u8]>> {
    match encoding {
        PayloadEncoding::Raw => Ok(Cow::Borrowed(payload)),
        PayloadEncoding::Zstd => {
            let length = usize::try_from(read_u64(payload, 0)?)
                .map_err(|_| Error::Corrupt("broad decoded length overflows".into()))?;
            if !(ANSWER_HEADER_BYTES..=maximum_bytes).contains(&length) {
                return Err(Error::Corrupt(
                    "broad decoded length exceeds corpus bounds".into(),
                ));
            }
            let frame = &payload[8..];
            let frame_bytes = zstd::zstd_safe::find_frame_compressed_size(frame)
                .map_err(|_| Error::Corrupt("invalid broad Zstd frame".into()))?;
            if frame_bytes != frame.len() {
                return Err(Error::Corrupt(
                    "broad compressed answer has trailing frames or bytes".into(),
                ));
            }
            let decoded = zstd::bulk::decompress(frame, length).map_err(|error| {
                Error::Corrupt(format!("invalid compressed broad answer: {error}"))
            })?;
            if decoded.len() != length {
                return Err(Error::Corrupt("broad decoded length does not match".into()));
            }
            Ok(Cow::Owned(decoded))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_every_byte() {
        let payload: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
        let encoded = encode(&payload).unwrap();
        assert!(encoded.len() < payload.len());
        assert_eq!(
            &*decode(&encoded, PayloadEncoding::Zstd, payload.len()).unwrap(),
            payload
        );
        assert!(matches!(
            decode(&payload, PayloadEncoding::Raw, payload.len()).unwrap(),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn malformed_frames_and_declared_lengths_fail_closed() {
        let payload = vec![42; 1024];
        let encoded = encode(&payload).unwrap();
        assert!(decode(&encoded, PayloadEncoding::Zstd, 1023).is_err());
        for length in [0, 7, 8, encoded.len() - 1] {
            assert!(decode(&encoded[..length], PayloadEncoding::Zstd, 1024).is_err());
        }
        for declared in [0_u64, 1023, 1025, u64::MAX] {
            let mut changed = encoded.clone();
            changed[..8].copy_from_slice(&declared.to_le_bytes());
            assert!(decode(&changed, PayloadEncoding::Zstd, 2048).is_err());
        }
        let mut changed = encoded;
        changed[8] ^= 0xff;
        assert!(decode(&changed, PayloadEncoding::Zstd, 1024).is_err());
        let mut trailing = encode(&payload).unwrap();
        trailing.extend_from_slice(&zstd::bulk::compress(&[], 1).unwrap());
        assert!(decode(&trailing, PayloadEncoding::Zstd, 1024).is_err());
    }

    #[test]
    fn compressed_forged_group_count_is_rejected_before_allocation() {
        let mut payload = vec![0; ANSWER_HEADER_BYTES];
        payload[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        payload[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        let encoded = encode(&payload).unwrap();
        let decoded = decode(&encoded, PayloadEncoding::Zstd, 176).unwrap();
        assert!(
            matches!(decode_answer(&decoded, &(0..decoded.len())), Err(Error::Corrupt(message)) if message == "broad group count exceeds payload bytes")
        );
    }

    #[test]
    fn forged_document_count_is_rejected_before_allocation() {
        let mut payload = vec![0; ANSWER_HEADER_BYTES + GROUP_RECORD_BYTES];
        payload[..4].copy_from_slice(&1_u32.to_le_bytes());
        payload[12..16].copy_from_slice(&1_u32.to_le_bytes());
        payload[32..34].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(
            matches!(decode_answer(&payload, &(0..payload.len())), Err(Error::Corrupt(message)) if message == "broad document count exceeds payload bytes")
        );
    }

    #[test]
    #[ignore = "requires RAW_BROAD_PAYLOAD_FILE from a retained generation"]
    fn measure_retained_payload() {
        let path = std::env::var_os("RAW_BROAD_PAYLOAD_FILE").expect("set RAW_BROAD_PAYLOAD_FILE");
        assert!(std::fs::metadata(&path).unwrap().len() <= 64 * 1024 * 1024);
        let payload = std::fs::read(path).unwrap();
        let start = Instant::now();
        let encoded = encode(&payload).unwrap();
        let encode_us = start.elapsed().as_micros();
        let mut raw_times = Vec::new();
        let mut compressed_times = Vec::new();
        for _ in 0..100 {
            let start = Instant::now();
            let raw = decode_answer(std::hint::black_box(&payload), &(0..payload.len())).unwrap();
            raw_times.push(start.elapsed().as_micros());
            let start = Instant::now();
            let decoded = decode(
                std::hint::black_box(&encoded),
                PayloadEncoding::Zstd,
                payload.len(),
            )
            .unwrap();
            let answer = decode_answer(&decoded, &(0..decoded.len())).unwrap();
            compressed_times.push(start.elapsed().as_micros());
            assert_eq!(&*decoded, payload);
            assert_eq!(answer.found, raw.found);
            assert_eq!(answer.total_moments, raw.total_moments);
        }
        raw_times.sort_unstable();
        compressed_times.sort_unstable();
        println!(
            "{}",
            serde_json::json!({
                "raw_bytes": payload.len(), "zstd1_bytes": encoded.len(), "encode_us": encode_us,
                "samples": raw_times.len(), "raw_parse_p50_us": raw_times[50], "raw_parse_max_us": raw_times[99],
                "zstd_parse_p50_us": compressed_times[50], "zstd_parse_max_us": compressed_times[99],
                "payload_blake3": blake3::hash(&payload).to_hex().to_string(),
                "scope": "local warm decode+parse only; not request latency"
            })
        );
    }
}
