use ontology_graph::{Ontology, Subgraph};
use ontology_index::ScoredConcept;
use std::fmt::Write;

/// Renders retrieved context into a single text block suitable for inlining
/// into a chat-style prompt. The renderer is intentionally conservative —
/// deterministic ordering, bounded length, ontology-aware framing — so the
/// LLM sees the same structure run-to-run for the same inputs.
pub struct PromptBuilder<'a> {
    ontology: &'a Ontology,
    /// Soft cap on the context length, in characters. The builder stops
    /// adding rows once exceeded.
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

    pub fn render(&self, scored: &[ScoredConcept], subgraph: &Subgraph) -> String {
        let mut out = String::new();

        out.push_str("# Ontology\n");
        for ct in self.ontology.concept_types.values() {
            let _ = writeln!(
                out, "- {} :: {}", ct.name,
                if ct.description.is_empty() { "(no description)" } else { &ct.description },
            );
        }
        for rt in self.ontology.relation_types.values() {
            let _ = writeln!(
                out, "- ({}) -[{}]-> ({}){}",
                rt.domain, rt.name, rt.range,
                if rt.symmetric { " [symmetric]" } else { "" },
            );
        }

        if !scored.is_empty() {
            out.push_str("\n# Top concepts\n");
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

    pub fn system_message() -> &'static str {
        "You are a question-answering assistant. Use ONLY the supplied \
         ontology, concept list and edge list to answer. If the answer is not \
         supported by the supplied context, say you don't know. Cite concept \
         names verbatim."
    }
}
