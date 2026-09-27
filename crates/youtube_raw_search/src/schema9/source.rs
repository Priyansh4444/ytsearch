//! Provenance-preserving parse of one canonical chunk before ID reassignment.

use serde_json::{Map, Value};
use youtube_time::parse_rfc3339_timestamp;

const MAX_SOURCE_LINE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceKind {
    Unknown,
    Short,
    Video,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceDocument {
    pub(crate) external_id: String,
    pub(crate) external_video_id: String,
    pub(crate) creator_id: String,
    pub(crate) title: String,
    pub(crate) text: String,
    pub(crate) t_start_ms: u32,
    pub(crate) t_end_ms: u32,
    pub(crate) duration_ms: Option<u32>,
    pub(crate) published_at_s: Option<i64>,
    pub(crate) views: Option<u64>,
    pub(crate) likes: Option<u64>,
    pub(crate) comments: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SourceError {
    #[error("source chunk exceeds the 16 MiB line limit")]
    Size,
    #[error("source chunk JSON is malformed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("source chunk field {0} is absent or invalid")]
    Field(&'static str),
    #[error("source chunk timestamps or metadata disagree")]
    Conflict,
}

impl SourceDocument {
    pub(crate) fn parse_json_line(line: &str) -> Result<Self, SourceError> {
        if line.len() > MAX_SOURCE_LINE_BYTES {
            return Err(SourceError::Size);
        }
        let value: Value = serde_json::from_str(line)?;
        let object = value.as_object().ok_or(SourceError::Field("object"))?;
        let external_id = required_string(object, "id")?.to_owned();
        let external_video_id = required_string(object, "video_id")?.to_owned();
        let creator_id = required_string(object, "youtuber_id")?.to_owned();
        let title = string(object, "title")?.to_owned();
        let text = string(object, "text")?.to_owned();
        let t_start_ms = required_u32(object, "t_start_ms")?;
        let t_end_ms = required_u32(object, "t_end_ms")?;
        if t_end_ms < t_start_ms {
            return Err(SourceError::Conflict);
        }
        let duration_ms = optional_u32(object, "duration")?;
        let timestamp = optional_i64(object, "published_at_timestamp")?;
        let parsed_date = match object.get("published_at") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => {
                Some(parse_rfc3339_timestamp(value).ok_or(SourceError::Field("published_at"))?)
            }
            _ => return Err(SourceError::Field("published_at")),
        };
        if timestamp.is_some() && parsed_date.is_some() && timestamp != parsed_date {
            return Err(SourceError::Conflict);
        }
        Ok(Self {
            external_id,
            external_video_id,
            creator_id,
            title,
            text,
            t_start_ms,
            t_end_ms,
            duration_ms,
            published_at_s: timestamp.or(parsed_date),
            views: optional_u64(object, "view_count")?,
            likes: optional_u64(object, "like_count")?,
            comments: optional_u64(object, "comment_count")?,
        })
    }

    pub(crate) fn content_kind(&self) -> SourceKind {
        match self.duration_ms {
            None => SourceKind::Unknown,
            Some(0..=60_000) => SourceKind::Short,
            Some(_) => SourceKind::Video,
        }
    }
}

fn string<'a>(object: &'a Map<String, Value>, key: &'static str) -> Result<&'a str, SourceError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(SourceError::Field(key))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, SourceError> {
    let value = string(object, key)?;
    if value.is_empty() {
        return Err(SourceError::Field(key));
    }
    Ok(value)
}

fn required_u32(object: &Map<String, Value>, key: &'static str) -> Result<u32, SourceError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(SourceError::Field(key))
}

fn optional_u32(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<u32>, SourceError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or(SourceError::Field(key)),
    }
}

fn optional_u64(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<u64>, SourceError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or(SourceError::Field(key)),
    }
}

fn optional_i64(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<i64>, SourceError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or(SourceError::Field(key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"{"id":"abc_0","video_id":"abc","youtuber_id":"creator","title":"Title","text":"text","t_start_ms":0,"t_end_ms":1000,"duration":60000,"published_at":"1969-12-31T23:59:59Z"}"#;

    #[test]
    fn preserves_exact_ids_known_zero_and_missing_counters() {
        let base = SourceDocument::parse_json_line(BASE).unwrap();
        assert_eq!(base.external_id, "abc_0");
        assert_eq!(base.published_at_s, Some(-1));
        assert_eq!(base.content_kind(), SourceKind::Short);
        assert_eq!((base.views, base.likes, base.comments), (None, None, None));
        let mut value: Value = serde_json::from_str(BASE).unwrap();
        value["view_count"] = Value::from(0);
        value["like_count"] = Value::Null;
        let present = SourceDocument::parse_json_line(&value.to_string()).unwrap();
        assert_eq!(present.views, Some(0));
        assert_eq!(present.likes, None);
        value["duration"] = Value::from(60_001);
        assert_eq!(
            SourceDocument::parse_json_line(&value.to_string())
                .unwrap()
                .content_kind(),
            SourceKind::Video
        );
        value["duration"] = Value::Null;
        assert_eq!(
            SourceDocument::parse_json_line(&value.to_string())
                .unwrap()
                .content_kind(),
            SourceKind::Unknown
        );
    }

    #[test]
    fn rejects_missing_identity_invalid_numeric_and_conflicting_time() {
        let mut value: Value = serde_json::from_str(BASE).unwrap();
        value.as_object_mut().unwrap().remove("id");
        assert!(matches!(
            SourceDocument::parse_json_line(&value.to_string()),
            Err(SourceError::Field("id"))
        ));
        value["id"] = Value::from("abc_0");
        value["view_count"] = Value::from(-1);
        assert!(matches!(
            SourceDocument::parse_json_line(&value.to_string()),
            Err(SourceError::Field("view_count"))
        ));
        value.as_object_mut().unwrap().remove("view_count");
        value["published_at_timestamp"] = Value::from(0);
        assert!(matches!(
            SourceDocument::parse_json_line(&value.to_string()),
            Err(SourceError::Conflict)
        ));
        value
            .as_object_mut()
            .unwrap()
            .remove("published_at_timestamp");
        value["t_end_ms"] = Value::from(0);
        value["t_start_ms"] = Value::from(1);
        assert!(matches!(
            SourceDocument::parse_json_line(&value.to_string()),
            Err(SourceError::Conflict)
        ));
        value.as_object_mut().unwrap().remove("t_end_ms");
        assert!(matches!(
            SourceDocument::parse_json_line(&value.to_string()),
            Err(SourceError::Field("t_end_ms"))
        ));
    }
}
