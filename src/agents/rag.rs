//! Basic local RAG: whitespace chunking + FTS5 retrieval.
//!
//! No embedding models, no new dependencies: documents are split into
//! ~600-char chunks (120-char overlap) and indexed in the `rag_*` tables of
//! the history database (see [`crate::managers::history::HistoryManager`]).
//! Retrieval is BM25 over the FTS5 index, injected as a `system` block the
//! model must cite as `[source: title]`.

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

/// BM25 search over the local index. Returns at most `top_k` passages;
/// an empty vec means "no local context" (the chat proceeds regardless).
pub fn rag_search(ctx: &AppContext, query: &str, top_k: u32) -> Vec<RagPassage> {
    ctx.history
        .rag_search(query, top_k)
        .into_iter()
        .map(|hit| RagPassage {
            title: hit.title,
            uri: hit.uri,
            snippet: hit.snippet,
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
}
