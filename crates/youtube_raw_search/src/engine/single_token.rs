use super::*;

const MAX_FUZZY_SPLIT_TERMS: usize = 8;
// Two-character components are overwhelmingly common abbreviations/particles
// (`react` -> `re act`, `fire` -> `fi re`) rather than omitted separators.
// Requiring three characters per side retains useful recoveries such as
// `blackhole` -> `black hole` while bounding both noise and posting work.
const MIN_SPLIT_TERM_CHARS: usize = 3;
const MIN_FUZZY_SPLIT_COMPONENT_CHARS: usize = 3;
const MAX_SPLIT_ALTERNATIVES: usize = 8;
const SPLIT_SCORE_BONUS: f32 = 8.0;

// Compact missing-separator recovery is optional ranking, not a recall gate.
// Avoid a second full positional sweep when both split terms are common.
pub(super) const MAX_COMPACT_SPLIT_PROXIMITY_MATCHES: usize = 50_000;
// Also avoid decoding and intersecting enormous component posting lists. Common
// word joins are better handled as an unmatched whole token than as an
// unbounded missing-separator fallback.
pub(super) const MAX_COMPACT_SPLIT_POSTING_WORK: u64 = 1_000_000;

struct SplitComponent {
    entry: TermEntry,
    posting_work: u64,
    highlight: String,
    fuzzy: bool,
}

pub(super) struct SplitWorkBudget {
    pub(super) remaining: u64,
    pub(super) exceeded: bool,
}

impl SplitWorkBudget {
    pub(super) const fn new() -> Self {
        Self {
            remaining: MAX_COMPACT_SPLIT_POSTING_WORK,
            exceeded: false,
        }
    }

    pub(super) fn charge(&mut self, work: u64) -> bool {
        if work > self.remaining {
            self.exceeded = true;
            return false;
        }
        self.remaining -= work;
        true
    }
}

struct ExactSplitMatches {
    matches: Vec<DocMatch>,
    highlights: Vec<String>,
    proximity_terms: Option<[String; 2]>,
    work_exceeded: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OrderedSplitRank {
    BelowLiteral,
    QueryEquivalent,
}

impl RawSearchEngine {
    pub(super) fn search_single_token(
        &self,
        params: &SearchParams,
        term: &str,
        fields: QueryFields,
        excluded_documents: &[u32],
    ) -> Result<Value> {
        let exact = self.matches_for_term(term, fields)?;
        // A common literal token is stronger evidence than an accidental
        // separator split such as `convex` -> `con vex`. Sparse compounds such
        // as `createeffect` and `blackhole` still treat an adjacent split as
        // equivalent intent.
        let ordered_split_rank =
            if self.reaches_unfiltered_video_threshold(&exact, TYPO_RESULT_THRESHOLD)? {
                OrderedSplitRank::BelowLiteral
            } else {
                OrderedSplitRank::QueryEquivalent
            };
        let mut cache = BlockCache::default();
        let mut matches = exact;
        retain_not_excluded(&mut matches, excluded_documents);
        let mut highlight_group = vec![term.to_string()];

        // Prefix matching is part of the ordinary candidate set, not a
        // typo fallback. Always merge it before applying the typo threshold so
        // five exact hits do not hide `exploit` -> `exploiting` matches.
        let (prefix, prefix_terms) = self.fuzzy_alternatives(term, fields, 0, None, true)?;
        matches = union_sorted(matches, prefix);
        retain_not_excluded(&mut matches, excluded_documents);
        highlight_group.extend(prefix_terms);
        if bounded_typo_distance(term) == 0
            || self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)?
        {
            return self.format_matches(params, &[highlight_group], matches, &mut cache);
        }

        // Missing-separator recovery is fallback behavior. Running it before
        // the exact/prefix threshold makes ordinary words such as `react` and
        // `fire` explode into `re + act` and `fi + re`, respectively. Besides
        // admitting unrelated results, those common-token intersections can
        // dominate the entire request. Sparse compact terms such as
        // `blackhole` still reach this path and recover `black hole`.
        let mut split_work = SplitWorkBudget::new();
        let split = self.split_term_matches(term, fields, &mut split_work)?;
        let has_exact_split = !split.matches.is_empty();
        matches = union_sorted(matches, split.matches);
        retain_not_excluded(&mut matches, excluded_documents);
        highlight_group.extend(split.highlights);
        if has_exact_split
            && self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)?
        {
            self.apply_compact_split_proximity(
                split.proximity_terms.as_ref(),
                fields,
                ordered_split_rank,
                &mut matches,
                &mut cache,
            )?;
            return self.format_matches(params, &[highlight_group], matches, &mut cache);
        }

        // Treat a missing separator plus one corrected component as a more
        // specific hypothesis than an unrelated whole-token typo. The path is
        // still guarded by lexicon membership, one exact component, one
        // distance-one component, and a same-document intersection.
        if !has_exact_split && !split.work_exceeded {
            let (fuzzy_split, fuzzy_split_terms) =
                self.fuzzy_split_term_matches(term, fields, &mut split_work, &mut cache)?;
            matches = union_sorted(matches, fuzzy_split);
            retain_not_excluded(&mut matches, excluded_documents);
            highlight_group.extend(fuzzy_split_terms);
            if self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)? {
                return self.format_matches(params, &[highlight_group], matches, &mut cache);
            }
        }
        let maximum_distance = bounded_typo_distance(term);
        for distance in 1..=maximum_distance {
            let (fuzzy, fuzzy_terms) =
                self.fuzzy_alternatives(term, fields, distance, Some(distance), false)?;
            highlight_group.extend(fuzzy_terms);
            matches = union_sorted(matches, fuzzy);
            retain_not_excluded(&mut matches, excluded_documents);
            if distance == maximum_distance
                || self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)?
            {
                break;
            }
        }
        self.apply_compact_split_proximity(
            split.proximity_terms.as_ref(),
            fields,
            ordered_split_rank,
            &mut matches,
            &mut cache,
        )?;
        self.format_matches(params, &[highlight_group], matches, &mut cache)
    }

    pub(super) fn reaches_result_threshold(
        &self,
        params: &SearchParams,
        matches: &[DocMatch],
        threshold: usize,
    ) -> Result<bool> {
        let filter = parse_filter(params.filter_by.as_deref())?;
        // Grouped results count each video once.
        let mut videos = (params.group_by.as_deref() == Some("video_id"))
            .then(|| HashSet::with_capacity(threshold));
        let mut found = 0_usize;
        for candidate in matches {
            let (record, video) = self.document_video(candidate.document())?;
            if video_matches_filter(video, filter) {
                found += usize::from(
                    videos
                        .as_mut()
                        .is_none_or(|videos| videos.insert(record.video)),
                );
                if found >= threshold {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn reaches_unfiltered_video_threshold(
        &self,
        matches: &[DocMatch],
        threshold: usize,
    ) -> Result<bool> {
        let mut videos = HashSet::with_capacity(threshold);
        for candidate in matches {
            videos.insert(
                self.inner
                    .metadata
                    .search_document(candidate.document())?
                    .video,
            );
            if videos.len() >= threshold {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn split_term_matches(
        &self,
        term: &str,
        fields: QueryFields,
        work: &mut SplitWorkBudget,
    ) -> Result<ExactSplitMatches> {
        let mut alternatives = Vec::new();
        let mut highlight_terms = Vec::new();
        let mut proximity_terms = None;
        let mut proximity_ambiguous = false;

        for (left, right) in split_points(term, MIN_SPLIT_TERM_CHARS) {
            let left_candidates = self.exact_split_components(left, fields)?;
            let right_candidates = self.exact_split_components(right, fields)?;
            'pairs: for left_candidate in &left_candidates {
                for right_candidate in &right_candidates {
                    let Some(mut matches) =
                        self.split_pair_matches(left_candidate, right_candidate, fields, work)?
                    else {
                        continue;
                    };
                    if left_candidate.highlight == left && right_candidate.highlight == right {
                        if proximity_terms.is_some() {
                            proximity_ambiguous = true;
                        } else {
                            proximity_terms = Some([left.to_string(), right.to_string()]);
                        }
                    }
                    for candidate in &mut matches {
                        // The components are exact terms, but they are not an
                        // exact match for the user's original token. Preserve
                        // their recall without allowing `con + vex` to outrank
                        // the literal `convex` token on the exactness tier.
                        candidate.set_exact_mask(0);
                        candidate.score += SPLIT_SCORE_BONUS;
                    }
                    alternatives.push(matches);
                    highlight_terms.push(left_candidate.highlight.clone());
                    highlight_terms.push(right_candidate.highlight.clone());
                    if alternatives.len() == MAX_SPLIT_ALTERNATIVES {
                        break 'pairs;
                    }
                }
            }
            if alternatives.len() == MAX_SPLIT_ALTERNATIVES {
                break;
            }
        }

        Ok(ExactSplitMatches {
            matches: union_all(alternatives),
            highlights: highlight_terms,
            proximity_terms: if proximity_ambiguous {
                None
            } else {
                proximity_terms
            },
            work_exceeded: work.exceeded,
        })
    }

    fn exact_split_components(
        &self,
        term: &str,
        fields: QueryFields,
    ) -> Result<Vec<SplitComponent>> {
        let mut alternatives = Vec::with_capacity(2);
        if let Some(entry) = self.inner.lexicon.lookup(term)? {
            alternatives.push(SplitComponent {
                entry,
                posting_work: self.split_component_posting_work(entry, fields)?,
                highlight: term.to_string(),
                fuzzy: false,
            });
        }
        if term.len() >= 3 && !term.ends_with('s') {
            let plural = format!("{term}s");
            if let Some(entry) = self.inner.lexicon.lookup(&plural)? {
                alternatives.push(SplitComponent {
                    entry,
                    posting_work: self.split_component_posting_work(entry, fields)?,
                    highlight: plural,
                    fuzzy: false,
                });
            }
        }
        Ok(alternatives)
    }

    fn fuzzy_split_term_matches(
        &self,
        term: &str,
        fields: QueryFields,
        work: &mut SplitWorkBudget,
        cache: &mut BlockCache,
    ) -> Result<(Vec<DocMatch>, Vec<String>)> {
        let mut loose_alternatives = Vec::new();
        let mut loose_highlight_terms = Vec::new();
        let mut phrase_alternatives = Vec::new();
        let mut phrase_highlight_terms = Vec::new();

        'splits: for (left, right) in split_points(term, MIN_FUZZY_SPLIT_COMPONENT_CHARS) {
            let left = self.split_component_candidates(left, fields)?;
            let right = self.split_component_candidates(right, fields)?;
            for left_candidate in &left {
                for right_candidate in &right {
                    if left_candidate.fuzzy == right_candidate.fuzzy {
                        continue;
                    }
                    let Some(mut matches) =
                        self.split_pair_matches(left_candidate, right_candidate, fields, work)?
                    else {
                        continue;
                    };
                    for candidate in &mut matches {
                        // Preserve the corrected two-term query's relative field/term
                        // weights. A smaller base bonus avoids over-rewarding the
                        // correction; adjacent phrase evidence is handled separately.
                        // This keeps `blak + hole` and `black + hloe` from ranking the
                        // same corrected intent differently.
                        candidate.score += SPLIT_SCORE_BONUS * 0.5;
                        candidate.set_exact_mask(0);
                    }
                    let mut phrase_matches = matches.clone();
                    let phrase =
                        format!("{} {}", left_candidate.highlight, right_candidate.highlight);
                    self.retain_phrase_matches(&mut phrase_matches, &phrase, cache)?;
                    if !phrase_matches.is_empty() {
                        phrase_alternatives.push(phrase_matches);
                        phrase_highlight_terms.push(left_candidate.highlight.clone());
                        phrase_highlight_terms.push(right_candidate.highlight.clone());
                    }
                    loose_alternatives.push(matches);
                    loose_highlight_terms.push(left_candidate.highlight.clone());
                    loose_highlight_terms.push(right_candidate.highlight.clone());
                    if loose_alternatives.len() == MAX_SPLIT_ALTERNATIVES {
                        break 'splits;
                    }
                }
            }
        }
        if phrase_alternatives.is_empty() {
            Ok((union_all(loose_alternatives), loose_highlight_terms))
        } else {
            Ok((union_all(phrase_alternatives), phrase_highlight_terms))
        }
    }

    /// Intersects the postings of one split pair, charging its work to `work`.
    /// `None` means the budget refused the pair or the components never co-occur.
    fn split_pair_matches(
        &self,
        left: &SplitComponent,
        right: &SplitComponent,
        fields: QueryFields,
        work: &mut SplitWorkBudget,
    ) -> Result<Option<Vec<DocMatch>>> {
        if !work.charge(left.posting_work.saturating_add(right.posting_work)) {
            return Ok(None);
        }
        let matches = intersect_sorted(
            self.matches_for_entry(left.entry, fields)?,
            self.matches_for_entry(right.entry, fields)?,
        );
        Ok((!matches.is_empty()).then_some(matches))
    }

    fn split_component_candidates(
        &self,
        term: &str,
        fields: QueryFields,
    ) -> Result<Vec<SplitComponent>> {
        let mut output = self.exact_split_components(term, fields)?;
        if term.len() < MIN_FUZZY_SPLIT_COMPONENT_CHARS {
            return Ok(output);
        }

        let candidates = self.inner.fuzzy.candidates(
            term,
            1,
            MAX_FUZZY_SPLIT_TERMS,
            false,
            fields.text,
            fields.title,
        )?;
        output.reserve(candidates.len());
        for candidate in candidates {
            if candidate.exact || candidate.distance != 1 {
                continue;
            }
            let (candidate_term, _) = self.inner.lexicon.entry_at(candidate.term as usize)?;
            // A compact-token split already hypothesizes a missing separator. Avoid also
            // deleting characters from a component: `blackhle` -> `black he` is much
            // broader than the intended insertion correction `black hole`.
            if candidate_term.len() < term.len() {
                continue;
            }
            let candidates = self.exact_split_components(
                candidate_term,
                QueryFields {
                    prioritize_position: false,
                    ..fields
                },
            )?;
            output.extend(candidates.into_iter().map(|mut candidate| {
                candidate.fuzzy = true;
                candidate
            }));
        }
        Ok(output)
    }

    fn split_component_posting_work(&self, entry: TermEntry, fields: QueryFields) -> Result<u64> {
        let text = if fields.text {
            u64::from(entry.text_frequency)
        } else {
            0
        };
        let title = if fields.title {
            self.title_expanded_document_work(entry)?
        } else {
            0
        };
        Ok(text.saturating_add(title))
    }

    fn title_expanded_document_work(&self, entry: TermEntry) -> Result<u64> {
        let mut documents = 0_u64;
        self.visit_title_postings(entry, false, |posting| {
            let video = &self.inner.videos[posting.video as usize];
            documents = documents.saturating_add(video.documents.len() as u64);
            Ok(())
        })?;
        Ok(documents)
    }

    fn apply_compact_split_proximity(
        &self,
        terms: Option<&[String; 2]>,
        fields: QueryFields,
        rank: OrderedSplitRank,
        matches: &mut [DocMatch],
        cache: &mut BlockCache,
    ) -> Result<()> {
        if let Some(terms) = terms
            && compact_split_proximity_is_bounded(matches.len())
        {
            self.apply_exact_proximity(terms, terms, fields, matches, cache)?;
            if rank == OrderedSplitRank::QueryEquivalent {
                for candidate in matches {
                    if candidate.ordered_phrase() && candidate.exact_mask() == 0 {
                        candidate.set_exact_mask(fields.exact_mask);
                        candidate.set_all_retained_exact(true);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Every way to cut `term` into two non-empty halves of at least `min_chars`
/// characters each, shortest left half first.
fn split_points(term: &str, min_chars: usize) -> impl Iterator<Item = (&str, &str)> {
    let characters = term.chars().count();
    term.char_indices()
        .enumerate()
        .skip(min_chars.max(1))
        .take_while(move |(index, _)| characters - index >= min_chars)
        .map(|(_, (byte, _))| term.split_at(byte))
}

pub(super) const fn compact_split_proximity_is_bounded(matches: usize) -> bool {
    matches <= MAX_COMPACT_SPLIT_PROXIMITY_MATCHES
}
