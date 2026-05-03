use ontology_graph::{Ontology, Subgraph};
use ontology_index::ScoredConcept;
use std::fmt::Write;

/// Renders retrieved context into prompt-ready text.
///
/// Output is split into two halves so the caller can route them to different
/// places in the request:
///
/// * [`render_static_context`] — the ontology schema. Stable per-knowledge-base,
///   so it ships as a separately-cached `system` block to the LLM.
/// * [`render_query_context`] — ranked concepts + retrieved subgraph + edges.
///   Volatile per-query; ships in the user message and is never cached.
///
/// [`render`] returns the two concatenated for callers that don't care about
/// caching (e.g. the `EchoModel` test fake).
pub struct PromptBuilder<'a> {
    ontology: &'a Ontology,
    /// Soft cap on the *combined* context length, in characters. The query
    /// half stops adding rows once exceeded; the static half is always
    /// rendered in full so the cache key stays stable.
    pub max_context_chars: usize,
}

impl<'a> PromptBuilder<'a> {
    pub fn new(ontology: &'a Ontology) -> Self {
        Self { ontology, max_context_chars: 6000 }
    }

    pub fn with_max_chars(mut self, n: usize) -> Self {
        self.max_context_chars = n;
        self
    }

    /// Stable per-knowledge-base context: just the ontology schema. Suitable
    /// for placement behind a `cache_control: ephemeral` breakpoint.
    pub fn render_static_context(&self) -> String {
        let mut out = String::new();
        out.push_str("# Ontology\n");
        // Sort by name so byte-for-byte output is stable across runs — required
        // for the prompt cache to actually hit.
        let mut concept_types: Vec<_> = self.ontology.concept_types.values().collect();
        concept_types.sort_by(|a, b| a.name.cmp(&b.name));
        for ct in concept_types {
            let _ = writeln!(
                out, "- {} :: {}", ct.name,
                if ct.description.is_empty() { "(no description)" } else { &ct.description },
            );
        }
        let mut relation_types: Vec<_> = self.ontology.relation_types.values().collect();
        relation_types.sort_by(|a, b| a.name.cmp(&b.name));
        for rt in relation_types {
            let _ = writeln!(
                out, "- ({}) -[{}]-> ({}){}",
                rt.domain, rt.name, rt.range,
                if rt.symmetric { " [symmetric]" } else { "" },
            );
        }
        out
    }

    /// Volatile per-query context: ranked seeds, retrieved subgraph, edges.
    pub fn render_query_context(
        &self,
        scored: &[ScoredConcept],
        subgraph: &Subgraph,
    ) -> String {
        let mut out = String::new();
        if !scored.is_empty() {
            out.push_str("# Top concepts\n");
            for s in scored {
                if let Some(c) = subgraph.concepts.iter().find(|c| c.id == s.id) {
                    let _ = writeln!(
                        out,
                        "- ({}) {} [score={:.3} lex={:.3} vec={:.3}]",
                        c.concept_type, c.name, s.score, s.lexical, s.vector
                    );
                }
                if out.len() >= self.max_context_chars { break; }
            }
        }
        out.push_str("\n# Subgraph\n");
        for c in &subgraph.concepts {
            let depth = subgraph.depth_of.get(&c.id).copied().unwrap_or(0);
            let desc = if c.description.is_empty() { "" } else { &c.description };
            let _ = writeln!(
                out, "- [{}] ({}) {}{}", depth, c.concept_type, c.name,
                if desc.is_empty() { String::new() } else { format!(" — {desc}") },
            );
            if out.len() >= self.max_context_chars { break; }
        }
        out.push_str("\n# Edges\n");
        for r in &subgraph.relations {
            let s = subgraph.concepts.iter().find(|c| c.id == r.source);
            let t = subgraph.concepts.iter().find(|c| c.id == r.target);
            if let (Some(s), Some(t)) = (s, t) {
                let _ = writeln!(out, "- {} -[{}]-> {}", s.name, r.relation_type, t.name);
            }
            if out.len() >= self.max_context_chars { break; }
        }
        if out.len() > self.max_context_chars {
            out.truncate(self.max_context_chars);
            out.push_str("\n…[truncated]\n");
        }
        out
    }

    /// Convenience: static + query, joined with a blank line.
    pub fn render(&self, scored: &[ScoredConcept], subgraph: &Subgraph) -> String {
        let mut out = self.render_static_context();
        out.push('\n');
        out.push_str(&self.render_query_context(scored, subgraph));
        out
    }

    pub fn system_message() -> &'static str {
        "You are a question-answering assistant. Use ONLY the supplied \
         ontology, concept list and edge list to answer. If the answer is not \
         supported by the supplied context, say you don't know. Cite concept \
         names verbatim."
    }
}
