//! In-memory cache of every tag's searchable text, so repeat field-value
//! searches are instant (no per-query tag reads). Built once per session in the
//! background and invalidated whenever the source changes (generation bump).
//! Not persisted to disk yet — that's a safe follow-up.

#[derive(Default)]
pub(super) struct FieldValueIndex {
    generation: u64,
    ready: bool,
    building: bool,
    /// (entry key, lowercased searchable blob) for every non-empty tag.
    blobs: Vec<(String, String)>,
}

impl FieldValueIndex {
    pub(super) fn is_ready_for(&self, generation: u64) -> bool {
        self.ready && self.generation == generation
    }

    pub(super) fn is_building(&self) -> bool {
        self.building
    }

    pub(super) fn mark_building(&mut self) {
        self.building = true;
    }

    /// Install freshly-built blobs from the worker.
    pub(super) fn install(&mut self, generation: u64, blobs: Vec<(String, String)>) {
        self.generation = generation;
        self.blobs = blobs;
        self.ready = true;
        self.building = false;
    }

    /// Drop the index (called on source reload).
    pub(super) fn invalidate(&mut self) {
        self.ready = false;
        self.building = false;
        self.blobs.clear();
    }

    /// Substring query over the cached blobs → (entry key, snippet) pairs, up to
    /// `cap`. `query_lower` must already be lowercased.
    pub(super) fn query(&self, query_lower: &str, cap: usize) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (key, blob) in &self.blobs {
            if let Some(byte_pos) = blob.find(query_lower) {
                out.push((
                    key.clone(),
                    index_snippet(blob, byte_pos, query_lower.len()),
                ));
                if out.len() >= cap {
                    break;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_query_and_generation_invalidation() {
        let mut index = FieldValueIndex::default();
        assert!(!index.is_ready_for(1));
        index.install(
            1,
            vec![
                ("a".to_owned(), "weapons · objects\\rifle".to_owned()),
                ("b".to_owned(), "bipeds · masterchief".to_owned()),
            ],
        );
        assert!(index.is_ready_for(1));
        // Wrong generation → treated as not ready.
        assert!(!index.is_ready_for(2));

        let hits = index.query("rifle", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "a");
        assert!(hits[0].1.contains("rifle"));

        assert_eq!(index.query("chief", 10).len(), 1);
        assert!(index.query("nonexistent", 10).is_empty());

        index.invalidate();
        assert!(!index.is_ready_for(1));
        assert!(index.query("rifle", 10).is_empty());
    }

    #[test]
    fn query_respects_cap() {
        let mut index = FieldValueIndex::default();
        index.install(
            1,
            (0..50)
                .map(|i| (format!("k{i}"), "shared token".to_owned()))
                .collect(),
        );
        assert_eq!(index.query("token", 10).len(), 10);
    }
}

/// A short context window around a match for display as a result annotation.
fn index_snippet(blob: &str, byte_pos: usize, query_byte_len: usize) -> String {
    const PAD: usize = 24;
    let chars: Vec<char> = blob.chars().collect();
    let char_pos = blob[..byte_pos].chars().count();
    let match_chars = blob[byte_pos..(byte_pos + query_byte_len).min(blob.len())]
        .chars()
        .count();
    let start = char_pos.saturating_sub(PAD);
    let end = (char_pos + match_chars + PAD).min(chars.len());
    let mut snippet: String = chars[start..end].iter().collect();
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < chars.len() {
        snippet.push('…');
    }
    snippet
}
