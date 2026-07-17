use crate::{IngestError, Result, TimestampMs, TranscriptChunk, TranscriptSegment, VideoMetadata};
use std::{collections::BTreeMap, fmt::Write};
use youtube_time::parse_rfc3339_timestamp;

pub const DEFAULT_CHUNK_SECONDS: u32 = 5;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chunker {
    window_ms: TimestampMs,
}

impl Chunker {
    pub fn five_seconds() -> Self {
        Self::new(DEFAULT_CHUNK_SECONDS)
    }

    pub fn new(window_seconds: u32) -> Self {
        let safe_seconds = window_seconds.max(1);
        Self {
            window_ms: safe_seconds.saturating_mul(1000),
        }
    }

    pub fn chunks(
        &self,
        metadata: &VideoMetadata,
        transcript: &[TranscriptSegment],
    ) -> Result<Vec<TranscriptChunk>> {
        let grouped_text = self.group_segments(transcript);
        if grouped_text.is_empty() {
            return Err(IngestError::EmptyTranscript {
                video_id: metadata.video_id.clone(),
            });
        }

        Ok(build_chunks(metadata, &grouped_text))
    }

    fn group_segments(&self, transcript: &[TranscriptSegment]) -> Vec<GroupedText> {
        let mut groups = Vec::with_capacity(transcript.len());
        let mut current: Option<GroupedText> = None;

        for segment in transcript {
            if segment.end_ms <= segment.start_ms || !has_text(&segment.text) {
                continue;
            }

            let Some(group) = &mut current else {
                current = Some(GroupedText::from_segment(segment));
                continue;
            };

            let group_duration = segment.end_ms.saturating_sub(group.start_ms);
            if group_duration <= self.window_ms {
                group.push_segment(segment);
                continue;
            }

            if let Some(group) = current.take() {
                groups.push(group);
            }
            current = Some(GroupedText::from_segment(segment));
        }

        if let Some(group) = current {
            groups.push(group);
        }

        groups
    }
}

#[derive(Debug, Eq, PartialEq)]
struct GroupedText {
    start_ms: TimestampMs,
    end_ms: TimestampMs,
    text: String,
}

impl GroupedText {
    fn from_segment(segment: &TranscriptSegment) -> Self {
        let mut text = String::with_capacity(segment.text.trim().len());
        append_normalized_text(&mut text, &segment.text);

        Self {
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            text,
        }
    }

    fn push_segment(&mut self, segment: &TranscriptSegment) {
        self.end_ms = self.end_ms.max(segment.end_ms);
        self.text.reserve(segment.text.len() + 1);
        self.text.push(' ');
        append_normalized_text(&mut self.text, &segment.text);
    }
}

fn build_chunks(metadata: &VideoMetadata, grouped_text: &[GroupedText]) -> Vec<TranscriptChunk> {
    let mut chunks = Vec::with_capacity(grouped_text.len());

    for (sequence, group) in grouped_text.iter().enumerate() {
        let prev_text = previous_text(grouped_text, sequence);
        let next_text = next_text(grouped_text, sequence);

        chunks.push(TranscriptChunk {
            id: chunk_id(&metadata.video_id, sequence),
            youtuber_id: metadata.youtuber.id.clone(),
            youtuber_handle: metadata.youtuber.handle.clone(),
            youtuber_name: metadata.youtuber.name.clone(),
            channel_id: metadata.youtuber.channel_id.clone(),
            video_id: metadata.video_id.clone(),
            title: metadata.title.clone(),
            text: group.text.clone(),
            prev_text,
            next_text,
            sequence: sequence as u32,
            chapter: None,
            chapter_index: None,
            duration: metadata.duration_ms,
            t_start_ms: group.start_ms,
            t_end_ms: group.end_ms,
            thumbnail: metadata.thumbnail.clone(),
            upload_date: metadata.upload_date.clone(),
            published_at: metadata.published_at.clone(),
            published_at_timestamp: metadata
                .published_at
                .as_deref()
                .and_then(parse_rfc3339_timestamp),
            uploader_id: metadata.youtuber.handle.clone(),
            embedding: None,
            extra: BTreeMap::new(),
        });
    }

    chunks
}

fn previous_text(grouped_text: &[GroupedText], sequence: usize) -> Option<String> {
    sequence
        .checked_sub(1)
        .and_then(|index| grouped_text.get(index))
        .map(|prev| prev.text.clone())
}

fn next_text(grouped_text: &[GroupedText], sequence: usize) -> Option<String> {
    grouped_text.get(sequence + 1).map(|next| next.text.clone())
}

fn chunk_id(video_id: &str, sequence: usize) -> String {
    let sequence_digits = if sequence == 0 {
        1
    } else {
        sequence.ilog10() as usize + 1
    };
    let mut id = String::with_capacity(video_id.len() + 1 + sequence_digits);
    id.push_str(video_id);
    id.push('_');
    write!(&mut id, "{sequence}").expect("writing to String cannot fail");
    id
}

fn has_text(text: &str) -> bool {
    text.split_whitespace().next().is_some()
}

fn append_normalized_text(output: &mut String, text: &str) {
    for word in text.split_whitespace() {
        if !output.is_empty() && !output.ends_with(' ') {
            output.push(' ');
        }
        output.push_str(word);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Youtuber;

    fn metadata() -> VideoMetadata {
        VideoMetadata {
            video_id: "abc123".to_string(),
            title: "A useful video".to_string(),
            youtuber: Youtuber {
                id: "t3dotgg".to_string(),
                handle: "@t3dotgg".to_string(),
                name: "Theo - t3.gg".to_string(),
                channel_id: "channel-1".to_string(),
            },
            duration_ms: Some(12_000),
            thumbnail: Some("https://example.com/thumb.jpg".to_string()),
            published_at: Some("2026-01-01T00:00:00Z".to_string()),
            upload_date: Some("20260101".to_string()),
        }
    }

    #[test]
    fn builds_five_second_chunks_with_context() {
        let transcript = vec![
            TranscriptSegment {
                start_ms: 0,
                end_ms: 2_000,
                text: "hello".to_string(),
            },
            TranscriptSegment {
                start_ms: 2_000,
                end_ms: 4_900,
                text: "world".to_string(),
            },
            TranscriptSegment {
                start_ms: 5_000,
                end_ms: 8_000,
                text: "second chunk".to_string(),
            },
        ];

        let chunks = Chunker::five_seconds()
            .chunks(&metadata(), &transcript)
            .unwrap();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].id, "abc123_0");
        assert_eq!(chunks[0].text, "hello world");
        assert_eq!(chunks[0].next_text.as_deref(), Some("second chunk"));
        assert_eq!(chunks[1].prev_text.as_deref(), Some("hello world"));
        assert_eq!(chunks[1].t_start_ms, 5_000);
    }

    #[test]
    fn normalizes_text_without_intermediate_word_vector() {
        let transcript = vec![TranscriptSegment {
            start_ms: 0,
            end_ms: 1_000,
            text: "  hello\n\t  world  ".to_string(),
        }];

        let chunks = Chunker::five_seconds()
            .chunks(&metadata(), &transcript)
            .unwrap();

        assert_eq!(chunks[0].text, "hello world");
    }

    #[test]
    fn rejects_empty_transcripts_clearly() {
        let transcript = vec![TranscriptSegment {
            start_ms: 0,
            end_ms: 1_000,
            text: "   ".to_string(),
        }];

        let error = Chunker::five_seconds()
            .chunks(&metadata(), &transcript)
            .unwrap_err();

        assert!(matches!(error, IngestError::EmptyTranscript { .. }));
        assert_eq!(
            error.to_string(),
            "video abc123 has no usable transcript segments"
        );
    }
}
