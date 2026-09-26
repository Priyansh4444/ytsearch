use super::*;

impl DocMatch {
    const EXACT_MASK_BITS: u8 = 0b0000_1111;
    const TEXT_MATCH_FLAG: u8 = 1 << 4;
    const ORDERED_PHRASE_FLAG: u8 = 1 << 5;
    const ALL_RETAINED_EXACT_FLAG: u8 = 1 << 6;

    pub(super) fn new(document: u32, score: f32, matches_text: bool) -> Self {
        Self::with_exact_mask(document, score, matches_text, 1)
    }

    pub(super) fn with_exact_mask(
        document: u32,
        score: f32,
        matches_text: bool,
        exact_mask: u8,
    ) -> Self {
        Self {
            document,
            score,
            flags: (exact_mask & Self::EXACT_MASK_BITS)
                | (u8::from(matches_text) * Self::TEXT_MATCH_FLAG)
                | (u8::from(exact_mask != 0) * Self::ALL_RETAINED_EXACT_FLAG),
        }
    }

    pub(super) fn document(self) -> u32 {
        self.document
    }

    pub(super) fn matches_text(self) -> bool {
        self.flags & Self::TEXT_MATCH_FLAG != 0
    }

    pub(super) fn set_matches_text(&mut self, matches_text: bool) {
        self.flags = (self.flags & !Self::TEXT_MATCH_FLAG)
            | (u8::from(matches_text) * Self::TEXT_MATCH_FLAG);
    }

    pub(super) fn exact_mask(self) -> u8 {
        self.flags & Self::EXACT_MASK_BITS
    }

    pub(super) fn exact_tokens(self) -> u8 {
        self.exact_mask().count_ones() as u8
    }

    pub(super) fn set_exact_mask(&mut self, exact_mask: u8) {
        self.flags = (self.flags & !Self::EXACT_MASK_BITS) | (exact_mask & Self::EXACT_MASK_BITS);
        if exact_mask == 0 {
            self.set_all_retained_exact(false);
        }
    }

    pub(super) fn all_retained_exact(self) -> bool {
        self.flags & Self::ALL_RETAINED_EXACT_FLAG != 0
    }

    pub(super) fn set_all_retained_exact(&mut self, exact: bool) {
        self.flags = (self.flags & !Self::ALL_RETAINED_EXACT_FLAG)
            | (u8::from(exact) * Self::ALL_RETAINED_EXACT_FLAG);
    }

    pub(super) fn ordered_phrase(self) -> bool {
        self.flags & Self::ORDERED_PHRASE_FLAG != 0
    }

    pub(super) fn set_ordered_phrase(&mut self, ordered: bool) {
        self.flags = (self.flags & !Self::ORDERED_PHRASE_FLAG)
            | (u8::from(ordered) * Self::ORDERED_PHRASE_FLAG);
    }
}

const _: () = assert!(std::mem::size_of::<DocMatch>() <= 12);

pub(super) fn compare_video_sort(
    left: &VideoMeta,
    left_rank: RelevanceRank,
    right: &VideoMeta,
    right_rank: RelevanceRank,
    sort_by: Option<&str>,
    manifest: &Manifest,
) -> Ordering {
    let ordering =
        match sort_by {
            Some("published_at_timestamp:desc") => right
                .published_at_timestamp
                .cmp(&left.published_at_timestamp),
            Some("published_at_timestamp:asc") => left
                .published_at_timestamp
                .cmp(&right.published_at_timestamp),
            Some("view_count:desc") => right.view_count.cmp(&left.view_count),
            Some("like_count:desc") => right.like_count.cmp(&left.like_count),
            Some("comment_count:desc") => right.comment_count.cmp(&left.comment_count),
            Some("engagement_score:desc") => right_rank
                .exact_tokens
                .cmp(&left_rank.exact_tokens)
                .then_with(|| right_rank.exact_mask.cmp(&left_rank.exact_mask))
                .then_with(|| right_rank.ordered_phrase.cmp(&left_rank.ordered_phrase))
                .then_with(|| {
                    blended_relevance(right_rank.score, right, manifest)
                        .total_cmp(&blended_relevance(left_rank.score, left, manifest))
                }),
            _ => right_rank
                .exact_tokens
                .cmp(&left_rank.exact_tokens)
                .then_with(|| right_rank.exact_mask.cmp(&left_rank.exact_mask))
                .then_with(|| right_rank.ordered_phrase.cmp(&left_rank.ordered_phrase))
                .then_with(|| right_rank.score.total_cmp(&left_rank.score)),
        };
    ordering
        .then_with(|| right_rank.score.total_cmp(&left_rank.score))
        .then_with(|| left.id.cmp(&right.id))
}

pub(super) fn blended_relevance(relevance: f32, video: &VideoMeta, manifest: &Manifest) -> f32 {
    relevance
        + relevance.max(1.0) * ENGAGEMENT_RELEVANCE_BOOST * normalized_engagement(video, manifest)
}

pub(super) fn normalized_engagement(video: &VideoMeta, manifest: &Manifest) -> f32 {
    VIEW_WEIGHT * normalized_log(video.view_count, manifest.max_view_count)
        + LIKE_WEIGHT * normalized_log(video.like_count, manifest.max_like_count)
        + COMMENT_WEIGHT * normalized_log(video.comment_count, manifest.max_comment_count)
}

pub(super) fn normalized_log(value: u64, maximum: u64) -> f32 {
    if maximum == 0 {
        return 0.0;
    }
    (value as f32).ln_1p() / (maximum as f32).ln_1p()
}

pub(super) fn visible_group_moments(group: &VideoMatches) -> usize {
    group.moments.visible()
}

pub(super) fn retain_best_document(
    documents: &mut SmallVec<[DocMatch; 3]>,
    candidate: DocMatch,
    limit: usize,
) {
    if limit == 0 {
        return;
    }
    if documents.len() < limit {
        documents.push(candidate);
    } else {
        let Some(worst) = documents.last_mut() else {
            return;
        };
        if descending_doc_score(&candidate, worst) != Ordering::Less {
            return;
        }
        *worst = candidate;
    }
    let mut index = documents.len() - 1;
    while index > 0
        && descending_doc_score(&documents[index], &documents[index - 1]) == Ordering::Less
    {
        documents.swap(index, index - 1);
        index -= 1;
    }
}

pub(super) fn descending_doc_score(left: &DocMatch, right: &DocMatch) -> Ordering {
    right
        .exact_tokens()
        .cmp(&left.exact_tokens())
        .then_with(|| right.exact_mask().cmp(&left.exact_mask()))
        .then_with(|| right.ordered_phrase().cmp(&left.ordered_phrase()))
        .then_with(|| right.score.total_cmp(&left.score))
        .then_with(|| left.document().cmp(&right.document()))
}

pub(super) fn apply_bigram_proximity(
    matches: &mut [DocMatch],
    metadata: &DocumentTable,
    text: &[bigram::BigramPosting],
    title: &[bigram::BigramPosting],
    prioritize_position: bool,
) -> Result<()> {
    let mut text_index = 0_usize;
    for candidate in matches {
        while text
            .get(text_index)
            .is_some_and(|posting| posting.document < candidate.document())
        {
            text_index += 1;
        }
        let text_posting = text
            .get(text_index)
            .filter(|posting| posting.document == candidate.document());
        let video = metadata.document(candidate.document())?.video;
        let title_posting = title
            .binary_search_by_key(&video, |posting| posting.document)
            .ok()
            .and_then(|index| title.get(index));
        let ordered = text_posting.is_some() || title_posting.is_some();
        if ordered {
            let text_multiplier = text_posting
                .map(|posting| {
                    proximity_multiplier(
                        PositionalWindow {
                            displacement: 0,
                            max_offset: posting.earliest_end_offset,
                        },
                        2,
                        prioritize_position,
                    )
                })
                .unwrap_or(1.0);
            let title_multiplier = title_posting
                .map(|posting| {
                    proximity_multiplier(
                        PositionalWindow {
                            displacement: 0,
                            max_offset: posting.earliest_end_offset,
                        },
                        2,
                        prioritize_position,
                    )
                })
                .unwrap_or(1.0);
            candidate.score *= text_multiplier.max(title_multiplier) * ORDERED_PHRASE_MULTIPLIER;
        }
        candidate.set_ordered_phrase(ordered);
    }
    Ok(())
}

pub(super) fn intersect_all_borrowed(lists: &[Vec<DocMatch>]) -> Vec<DocMatch> {
    let Some((smallest_index, smallest)) = lists
        .iter()
        .enumerate()
        .min_by_key(|(_, matches)| matches.len())
    else {
        return Vec::new();
    };
    let mut intersection = smallest.clone();
    for (index, list) in lists.iter().enumerate() {
        if index == smallest_index {
            continue;
        }
        intersection = intersect_sorted_borrowed(intersection, list);
        if intersection.is_empty() {
            break;
        }
    }
    intersection
}

pub(super) fn retain_not_excluded(matches: &mut Vec<DocMatch>, excluded: &[u32]) {
    if excluded.is_empty() {
        return;
    }
    let mut excluded_index = 0_usize;
    matches.retain(|candidate| {
        let document = candidate.document();
        while excluded
            .get(excluded_index)
            .is_some_and(|excluded_document| *excluded_document < document)
        {
            excluded_index += 1;
        }
        excluded.get(excluded_index).copied() != Some(document)
    });
}

pub(super) fn intersect_sorted_borrowed(left: Vec<DocMatch>, right: &[DocMatch]) -> Vec<DocMatch> {
    let mut output = Vec::with_capacity(left.len().min(right.len()));
    let mut left_index = 0_usize;
    let mut right_index = 0_usize;
    while left_index < left.len() && right_index < right.len() {
        match left[left_index]
            .document()
            .cmp(&right[right_index].document())
        {
            Ordering::Less => left_index += 1,
            Ordering::Greater => right_index += 1,
            Ordering::Equal => {
                let mut combined = DocMatch::new(
                    left[left_index].document(),
                    left[left_index].score + right[right_index].score,
                    left[left_index].matches_text() || right[right_index].matches_text(),
                );
                combined.set_exact_mask(
                    left[left_index].exact_mask() | right[right_index].exact_mask(),
                );
                combined.set_all_retained_exact(
                    left[left_index].all_retained_exact()
                        && right[right_index].all_retained_exact(),
                );
                output.push(combined);
                left_index += 1;
                right_index += 1;
            }
        }
    }
    output
}

pub(super) fn intersect_sorted(left: Vec<DocMatch>, right: Vec<DocMatch>) -> Vec<DocMatch> {
    let mut output = Vec::with_capacity(left.len().min(right.len()));
    let mut left_index = 0_usize;
    let mut right_index = 0_usize;
    while left_index < left.len() && right_index < right.len() {
        match left[left_index]
            .document()
            .cmp(&right[right_index].document())
        {
            Ordering::Less => left_index += 1,
            Ordering::Greater => right_index += 1,
            Ordering::Equal => {
                let mut combined = DocMatch::new(
                    left[left_index].document(),
                    left[left_index].score + right[right_index].score,
                    left[left_index].matches_text() || right[right_index].matches_text(),
                );
                combined.set_exact_mask(
                    left[left_index].exact_mask() | right[right_index].exact_mask(),
                );
                combined.set_all_retained_exact(
                    left[left_index].all_retained_exact()
                        && right[right_index].all_retained_exact(),
                );
                output.push(combined);
                left_index += 1;
                right_index += 1;
            }
        }
    }
    output
}

pub(super) fn union_all(mut lists: Vec<Vec<DocMatch>>) -> Vec<DocMatch> {
    if lists.is_empty() {
        return Vec::new();
    }
    let mut union = lists.remove(0);
    for list in lists {
        union = union_sorted(union, list);
    }
    union
}

pub(super) fn union_sorted_ids(left: Vec<u32>, right: Vec<u32>) -> Vec<u32> {
    if left.is_empty() {
        return right;
    }
    if right.is_empty() {
        return left;
    }
    let mut output = Vec::with_capacity(left.len().saturating_add(right.len()));
    let mut left = left.into_iter().peekable();
    let mut right = right.into_iter().peekable();
    loop {
        match (left.peek(), right.peek()) {
            (Some(left_value), Some(right_value)) => match left_value.cmp(right_value) {
                Ordering::Less => output.push(left.next().expect("peeked left")),
                Ordering::Greater => output.push(right.next().expect("peeked right")),
                Ordering::Equal => {
                    output.push(left.next().expect("peeked left"));
                    right.next();
                }
            },
            (Some(_), None) => {
                output.extend(left);
                break;
            }
            (None, Some(_)) => {
                output.extend(right);
                break;
            }
            (None, None) => break,
        }
    }
    output
}

pub(super) fn union_sorted(left: Vec<DocMatch>, right: Vec<DocMatch>) -> Vec<DocMatch> {
    if left.is_empty() {
        return right;
    }
    if right.is_empty() {
        return left;
    }
    let mut output = Vec::with_capacity(left.len().saturating_add(right.len()));
    let mut left = left.into_iter().peekable();
    let mut right = right.into_iter().peekable();
    loop {
        match (left.peek(), right.peek()) {
            (Some(left_value), Some(right_value)) => {
                match left_value.document().cmp(&right_value.document()) {
                    Ordering::Less => output.push(left.next().expect("peeked left")),
                    Ordering::Greater => output.push(right.next().expect("peeked right")),
                    Ordering::Equal => {
                        let left_value = left.next().expect("peeked left");
                        let right_value = right.next().expect("peeked right");
                        let mut combined = DocMatch::new(
                            left_value.document(),
                            left_value.score + right_value.score,
                            left_value.matches_text() || right_value.matches_text(),
                        );
                        combined.set_exact_mask(left_value.exact_mask() | right_value.exact_mask());
                        combined.set_all_retained_exact(
                            left_value.all_retained_exact() || right_value.all_retained_exact(),
                        );
                        output.push(combined);
                    }
                }
            }
            (Some(_), None) => {
                output.extend(left);
                break;
            }
            (None, Some(_)) => {
                output.extend(right);
                break;
            }
            (None, None) => break,
        }
    }
    output
}

pub(super) fn parse_query(value: &str) -> Result<ParsedQuery> {
    let mut parser = QueryParser::default();
    let mut segment = String::new();
    let mut quoted = false;
    let mut quote_excluded = false;
    let mut allow_leading_exclusion = true;
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\'
            && characters
                .peek()
                .is_some_and(|next| matches!(next, '"' | '\\'))
        {
            segment.push(characters.next().expect("peeked escaped character"));
            continue;
        }
        if character == '"' {
            if quoted {
                record_quoted_segment(&mut segment, quote_excluded, &mut parser)?;
                quoted = false;
                allow_leading_exclusion = false;
            } else {
                quote_excluded = take_negative_phrase_marker(&mut segment, allow_leading_exclusion);
                record_unquoted_segment(&mut segment, allow_leading_exclusion, &mut parser)?;
                quoted = true;
            }
        } else {
            segment.push(character);
        }
    }
    if quoted {
        return Err(Error::InvalidInput(
            "query contains an unterminated quoted phrase".to_string(),
        ));
    }
    record_unquoted_segment(&mut segment, allow_leading_exclusion, &mut parser)?;
    Ok(ParsedQuery {
        terms: parser.terms,
        sequence: parser.sequence,
        phrases: parser.phrases,
        excluded_terms: parser.excluded_terms,
        excluded_phrases: parser.excluded_phrases,
    })
}

pub(super) fn record_quoted_segment(
    segment: &mut String,
    excluded: bool,
    parser: &mut QueryParser,
) -> Result<()> {
    let tokens = ordered_tokens(segment);
    segment.clear();
    if tokens.is_empty() {
        return Err(Error::InvalidInput(
            "quoted phrase must contain at least one searchable token".to_string(),
        ));
    }
    if tokens.len() > MAX_PHRASE_TOKENS {
        return Err(Error::InvalidInput(format!(
            "quoted phrase exceeds {MAX_PHRASE_TOKENS} tokens"
        )));
    }
    if excluded {
        if tokens.len() == 1 {
            let token = tokens.into_iter().next().expect("one quoted token");
            if parser.excluded_seen.insert(token.clone()) {
                parser.excluded_terms.push(token);
            }
        } else if !parser.excluded_phrases.contains(&tokens) {
            parser.excluded_phrases.push(tokens);
        }
    } else {
        for token in &tokens {
            if parser.seen.insert(token.clone()) {
                parser.terms.push(token.clone());
            }
        }
        parser.phrases.push(tokens);
    }
    Ok(())
}

pub(super) fn record_unquoted_segment(
    segment: &mut String,
    allow_leading_exclusion: bool,
    parser: &mut QueryParser,
) -> Result<()> {
    for (token, excluded) in signed_tokens(segment, allow_leading_exclusion)? {
        if excluded {
            if parser.excluded_seen.insert(token.clone()) {
                parser.excluded_terms.push(token);
            }
        } else {
            if parser.seen.insert(token.clone()) {
                parser.terms.push(token.clone());
            }
            parser.sequence.push(token);
        }
    }
    segment.clear();
    Ok(())
}

pub(super) fn take_negative_phrase_marker(
    segment: &mut String,
    allow_leading_exclusion: bool,
) -> bool {
    let Some(marker_index) = segment.len().checked_sub(1) else {
        return false;
    };
    if segment.as_bytes()[marker_index] != b'-' {
        return false;
    }
    let valid = if marker_index == 0 {
        allow_leading_exclusion
    } else {
        segment[..marker_index]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    };
    if valid {
        segment.truncate(marker_index);
    }
    valid
}

pub(super) fn signed_tokens(
    value: &str,
    allow_leading_exclusion: bool,
) -> Result<Vec<(String, bool)>> {
    let mut output = Vec::new();
    let mut token = String::new();
    let mut excluded = false;
    let mut can_exclude = allow_leading_exclusion;
    for character in value.chars() {
        if character.is_alphanumeric() {
            token.extend(character.to_lowercase());
            can_exclude = false;
        } else if character == '-' && token.is_empty() && can_exclude {
            excluded = true;
            can_exclude = false;
        } else {
            if excluded && token.is_empty() {
                return Err(Error::InvalidInput(
                    "negative marker must directly prefix a searchable token".to_string(),
                ));
            }
            if !token.is_empty() {
                output.push((std::mem::take(&mut token), excluded));
                excluded = false;
            }
            can_exclude = character.is_whitespace();
        }
    }
    if excluded && token.is_empty() {
        return Err(Error::InvalidInput(
            "negative marker must directly prefix a searchable token".to_string(),
        ));
    }
    if !token.is_empty() {
        output.push((token, excluded));
    }
    Ok(output)
}

pub(super) fn ordered_tokens(value: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut token = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            token.extend(character.to_lowercase());
        } else if !token.is_empty() {
            output.push(std::mem::take(&mut token));
        }
    }
    if !token.is_empty() {
        output.push(token);
    }
    output
}

#[derive(Clone, Copy)]
pub(super) struct PositionalSummary {
    pub(super) window: Option<PositionalWindow>,
    pub(super) ordered: bool,
}

pub(super) fn positional_summary(
    value: &str,
    terms: &[String],
    sequence: &[String],
) -> PositionalSummary {
    let tokens = ordered_tokens(value);
    let mut positions = vec![Vec::new(); terms.len()];
    for (position, token) in tokens.iter().enumerate() {
        let Ok(position) = u32::try_from(position) else {
            break;
        };
        for (term_positions, term) in positions.iter_mut().zip(terms) {
            if token == term {
                term_positions.push(position);
            }
        }
    }
    let window =
        minimum_position_window_slices(&positions.iter().map(Vec::as_slice).collect::<Vec<_>>());
    let ordered = tokens
        .windows(sequence.len())
        .any(|window| window == sequence);
    PositionalSummary { window, ordered }
}

/// `positions[i]` holds the positions of the phrase's `i`th token; repeated
/// tokens share one list.
pub(super) fn ordered_phrase_positions_match(positions: &[&[u32]], slop: u8) -> bool {
    let Some(first) = positions.first() else {
        return false;
    };
    first.iter().copied().any(|start| {
        let mut previous = start;
        let mut used_slop = 0_u32;
        for token_positions in positions.iter().skip(1) {
            let next_index = token_positions.partition_point(|position| *position <= previous);
            let Some(&next) = token_positions.get(next_index) else {
                return false;
            };
            used_slop = used_slop.saturating_add(next - previous - 1);
            if used_slop > u32::from(slop) {
                return false;
            }
            previous = next;
        }
        true
    })
}

pub(super) fn minimum_position_window_slices(positions: &[&[u32]]) -> Option<PositionalWindow> {
    if positions.len() < 2 || positions.iter().any(|values| values.is_empty()) {
        return None;
    }
    let mut indices = vec![0_usize; positions.len()];
    let mut best: Option<PositionalWindow> = None;
    loop {
        let mut minimum = (0_usize, u32::MAX);
        let mut maximum = 0_u32;
        for (cursor_index, (values, &position_index)) in positions.iter().zip(&indices).enumerate()
        {
            let position = values[position_index];
            if position < minimum.1 {
                minimum = (cursor_index, position);
            }
            maximum = maximum.max(position);
        }
        let window = PositionalWindow {
            displacement: maximum - minimum.1,
            max_offset: maximum,
        };
        if best.is_none_or(|current| {
            window.displacement < current.displacement
                || (window.displacement == current.displacement
                    && window.max_offset < current.max_offset)
        }) {
            best = Some(window);
        }
        if window.displacement == positions.len().saturating_sub(1) as u32 {
            break;
        }
        let minimum_index = &mut indices[minimum.0];
        *minimum_index += 1;
        if *minimum_index == positions[minimum.0].len() {
            break;
        }
    }
    best
}

pub(super) fn positional_window(
    windows: &[(u32, PositionalWindow)],
    id: u32,
) -> Option<PositionalWindow> {
    windows
        .binary_search_by_key(&id, |(candidate, _)| *candidate)
        .ok()
        .map(|index| windows[index].1)
}

pub(super) fn proximity_multiplier(
    window: PositionalWindow,
    tokens: usize,
    prioritize_position: bool,
) -> f32 {
    const COMPATIBILITY_PROXIMITY_WINDOW: u32 = 10;
    if window.displacement > COMPATIBILITY_PROXIMITY_WINDOW {
        return 1.0;
    }
    let minimum_displacement = tokens.saturating_sub(1) as u32;
    let extra_gap = window.displacement.saturating_sub(minimum_displacement);
    let proximity = PROXIMITY_SCORE_BOOST / (extra_gap as f32 + 1.0).sqrt();
    let position = if prioritize_position {
        PROXIMITY_POSITION_BOOST / (window.max_offset as f32 + 1.0).sqrt()
    } else {
        0.0
    };
    1.0 + proximity + position
}

pub(super) fn validate_search_params(params: &SearchParams) -> Result<()> {
    validate_query_and_window(params)?;
    query_fields(params.query_by.as_deref(), false)?;
    parse_filter(params.filter_by.as_deref())?;
    if let Some(group_by) = params.group_by.as_deref()
        && group_by != "video_id"
    {
        return Err(Error::InvalidInput(format!(
            "unsupported group_by field: {group_by}"
        )));
    }
    if params.group_limit == Some(0) {
        return Err(Error::InvalidInput(
            "group_limit must be at least 1".to_string(),
        ));
    }
    if params
        .group_limit
        .is_some_and(|value| value > MAX_PAGE_SIZE)
    {
        return Err(Error::InvalidInput(format!(
            "group_limit cannot exceed {MAX_PAGE_SIZE}"
        )));
    }
    if let Some(sort_by) = params.sort_by.as_deref()
        && !matches!(
            sort_by,
            "published_at_timestamp:desc"
                | "published_at_timestamp:asc"
                | "view_count:desc"
                | "like_count:desc"
                | "comment_count:desc"
                | "engagement_score:desc"
        )
    {
        return Err(Error::InvalidInput(format!(
            "unsupported sort_by expression: {sort_by}"
        )));
    }

    if let Some(weights) = params.query_by_weights.as_deref() {
        let fields = params.query_by.as_deref().unwrap_or("text,title");
        let expected = match fields {
            "text,title" => "4,6",
            "title,text" => "6,4",
            "text" => "4",
            "title" => "6",
            _ => {
                return Err(Error::InvalidInput(
                    "query_by_weights requires an exact supported query_by order".to_string(),
                ));
            }
        };
        if weights != expected {
            return Err(Error::InvalidInput(format!(
                "query_by_weights must be {expected} for query_by={fields}"
            )));
        }
    }
    if params
        .text_match_type
        .as_deref()
        .is_some_and(|value| value != "sum_score")
    {
        return Err(Error::InvalidInput(
            "text_match_type must be sum_score".to_string(),
        ));
    }
    if params.prioritize_exact_match == Some(false) {
        return Err(Error::InvalidInput(
            "prioritize_exact_match=false is unsupported".to_string(),
        ));
    }
    if params.prioritize_token_position == Some(false) {
        return Err(Error::InvalidInput(
            "prioritize_token_position=false is unsupported".to_string(),
        ));
    }
    if params
        .phrase_slop
        .is_some_and(|slop| slop > MAX_PHRASE_SLOP)
    {
        return Err(Error::InvalidInput(format!(
            "phrase_slop cannot exceed {MAX_PHRASE_SLOP}"
        )));
    }
    if params.typo_tokens_threshold.is_some_and(|value| value != 5) {
        return Err(Error::InvalidInput(
            "typo_tokens_threshold must be 5".to_string(),
        ));
    }
    if params.drop_tokens_threshold.is_some_and(|value| value != 1) {
        return Err(Error::InvalidInput(
            "drop_tokens_threshold must be 1".to_string(),
        ));
    }
    if params
        .drop_tokens_mode
        .as_deref()
        .is_some_and(|value| value != "right_to_left")
    {
        return Err(Error::InvalidInput(
            "drop_tokens_mode must be right_to_left".to_string(),
        ));
    }
    if params
        .split_join_tokens
        .as_deref()
        .is_some_and(|value| value != "always")
    {
        return Err(Error::InvalidInput(
            "split_join_tokens must be always".to_string(),
        ));
    }
    if let Some(fields) = params.highlight_fields.as_deref() {
        query_fields(Some(fields), false)?;
    }
    if params
        .include_fields
        .as_deref()
        .is_some_and(|fields| fields != ACTIVE_INCLUDE_FIELDS)
    {
        return Err(Error::InvalidInput(
            "unsupported include_fields projection".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_query_and_window(params: &SearchParams) -> Result<()> {
    let query = params.q.trim();
    if query.is_empty() {
        return Err(Error::InvalidInput("q cannot be empty".to_string()));
    }
    if params.q.len() > MAX_QUERY_BYTES {
        return Err(Error::InvalidInput(format!(
            "q exceeds {MAX_QUERY_BYTES} bytes"
        )));
    }

    for (name, value) in [("per_page", params.per_page), ("limit", params.limit)] {
        if value.is_some_and(|value| value > MAX_PAGE_SIZE) {
            return Err(Error::InvalidInput(format!(
                "{name} cannot exceed {MAX_PAGE_SIZE}"
            )));
        }
    }
    if params.limit.is_some() && (params.page.is_some() || params.per_page.is_some()) {
        return Err(Error::InvalidInput(
            "limit/offset cannot be combined with page/per_page".to_string(),
        ));
    }
    if params.offset.is_some() && params.limit.is_none() {
        return Err(Error::InvalidInput("offset requires limit".to_string()));
    }
    Ok(())
}

pub(super) fn validate_channel_search_params(params: &SearchParams) -> Result<()> {
    validate_query_and_window(params)?;
    if params
        .query_by
        .as_deref()
        .is_some_and(|fields| fields != "name,handle")
    {
        return Err(Error::InvalidInput(
            "channel query_by must be name,handle".to_string(),
        ));
    }
    for (name, present) in [
        ("query_by_weights", params.query_by_weights.is_some()),
        ("filter_by", params.filter_by.is_some()),
        ("group_by", params.group_by.is_some()),
        ("group_limit", params.group_limit.is_some()),
        ("sort_by", params.sort_by.is_some()),
        ("text_match_type", params.text_match_type.is_some()),
        (
            "prioritize_exact_match",
            params.prioritize_exact_match.is_some(),
        ),
        (
            "prioritize_token_position",
            params.prioritize_token_position.is_some(),
        ),
        ("phrase_slop", params.phrase_slop.is_some()),
        (
            "typo_tokens_threshold",
            params.typo_tokens_threshold.is_some(),
        ),
        (
            "drop_tokens_threshold",
            params.drop_tokens_threshold.is_some(),
        ),
        ("drop_tokens_mode", params.drop_tokens_mode.is_some()),
        ("split_join_tokens", params.split_join_tokens.is_some()),
        ("highlight_fields", params.highlight_fields.is_some()),
        ("include_fields", params.include_fields.is_some()),
    ] {
        if present {
            return Err(Error::InvalidInput(format!(
                "unsupported channel search parameter: {name}"
            )));
        }
    }
    Ok(())
}

pub(super) fn query_fields(
    query_by: Option<&str>,
    prioritize_position: bool,
) -> Result<QueryFields> {
    let requested = query_by.unwrap_or("text,title");
    let mut text = false;
    let mut title = false;
    for field in requested.split(',').map(str::trim) {
        match field {
            "text" if !text => text = true,
            "title" if !title => title = true,
            "text" | "title" => {
                return Err(Error::InvalidInput(format!(
                    "query_by contains duplicate field: {field}"
                )));
            }
            "" => {
                return Err(Error::InvalidInput(
                    "query_by cannot contain an empty field".to_string(),
                ));
            }
            unsupported => {
                return Err(Error::InvalidInput(format!(
                    "unsupported query_by field: {unsupported}"
                )));
            }
        }
    }
    Ok(QueryFields {
        text,
        title,
        prioritize_position,
        exact_mask: 1,
    })
}

pub(super) fn inverse_document_frequency(documents: u32, frequency: u32) -> f32 {
    let documents = documents as f32;
    let frequency = frequency as f32;
    (1.0 + (documents - frequency + 0.5) / (frequency + 0.5)).ln()
}

const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;

pub(super) fn bm25(frequency: u32, document_tokens: u32, average_tokens: f32) -> f32 {
    bm25_with_length_norm(frequency, bm25_length_norm(document_tokens, average_tokens))
}

/// The document-dependent BM25 denominator term. Streaming scans compute it
/// once per document and share it across every matching term alternative.
pub(super) fn bm25_length_norm(document_tokens: u32, average_tokens: f32) -> f32 {
    let normalized_length = if average_tokens > 0.0 {
        document_tokens as f32 / average_tokens
    } else {
        1.0
    };
    BM25_K1 * (1.0 - BM25_B + BM25_B * normalized_length)
}

pub(super) fn bm25_with_length_norm(frequency: u32, length_norm: f32) -> f32 {
    let frequency = frequency as f32;
    frequency * (BM25_K1 + 1.0) / (frequency + length_norm)
}

pub(super) fn token_position_multiplier(first_position: u32, enabled: bool) -> f32 {
    if !enabled {
        return 1.0;
    }
    1.0 + POSITION_SCORE_BOOST / (first_position as f32 + 1.0).sqrt()
}

pub(super) fn result_window(params: &SearchParams) -> (usize, usize) {
    if let Some(limit) = params.limit {
        return (
            params.offset.unwrap_or_default() as usize,
            usize::from(limit),
        );
    }
    let limit = usize::from(params.per_page.unwrap_or(10));
    let page = params.page.unwrap_or(1).max(1) as usize;
    ((page - 1).saturating_mul(limit), limit)
}

pub(super) fn parse_filter(filter: Option<&str>) -> Result<Option<(&str, &str)>> {
    let Some(filter) = filter else {
        return Ok(None);
    };
    let (field, value) = filter
        .split_once(":=")
        .ok_or_else(|| Error::InvalidInput("filter_by must use an exact := filter".to_string()))?;
    let field = field.trim();
    if !matches!(field, "video_id" | "youtuber_handle") {
        return Err(Error::InvalidInput(format!(
            "unsupported filter_by field: {field}"
        )));
    }
    let value = value.trim();
    if value.contains(":=")
        || value.contains("&&")
        || value.contains("||")
        || value.chars().any(|character| {
            matches!(
                character,
                '[' | ']' | '(' | ')' | '{' | '}' | ',' | '<' | '>'
            )
        })
    {
        return Err(Error::InvalidInput(
            "compound filter_by expressions are unsupported".to_string(),
        ));
    }
    let value = match (value.strip_prefix('`'), value.strip_suffix('`')) {
        (Some(without_prefix), Some(_)) => without_prefix
            .strip_suffix('`')
            .ok_or_else(|| Error::InvalidInput("filter_by has malformed backticks".to_string()))?,
        (None, None) => value,
        _ => {
            return Err(Error::InvalidInput(
                "filter_by has malformed backticks".to_string(),
            ));
        }
    };
    if value.contains('`') {
        return Err(Error::InvalidInput(
            "filter_by has malformed backticks".to_string(),
        ));
    }
    if value.is_empty() {
        return Err(Error::InvalidInput(
            "filter_by value cannot be empty".to_string(),
        ));
    }
    if value.len() > MAX_FILTER_VALUE_BYTES {
        return Err(Error::InvalidInput(format!(
            "filter_by value exceeds {MAX_FILTER_VALUE_BYTES} bytes"
        )));
    }
    Ok(Some((field, value)))
}

pub(super) fn video_matches_filter(video: &VideoMeta, filter: Option<(&str, &str)>) -> bool {
    let Some((field, value)) = filter else {
        return true;
    };
    match field {
        "video_id" => video.id == value,
        "youtuber_handle" => video.handle.eq_ignore_ascii_case(value),
        _ => true,
    }
}

pub(super) fn bounded_typo_distance(term: &str) -> u8 {
    if term.len() < MIN_ONE_TYPO_BYTES {
        0
    } else if term.len() < MIN_TWO_TYPO_BYTES {
        1
    } else {
        2
    }
}

pub(super) fn highlighted_snippet(
    value: &str,
    terms: &[String],
    maximum_chars: usize,
) -> Option<String> {
    let terms = terms.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut matches = Vec::new();
    let mut token_start = None;
    let mut token = String::new();
    for (index, character) in value
        .char_indices()
        .chain(std::iter::once((value.len(), ' ')))
    {
        if character.is_alphanumeric() {
            token_start.get_or_insert(index);
            token.extend(character.to_lowercase());
            continue;
        }
        if let Some(start) = token_start.take()
            && terms.contains(token.as_str())
        {
            matches.push((start, index));
        }
        token.clear();
    }
    let &(start, _) = matches.first()?;
    let mut snippet_start = start.saturating_sub(maximum_chars / 3);
    while !value.is_char_boundary(snippet_start) {
        snippet_start += 1;
    }
    let mut snippet_end = (snippet_start + maximum_chars).min(value.len());
    while !value.is_char_boundary(snippet_end) {
        snippet_end -= 1;
    }
    let mut output = String::with_capacity(snippet_end - snippet_start + matches.len() * 13);
    let mut cursor = snippet_start;
    for (mark_start, mark_end) in matches {
        if mark_start < snippet_start || mark_end > snippet_end {
            continue;
        }
        escape_html(&value[cursor..mark_start], &mut output);
        output.push_str("<mark>");
        escape_html(&value[mark_start..mark_end], &mut output);
        output.push_str("</mark>");
        cursor = mark_end;
    }
    escape_html(&value[cursor..snippet_end], &mut output);
    Some(output)
}

pub(super) fn matched_highlight_terms(value: &str, groups: &[Vec<String>]) -> Vec<String> {
    let present = ordered_tokens(value).into_iter().collect::<HashSet<_>>();
    let mut selected = Vec::with_capacity(groups.len());
    for group in groups {
        let Some(original) = group.first() else {
            continue;
        };
        if present.contains(original.as_str()) {
            if !selected.contains(original) {
                selected.push(original.clone());
            }
            continue;
        }
        for alternative in &group[1..] {
            if present.contains(alternative.as_str()) && !selected.contains(alternative) {
                selected.push(alternative.clone());
            }
        }
    }
    selected
}

pub(super) fn escape_html(value: &str, output: &mut String) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(character),
        }
    }
}

pub(super) fn read_json_lines(path: &Path) -> Result<Vec<Value>> {
    let mut values = Vec::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if !line.trim().is_empty() {
            values.push(serde_json::from_str(&line)?);
        }
    }
    Ok(values)
}

pub(super) fn directory_size(path: &Path) -> Result<u64> {
    let mut pending = vec![PathBuf::from(path)];
    let mut bytes = 0_u64;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    Ok(bytes)
}

pub(super) fn generation_files(schema_version: u8) -> Result<&'static [&'static str]> {
    match schema_version {
        5 | 6 => Ok(&GENERATION_FILES_SCHEMA_6),
        7 => Ok(&GENERATION_FILES_SCHEMA_7),
        8 => Ok(&GENERATION_FILES_SCHEMA_8),
        _ => Err(Error::Corrupt(format!(
            "unsupported schema version {schema_version}"
        ))),
    }
}

pub(super) fn build_manifest_files(
    index_path: &Path,
    schema_version: u8,
) -> Result<BTreeMap<String, ManifestFile>> {
    let mut files = BTreeMap::new();
    for &name in generation_files(schema_version)? {
        let path = index_path.join(name);
        let bytes = fs::metadata(&path)?.len();
        files.insert(
            name.to_string(),
            ManifestFile {
                bytes,
                blake3: checksum_file(&path)?,
            },
        );
    }
    Ok(files)
}

pub(super) fn validate_manifest_files(
    index_path: &Path,
    manifest: &Manifest,
    validation: ManifestValidation,
) -> Result<()> {
    let generation_files = generation_files(manifest.schema_version)?;
    if manifest.files.len() != generation_files.len() {
        return Err(Error::Corrupt(format!(
            "manifest records {} generation files, expected {}",
            manifest.files.len(),
            generation_files.len()
        )));
    }
    for &name in generation_files {
        let expected = manifest
            .files
            .get(name)
            .ok_or_else(|| Error::Corrupt(format!("manifest is missing generation file {name}")))?;
        let path = index_path.join(name);
        let actual_bytes = fs::metadata(&path)?.len();
        if actual_bytes != expected.bytes {
            return Err(Error::Corrupt(format!(
                "generation file {name} is {actual_bytes} bytes, expected {}",
                expected.bytes
            )));
        }
        if matches!(validation, ManifestValidation::Full) {
            tracing::info!(
                file = name,
                bytes = actual_bytes,
                "validating raw generation file checksum"
            );
            let started = std::time::Instant::now();
            let actual_checksum = checksum_file(&path)?;
            if actual_checksum != expected.blake3 {
                return Err(Error::Corrupt(format!(
                    "generation file {name} failed its BLAKE3 checksum"
                )));
            }
            tracing::info!(
                file = name,
                bytes = actual_bytes,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "validated raw generation file checksum"
            );
        }
    }
    Ok(())
}

pub(super) fn checksum_file(path: &Path) -> Result<String> {
    let mut input = BufReader::new(ColdReader::open(path)?);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub(super) fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
