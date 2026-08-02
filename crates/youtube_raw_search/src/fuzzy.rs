use crate::{
    Error, Result,
    format::{FUZZY_MAGIC, HEADER_BYTES, MappedFile, read_u16, read_u32, write_header},
};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

pub const FUZZY_FILE: &str = "fuzzy.bin";

const NODE_BYTES: usize = 20;
const NO_NODE: u32 = u32::MAX;
const NO_TERM: u32 = u32::MAX;
const MAX_QUERY_BYTES: usize = 64;
const TITLE_FREQUENCY_WEIGHT: u64 = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FuzzyCandidate {
    pub term: u32,
    pub distance: u8,
    pub exact: bool,
}

#[derive(Default)]
struct BuildNode {
    label: u8,
    children: Vec<(u8, usize)>,
    terminal: Option<Terminal>,
}

#[derive(Clone, Copy)]
struct Terminal {
    term: u32,
    text_frequency: u32,
    title_frequency: u32,
}

#[derive(Clone, Copy)]
struct Node {
    label: u8,
    first_child: u32,
    child_count: u16,
    terminal: u32,
    text_frequency: u32,
    title_frequency: u32,
}

pub struct FuzzyIndex {
    file: MappedFile,
    nodes: usize,
}

pub fn write_index<'a, I>(path: &Path, terms: I) -> Result<()>
where
    I: IntoIterator<Item = (u32, &'a str, u32, u32)>,
{
    let mut nodes = vec![BuildNode::default()];
    let mut previous = None::<&str>;
    for (term_id, term, text_frequency, title_frequency) in terms {
        if previous.is_some_and(|value| value >= term) {
            return Err(Error::Corrupt(
                "fuzzy terms must be strictly sorted".to_string(),
            ));
        }
        previous = Some(term);
        let mut node_id = 0_usize;
        for label in term.bytes() {
            let child = nodes[node_id]
                .children
                .last()
                .filter(|(candidate, _)| *candidate == label)
                .map(|(_, child)| *child);
            node_id = if let Some(child) = child {
                child
            } else {
                if nodes[node_id]
                    .children
                    .last()
                    .is_some_and(|(candidate, _)| *candidate >= label)
                {
                    return Err(Error::Corrupt(
                        "fuzzy trie children are not sorted".to_string(),
                    ));
                }
                let child = nodes.len();
                nodes.push(BuildNode {
                    label,
                    ..BuildNode::default()
                });
                nodes[node_id].children.push((label, child));
                child
            };
        }
        nodes[node_id].terminal = Some(Terminal {
            term: term_id,
            text_frequency,
            title_frequency,
        });
    }

    let mut order = Vec::with_capacity(nodes.len());
    let mut old_to_new = vec![NO_NODE; nodes.len()];
    order.push(0_usize);
    old_to_new[0] = 0;
    let mut cursor = 0_usize;
    while cursor < order.len() {
        let old_id = order[cursor];
        for (_, child) in &nodes[old_id].children {
            let new_id = u32::try_from(order.len())
                .map_err(|_| Error::Corrupt("fuzzy trie exceeds u32 nodes".to_string()))?;
            old_to_new[*child] = new_id;
            order.push(*child);
        }
        cursor += 1;
    }

    let node_count = u32::try_from(order.len())
        .map_err(|_| Error::Corrupt("fuzzy trie exceeds u32 nodes".to_string()))?;
    let mut output = BufWriter::new(File::create(path.join(FUZZY_FILE))?);
    let mut header = Vec::with_capacity(HEADER_BYTES);
    write_header(&mut header, FUZZY_MAGIC, node_count);
    output.write_all(&header)?;
    for old_id in order {
        let node = &nodes[old_id];
        let child_count = u16::try_from(node.children.len())
            .map_err(|_| Error::Corrupt("fuzzy trie node exceeds u16 children".to_string()))?;
        let first_child = node
            .children
            .first()
            .map_or(NO_NODE, |(_, child)| old_to_new[*child]);
        let terminal = node.terminal;
        output.write_all(&[node.label, 0])?;
        output.write_all(&child_count.to_le_bytes())?;
        output.write_all(&first_child.to_le_bytes())?;
        output.write_all(&terminal.map_or(NO_TERM, |value| value.term).to_le_bytes())?;
        output.write_all(
            &terminal
                .map_or(0, |value| value.text_frequency)
                .to_le_bytes(),
        )?;
        output.write_all(
            &terminal
                .map_or(0, |value| value.title_frequency)
                .to_le_bytes(),
        )?;
    }
    output.flush()?;
    Ok(())
}

impl FuzzyIndex {
    pub fn open(path: &Path, expected_terms: usize) -> Result<Self> {
        let file = MappedFile::open(&path.join(FUZZY_FILE), FUZZY_MAGIC)?;
        let nodes = read_u32(file.bytes(), 8)? as usize;
        let records_bytes = nodes
            .checked_mul(NODE_BYTES)
            .ok_or_else(|| Error::Corrupt("fuzzy index length overflows".to_string()))?;
        let expected_bytes = HEADER_BYTES
            .checked_add(records_bytes)
            .ok_or_else(|| Error::Corrupt("fuzzy index boundary overflows".to_string()))?;
        if nodes == 0 || expected_bytes != file.bytes().len() {
            return Err(Error::Corrupt(
                "fuzzy index has an invalid node boundary".to_string(),
            ));
        }
        let index = Self { file, nodes };
        let mut parent_counts = vec![0_u8; nodes];
        let mut terminal_seen = vec![false; expected_terms];
        for node_id in 0..nodes {
            let node = index.node(node_id as u32)?;
            let children = index.children(node)?;
            let mut previous = None;
            for child_id in children {
                if child_id as usize <= node_id {
                    return Err(Error::Corrupt(
                        "fuzzy trie child does not follow its parent".to_string(),
                    ));
                }
                parent_counts[child_id as usize] = parent_counts[child_id as usize]
                    .checked_add(1)
                    .ok_or_else(|| Error::Corrupt("fuzzy parent count overflows".to_string()))?;
                let label = index.node(child_id)?.label;
                if previous.is_some_and(|value| value >= label) {
                    return Err(Error::Corrupt(
                        "fuzzy trie children are not strictly sorted".to_string(),
                    ));
                }
                previous = Some(label);
            }
            if node.terminal != NO_TERM && node.terminal as usize >= expected_terms {
                return Err(Error::Corrupt(
                    "fuzzy trie terminal is outside the lexicon".to_string(),
                ));
            }
            if node.terminal != NO_TERM {
                let seen = &mut terminal_seen[node.terminal as usize];
                if *seen {
                    return Err(Error::Corrupt(
                        "fuzzy trie contains a duplicate terminal".to_string(),
                    ));
                }
                *seen = true;
            }
        }
        if parent_counts[0] != 0
            || parent_counts.iter().skip(1).any(|parents| *parents != 1)
            || terminal_seen.iter().any(|seen| !seen)
        {
            return Err(Error::Corrupt(
                "fuzzy trie is not a complete tree".to_string(),
            ));
        }
        Ok(index)
    }

    pub fn candidates(
        &self,
        query: &str,
        maximum_distance: u8,
        maximum_per_distance: usize,
        prefix: bool,
        search_text: bool,
        search_title: bool,
    ) -> Result<Vec<FuzzyCandidate>> {
        let query = query.as_bytes();
        if query.is_empty()
            || query.len() > MAX_QUERY_BYTES
            || maximum_per_distance == 0
            || maximum_distance > 2
        {
            return Ok(Vec::new());
        }
        let mut initial = [0_u8; MAX_QUERY_BYTES + 1];
        for (index, value) in initial.iter_mut().enumerate().take(query.len() + 1) {
            *value = index as u8;
        }
        let mut collector = CandidateCollector::new(
            maximum_distance,
            maximum_per_distance,
            search_text,
            search_title,
        );
        for child in self.children(self.node(0)?)? {
            self.walk(
                child,
                1,
                0,
                query,
                maximum_distance,
                prefix,
                &initial,
                &initial,
                &mut collector,
            )?;
        }
        Ok(collector.finish())
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        node_id: u32,
        depth: usize,
        previous_label: u8,
        query: &[u8],
        maximum_distance: u8,
        prefix: bool,
        previous_previous_row: &[u8; MAX_QUERY_BYTES + 1],
        previous_row: &[u8; MAX_QUERY_BYTES + 1],
        collector: &mut CandidateCollector,
    ) -> Result<()> {
        let node = self.node(node_id)?;
        let mut current = [u8::MAX; MAX_QUERY_BYTES + 1];
        current[0] = depth.min(u8::MAX as usize) as u8;
        let mut row_minimum = current[0];
        for column in 1..=query.len() {
            let substitution =
                previous_row[column - 1].saturating_add(u8::from(node.label != query[column - 1]));
            let insertion = current[column - 1].saturating_add(1);
            let deletion = previous_row[column].saturating_add(1);
            let mut distance = substitution.min(insertion).min(deletion);
            if depth > 1
                && column > 1
                && node.label == query[column - 2]
                && previous_label == query[column - 1]
            {
                distance = distance.min(previous_previous_row[column - 2].saturating_add(1));
            }
            current[column] = distance;
            row_minimum = row_minimum.min(distance);
        }

        let distance = current[query.len()];
        if prefix && depth >= query.len() && distance <= maximum_distance {
            self.collect_subtree(node_id, distance, depth == query.len(), collector)?;
            return Ok(());
        }
        if node.terminal != NO_TERM && distance <= maximum_distance {
            collector.insert(node, distance, depth == query.len() && distance == 0);
        }
        if row_minimum > maximum_distance
            || (!prefix && depth >= query.len().saturating_add(maximum_distance as usize))
        {
            return Ok(());
        }
        for child in self.children(node)? {
            self.walk(
                child,
                depth + 1,
                node.label,
                query,
                maximum_distance,
                prefix,
                previous_row,
                &current,
                collector,
            )?;
        }
        Ok(())
    }

    fn collect_subtree(
        &self,
        root: u32,
        distance: u8,
        root_is_exact_length: bool,
        collector: &mut CandidateCollector,
    ) -> Result<()> {
        let mut pending = vec![(root, root_is_exact_length)];
        while let Some((node_id, exact_length)) = pending.pop() {
            let node = self.node(node_id)?;
            if node.terminal != NO_TERM {
                collector.insert(node, distance, exact_length && distance == 0);
            }
            pending.extend(self.children(node)?.map(|child| (child, false)));
        }
        Ok(())
    }

    fn node(&self, id: u32) -> Result<Node> {
        if id as usize >= self.nodes {
            return Err(Error::Corrupt(
                "fuzzy trie node is out of bounds".to_string(),
            ));
        }
        let offset = HEADER_BYTES + id as usize * NODE_BYTES;
        Ok(Node {
            label: *self
                .file
                .bytes()
                .get(offset)
                .ok_or_else(|| Error::Corrupt("fuzzy node is truncated".to_string()))?,
            child_count: read_u16(self.file.bytes(), offset + 2)?,
            first_child: read_u32(self.file.bytes(), offset + 4)?,
            terminal: read_u32(self.file.bytes(), offset + 8)?,
            text_frequency: read_u32(self.file.bytes(), offset + 12)?,
            title_frequency: read_u32(self.file.bytes(), offset + 16)?,
        })
    }

    fn children(&self, node: Node) -> Result<std::ops::Range<u32>> {
        if node.child_count == 0 {
            return Ok(0..0);
        }
        if node.first_child == NO_NODE {
            return Err(Error::Corrupt(
                "fuzzy node has children without an offset".to_string(),
            ));
        }
        let end = node
            .first_child
            .checked_add(u32::from(node.child_count))
            .ok_or_else(|| Error::Corrupt("fuzzy child range overflows".to_string()))?;
        if end as usize > self.nodes {
            return Err(Error::Corrupt(
                "fuzzy child range extends beyond the index".to_string(),
            ));
        }
        Ok(node.first_child..end)
    }
}

struct RankedCandidate {
    candidate: FuzzyCandidate,
    frequency: u64,
}

struct CandidateCollector {
    by_distance: [Vec<RankedCandidate>; 3],
    maximum_distance: u8,
    maximum_per_distance: usize,
    search_text: bool,
    search_title: bool,
}

impl CandidateCollector {
    fn new(
        maximum_distance: u8,
        maximum_per_distance: usize,
        search_text: bool,
        search_title: bool,
    ) -> Self {
        Self {
            by_distance: std::array::from_fn(|_| Vec::with_capacity(maximum_per_distance)),
            maximum_distance,
            maximum_per_distance,
            search_text,
            search_title,
        }
    }

    fn insert(&mut self, node: Node, distance: u8, exact: bool) {
        if node.terminal == NO_TERM || distance > self.maximum_distance {
            return;
        }
        let frequency = u64::from(node.text_frequency) * u64::from(self.search_text)
            + u64::from(node.title_frequency)
                * u64::from(self.search_title)
                * TITLE_FREQUENCY_WEIGHT;
        let value = RankedCandidate {
            candidate: FuzzyCandidate {
                term: node.terminal,
                distance,
                exact,
            },
            frequency,
        };
        let values = &mut self.by_distance[distance as usize];
        if values.len() < self.maximum_per_distance {
            values.push(value);
            return;
        }
        let Some((worst_index, worst)) = values
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| compare_ranked(left, right))
        else {
            return;
        };
        if compare_ranked(&value, worst).is_gt() {
            values[worst_index] = value;
        }
    }

    fn finish(mut self) -> Vec<FuzzyCandidate> {
        let mut output = Vec::new();
        for distance in 0..=self.maximum_distance as usize {
            self.by_distance[distance].sort_unstable_by(|left, right| compare_ranked(right, left));
            output.extend(
                self.by_distance[distance]
                    .drain(..)
                    .map(|value| value.candidate),
            );
        }
        output
    }
}

fn compare_ranked(left: &RankedCandidate, right: &RankedCandidate) -> std::cmp::Ordering {
    left.candidate
        .exact
        .cmp(&right.candidate.exact)
        .then_with(|| left.frequency.cmp(&right.frequency))
        .then_with(|| right.candidate.term.cmp(&left.candidate.term))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::OpenOptions,
        io::{Seek, SeekFrom},
    };
    use tempfile::TempDir;

    fn fixture() -> (TempDir, FuzzyIndex) {
        let root = TempDir::new().expect("temp dir");
        write_index(
            root.path(),
            [
                (0, "black", 20, 0),
                (1, "blackhole", 8, 1),
                (2, "blackout", 30, 0),
                (3, "quantum", 50, 2),
                (4, "relative", 5, 0),
                (5, "relativity", 40, 3),
            ],
        )
        .expect("write fuzzy index");
        let index = FuzzyIndex::open(root.path(), 6).expect("open fuzzy index");
        (root, index)
    }

    #[test]
    fn prefix_and_damerau_candidates_are_frequency_ranked() {
        let (_root, index) = fixture();
        let prefix = index
            .candidates("black", 0, 4, true, true, true)
            .expect("prefix candidates");
        assert_eq!(
            prefix.iter().map(|value| value.term).collect::<Vec<_>>(),
            vec![0, 2, 1]
        );
        assert!(prefix[0].exact);

        let transposed = index
            .candidates("blackhloe", 1, 4, false, true, true)
            .expect("transposed candidate");
        assert!(
            transposed
                .iter()
                .any(|value| value.term == 1 && value.distance == 1)
        );
    }

    #[test]
    fn two_edit_candidates_respect_the_distance_gate() {
        let (_root, index) = fixture();
        assert!(
            index
                .candidates("relativtyy", 1, 4, false, true, true)
                .expect("one edit")
                .is_empty()
        );
        assert!(
            index
                .candidates("relativtyy", 2, 4, false, true, true)
                .expect("two edits")
                .iter()
                .any(|value| value.term == 5 && value.distance == 2)
        );
    }

    #[test]
    fn cyclic_or_overlapping_child_ranges_are_rejected() {
        let (root, index) = fixture();
        drop(index);
        let mut file = OpenOptions::new()
            .write(true)
            .open(root.path().join(FUZZY_FILE))
            .expect("fuzzy file");
        file.seek(SeekFrom::Start((HEADER_BYTES + 4) as u64))
            .expect("seek root child");
        file.write_all(&0_u32.to_le_bytes())
            .expect("write cyclic child");
        assert!(matches!(
            FuzzyIndex::open(root.path(), 6),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn trie_matches_a_reference_damerau_distance_on_small_words() {
        let root = TempDir::new().expect("temp dir");
        let mut words = Vec::new();
        for length in 1..=3 {
            generate_words(&mut words, String::new(), length);
        }
        words.sort_unstable();
        write_index(
            root.path(),
            words
                .iter()
                .enumerate()
                .map(|(index, word)| (index as u32, word.as_str(), 1, 0)),
        )
        .expect("write fuzzy index");
        let index = FuzzyIndex::open(root.path(), words.len()).expect("open fuzzy index");

        for query in &words {
            let actual = index
                .candidates(query, 2, words.len(), false, true, false)
                .expect("candidates")
                .into_iter()
                .map(|candidate| (candidate.term as usize, candidate.distance))
                .collect::<std::collections::HashMap<_, _>>();
            for (term, word) in words.iter().enumerate() {
                let distance = reference_osa_distance(query.as_bytes(), word.as_bytes());
                assert_eq!(
                    actual.get(&term).copied(),
                    (distance <= 2).then_some(distance),
                    "query={query} term={word}"
                );
            }
        }

        let prefix = index
            .candidates("ab", 0, words.len(), true, true, false)
            .expect("prefix candidates")
            .into_iter()
            .map(|candidate| candidate.term as usize)
            .collect::<std::collections::HashSet<_>>();
        let expected = words
            .iter()
            .enumerate()
            .filter_map(|(index, word)| word.starts_with("ab").then_some(index))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(prefix, expected);
    }

    fn generate_words(output: &mut Vec<String>, prefix: String, remaining: usize) {
        if remaining == 0 {
            output.push(prefix);
            return;
        }
        for character in ['a', 'b', 'c'] {
            let mut next = prefix.clone();
            next.push(character);
            generate_words(output, next, remaining - 1);
        }
    }

    fn reference_osa_distance(left: &[u8], right: &[u8]) -> u8 {
        let mut rows = vec![vec![0_u8; right.len() + 1]; left.len() + 1];
        for (index, row) in rows.iter_mut().enumerate() {
            row[0] = index as u8;
        }
        for (column, value) in rows[0].iter_mut().enumerate() {
            *value = column as u8;
        }
        for row in 1..=left.len() {
            for column in 1..=right.len() {
                let substitution = rows[row - 1][column - 1]
                    .saturating_add(u8::from(left[row - 1] != right[column - 1]));
                rows[row][column] = rows[row - 1][column]
                    .saturating_add(1)
                    .min(rows[row][column - 1].saturating_add(1))
                    .min(substitution);
                if row > 1
                    && column > 1
                    && left[row - 1] == right[column - 2]
                    && left[row - 2] == right[column - 1]
                {
                    rows[row][column] =
                        rows[row][column].min(rows[row - 2][column - 2].saturating_add(1));
                }
            }
        }
        rows[left.len()][right.len()]
    }
}
