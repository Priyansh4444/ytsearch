use super::*;
use serde::Serialize;
use std::{collections::BTreeSet, fs, time::Instant};

const RETAINED_DEPTHS: [usize; 3] = [5, 20, 250];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BroadMeasureFields {
    Text,
    Title,
    TextAndTitle,
}

impl BroadMeasureFields {
    const ALL: [Self; 3] = [Self::Text, Self::Title, Self::TextAndTitle];

    const fn query_by(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Title => "title",
            Self::TextAndTitle => "text,title",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BroadMeasurePosition {
    Disabled,
    Enabled,
}

impl BroadMeasurePosition {
    const ALL: [Self; 2] = [Self::Disabled, Self::Enabled];

    const fn enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

#[derive(Debug, Serialize)]
pub struct BroadMeasureReport {
    pub schema_version: u8,
    pub documents: u32,
    pub videos: u32,
    pub queries: Vec<BroadQueryMeasure>,
    pub elapsed_ms: u128,
    pub rss_before_bytes: Option<u64>,
    pub rss_after_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct BroadQueryMeasure {
    pub query: String,
    pub fields: BroadMeasureFields,
    pub position: BroadMeasurePosition,
    pub alternatives: usize,
    pub estimated_documents: u64,
    pub emitted_documents: u64,
    pub matching_videos: usize,
    pub total_moments: usize,
    pub retained_documents_5: usize,
    pub retained_documents_20: usize,
    pub retained_documents_250: usize,
    pub fixed_width_bytes: u64,
    pub packed_upper_bound_bytes: u64,
    pub elapsed_ms: u128,
}

impl RawSearchEngine {
    pub fn measure_broad_single(&self, queries: &[String]) -> Result<BroadMeasureReport> {
        let started = Instant::now();
        let rss_before_bytes = process_rss_bytes();
        let normalized = normalize_measure_queries(queries)?;
        let mut measured = Vec::with_capacity(normalized.len() * 6);

        for query in normalized {
            for fields in BroadMeasureFields::ALL {
                for position in BroadMeasurePosition::ALL {
                    measured.push(self.measure_broad_profile(&query, fields, position)?);
                }
            }
        }

        Ok(BroadMeasureReport {
            schema_version: self.inner.manifest.schema_version,
            documents: self.inner.manifest.documents,
            videos: self.inner.manifest.videos,
            queries: measured,
            elapsed_ms: started.elapsed().as_millis(),
            rss_before_bytes,
            rss_after_bytes: process_rss_bytes(),
        })
    }

    fn measure_broad_profile(
        &self,
        query: &str,
        fields: BroadMeasureFields,
        position: BroadMeasurePosition,
    ) -> Result<BroadQueryMeasure> {
        let started = Instant::now();
        let query_fields = query_fields(Some(fields.query_by()), position.enabled())?;
        let terms = [query.to_string()];
        let (plans, _) = self.conjunction_plans(&terms, query_fields, true, 0)?;
        let [plan] = plans.as_slice() else {
            return Ok(empty_measure(query, fields, position, started));
        };
        let dominant_exact_mask = plan.alternatives.iter().fold(0_u8, |mask, alternative| {
            mask | alternative.fields.exact_mask
        });
        let mut groups = self.empty_group_slots();
        let mut emitted_documents = 0_u64;
        self.visit_term_plan(plan, |candidate, record| {
            emitted_documents = emitted_documents.saturating_add(1);
            self.record_group_match_values(
                &mut groups,
                candidate,
                record.video,
                record.t_start_ms,
                250,
                Some(dominant_exact_mask),
            )
        })?;

        let matching_videos = groups.iter().flatten().count();
        let total_moments = groups.iter().flatten().map(visible_group_moments).sum();
        let retained = RETAINED_DEPTHS.map(|depth| {
            groups
                .iter()
                .flatten()
                .map(|group| group.documents.len().min(depth))
                .sum::<usize>()
        });
        let fixed_width_bytes = estimate_fixed_width_bytes(matching_videos, retained[2]);
        // Varints and block compression can only improve on this conservative
        // packed record estimate; the schema decision still requires a real
        // throwaway artifact before publication.
        let packed_upper_bound_bytes = estimate_packed_upper_bound(matching_videos, retained[2]);

        Ok(BroadQueryMeasure {
            query: query.to_string(),
            fields,
            position,
            alternatives: plan.alternatives.len(),
            estimated_documents: plan.estimated_documents,
            emitted_documents,
            matching_videos,
            total_moments,
            retained_documents_5: retained[0],
            retained_documents_20: retained[1],
            retained_documents_250: retained[2],
            fixed_width_bytes,
            packed_upper_bound_bytes,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }
}

fn normalize_measure_queries(queries: &[String]) -> Result<Vec<String>> {
    if queries.is_empty() {
        return Err(Error::InvalidInput(
            "measure-broad-single requires at least one query".to_string(),
        ));
    }
    let mut normalized = BTreeSet::new();
    for query in queries {
        let parsed = parse_query(query)?;
        if !parsed.phrases.is_empty()
            || !parsed.excluded_terms.is_empty()
            || !parsed.excluded_phrases.is_empty()
            || parsed.terms.len() != 1
        {
            return Err(Error::InvalidInput(format!(
                "broad measurement requires one positive unquoted token: {query}"
            )));
        }
        normalized.insert(parsed.terms[0].clone());
    }
    Ok(normalized.into_iter().collect())
}

fn empty_measure(
    query: &str,
    fields: BroadMeasureFields,
    position: BroadMeasurePosition,
    started: Instant,
) -> BroadQueryMeasure {
    BroadQueryMeasure {
        query: query.to_string(),
        fields,
        position,
        alternatives: 0,
        estimated_documents: 0,
        emitted_documents: 0,
        matching_videos: 0,
        total_moments: 0,
        retained_documents_5: 0,
        retained_documents_20: 0,
        retained_documents_250: 0,
        fixed_width_bytes: 0,
        packed_upper_bound_bytes: 0,
        elapsed_ms: started.elapsed().as_millis(),
    }
}

fn estimate_fixed_width_bytes(groups: usize, documents: usize) -> u64 {
    // video + moments + score + rank flags/count/padding, then document +
    // score + flags/padding. This intentionally models the simple baseline.
    (groups as u64)
        .saturating_mul(20)
        .saturating_add((documents as u64).saturating_mul(12))
}

fn estimate_packed_upper_bound(groups: usize, documents: usize) -> u64 {
    (groups as u64)
        .saturating_mul(16)
        .saturating_add((documents as u64).saturating_mul(9))
}

fn process_rss_bytes() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let kibibytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_ascii_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    kibibytes.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_queries_require_one_positive_token() {
        for invalid in [
            vec![],
            vec!["two terms".to_string()],
            vec!["-term".to_string()],
        ] {
            assert!(normalize_measure_queries(&invalid).is_err());
        }
        assert_eq!(
            normalize_measure_queries(&["React".to_string(), "react".to_string()]).unwrap(),
            vec!["react"]
        );
    }

    #[test]
    fn byte_estimates_are_monotonic() {
        assert!(estimate_fixed_width_bytes(2, 3) > estimate_fixed_width_bytes(1, 3));
        assert!(estimate_fixed_width_bytes(2, 4) > estimate_fixed_width_bytes(2, 3));
        assert!(estimate_packed_upper_bound(2, 3) < estimate_fixed_width_bytes(2, 3));
    }
}
