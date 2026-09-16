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
                let when = if r.when.is_empty() {
                    "-"
                } else {
                    r.when.as_str()
                };
                let then = if r.then.is_empty() {
                    "-"
                } else {
                    r.then.as_str()
                };
                let _ = writeln!(
                    out,
                    "- [{}] {} ({}): when {} then {}",
                    kind, r.name, scope, when, then
                );
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
                let _ = writeln!(
                    out,
                    "- {}: ({}) -> ({}){}{}",
                    a.name, a.subject, obj, params, effect
                );
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
        out.push_str(
            "# Each line: `#<id> [depth] (Type) Name — description`. \
                      Cite the `#<id>` tokens verbatim in your answer.\n",
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use ontology_graph::{
        ActionType, Concept, ConceptId, ConceptType, Ontology, Relation, RelationType, RuleType,
    };

    fn ct(name: &str, description: &str) -> ConceptType {
        ConceptType {
            name: name.into(),
            description: description.into(),
            ..Default::default()
        }
    }

    fn rt(name: &str, domain: &str, range: &str) -> RelationType {
        RelationType {
            name: name.into(),
            domain: domain.into(),
            range: range.into(),
            ..Default::default()
        }
    }

    fn rich_ontology(order: &[&str]) -> Ontology {
        let mut o = Ontology::new();
        for name in order {
            match *name {
                "Company" => o.add_concept_type(ct("Company", "a legal entity")),
                "Person" => o.add_concept_type(ct("Person", "")),
                "Contract" => o.add_concept_type(ct("Contract", "an agreement")),
                other => panic!("unknown fixture {other}"),
            }
        }
        o.add_relation_type(RelationType {
            transitive: true,
            ..rt("parent_company", "Company", "Company")
        })
        .unwrap();
        o.add_relation_type(RelationType {
            symmetric: true,
            description: "works alongside".into(),
            ..rt("WorksWith", "Person", "Person")
        })
        .unwrap();
        o.add_relation_type(RelationType {
            inverse_of: Some("signed".into()),
            ..rt("signed_by", "Contract", "Person")
        })
        .unwrap();
        o.add_relation_type(RelationType {
            inverse_of: Some("signed_by".into()),
            ..rt("signed", "Person", "Contract")
        })
        .unwrap();
        o.add_rule_type(RuleType {
            name: "must_have_party".into(),
            when: "a Contract exists".into(),
            then: "it names a Company".into(),
            applies_to: vec!["Contract".into()],
            strict: true,
            description: String::new(),
        })
        .unwrap();
        o.add_rule_type(RuleType {
            name: "advisory".into(),
            when: String::new(),
            then: String::new(),
            applies_to: vec![],
            strict: false,
            description: String::new(),
        })
        .unwrap();
        o.add_action_type(ActionType {
            name: "sign".into(),
            subject: "Person".into(),
            object: Some("Contract".into()),
            parameters: vec!["date".into(), "place".into()],
            effect: "adds signed_by".into(),
            description: String::new(),
        })
        .unwrap();
        o.add_action_type(ActionType {
            name: "audit".into(),
            subject: "Company".into(),
            object: None,
            parameters: vec![],
            effect: String::new(),
            description: String::new(),
        })
        .unwrap();
        o
    }

    /// The cached system block must be byte-identical for the same schema
    /// whatever the insertion order of the underlying hash maps.
    #[test]
    fn static_context_is_byte_stable_across_insertion_orders() {
        let a = rich_ontology(&["Company", "Person", "Contract"]);
        let b = rich_ontology(&["Contract", "Company", "Person"]);
        let ra = PromptBuilder::new(&a).render_static_context();
        let rb = PromptBuilder::new(&b).render_static_context();
        assert_eq!(ra, rb);
        // Rendered twice from the same ontology: identical too.
        assert_eq!(ra, PromptBuilder::new(&a).render_static_context());
        // And with_max_chars never affects the static half.
        assert_eq!(
            ra,
            PromptBuilder::new(&a)
                .with_max_chars(10)
                .render_static_context()
        );
    }

    /// Every schema family is rendered, sorted by name, with the documented
    /// markers: `(no description)`, relation tags, MUST/SHOULD rules with
    /// `*` scope and `-` placeholders, actions with params and effect.
    #[test]
    fn static_context_renders_every_schema_family_sorted() {
        let o = rich_ontology(&["Person", "Contract", "Company"]);
        let out = PromptBuilder::new(&o).render_static_context();
        let expected = "\
# Ontology
- Company :: a legal entity
- Contract :: an agreement
- Person :: (no description)
- (Person) -[WorksWith]-> (Person) [symmetric]
- (Company) -[parent_company]-> (Company) [transitive]
- (Person) -[signed]-> (Contract) [inverse: signed_by]
- (Contract) -[signed_by]-> (Person) [inverse: signed]
# Rules
- [SHOULD] advisory (*): when - then -
- [MUST] must_have_party (Contract): when a Contract exists then it names a Company
# Actions
- audit: (Company) -> (-)
- sign: (Person) -> (Contract) [date, place] => adds signed_by
";
        assert_eq!(out, expected);
    }

    /// Without rules or actions those sections are absent entirely, so an
    /// empty schema renders only the header.
    #[test]
    fn static_context_omits_empty_rule_and_action_sections() {
        let o = Ontology::new();
        assert_eq!(
            PromptBuilder::new(&o).render_static_context(),
            "# Ontology\n"
        );
    }

    /// Relation identifiers become lowercase verb phrases for the prose
    /// rendering of facts.
    #[test]
    fn humanize_relation_turns_identifiers_into_verb_phrases() {
        assert_eq!(humanize_relation("WorksFor"), "works for");
        assert_eq!(humanize_relation("employed_by"), "employed by");
        assert_eq!(humanize_relation("issuedTo"), "issued to");
        assert_eq!(humanize_relation("related_to"), "related to");
        assert_eq!(humanize_relation("HTTPServer"), "httpserver");
        assert_eq!(humanize_relation("has2Parts"), "has2 parts");
        assert_eq!(humanize_relation(""), "");
    }

    fn subgraph_fixture() -> (Ontology, Vec<ScoredConcept>, Subgraph) {
        let o = rich_ontology(&["Company", "Person", "Contract"]);
        let alice = Concept::new(ConceptId(7), "Person", "Alice");
        let bob = Concept::new(ConceptId(9), "Person", "Bob").with_description("signs things");
        let c1 = Concept::new(ConceptId(11), "Contract", "C-1");
        let mut subgraph = Subgraph {
            seeds: vec![ConceptId(7)],
            concepts: vec![alice, bob, c1],
            relations: vec![
                Relation::new(Default::default(), "WorksWith", ConceptId(7), ConceptId(9)),
                Relation::new(Default::default(), "signed", ConceptId(9), ConceptId(11)),
                // A dangling edge (endpoint not in the subgraph) is skipped.
                Relation::new(Default::default(), "signed", ConceptId(9), ConceptId(404)),
            ],
            depth_of: Default::default(),
        };
        subgraph.depth_of.insert(ConceptId(7), 0);
        subgraph.depth_of.insert(ConceptId(9), 1);
        subgraph.depth_of.insert(ConceptId(11), 2);
        let scored = vec![ScoredConcept {
            id: ConceptId(7),
            score: 0.75,
            lexical: 0.5,
            vector: 1.0,
        }];
        (o, scored, subgraph)
    }

    /// Facts use the relation description when present, else the humanized
    /// name; each subgraph line carries the `#<id>` token and depth.
    #[test]
    fn query_context_renders_ids_depths_and_verbs() {
        let (o, scored, subgraph) = subgraph_fixture();
        let out = PromptBuilder::new(&o).render_query_context(&scored, &subgraph);
        assert!(
            out.contains("# Top concepts\n- (Person) Alice [score=0.750 lex=0.500 vec=1.000]\n")
        );
        assert!(out.contains("- #7 [0] (Person) Alice\n"), "{out}");
        assert!(
            out.contains("- #9 [1] (Person) Bob — signs things\n"),
            "{out}"
        );
        assert!(out.contains("- #11 [2] (Contract) C-1\n"), "{out}");
        assert!(
            out.contains("- #7 Alice — works alongside Bob.   (raw: Alice -[WorksWith]-> Bob)\n"),
            "description used as verb: {out}"
        );
        assert!(
            out.contains("- #9 Bob — signed C-1.   (raw: Bob -[signed]-> C-1)\n"),
            "humanized fallback: {out}"
        );
        assert_eq!(out.matches("(raw:").count(), 2, "dangling edge skipped");
        assert!(!out.contains("[truncated]"));
    }

    /// With no seeds the `# Top concepts` header is omitted but the
    /// subgraph and facts sections are always present.
    #[test]
    fn query_context_without_seeds_has_no_top_concepts_section() {
        let (o, _, subgraph) = subgraph_fixture();
        let out = PromptBuilder::new(&o).render_query_context(&[], &subgraph);
        assert!(!out.contains("# Top concepts"));
        assert!(out.starts_with("\n# Subgraph\n"));
        assert!(out.contains("\n# Facts\n"));
    }

    /// Over the budget, the query half is cut on a char boundary (never
    /// inside a multi-byte name) and ends with the truncation marker.
    #[test]
    fn query_context_truncates_on_a_char_boundary_with_a_marker() {
        let o = rich_ontology(&["Company", "Person", "Contract"]);
        let concepts: Vec<Concept> = (0..40)
            .map(|i| {
                Concept::new(
                    ConceptId(i),
                    "Company",
                    "Société Générale 漢字 😀".repeat(3),
                )
            })
            .collect();
        let subgraph = Subgraph {
            seeds: vec![],
            concepts,
            relations: vec![],
            depth_of: Default::default(),
        };
        for max in 150..=260 {
            let out = PromptBuilder::new(&o)
                .with_max_chars(max)
                .render_query_context(&[], &subgraph);
            assert!(out.ends_with("\n…[truncated]\n"), "max={max}: {out:?}");
            let body = out.trim_end_matches("\n…[truncated]\n");
            assert!(body.len() <= max, "max={max}: {} bytes kept", body.len());
        }
        // Under budget: untouched.
        let out = PromptBuilder::new(&o)
            .with_max_chars(100_000)
            .render_query_context(&[], &subgraph);
        assert!(!out.contains("[truncated]"));
        assert_eq!(out.matches("- #").count(), 40);
    }

    /// `render` is exactly the static half, a blank line, then the query half.
    #[test]
    fn render_concatenates_static_and_query_halves() {
        let (o, scored, subgraph) = subgraph_fixture();
        let b = PromptBuilder::new(&o);
        assert_eq!(
            b.render(&scored, &subgraph),
            format!(
                "{}\n{}",
                b.render_static_context(),
                b.render_query_context(&scored, &subgraph)
            )
        );
    }

    /// The JSON shape documented in the ontology-generation prompt is the
    /// one `Ontology` actually deserializes — including the `null`
    /// spellings the prompt allows and empty rule/action maps.
    #[test]
    fn ontology_generation_schema_example_deserializes_into_ontology() {
        let msg = PromptBuilder::ontology_generation_system_message();
        for key in [
            "concept_types",
            "relation_types",
            "rule_types",
            "action_types",
        ] {
            assert!(
                msg.contains(&format!("\"{key}\"")),
                "{key} missing from prompt"
            );
        }
        let sample = r#"{
            "concept_types": {
                "Person":  { "name": "Person",  "parent": null,    "description": "a human", "properties": null },
                "Manager": { "name": "Manager", "parent": "Person", "description": "leads",  "properties": ["team"] }
            },
            "relation_types": {
                "ReportsTo": { "name": "ReportsTo", "domain": "Person", "range": "Manager",
                               "cardinality": "ManyToOne", "symmetric": false, "description": "line management" }
            },
            "rule_types": {
                "OneManager": { "name": "OneManager", "when": "a Person exists", "then": "at most one ReportsTo",
                                "applies_to": ["Person"], "strict": true, "description": "…" }
            },
            "action_types": {
                "Promote": { "name": "Promote", "subject": "Manager", "object": null,
                             "parameters": ["date"], "effect": "makes a Manager", "description": "…" }
            }
        }"#;
        let o: Ontology = serde_json::from_str(sample).expect("documented shape deserializes");
        assert_eq!(o.concept_types["Manager"].parent.as_deref(), Some("Person"));
        assert!(o.concept_types["Person"].properties.is_none());
        assert_eq!(o.relation_types["ReportsTo"].range, "Manager");
        assert!(o.rule_types["OneManager"].strict);
        assert!(o.action_types["Promote"].object.is_none());
        // Rule 7 of the prompt: empty maps are valid.
        let minimal: Ontology = serde_json::from_str(
            r#"{"concept_types": {}, "relation_types": {}, "rule_types": {}, "action_types": {}}"#,
        )
        .unwrap();
        assert!(minimal.concept_types.is_empty());
        // The rule-generation shape deserializes into GeneratedRule too.
        let rule: crate::GeneratedRule = serde_json::from_str(
            r#"{"name": "R", "when": "w", "then": "t", "description": "d", "strict": false}"#,
        )
        .unwrap();
        assert_eq!(rule.name, "R");
    }

    /// User messages are deterministic: input is trimmed and an empty
    /// concept scope is spelled `(none)`.
    #[test]
    fn generation_user_messages_are_trimmed_and_name_empty_scopes() {
        assert_eq!(
            PromptBuilder::ontology_generation_user_message("  a brief \n"),
            "Brief:\na brief\n\nReturn the JSON ontology document now."
        );
        assert_eq!(
            PromptBuilder::rule_generation_user_message(" p ", " constraint ", &[]),
            "Rule type: constraint\nApplies to concepts: (none)\n\nPrompt:\np\n\nReturn the JSON rule object now."
        );
        assert_eq!(
            PromptBuilder::rule_generation_user_message("p", "t", &["A".into(), "B".into()]),
            "Rule type: t\nApplies to concepts: A, B\n\nPrompt:\np\n\nReturn the JSON rule object now."
        );
    }
}
