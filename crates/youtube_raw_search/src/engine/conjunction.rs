use super::*;

const ALWAYS_DOCUMENT_VALIDATE_TARGETS: usize = 128;
// Above the established direct boundary, decompress/tokenize a bounded target
// set only when it replaces a substantially larger posting scan. This protects
// sparse conjunctions while avoiding multi-million-entry scans for common
// terms after rare terms have already narrowed the candidate set.
const MAX_ADAPTIVE_DOCUMENT_VALIDATE_TARGETS: usize = 512;
const MIN_POSTING_TO_TARGET_RATIO: u64 = 512;
pub(super) const STREAMING_CONJUNCTION_MIN_DOCUMENTS: u64 = 100_000;
const _: () = assert!(MAX_FUZZY_TERMS_PER_DISTANCE * 2 < u64::BITS as usize);

pub(super) fn is_broad_two_plan(plans: &[TermPlan]) -> bool {
    plans.len() == 2
        && plans
            .iter()
            .all(|plan| plan.estimated_documents >= STREAMING_CONJUNCTION_MIN_DOCUMENTS)
}

pub(super) struct TermAlternative {
    pub(super) term: String,
    pub(super) entry: TermEntry,
    pub(super) fields: QueryFields,
    pub(super) score_multiplier: f32,
}

pub(super) struct TermPlan {
    pub(super) alternatives: Vec<TermAlternative>,
    pub(super) estimated_documents: u64,
}

struct TextAlternativeCursor<'a> {
    engine: &'a RawSearchEngine,
    alternative: &'a TermAlternative,
    postings: &'a [u8],
    positions: Option<&'a [u8]>,
    posting_cursor: usize,
    position_cursor: usize,
    previous_document: u32,
    decoded: u32,
    current: Option<(u32, u32, u32)>,
    idf: f32,
}

struct StreamingTermPlan<'a> {
    text: Vec<TextAlternativeCursor<'a>>,
    title: Vec<DocMatch>,
    title_index: usize,
    current_document: Option<u32>,
    active_text: u64,
}

#[derive(Clone, Copy, Default)]
struct TargetContribution {
    score: f32,
    exact_mask: u8,
    matched: bool,
    matches_text: bool,
}

impl RawSearchEngine {
    pub(super) fn exact_terms_are_broad(&self, terms: &[String]) -> Result<bool> {
        for term in terms {
            let Some(entry) = self.inner.lexicon.lookup(term)? else {
                return Ok(false);
            };
            if self.estimated_entry_documents(entry) < STREAMING_CONJUNCTION_MIN_DOCUMENTS {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn exact_prefix_conjunction(
        &self,
        terms: &[String],
        fields: QueryFields,
        prefix_last: bool,
        filter: Option<(&str, &str)>,
        block_cache: &mut BlockCache,
    ) -> Result<(Vec<DocMatch>, HighlightGroups)> {
        self.fuzzy_prefix_conjunction(terms, fields, prefix_last, 0, filter, block_cache)
    }

    pub(super) fn fuzzy_prefix_conjunction(
        &self,
        terms: &[String],
        fields: QueryFields,
        prefix_last: bool,
        maximum_distance: u8,
        filter: Option<(&str, &str)>,
        block_cache: &mut BlockCache,
    ) -> Result<(Vec<DocMatch>, HighlightGroups)> {
        let (mut plans, highlights) =
            self.conjunction_plans(terms, fields, prefix_last, maximum_distance)?;
        plans.sort_unstable_by_key(|plan| plan.estimated_documents);
        if is_broad_two_plan(&plans) {
            let mut matches = self.stream_two_plan_conjunction(&plans)?;
            self.retain_video_filter(&mut matches, filter)?;
            return Ok((matches, highlights));
        }
        let mut plans = plans.into_iter();
        let Some(first) = plans.next() else {
            return Ok((Vec::new(), highlights));
        };
        let mut matches = self.matches_for_plan(&first, None, block_cache)?;
        self.retain_video_filter(&mut matches, filter)?;
        let remaining = plans.collect::<Vec<_>>();
        for (index, plan) in remaining.iter().enumerate() {
            if prefer_document_validation(matches.len(), &remaining[index..]) {
                matches =
                    self.matches_for_plans_documents(&remaining[index..], &matches, block_cache)?;
                break;
            }
            let plan_matches = self.matches_for_plan(plan, Some(&matches), block_cache)?;
            matches = intersect_sorted(matches, plan_matches);
            if matches.is_empty() {
                break;
            }
        }
        Ok((matches, highlights))
    }

    pub(super) fn conjunction_plans(
        &self,
        terms: &[String],
        fields: QueryFields,
        prefix_last: bool,
        maximum_distance: u8,
    ) -> Result<(Vec<TermPlan>, HighlightGroups)> {
        let mut plans = Vec::with_capacity(terms.len());
        let mut highlights = terms
            .iter()
            .map(|term| vec![term.clone()])
            .collect::<HighlightGroups>();
        let last_index = terms.len().saturating_sub(1);

        for (index, term) in terms.iter().enumerate() {
            let exact_mask = 1 << (3 - index.min(3));
            let mut alternatives = Vec::with_capacity(if index == last_index { 5 } else { 1 });
            if let Some(entry) = self.inner.lexicon.lookup(term)? {
                alternatives.push(TermAlternative {
                    term: term.clone(),
                    entry,
                    fields: QueryFields {
                        exact_mask,
                        ..fields
                    },
                    score_multiplier: 1.0,
                });
            }

            let term_typo_distance = bounded_typo_distance(term);
            if (maximum_distance > 0 && term_typo_distance > 0)
                || (prefix_last && index == last_index)
            {
                for candidate in self.inner.fuzzy.candidates(
                    term,
                    maximum_distance.min(term_typo_distance),
                    MAX_FUZZY_TERMS_PER_DISTANCE,
                    prefix_last && index == last_index,
                    fields.text,
                    fields.title,
                )? {
                    let prefix_candidate =
                        candidate.distance == 0 && prefix_last && index == last_index;
                    let typo_candidate = candidate.distance > 0
                        && candidate.distance <= maximum_distance
                        && candidate.distance <= bounded_typo_distance(term);
                    if candidate.exact || (!prefix_candidate && !typo_candidate) {
                        continue;
                    }
                    let (candidate_term, entry) =
                        self.inner.lexicon.entry_at(candidate.term as usize)?;
                    alternatives.push(TermAlternative {
                        term: candidate_term.to_string(),
                        entry,
                        fields: QueryFields {
                            prioritize_position: false,
                            exact_mask: 0,
                            ..fields
                        },
                        score_multiplier: match candidate.distance {
                            0 => 0.9,
                            1 => 0.65,
                            _ => 0.4,
                        },
                    });
                    highlights[index].push(candidate_term.to_string());
                }
            }

            if alternatives.is_empty() {
                return Ok((Vec::new(), highlights));
            }
            let estimated_documents = alternatives
                .iter()
                .map(|alternative| self.estimated_entry_documents(alternative.entry))
                .sum();
            plans.push(TermPlan {
                alternatives,
                estimated_documents,
            });
        }

        Ok((plans, highlights))
    }

    pub(super) fn stream_two_plan_conjunction(&self, plans: &[TermPlan]) -> Result<Vec<DocMatch>> {
        let mut matches = Vec::new();
        self.visit_two_plan_conjunction(plans, |candidate, _| {
            matches.push(candidate);
            Ok(())
        })?;
        Ok(matches)
    }

    pub(super) fn visit_term_plan<F>(&self, plan: &TermPlan, mut visit: F) -> Result<()>
    where
        F: FnMut(DocMatch, SearchDocumentRecord) -> Result<()>,
    {
        let mut stream = StreamingTermPlan::new(self, plan)?;
        stream.advance_to(0)?;
        while let Some(document) = stream.current_document() {
            let record = self.inner.metadata.search_document(document)?;
            let candidate = stream.take_current(record.tokens)?;
            visit(candidate, record)?;
        }
        stream.finish()
    }

    pub(super) fn visit_two_plan_conjunction<F>(
        &self,
        plans: &[TermPlan],
        mut visit: F,
    ) -> Result<()>
    where
        F: FnMut(DocMatch, SearchDocumentRecord) -> Result<()>,
    {
        let [left_plan, right_plan] = plans else {
            return Err(Error::Corrupt(
                "streaming conjunction requires exactly two plans".to_string(),
            ));
        };
        let mut left = StreamingTermPlan::new(self, left_plan)?;
        let mut right = StreamingTermPlan::new(self, right_plan)?;
        left.advance_to(0)?;
        right.advance_to(0)?;
        while let (Some(left_document), Some(right_document)) =
            (left.current_document(), right.current_document())
        {
            let target = left_document.max(right_document);
            left.advance_to(target)?;
            right.advance_to(target)?;
            let (Some(left_document), Some(right_document)) =
                (left.current_document(), right.current_document())
            else {
                break;
            };
            if left_document != right_document {
                continue;
            }
            let record = self.inner.metadata.search_document(left_document)?;
            let left_match = left.take_current(record.tokens)?;
            let right_match = right.take_current(record.tokens)?;
            visit(
                combine_plan_matches(left_document, left_match, right_match),
                record,
            )?;
        }

        left.finish()?;
        right.finish()?;
        Ok(())
    }

    fn retain_video_filter(
        &self,
        matches: &mut Vec<DocMatch>,
        filter: Option<(&str, &str)>,
    ) -> Result<()> {
        let Some(filter) = filter else {
            return Ok(());
        };
        let mut retained = Vec::with_capacity(matches.len());
        for candidate in matches.drain(..) {
            let record = self.inner.metadata.document(candidate.document())?;
            let video = self
                .inner
                .videos
                .get(record.video as usize)
                .ok_or_else(|| Error::Corrupt("document video is out of bounds".to_string()))?;
            if video_matches_filter(video, Some(filter)) {
                retained.push(candidate);
            }
        }
        *matches = retained;
        Ok(())
    }

    pub(super) fn matches_for_plan(
        &self,
        plan: &TermPlan,
        targets: Option<&[DocMatch]>,
        block_cache: &mut BlockCache,
    ) -> Result<Vec<DocMatch>> {
        if let Some(targets) = targets
            && targets.len() <= ALWAYS_DOCUMENT_VALIDATE_TARGETS
        {
            return self.matches_for_plan_documents(plan, targets, block_cache);
        }
        let mut alternatives = Vec::with_capacity(plan.alternatives.len());
        for alternative in &plan.alternatives {
            let mut matches = if let Some(targets) = targets {
                self.matches_for_entry_targets(alternative.entry, alternative.fields, targets)?
            } else {
                self.matches_for_entry(alternative.entry, alternative.fields)?
            };
            for candidate in &mut matches {
                candidate.score *= alternative.score_multiplier;
            }
            alternatives.push(matches);
        }
        Ok(union_all(alternatives))
    }

    fn matches_for_plan_documents(
        &self,
        plan: &TermPlan,
        targets: &[DocMatch],
        block_cache: &mut BlockCache,
    ) -> Result<Vec<DocMatch>> {
        let mut matches = Vec::with_capacity(targets.len());
        for target in targets {
            let document = target.document();
            let record = self.inner.metadata.document(document)?;
            let video = self
                .inner
                .videos
                .get(record.video as usize)
                .ok_or_else(|| Error::Corrupt("document video is out of bounds".to_string()))?;
            let source = self.document_value(document, block_cache)?;
            let text_tokens = source
                .get("text")
                .and_then(Value::as_str)
                .map(ordered_tokens)
                .unwrap_or_default();
            let title_tokens = ordered_tokens(&video.title);
            let contribution =
                self.plan_document_contribution(plan, record, &text_tokens, &title_tokens);
            if contribution.matched {
                matches.push(DocMatch::with_exact_mask(
                    document,
                    contribution.score,
                    contribution.matches_text,
                    contribution.exact_mask,
                ));
            }
        }
        Ok(matches)
    }

    fn matches_for_plans_documents(
        &self,
        plans: &[TermPlan],
        targets: &[DocMatch],
        block_cache: &mut BlockCache,
    ) -> Result<Vec<DocMatch>> {
        let mut matches = Vec::with_capacity(targets.len());
        'documents: for target in targets {
            let document = target.document();
            let record = self.inner.metadata.document(document)?;
            let video = self
                .inner
                .videos
                .get(record.video as usize)
                .ok_or_else(|| Error::Corrupt("document video is out of bounds".to_string()))?;
            let source = self.document_value(document, block_cache)?;
            let text_tokens = source
                .get("text")
                .and_then(Value::as_str)
                .map(ordered_tokens)
                .unwrap_or_default();
            let title_tokens = ordered_tokens(&video.title);
            let mut combined = *target;

            for plan in plans {
                let contribution =
                    self.plan_document_contribution(plan, record, &text_tokens, &title_tokens);
                if !contribution.matched {
                    continue 'documents;
                }
                combined.score += contribution.score;
                combined.set_matches_text(combined.matches_text() || contribution.matches_text);
                combined.set_exact_mask(combined.exact_mask() | contribution.exact_mask);
                combined.set_all_retained_exact(
                    combined.all_retained_exact() && contribution.exact_mask != 0,
                );
            }
            matches.push(combined);
        }
        Ok(matches)
    }

    fn plan_document_contribution(
        &self,
        plan: &TermPlan,
        record: DocumentRecord,
        text_tokens: &[String],
        title_tokens: &[String],
    ) -> TargetContribution {
        let mut contribution = TargetContribution::default();
        for alternative in &plan.alternatives {
            let effective_frequency = self
                .estimated_entry_documents(alternative.entry)
                .min(u64::from(self.inner.manifest.documents));
            let idf = inverse_document_frequency(
                self.inner.manifest.documents,
                u32::try_from(effective_frequency)
                    .unwrap_or(u32::MAX)
                    .max(1),
            );
            if alternative.fields.text
                && let Some((frequency, first_position)) =
                    token_stats(text_tokens, &alternative.term)
            {
                contribution.score += bm25(
                    frequency,
                    record.tokens,
                    self.inner.manifest.average_document_tokens,
                ) * idf
                    * TEXT_QUERY_WEIGHT
                    * token_position_multiplier(
                        first_position,
                        alternative.fields.prioritize_position,
                    )
                    * alternative.score_multiplier;
                contribution.exact_mask |= alternative.fields.exact_mask;
                contribution.matched = true;
                contribution.matches_text = true;
            }
            if alternative.fields.title
                && let Some((frequency, first_position)) =
                    token_stats(title_tokens, &alternative.term)
            {
                contribution.score += idf
                    * TITLE_QUERY_WEIGHT
                    * (1.0 + (frequency as f32).ln())
                    * token_position_multiplier(
                        first_position,
                        alternative.fields.prioritize_position,
                    )
                    * alternative.score_multiplier;
                contribution.exact_mask |= alternative.fields.exact_mask;
                contribution.matched = true;
            }
        }
        contribution
    }

    pub(super) fn estimated_entry_documents(&self, entry: TermEntry) -> u64 {
        let average_documents_per_video = self
            .inner
            .manifest
            .documents
            .checked_div(self.inner.manifest.videos)
            .unwrap_or(0);
        u64::from(entry.text_frequency).saturating_add(u64::from(
            entry
                .title_frequency
                .saturating_mul(average_documents_per_video),
        ))
    }

    fn matches_for_entry_targets(
        &self,
        entry: TermEntry,
        fields: QueryFields,
        targets: &[DocMatch],
    ) -> Result<Vec<DocMatch>> {
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let effective_frequency = self
            .estimated_entry_documents(entry)
            .min(u64::from(self.inner.manifest.documents));
        let idf = inverse_document_frequency(
            self.inner.manifest.documents,
            u32::try_from(effective_frequency)
                .unwrap_or(u32::MAX)
                .max(1),
        );
        let mut contributions = vec![TargetContribution::default(); targets.len()];
        if fields.text {
            self.decode_targeted_text(entry, fields, targets, idf, &mut contributions)?;
        }
        if fields.title {
            self.decode_targeted_titles(entry, fields, targets, idf, &mut contributions)?;
        }

        Ok(targets
            .iter()
            .zip(contributions)
            .filter_map(|(target, contribution)| {
                if !contribution.matched {
                    return None;
                }
                Some(DocMatch::with_exact_mask(
                    target.document(),
                    contribution.score,
                    contribution.matches_text,
                    contribution.exact_mask,
                ))
            })
            .collect())
    }

    fn decode_targeted_text(
        &self,
        entry: TermEntry,
        fields: QueryFields,
        targets: &[DocMatch],
        idf: f32,
        contributions: &mut [TargetContribution],
    ) -> Result<()> {
        let bytes = self.postings_range(entry.text_offset, entry.text_length)?;
        let position_bytes = fields
            .prioritize_position
            .then(|| self.inner.positions.range(entry.term, IndexedField::Text))
            .transpose()?;
        let mut cursor = 0_usize;
        let mut position_cursor = 0_usize;
        let mut document = 0_u32;
        let mut postings = 0_u32;
        let mut target_index = 0_usize;
        while cursor < bytes.len() {
            document = document.saturating_add(read_varint(bytes, &mut cursor, bytes.len())?);
            let frequency = read_varint(bytes, &mut cursor, bytes.len())?;
            let first_position = position_bytes
                .map(|positions| read_varint(positions, &mut position_cursor, positions.len()))
                .transpose()?
                .unwrap_or_default();
            postings = postings.saturating_add(1);
            while targets
                .get(target_index)
                .is_some_and(|target| target.document() < document)
            {
                target_index += 1;
            }
            if targets.get(target_index).map(|target| target.document()) != Some(document) {
                continue;
            }
            let record = self.inner.metadata.document(document)?;
            let contribution = &mut contributions[target_index];
            contribution.score += bm25(
                frequency,
                record.tokens,
                self.inner.manifest.average_document_tokens,
            ) * idf
                * TEXT_QUERY_WEIGHT
                * token_position_multiplier(first_position, fields.prioritize_position);
            contribution.exact_mask |= fields.exact_mask;
            contribution.matched = true;
            contribution.matches_text = true;
        }
        if postings != entry.text_frequency
            || position_bytes.is_some_and(|positions| position_cursor != positions.len())
        {
            return Err(Error::Corrupt(
                "text posting/position count does not match lexicon".to_string(),
            ));
        }
        Ok(())
    }

    fn decode_targeted_titles(
        &self,
        entry: TermEntry,
        fields: QueryFields,
        targets: &[DocMatch],
        idf: f32,
        contributions: &mut [TargetContribution],
    ) -> Result<()> {
        let mut targets_by_video = HashMap::<u32, Vec<usize>>::new();
        for (index, target) in targets.iter().enumerate() {
            let video = self.inner.metadata.document(target.document())?.video;
            targets_by_video.entry(video).or_default().push(index);
        }
        let bytes = self.postings_range(entry.title_offset, entry.title_length)?;
        let position_bytes = fields
            .prioritize_position
            .then(|| self.inner.positions.range(entry.term, IndexedField::Title))
            .transpose()?;
        let mut cursor = 0_usize;
        let mut position_cursor = 0_usize;
        let mut video = 0_u32;
        let mut postings = 0_u32;
        while cursor < bytes.len() {
            video = video.saturating_add(read_varint(bytes, &mut cursor, bytes.len())?);
            let frequency = read_varint(bytes, &mut cursor, bytes.len())?;
            let first_position = position_bytes
                .map(|positions| read_varint(positions, &mut position_cursor, positions.len()))
                .transpose()?
                .unwrap_or_default();
            postings = postings.saturating_add(1);
            let Some(indices) = targets_by_video.get(&video) else {
                continue;
            };
            let score = idf
                * TITLE_QUERY_WEIGHT
                * (1.0 + (frequency as f32).ln())
                * token_position_multiplier(first_position, fields.prioritize_position);
            for &index in indices {
                let contribution = &mut contributions[index];
                contribution.score += score;
                contribution.exact_mask |= fields.exact_mask;
                contribution.matched = true;
            }
        }
        if postings != entry.title_frequency
            || position_bytes.is_some_and(|positions| position_cursor != positions.len())
        {
            return Err(Error::Corrupt(
                "title posting/position count does not match lexicon".to_string(),
            ));
        }
        Ok(())
    }
}

impl<'a> TextAlternativeCursor<'a> {
    fn current_document(&self) -> Option<u32> {
        self.current.map(|current| current.0)
    }

    fn new(engine: &'a RawSearchEngine, alternative: &'a TermAlternative) -> Result<Self> {
        let entry = alternative.entry;
        let effective_frequency = engine
            .estimated_entry_documents(entry)
            .min(u64::from(engine.inner.manifest.documents));
        Ok(Self {
            engine,
            alternative,
            postings: engine.postings_range(entry.text_offset, entry.text_length)?,
            positions: alternative
                .fields
                .prioritize_position
                .then(|| engine.inner.positions.range(entry.term, IndexedField::Text))
                .transpose()?,
            posting_cursor: 0,
            position_cursor: 0,
            previous_document: 0,
            decoded: 0,
            current: None,
            idf: inverse_document_frequency(
                engine.inner.manifest.documents,
                u32::try_from(effective_frequency)
                    .unwrap_or(u32::MAX)
                    .max(1),
            ),
        })
    }

    fn advance_to(&mut self, target: u32) -> Result<()> {
        if self
            .current
            .is_some_and(|(document, _, _)| document >= target)
        {
            return Ok(());
        }
        self.current = None;
        while self.posting_cursor < self.postings.len() {
            let delta = read_varint(self.postings, &mut self.posting_cursor, self.postings.len())?;
            if self.decoded > 0 && delta == 0 {
                return Err(Error::Corrupt(
                    "posting identifiers must be strictly increasing".to_string(),
                ));
            }
            let document = self
                .previous_document
                .checked_add(delta)
                .ok_or_else(|| Error::Corrupt("posting identifier overflows u32".to_string()))?;
            if document >= self.engine.inner.manifest.documents {
                return Err(Error::Corrupt(
                    "text posting document is out of bounds".to_string(),
                ));
            }
            let frequency =
                read_varint(self.postings, &mut self.posting_cursor, self.postings.len())?;
            let first_position = self
                .positions
                .map(|positions| read_varint(positions, &mut self.position_cursor, positions.len()))
                .transpose()?
                .unwrap_or_default();
            self.previous_document = document;
            self.decoded = self
                .decoded
                .checked_add(1)
                .ok_or_else(|| Error::Corrupt("posting count overflows u32".to_string()))?;
            if self.decoded > self.alternative.entry.text_frequency {
                return Err(Error::Corrupt(
                    "posting count exceeds the lexicon frequency".to_string(),
                ));
            }
            if document >= target {
                self.current = Some((document, frequency, first_position));
                return Ok(());
            }
        }
        self.validate_complete()
    }

    fn contribution(&self, document_tokens: u32) -> Option<DocMatch> {
        let (document, frequency, first_position) = self.current?;
        let fields = self.alternative.fields;
        let score = bm25(
            frequency,
            document_tokens,
            self.engine.inner.manifest.average_document_tokens,
        ) * self.idf
            * TEXT_QUERY_WEIGHT
            * token_position_multiplier(first_position, fields.prioritize_position)
            * self.alternative.score_multiplier;
        Some(DocMatch::with_exact_mask(
            document,
            score,
            true,
            fields.exact_mask,
        ))
    }

    fn consume_current(&mut self) -> Result<()> {
        let Some((document, _, _)) = self.current else {
            return Ok(());
        };
        self.current = None;
        if let Some(next) = document.checked_add(1) {
            self.advance_to(next)
        } else {
            self.validate_complete()
        }
    }

    fn finish(&mut self) -> Result<()> {
        self.current = None;
        while self.posting_cursor < self.postings.len() {
            self.advance_to(self.previous_document.saturating_add(1))?;
            self.current = None;
        }
        self.validate_complete()
    }

    fn validate_complete(&self) -> Result<()> {
        if self.decoded != self.alternative.entry.text_frequency
            || self
                .positions
                .is_some_and(|positions| self.position_cursor != positions.len())
        {
            return Err(Error::Corrupt(
                "text posting/position count does not match lexicon".to_string(),
            ));
        }
        Ok(())
    }
}

impl<'a> StreamingTermPlan<'a> {
    fn new(engine: &'a RawSearchEngine, plan: &'a TermPlan) -> Result<Self> {
        let mut text = Vec::with_capacity(plan.alternatives.len());
        let mut title = Vec::new();
        for alternative in &plan.alternatives {
            if alternative.fields.text {
                text.push(TextAlternativeCursor::new(engine, alternative)?);
            }
            if alternative.fields.title {
                let effective_frequency = engine
                    .estimated_entry_documents(alternative.entry)
                    .min(u64::from(engine.inner.manifest.documents));
                let mut matches = engine.decode_title_postings(
                    alternative.entry,
                    inverse_document_frequency(
                        engine.inner.manifest.documents,
                        u32::try_from(effective_frequency)
                            .unwrap_or(u32::MAX)
                            .max(1),
                    ),
                    alternative.fields.prioritize_position,
                    alternative.fields.exact_mask,
                )?;
                for candidate in &mut matches {
                    candidate.score *= alternative.score_multiplier;
                }
                title.push(matches);
            }
        }
        Ok(Self {
            text,
            title: union_all(title),
            title_index: 0,
            current_document: None,
            active_text: 0,
        })
    }

    fn current_document(&self) -> Option<u32> {
        self.current_document
    }

    fn refresh_current_document(&mut self) {
        let mut current_document = self
            .title
            .get(self.title_index)
            .map(|candidate| candidate.document());
        let mut active_text = 0_u64;
        for (index, cursor) in self.text.iter().enumerate() {
            let Some(document) = cursor.current_document() else {
                continue;
            };
            match current_document {
                Some(current) if document > current => {}
                Some(current) if document == current => active_text |= 1_u64 << index,
                _ => {
                    current_document = Some(document);
                    active_text = 1_u64 << index;
                }
            }
        }
        self.current_document = current_document;
        self.active_text = active_text;
    }

    fn advance_to(&mut self, target: u32) -> Result<()> {
        if self
            .current_document
            .is_some_and(|document| document >= target)
        {
            return Ok(());
        }
        for cursor in &mut self.text {
            cursor.advance_to(target)?;
        }
        while self
            .title
            .get(self.title_index)
            .is_some_and(|candidate| candidate.document() < target)
        {
            self.title_index += 1;
        }
        self.refresh_current_document();
        Ok(())
    }

    fn take_current(&mut self, document_tokens: u32) -> Result<DocMatch> {
        let document = self
            .current_document
            .ok_or_else(|| Error::Corrupt("streaming plan has no current document".to_string()))?;
        let mut combined: Option<DocMatch> = None;
        let mut active_text = self.active_text;
        while active_text != 0 {
            let index = active_text.trailing_zeros() as usize;
            active_text &= active_text - 1;
            let cursor = &mut self.text[index];
            debug_assert_eq!(cursor.current_document(), Some(document));
            if let Some(contribution) = cursor.contribution(document_tokens) {
                merge_plan_contribution(&mut combined, contribution);
            }
            cursor.consume_current()?;
        }
        if self
            .title
            .get(self.title_index)
            .is_some_and(|candidate| candidate.document() == document)
        {
            let contribution = self.title[self.title_index];
            merge_plan_contribution(&mut combined, contribution);
            self.title_index += 1;
        }
        self.refresh_current_document();
        combined.ok_or_else(|| Error::Corrupt("streaming plan contribution is empty".to_string()))
    }

    fn finish(&mut self) -> Result<()> {
        for cursor in &mut self.text {
            cursor.finish()?;
        }
        self.title_index = self.title.len();
        self.current_document = None;
        self.active_text = 0;
        Ok(())
    }
}

fn merge_plan_contribution(combined: &mut Option<DocMatch>, contribution: DocMatch) {
    let Some(current) = combined else {
        *combined = Some(contribution);
        return;
    };
    current.score += contribution.score;
    current.set_matches_text(current.matches_text() || contribution.matches_text());
    current.set_exact_mask(current.exact_mask() | contribution.exact_mask());
    // Alternatives within one query term are a union: one exact alternative is
    // sufficient to preserve the term's exactness. Conjunction across distinct
    // term plans remains an AND in `combine_plan_matches` below.
    current
        .set_all_retained_exact(current.all_retained_exact() || contribution.all_retained_exact());
}

fn combine_plan_matches(document: u32, left: DocMatch, right: DocMatch) -> DocMatch {
    let mut combined = DocMatch::new(
        document,
        left.score + right.score,
        left.matches_text() || right.matches_text(),
    );
    combined.set_exact_mask(left.exact_mask() | right.exact_mask());
    combined.set_all_retained_exact(left.all_retained_exact() && right.all_retained_exact());
    combined
}

fn prefer_document_validation(targets: usize, remaining: &[TermPlan]) -> bool {
    if targets == 0 || remaining.is_empty() {
        return false;
    }
    if targets <= ALWAYS_DOCUMENT_VALIDATE_TARGETS {
        return true;
    }
    if targets > MAX_ADAPTIVE_DOCUMENT_VALIDATE_TARGETS {
        return false;
    }
    let estimated_postings = remaining.iter().fold(0_u64, |total, plan| {
        total.saturating_add(plan.estimated_documents)
    });
    let document_validation_cost = (targets as u64).saturating_mul(MIN_POSTING_TO_TARGET_RATIO);
    estimated_postings >= document_validation_cost
}

fn token_stats(tokens: &[String], term: &str) -> Option<(u32, u32)> {
    let mut frequency = 0_u32;
    let mut first_position = 0_u32;
    for (position, token) in tokens.iter().enumerate() {
        if token != term {
            continue;
        }
        if frequency == 0 {
            first_position = u32::try_from(position).unwrap_or(u32::MAX);
        }
        frequency = frequency.saturating_add(1);
    }
    (frequency > 0).then_some((frequency, first_position))
}

#[cfg(test)]
mod strategy_tests {
    use super::*;

    fn plan(estimated_documents: u64) -> TermPlan {
        TermPlan {
            alternatives: Vec::new(),
            estimated_documents,
        }
    }

    #[test]
    fn document_validation_strategy_is_bounded_and_scan_sensitive() {
        assert!(!prefer_document_validation(0, &[plan(u64::MAX)]));
        assert!(!prefer_document_validation(1, &[]));
        assert!(prefer_document_validation(
            ALWAYS_DOCUMENT_VALIDATE_TARGETS,
            &[plan(1)]
        ));

        let adaptive_targets = ALWAYS_DOCUMENT_VALIDATE_TARGETS + 1;
        let adaptive_boundary = adaptive_targets as u64 * MIN_POSTING_TO_TARGET_RATIO;
        assert!(!prefer_document_validation(
            adaptive_targets,
            &[plan(adaptive_boundary - 1)]
        ));
        assert!(prefer_document_validation(
            adaptive_targets,
            &[plan(adaptive_boundary)]
        ));
        assert!(prefer_document_validation(
            MAX_ADAPTIVE_DOCUMENT_VALIDATE_TARGETS,
            &[plan(u64::MAX)]
        ));
        assert!(!prefer_document_validation(
            MAX_ADAPTIVE_DOCUMENT_VALIDATE_TARGETS + 1,
            &[plan(u64::MAX)]
        ));
    }
}
