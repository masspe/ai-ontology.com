// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use ontology_graph::{Ontology, Subgraph};
use ontology_index::ScoredConcept;
use std::fmt::Write;

/// Turn a `PascalCase` / `snake_case` relation type into a lowercase verb
/// phrase suitable for prose ("WorksFor" → "works for"). Used as a fallback
/// when a `RelationType.description` is empty.
fn humanize_relation(name: &str) -> String {
    let spaced = name.replace('_', " ");
    let mut out = String::with_capacity(spaced.len() + 4);
    let mut prev_lower = false;
    for ch in spaced.chars() {
        if ch.is_ascii_uppercase() && prev_lower {
            out.push(' ');
        }
        out.extend(ch.to_lowercase());
        prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    out
}

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
        Self {
            ontology,
            max_context_chars: 6000,
        }
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
                out,
                "- {} :: {}",
                ct.name,
                if ct.description.is_empty() {
                    "(no description)"
                } else {
                    &ct.description
                },
            );
        }
        let mut relation_types: Vec<_> = self.ontology.relation_types.values().collect();
        relation_types.sort_by(|a, b| a.name.cmp(&b.name));
        for rt in relation_types {
            let mut tags = String::new();
            if rt.symmetric {
                tags.push_str(" [symmetric]");
            }
            if rt.transitive {
                tags.push_str(" [transitive]");
            }
            if let Some(inv) = &rt.inverse_of {
                let _ = write!(tags, " [inverse: {}]", inv);
            }
            let _ = writeln!(
                out,
                "- ({}) -[{}]-> ({}){}",
                rt.domain, rt.name, rt.range, tags,
            );
        }
        if !self.ontology.rule_types.is_empty() {
            out.push_str("# Rules\n");
            let mut rules: Vec<_> = self.ontology.rule_types.values().collect();
            rules.sort_by(|a, b| a.name.cmp(&b.name));
            for r in rules {
                let scope = if r.applies_to.is_empty() {
                    "*".to_string()
                } else {
                    r.applies_to.join(",")
                };
                let kind = if r.strict { "MUST" } else { "SHOULD" };
                let when = if r.when.is_empty() { "-" } else { r.when.as_str() };
                let then = if r.then.is_empty() { "-" } else { r.then.as_str() };
                let _ = writeln!(out, "- [{}] {} ({}): when {} then {}", kind, r.name, scope, when, then);
            }
        }
        if !self.ontology.action_types.is_empty() {
            out.push_str("# Actions\n");
            let mut actions: Vec<_> = self.ontology.action_types.values().collect();
            actions.sort_by(|a, b| a.name.cmp(&b.name));
            for a in actions {
                let obj = a.object.as_deref().unwrap_or("-");
                let params = if a.parameters.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", a.parameters.join(", "))
                };
                let effect = if a.effect.is_empty() {
                    String::new()
                } else {
                    format!(" => {}", a.effect)
                };
                let _ = writeln!(out, "- {}: ({}) -> ({}){}{}", a.name, a.subject, obj, params, effect);
            }
        }
        out
    }

    /// Volatile per-query context: ranked seeds, retrieved subgraph, edges.
    pub fn render_query_context(&self, scored: &[ScoredConcept], subgraph: &Subgraph) -> String {
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
                if out.len() >= self.max_context_chars {
                    break;
                }
            }
        }
        out.push_str("\n# Subgraph\n");
        out.push_str("# Each line: `#<id> [depth] (Type) Name — description`. \
                      Cite the `#<id>` tokens verbatim in your answer.\n");
        for c in &subgraph.concepts {
            let depth = subgraph.depth_of.get(&c.id).copied().unwrap_or(0);
            let desc = if c.description.is_empty() {
                ""
            } else {
                &c.description
            };
            let _ = writeln!(
                out,
                "- #{} [{}] ({}) {}{}",
                c.id.0,
                depth,
                c.concept_type,
                c.name,
                if desc.is_empty() {
                    String::new()
                } else {
                    format!(" — {desc}")
                },
            );
            if out.len() >= self.max_context_chars {
                break;
            }
        }
        out.push_str("\n# Facts\n");
        for r in &subgraph.relations {
            let s = subgraph.concepts.iter().find(|c| c.id == r.source);
            let t = subgraph.concepts.iter().find(|c| c.id == r.target);
            if let (Some(s), Some(t)) = (s, t) {
                let verb = self
                    .ontology
                    .relation_types
                    .get(&r.relation_type)
                    .map(|rt| rt.description.as_str())
                    .filter(|d| !d.is_empty())
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| humanize_relation(&r.relation_type));
                let _ = writeln!(
                    out,
                    "- #{} {} — {} {}.   (raw: {} -[{}]-> {})",
                    s.id.0, s.name, verb, t.name, s.name, r.relation_type, t.name
                );
            }
            if out.len() >= self.max_context_chars {
                break;
            }
        }
        if out.len() > self.max_context_chars {
            let mut cut = self.max_context_chars;
            while cut > 0 && !out.is_char_boundary(cut) {
                cut -= 1;
            }
            out.truncate(cut);
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
        "You are a question-answering assistant grounded in an ontology graph. \
         Use ONLY the supplied ontology, top concepts, subgraph and facts. \
         If the supplied context does not support an answer, reply exactly: \
         `I don't know based on the supplied context.`\n\
         \n\
         Respond in this exact two-line format:\n\
         Cited: [#<id>, #<id>, …]\n\
         Answer: <one or more sentences citing concept names verbatim>\n\
         \n\
         Rules:\n\
         - Every `#<id>` in `Cited:` MUST appear in the supplied Subgraph. \
           Do NOT invent ids.\n\
         - Cite at least one id when answering; cite zero ids only when \
           replying `I don't know …`.\n\
         - Prefer facts at lower depth ([0], [1]) over deeper ones.\n\
         - Do not restate the ontology schema; use it only to disambiguate."
    }

    /// System prompt that instructs the LLM to emit a strict JSON
    /// [`ontology_graph::Ontology`] document from a natural-language brief.
    ///
    /// The exact schema mirrors `crates/graph/src/schema.rs` so the response
    /// can be deserialized with `serde_json::from_str::<Ontology>(...)`.
    pub fn ontology_generation_system_message() -> &'static str {
        "You are an ontology engineer. Given a natural-language brief, you \
         must emit a single JSON object describing the ontology. \
         Output rules:\n\
         1. Output ONLY the JSON object — no prose, no markdown fences, no \
            commentary before or after.\n\
         2. The JSON must match this schema exactly:\n\
         {\n  \
           \"concept_types\":   { \"<Name>\": { \"name\": \"<Name>\", \"parent\": <string|null>, \"description\": \"…\", \"properties\": <string[]|null> } },\n  \
           \"relation_types\":  { \"<Name>\": { \"name\": \"<Name>\", \"domain\": \"<ConceptType>\", \"range\": \"<ConceptType>\", \"cardinality\": \"OneToOne|OneToMany|ManyToOne|ManyToMany\", \"symmetric\": <bool>, \"description\": \"…\" } },\n  \
           \"rule_types\":      { \"<Name>\": { \"name\": \"<Name>\", \"when\": \"…\", \"then\": \"…\", \"applies_to\": [\"ConceptType\", …], \"strict\": <bool>, \"description\": \"…\" } },\n  \
           \"action_types\":    { \"<Name>\": { \"name\": \"<Name>\", \"subject\": \"<ConceptType>\", \"object\": <string|null>, \"parameters\": [\"…\"], \"effect\": \"…\", \"description\": \"…\" } }\n\
         }\n\
         3. Every `domain` and `range` referenced in `relation_types` MUST \
            appear as a key in `concept_types`.\n\
         4. `concept_types`, `relation_types`, `rule_types`, `action_types` \
            keys must be PascalCase identifiers (no spaces, no punctuation).\n\
         5. Keep concept names singular (Person, not Persons).\n\
         6. If the brief is vague, still produce a coherent ontology with \
            at least 3 concept types and 2 relation types.\n\
         7. Empty maps are fine for `rule_types` / `action_types` if not \
            implied by the brief — use `{}`, not `null`.\n"
    }

    /// Render the user message for an ontology-generation call. Keeps the
    /// prompt deterministic so callers can cache or replay it.
    pub fn ontology_generation_user_message(description: &str) -> String {
        format!(
            "Brief:\n{}\n\nReturn the JSON ontology document now.",
            description.trim()
        )
    }

    /// System prompt that instructs the LLM to emit a strict JSON object
    /// describing a single [`ontology_graph::Rule`]. The caller is expected
    /// to provide `id`, `rule_type`, `applies_to` and `properties` itself —
    /// the model only fills in the human-authored fields.
    pub fn rule_generation_system_message() -> &'static str {
        "You are an ontology rule author. Given a natural-language prompt, \
         the rule type and the list of concept names this rule will scope \
         to, emit a single JSON object describing the rule. \
         Output rules:\n\
         1. Output ONLY the JSON object — no prose, no markdown fences, no \
            commentary before or after.\n\
         2. The JSON must match this schema exactly:\n\
         {\n  \
           \"name\": \"<short PascalCase or Title Case identifier, unique per rule_type>\",\n  \
           \"when\": \"<antecedent / condition expression>\",\n  \
           \"then\": \"<consequent / conclusion expression>\",\n  \
           \"description\": \"<one-sentence human-readable summary>\",\n  \
           \"strict\": <bool>\n\
         }\n\
         3. Do NOT include `id`, `rule_type`, `applies_to` or `properties` \
            — the caller supplies those.\n\
         4. `when` and `then` should reference the supplied concept names \
            verbatim when relevant.\n\
         5. `strict` is `true` for hard constraints / validations and \
            `false` for soft inferences or suggestions.\n"
    }

    /// Render the user message for a rule-generation call. Keeps the
    /// prompt deterministic so callers can cache or replay it.
    pub fn rule_generation_user_message(
        prompt: &str,
        rule_type: &str,
        applies_to_concept_names: &[String],
    ) -> String {
        let concepts = if applies_to_concept_names.is_empty() {
            "(none)".to_string()
        } else {
            applies_to_concept_names.join(", ")
        };
        format!(
            "Rule type: {}\nApplies to concepts: {}\n\nPrompt:\n{}\n\nReturn the JSON rule object now.",
            rule_type.trim(),
            concepts,
            prompt.trim()
        )
    }
}
