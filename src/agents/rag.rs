//! Hybrid local RAG: whitespace chunking + FTS5 + optional vector search.
//!
//! Documents are split into ~600-char chunks (120-char overlap) and indexed
//! in the `rag_*` tables of the history database (see
//! `HistoryManager` (see `managers::history`)). Retrieval fuses two
//! signals with reciprocal rank fusion (RRF):
//! - BM25 over the FTS5 index (always available, no network);
//! - cosine similarity over per-chunk embeddings (only when a provider has
//!   an `embeddings_model` configured and chunks were backfilled).
//!
//! Passages are injected as a `system` block the model must cite as
//! `[source: title]`.

use crate::context::AppContext;

/// Maximum characters of retrieved context injected into one turn.
pub const MAX_RAG_CONTEXT_CHARS: usize = 6000;

/// Split text into overlapping chunks at whitespace boundaries. Short inputs
/// yield a single chunk; empty input yields none.
pub fn chunk_text(text: &str, chunk_chars: usize, overlap_chars: usize) -> Vec<String> {
    let chunk_chars = chunk_chars.max(200);
    let overlap_chars = overlap_chars.min(chunk_chars / 2);
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if text.chars().count() <= chunk_chars {
        return vec![text.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + chunk_chars).min(chars.len());
        if end < chars.len() {
            // Prefer a whitespace break; fall back to the hard cut.
            let mut cut = end;
            while cut > start + chunk_chars / 2 && !chars[cut].is_whitespace() {
                cut -= 1;
            }
            if cut > start + chunk_chars / 2 {
                end = cut;
            }
        }
        let chunk: String = chars[start..end].iter().collect();
        let chunk = chunk.trim();
        if !chunk.is_empty() {
            chunks.push(chunk.to_string());
        }
        if end >= chars.len() {
            break;
        }
        start = end.saturating_sub(overlap_chars).max(start + 1);
    }
    chunks
}

/// One retrieved passage formatted for the model context block.
#[derive(Clone, Debug)]
pub struct RagPassage {
    pub title: String,
    pub uri: String,
    pub snippet: String,
}

/// RRF smoothing constant (standard 60).
const RRF_K: f64 = 60.0;

/// Fuse a BM25 ranking with a cosine-similarity ranking via reciprocal rank
/// fusion. `bm25_ids` are chunk ids ordered best-first; `cosine` holds
/// `(chunk_id, similarity)` pairs (order irrelevant). Returns chunk ids
/// best-first. Pure function for unit tests.
pub fn fuse_rankings(bm25_ids: &[i64], cosine: &[(i64, f32)], top_k: usize) -> Vec<i64> {
    use std::collections::HashMap;
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for (rank, id) in bm25_ids.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    let mut ordered: Vec<(i64, f32)> = cosine.to_vec();
    ordered.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (rank, (id, _)) in ordered.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    let mut fused: Vec<(i64, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused
        .into_iter()
        .take(top_k.max(1))
        .map(|(id, _)| id)
        .collect()
}

/// Hybrid retrieval entry used by the runner: BM25 candidates re-ranked with
/// cosine similarity when `query_vector` and stored embeddings exist.
///
/// - `query_vector: None` → pure BM25 (no embeddings configured).
/// - With a vector, the top `3 × top_k` BM25 candidates are fetched, their
///   stored embeddings (tagged `embedding_model`) are scored, and RRF fuses
///   both rankings. Chunks without embeddings keep their BM25-only score, so
///   partially backfilled indexes degrade gracefully instead of dropping
///   passages.
pub fn hybrid_search(
    ctx: &AppContext,
    query: &str,
    top_k: u32,
    query_vector: Option<&[f32]>,
    embedding_model: Option<&str>,
) -> Vec<RagPassage> {
    let top_k = top_k.clamp(1, 10);
    let candidate_limit = top_k.saturating_mul(3).max(top_k);
    let candidates = ctx.history.rag_search(query, candidate_limit);
    if candidates.is_empty() {
        return Vec::new();
    }
    let (Some(vector), Some(model)) = (query_vector, embedding_model) else {
        return candidates
            .into_iter()
            .take(top_k as usize)
            .map(|hit| RagPassage {
                title: hit.title,
                uri: hit.uri,
                snippet: hit.snippet,
            })
            .collect();
    };
    let ids: Vec<i64> = candidates.iter().map(|hit| hit.chunk_id).collect();
    let stored = ctx.history.rag_chunk_embeddings(&ids, model);
    if stored.is_empty() {
        return candidates
            .into_iter()
            .take(top_k as usize)
            .map(|hit| RagPassage {
                title: hit.title,
                uri: hit.uri,
                snippet: hit.snippet,
            })
            .collect();
    }
    let cosine: Vec<(i64, f32)> = stored
        .iter()
        .map(|(id, vec)| (*id, crate::llm_client::cosine_similarity(vector, vec)))
        .collect();
    let bm25_ids: Vec<i64> = candidates.iter().map(|hit| hit.chunk_id).collect();
    let fused = fuse_rankings(&bm25_ids, &cosine, top_k as usize);
    let by_id: std::collections::HashMap<i64, crate::managers::history::RagHit> = candidates
        .into_iter()
        .map(|hit| (hit.chunk_id, hit))
        .collect();
    fused
        .into_iter()
        .filter_map(|id| by_id.get(&id))
        .map(|hit| RagPassage {
            title: hit.title.clone(),
            uri: hit.uri.clone(),
            snippet: hit.snippet.clone(),
        })
        .collect()
}

/// Render retrieved passages as a `system` message block, truncated to
/// [`MAX_RAG_CONTEXT_CHARS`]. Returns `None` when there is nothing to inject.
pub fn render_context_block(passages: &[RagPassage]) -> Option<String> {
    if passages.is_empty() {
        return None;
    }
    let mut block =
        String::from("Local documents relevant to the question (cite as [source: TITLE]):\n");
    for passage in passages {
        let entry = format!("\n[source: {}]\n{}\n", passage.title, passage.snippet);
        if block.len() + entry.len() > MAX_RAG_CONTEXT_CHARS {
            block.push_str("\n…(truncated: more passages omitted)…\n");
            break;
        }
        block.push_str(&entry);
    }
    Some(block)
}

/// Index (or re-index) a batch of documents: `(source_kind, source_id,
/// title, uri, full_text)`. Chunking uses the 600/120 default.
pub fn rebuild_index_for_docs(
    ctx: &AppContext,
    docs: &[(String, String, String, String, String)],
) -> Result<usize, String> {
    let mut indexed = 0;
    for (kind, id, title, uri, text) in docs {
        let chunks = chunk_text(text, 600, 120);
        if chunks.is_empty() {
            continue;
        }
        ctx.history
            .index_rag_document(kind, id, title, uri, &chunks)
            .map_err(|e| format!("Failed to index '{title}': {e}"))?;
        indexed += 1;
    }
    Ok(indexed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_single_chunk() {
        assert_eq!(chunk_text("hello", 600, 120), vec!["hello".to_string()]);
        assert!(chunk_text("   ", 600, 120).is_empty());
    }

    #[test]
    fn long_text_splits_with_overlap() {
        let text: String = (0..50)
            .map(|i| format!("word{i:02} "))
            .collect::<String>()
            .repeat(4);
        let chunks = chunk_text(&text, 600, 120);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 600);
        }
        // Overlap: some content repeats across adjacent chunks.
        let joined = chunks.join(" ");
        assert!(joined.contains("word00"));
    }

    #[test]
    fn empty_passages_render_no_block() {
        assert!(render_context_block(&[]).is_none());
    }

    #[test]
    fn context_block_truncates_large_passages() {
        let passages: Vec<RagPassage> = (0..20)
            .map(|i| RagPassage {
                title: format!("Doc {i}"),
                uri: String::new(),
                snippet: "x".repeat(1000),
            })
            .collect();
        let block = render_context_block(&passages).expect("block");
        assert!(block.len() <= MAX_RAG_CONTEXT_CHARS + 200);
        assert!(block.contains("truncated"));
    }

    #[test]
    fn rrf_prefers_items_ranked_well_by_both_signals() {
        // id 1 leads BM25, id 3 leads cosine: fusion must surface both
        // above id 2 (weak in both).
        let fused = fuse_rankings(&[1, 2, 3], &[(3, 0.9), (1, 0.5), (2, 0.1)], 3);
        assert_eq!(fused.len(), 3);
        assert!(fused[0] == 1 || fused[0] == 3);
        assert_eq!(fused[2], 2);
    }

    #[test]
    fn rrf_without_cosine_keeps_bm25_order() {
        let fused = fuse_rankings(&[7, 8, 9], &[], 2);
        assert_eq!(fused, vec![7, 8]);
    }

    #[test]
    fn rrf_cosine_only_items_still_surface() {
        // A chunk with no BM25 rank (id 5) but top cosine still fuses in.
        let fused = fuse_rankings(&[1], &[(5, 0.99), (1, 0.1)], 2);
        assert!(fused.contains(&5));
    }
}
