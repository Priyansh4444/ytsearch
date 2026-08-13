use super::*;

struct TermAlternative {
    term: String,
    entry: TermEntry,
    fields: QueryFields,
    score_multiplier: f32,
}

struct TermPlan {
    alternatives: Vec<TermAlternative>,
    estimated_documents: u64,
}

#[derive(Clone, Copy, Default)]
struct TargetContribution {
    score: f32,
    exact_mask: u8,
    matched: bool,
    matches_text: bool,
}

impl RawSearchEngine {
    pub(super) fn exact_prefix_conjunction(
        &self,
        terms: &[String],
        fields: QueryFields,
        prefix_last: bool,
        filter: Option<(&str, &str)>,
        block_cache: &mut BlockCache,
    ) -> Result<(Vec<DocMatch>, Vec<String>)> {
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
    ) -> Result<(Vec<DocMatch>, Vec<String>)> {
        let mut plans = Vec::with_capacity(terms.len());
        let mut highlights = terms.to_vec();
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

            if maximum_distance > 0 || (prefix_last && index == last_index) {
                for candidate in self.inner.fuzzy.candidates(
                    term,
                    maximum_distance.min(bounded_typo_distance(term)),
                    MAX_FUZZY_TERMS_PER_DISTANCE,
                    true,
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
                    highlights.push(candidate_term.to_string());
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

        plans.sort_unstable_by_key(|plan| plan.estimated_documents);
        let mut plans = plans.into_iter();
        let Some(first) = plans.next() else {
            return Ok((Vec::new(), highlights));
        };
        let mut matches = self.matches_for_plan(&first, None, block_cache)?;
        self.retain_video_filter(&mut matches, filter)?;
        for plan in plans {
            let plan_matches = self.matches_for_plan(&plan, Some(&matches), block_cache)?;
            matches = intersect_sorted(matches, plan_matches);
            if matches.is_empty() {
                break;
            }
        }
        Ok((matches, highlights))
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

    fn matches_for_plan(
        &self,
        plan: &TermPlan,
        targets: Option<&[DocMatch]>,
        block_cache: &mut BlockCache,
    ) -> Result<Vec<DocMatch>> {
        if let Some(targets) = targets
            && targets.len() <= 128
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
                        token_stats(&text_tokens, &alternative.term)
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
                        token_stats(&title_tokens, &alternative.term)
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

    fn estimated_entry_documents(&self, entry: TermEntry) -> u64 {
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
            .then(|| self.inner.positions.range(entry.term, false))
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
            .then(|| self.inner.positions.range(entry.term, true))
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
                * 6.0
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
