// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! LLM-assisted ingest review workflow.
//!
//! Two endpoints make up the loop:
//!
//! * `POST /ingest/analyze` — decode the uploaded bytes, detect language,
//!   call the configured LLM to extract concepts / relations / rules /
//!   actions, attach conflict information against the live graph, and
//!   return the full [`OntologyProposal`] as JSON.
//! * `POST /ingest/apply` — receive a (possibly edited) proposal plus a
//!   list of per-item [`ApplyDecision`]s, validate against the live
//!   schema, and write accepted items in topological order
//!   (concept types → relation types → concepts → relations → rules →
//!   actions). Returns a per-`client_ref` outcome report.
//!
//! The server stores nothing between the two calls — the wizard UI is
//! the proposal's source of truth.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Multipart, State},
    Json,
};
use ontology_graph::{
    Action, ActionId, Concept, ConceptId, OntologyGraph, Relation, RelationId, Rule,
    RuleId,
};
use ontology_io::{
    decode_to_utf8, detect_language, ApplyDecision, ApplyOutcome, ApplyReport, DecisionAction,
    OntologyProposal, ProposalSource,
};
use ontology_rag::{
    attach_conflicts, extract_proposal, AnthropicModel, LanguageModel, OpenAiModel,
};
use ontology_storage::LogRecord;
use serde::Deserialize;
use tracing::warn;

use crate::{ApiError, AppState};

/// Form fields accepted by `POST /ingest/analyze`.
///
/// Provider selection is per-request, not server-global, so the same
/// deployment can fan out OpenAI for cheap drafts and Anthropic for
/// higher-quality runs. When `provider` is omitted or set to `"default"`
/// the request is dispatched through the pipeline's pre-configured LLM
/// (useful for tests and offline mode).
#[derive(Debug, Default)]
struct AnalyzeForm {
    file_name: Option<String>,
    bytes: Option<Vec<u8>>,
    provider: Option<String>,
    model: Option<String>,
    language_hint: Option<String>,
}

/// `POST /ingest/analyze` — multipart entry point. Returns the
/// LLM-generated proposal annotated with per-item conflict info.
pub(crate) async fn analyze(
    State(s): State<AppState>,
    form: Multipart,
) -> Result<Json<OntologyProposal>, ApiError> {
    let form = read_analyze_form(form).await?;
    let bytes = form
        .bytes
        .ok_or_else(|| ApiError::BadRequest("missing `file`".into()))?;

    // 1. Decode + normalize (BOM strip, NFC).
    let decoded = decode_to_utf8(&bytes);

    // 1b. Reject binary blobs up front. Formats like .xlsx/.zip/images decode
    // "successfully" into control-character garbage; feeding that to the LLM
    // wastes a call and risks persisting a multi-megabyte blob downstream.
    // The client is expected to flatten such formats to text first.
    if ontology_io::looks_binary(&decoded.text) {
        return Err(ApiError::Unprocessable(
            "file looks like a binary format (e.g. .xlsx, .zip, image) rather than text; \
             convert it to text before ingesting"
                .into(),
        ));
    }

    // 2. Language detection (best-effort; empty / very short docs return None).
    let language = form
        .language_hint
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|code| ontology_io::LangTag {
            code: code.to_string(),
            script: String::new(),
            confidence: 1.0,
        })
        .or_else(|| detect_language(&decoded.text));

    // 3. Pick the LLM client for this request.
    let model: Arc<dyn LanguageModel> = pick_model(&s, form.provider.as_deref(), form.model)?;

    // 4. Extract — runs one or more LLM calls, depending on chunking.
    let schema = s.graph.ontology();
    let mut proposal = extract_proposal(&*model, &decoded.text, language.as_ref(), &schema)
        .await
        .map_err(|e| match e {
            ontology_rag::ExtractError::Llm(e) => ApiError::Llm(e.to_string()),
            ontology_rag::ExtractError::Parse(msg) => ApiError::Unprocessable(format!(
                "LLM returned unparseable JSON: {msg}"
            )),
        })?;

    // 5. Decorate with provenance and conflicts.
    proposal.source = Some(ProposalSource {
        name: form.file_name.unwrap_or_else(|| "upload".into()),
        kind: String::new(),
        encoding: decoded.encoding.to_string(),
        had_bom: decoded.had_bom,
        provider: form.provider.unwrap_or_else(|| "default".into()),
        model: String::new(),
    });
    proposal.language = language;
    attach_conflicts(&mut proposal, &s.graph);

    Ok(Json(proposal))
}

/// `POST /ingest/apply` body.
#[derive(Debug, Deserialize)]
pub(crate) struct ApplyRequest {
    pub proposal: OntologyProposal,
    #[serde(default)]
    pub decisions: Vec<ApplyDecision>,
    /// When `true`, abort on the first non-skip failure. Defaults to
    /// `false` (best-effort: per-item failures recorded in the report).
    #[serde(default)]
    pub strict: bool,
    /// Fallback decision applied to items whose `client_ref` is missing
    /// from `decisions`. Defaults to `Skip` — never auto-accept anything
    /// the user did not explicitly review.
    #[serde(default = "default_fallback")]
    pub default_action: DecisionAction,
}

fn default_fallback() -> DecisionAction {
    DecisionAction::Skip
}

/// `POST /ingest/apply` — write the validated proposal into the graph.
pub(crate) async fn apply(
    State(s): State<AppState>,
    Json(req): Json<ApplyRequest>,
) -> Result<Json<ApplyReport>, ApiError> {
    let ApplyRequest {
        proposal,
        decisions,
        strict,
        default_action,
    } = req;

    let decisions: HashMap<String, DecisionAction> = decisions
        .into_iter()
        .map(|d| (d.client_ref, d.action))
        .collect();
    let decide = |client_ref: &str| -> DecisionAction {
        decisions
            .get(client_ref)
            .copied()
            .unwrap_or(default_action)
    };

    let mut report = ApplyReport::default();
    // Maps every proposal `client_ref` to a resolved live ConceptId so
    // later relations / actions can target newly-created concepts.
    let mut concept_refs: HashMap<String, ConceptId> = HashMap::new();

    // ---- concept types ----
    for ct in &proposal.concept_types {
        let action = decide(&ct.client_ref);
        if action == DecisionAction::Skip {
            report.skipped += 1;
            report.concept_types.push((ct.client_ref.clone(), ApplyOutcome::Skipped));
            continue;
        }
        let res = s.graph.extend_ontology(|onto| {
            onto.add_concept_type(ontology_graph::ConceptType {
                name: ct.name.clone(),
                properties: if ct.properties.is_empty() {
                    None
                } else {
                    Some(ct.properties.clone())
                },
                parent: ct.parent.clone(),
                description: ct.description.clone(),
                ..Default::default()
            });
            Ok(())
        });
        match res {
            Ok(()) => {
                // Schema lives in the ontology, which has no per-item log
                // record — persist a full snapshot so the new type survives a
                // restart. Replay applies snapshots in order (last wins), so a
                // snapshot per accepted type is correct if slightly redundant.
                if let Err(e) = s.store.append(&LogRecord::ontology(s.graph.ontology())).await {
                    warn!(error=%e, name=%ct.name, "wal append failed for concept type");
                    report.failed += 1;
                    report.concept_types.push((
                        ct.client_ref.clone(),
                        ApplyOutcome::Failed { error: e.to_string() },
                    ));
                    if strict {
                        return Ok(Json(report));
                    }
                    continue;
                }
                report.created += 1;
                report.concept_types.push((
                    ct.client_ref.clone(),
                    ApplyOutcome::Created { id: ct.name.clone() },
                ));
            }
            Err(e) => {
                report.failed += 1;
                report.concept_types.push((
                    ct.client_ref.clone(),
                    ApplyOutcome::Failed { error: e.to_string() },
                ));
                if strict {
                    return Ok(Json(report));
                }
            }
        }
    }

    // ---- relation types ----
    for rt in &proposal.relation_types {
        let action = decide(&rt.client_ref);
        if action == DecisionAction::Skip {
            report.relation_types.push((rt.client_ref.clone(), ApplyOutcome::Skipped));
            report.skipped += 1;
            continue;
        }
        let res = s.graph.extend_ontology(|onto| {
            onto.add_relation_type(ontology_graph::RelationType {
                name: rt.name.clone(),
                domain: rt.domain.clone(),
                range: rt.range.clone(),
                cardinality: ontology_graph::Cardinality::default(),
                symmetric: rt.symmetric,
                description: rt.description.clone(),
                ..Default::default()
            })
        });
        match res {
            Ok(()) => {
                if let Err(e) = s.store.append(&LogRecord::ontology(s.graph.ontology())).await {
                    warn!(error=%e, name=%rt.name, "wal append failed for relation type");
                    report.failed += 1;
                    report.relation_types.push((
                        rt.client_ref.clone(),
                        ApplyOutcome::Failed { error: e.to_string() },
                    ));
                    if strict {
                        return Ok(Json(report));
                    }
                    continue;
                }
                report.created += 1;
                report.relation_types.push((
                    rt.client_ref.clone(),
                    ApplyOutcome::Created { id: rt.name.clone() },
                ));
            }
            Err(e) => {
                report.failed += 1;
                report.relation_types.push((
                    rt.client_ref.clone(),
                    ApplyOutcome::Failed { error: e.to_string() },
                ));
                if strict {
                    return Ok(Json(report));
                }
            }
        }
    }

    // ---- concepts ----
    for c in &proposal.concepts {
        let action = decide(&c.client_ref);
        if action == DecisionAction::Skip {
            report.concepts.push((c.client_ref.clone(), ApplyOutcome::Skipped));
            report.skipped += 1;
            continue;
        }

        // Merge maps to an existing concept (if any); otherwise behave like CreateNew.
        // Each arm returns the live id, whether it was a merge, and the WAL
        // record that makes the change durable.
        let existing_id = s.graph.find_by_name(&c.concept_type, &c.name);
        let result: Result<(ConceptId, bool, LogRecord), ontology_graph::GraphError> =
            match (action, existing_id) {
                (DecisionAction::Merge, Some(id)) => {
                    let patch = ontology_graph::ConceptPatch {
                        name: Some(c.name.clone()),
                        description: if c.description.is_empty() {
                            None
                        } else {
                            Some(c.description.clone())
                        },
                        properties: if c.properties.is_empty() {
                            None
                        } else {
                            let mut m = ahash::AHashMap::new();
                            for (k, v) in &c.properties {
                                m.insert(k.clone(), ontology_graph::PropertyValue::Text(v.clone()));
                            }
                            Some(m)
                        },
                    };
                    s.graph
                        .update_concept(id, patch)
                        .map(|updated| (id, true, LogRecord::update_concept(updated)))
                }
                _ => {
                    let mut concept = Concept::new(ConceptId(0), c.concept_type.clone(), c.name.clone())
                        .with_description(c.description.clone());
                    // Stamp the detected language onto the concept so search
                    // and downstream filters can disambiguate by locale.
                    if let Some(lang) = proposal.language.as_ref() {
                        concept.properties.insert(
                            "lang".into(),
                            ontology_graph::PropertyValue::Text(lang.code.clone()),
                        );
                    }
                    for (k, v) in &c.properties {
                        concept.properties.insert(
                            k.clone(),
                            ontology_graph::PropertyValue::Text(v.clone()),
                        );
                    }
                    let mut record_concept = concept.clone();
                    s.graph.upsert_concept(concept).map(|id| {
                        record_concept.id = id;
                        (id, false, LogRecord::concept(record_concept))
                    })
                }
            };

        match result {
            Ok((id, merged, record)) => {
                // Persist before registering the ref / counting success: if the
                // WAL write fails the concept must not be advertised as created
                // (and dependent relations must dangle rather than target an
                // unpersisted node).
                if let Err(e) = s.store.append(&record).await {
                    warn!(error=%e, name=%c.name, "wal append failed for concept");
                    report.failed += 1;
                    report.concepts.push((
                        c.client_ref.clone(),
                        ApplyOutcome::Failed {
                            error: e.to_string(),
                        },
                    ));
                    if strict {
                        return Ok(Json(report));
                    }
                    continue;
                }
                concept_refs.insert(c.client_ref.clone(), id);
                concept_refs.insert(format!("{}:{}", c.concept_type, c.name), id);
                let outcome = if merged {
                    report.merged += 1;
                    ApplyOutcome::Merged {
                        id: id.0.to_string(),
                    }
                } else {
                    report.created += 1;
                    ApplyOutcome::Created {
                        id: id.0.to_string(),
                    }
                };
                report.concepts.push((c.client_ref.clone(), outcome));
            }
            Err(e) => {
                report.failed += 1;
                report.concepts.push((
                    c.client_ref.clone(),
                    ApplyOutcome::Failed {
                        error: e.to_string(),
                    },
                ));
                if strict {
                    return Ok(Json(report));
                }
            }
        }
    }

    // ---- relations ----
    for r in &proposal.relations {
        let action = decide(&r.client_ref);
        if action == DecisionAction::Skip {
            report.relations.push((r.client_ref.clone(), ApplyOutcome::Skipped));
            report.skipped += 1;
            continue;
        }
        let src_id = resolve_ref(&r.source_ref, &concept_refs, &s.graph);
        let tgt_id = resolve_ref(&r.target_ref, &concept_refs, &s.graph);
        let (src, tgt) = match (src_id, tgt_id) {
            (Some(s), Some(t)) => (s, t),
            _ => {
                report.failed += 1;
                report.relations.push((
                    r.client_ref.clone(),
                    ApplyOutcome::Failed {
                        error: format!(
                            "dangling refs: source={} ({}), target={} ({})",
                            r.source_ref,
                            src_id.is_some(),
                            r.target_ref,
                            tgt_id.is_some()
                        ),
                    },
                ));
                if strict {
                    return Ok(Json(report));
                }
                continue;
            }
        };
        let mut rel = Relation::new(RelationId(0), r.relation_type.clone(), src, tgt);
        if let Some(w) = r.weight {
            rel.weight = w;
        }
        match s.graph.add_relation(rel.clone()) {
            Ok(id) => {
                let mut stored = rel;
                stored.id = id;
                if let Err(e) = s.store.append(&LogRecord::relation(stored)).await {
                    warn!(error=%e, "wal append failed for relation");
                    report.failed += 1;
                    report.relations.push((
                        r.client_ref.clone(),
                        ApplyOutcome::Failed {
                            error: e.to_string(),
                        },
                    ));
                    if strict {
                        return Ok(Json(report));
                    }
                    continue;
                }
                report.created += 1;
                report.relations.push((
                    r.client_ref.clone(),
                    ApplyOutcome::Created {
                        id: id.0.to_string(),
                    },
                ));
            }
            Err(e) => {
                report.failed += 1;
                report.relations.push((
                    r.client_ref.clone(),
                    ApplyOutcome::Failed {
                        error: e.to_string(),
                    },
                ));
                if strict {
                    return Ok(Json(report));
                }
            }
        }
    }

    // ---- rules ----
    for r in &proposal.rules {
        let action = decide(&r.client_ref);
        if action == DecisionAction::Skip {
            report.rules.push((r.client_ref.clone(), ApplyOutcome::Skipped));
            report.skipped += 1;
            continue;
        }
        let mut rule = Rule::new(RuleId(0), r.rule_type.clone(), r.name.clone());
        rule.when = r.when.clone();
        rule.then = r.then.clone();
        rule.strict = r.strict;
        rule.description = r.description.clone();
        // `applies_to` in the proposal is a list of concept-ref strings
        // (either client_refs or "<type>:<name>" forms). Resolve each.
        for tref in &r.applies_to {
            if let Some(id) = resolve_ref(tref, &concept_refs, &s.graph) {
                rule.applies_to.push(id);
            }
        }
        let mut record_rule = rule.clone();
        match s.graph.upsert_rule(rule) {
            Ok(id) => {
                record_rule.id = id;
                if let Err(e) = s.store.append(&LogRecord::rule(record_rule)).await {
                    warn!(error=%e, "wal append failed for rule");
                    report.failed += 1;
                    report.rules.push((
                        r.client_ref.clone(),
                        ApplyOutcome::Failed {
                            error: e.to_string(),
                        },
                    ));
                    if strict {
                        return Ok(Json(report));
                    }
                    continue;
                }
                report.created += 1;
                report.rules.push((
                    r.client_ref.clone(),
                    ApplyOutcome::Created {
                        id: id.0.to_string(),
                    },
                ));
            }
            Err(e) => {
                report.failed += 1;
                report.rules.push((
                    r.client_ref.clone(),
                    ApplyOutcome::Failed {
                        error: e.to_string(),
                    },
                ));
                if strict {
                    return Ok(Json(report));
                }
            }
        }
    }

    // ---- actions ----
    for a in &proposal.actions {
        let action = decide(&a.client_ref);
        if action == DecisionAction::Skip {
            report.actions.push((a.client_ref.clone(), ApplyOutcome::Skipped));
            report.skipped += 1;
            continue;
        }
        let subj = match resolve_ref(&a.subject_ref, &concept_refs, &s.graph) {
            Some(id) => id,
            None => {
                report.failed += 1;
                report.actions.push((
                    a.client_ref.clone(),
                    ApplyOutcome::Failed {
                        error: format!("dangling subject `{}`", a.subject_ref),
                    },
                ));
                if strict {
                    return Ok(Json(report));
                }
                continue;
            }
        };
        let obj = a
            .object_ref
            .as_ref()
            .and_then(|r| resolve_ref(r, &concept_refs, &s.graph));
        let mut act = Action::new(ActionId(0), a.action_type.clone(), a.name.clone(), subj);
        act.object = obj;
        act.effect = a.effect.clone();
        act.description = a.description.clone();
        for (k, v) in &a.parameters {
            act.parameters.insert(
                k.clone(),
                ontology_graph::PropertyValue::Text(v.clone()),
            );
        }
        let mut record_action = act.clone();
        match s.graph.upsert_action(act) {
            Ok(id) => {
                record_action.id = id;
                if let Err(e) = s.store.append(&LogRecord::action(record_action)).await {
                    warn!(error=%e, "wal append failed for action");
                    report.failed += 1;
                    report.actions.push((
                        a.client_ref.clone(),
                        ApplyOutcome::Failed {
                            error: e.to_string(),
                        },
                    ));
                    if strict {
                        return Ok(Json(report));
                    }
                    continue;
                }
                report.created += 1;
                report.actions.push((
                    a.client_ref.clone(),
                    ApplyOutcome::Created {
                        id: id.0.to_string(),
                    },
                ));
            }
            Err(e) => {
                report.failed += 1;
                report.actions.push((
                    a.client_ref.clone(),
                    ApplyOutcome::Failed {
                        error: e.to_string(),
                    },
                ));
                if strict {
                    return Ok(Json(report));
                }
            }
        }
    }

    // Refresh the hybrid index so search reflects new content immediately.
    s.index.reindex_all();

    Ok(Json(report))
}

// ---------- helpers ----------

async fn read_analyze_form(mut form: Multipart) -> Result<AnalyzeForm, ApiError> {
    let mut out = AnalyzeForm::default();
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart: {e}")))?
    {
        match field.name().unwrap_or("") {
            "file" => {
                out.file_name = field.file_name().map(|s| s.to_string());
                out.bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::BadRequest(e.to_string()))?
                        .to_vec(),
                );
            }
            "provider" => {
                out.provider = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(e.to_string()))?,
                );
            }
            "model" => {
                out.model = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(e.to_string()))?,
                );
            }
            "language_hint" => {
                out.language_hint = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(e.to_string()))?,
                );
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Choose the LLM client for an analyze request.
///
/// * `"default"` / unset → the pipeline's pre-configured LLM (tests use
///   this to inject a deterministic fake).
/// * `"openai"`   → fresh [`OpenAiModel`] from `$OPENAI_API_KEY`.
/// * `"anthropic"`→ fresh [`AnthropicModel`] from `$ANTHROPIC_API_KEY`.
fn pick_model(
    s: &AppState,
    provider: Option<&str>,
    model_override: Option<String>,
) -> Result<Arc<dyn LanguageModel>, ApiError> {
    // Resolve the effective provider: explicit > settings.llm.active_provider
    // > "default". This lets the UI configure everything via /settings.
    let llm_cfg = s.settings.read().llm.clone();
    let effective = match provider {
        Some(p) if !p.is_empty() && p != "default" => p.to_string(),
        _ => {
            if llm_cfg.active_provider != "default" && !llm_cfg.active_provider.is_empty() {
                llm_cfg.active_provider.clone()
            } else {
                "default".to_string()
            }
        }
    };

    match effective.as_str() {
        "default" | "" => Ok(s.pipeline.llm.clone()),
        "openai" => {
            let key = if !llm_cfg.openai_api_key.is_empty() {
                llm_cfg.openai_api_key.clone()
            } else {
                std::env::var("OPENAI_API_KEY").map_err(|_| {
                    ApiError::BadRequest(
                        "Clé OpenAI manquante — configurez-la dans Settings".into(),
                    )
                })?
            };
            let mut m = OpenAiModel::new(key);
            let model = model_override
                .or_else(|| {
                    if llm_cfg.openai_model.is_empty() {
                        None
                    } else {
                        Some(llm_cfg.openai_model.clone())
                    }
                });
            if let Some(model) = model {
                m = m.with_model(model);
            }
            if !llm_cfg.openai_base_url.is_empty() {
                m = m.with_base_url(llm_cfg.openai_base_url.clone());
            }
            Ok(Arc::new(m))
        }
        "anthropic" => {
            let key = if !llm_cfg.anthropic_api_key.is_empty() {
                llm_cfg.anthropic_api_key.clone()
            } else {
                std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
                    ApiError::BadRequest(
                        "Clé Anthropic manquante — configurez-la dans Settings".into(),
                    )
                })?
            };
            let mut m = AnthropicModel::new(key);
            let model = model_override
                .or_else(|| {
                    if llm_cfg.anthropic_model.is_empty() {
                        None
                    } else {
                        Some(llm_cfg.anthropic_model.clone())
                    }
                });
            if let Some(model) = model {
                m = m.with_model(model);
            }
            if !llm_cfg.anthropic_base_url.is_empty() {
                m = m.with_base_url(llm_cfg.anthropic_base_url.clone());
            }
            Ok(Arc::new(m))
        }
        "infomaniak" => {
            // Infomaniak exposes an OpenAI-compatible chat-completions API.
            let key = if !llm_cfg.infomaniak_api_key.is_empty() {
                llm_cfg.infomaniak_api_key.clone()
            } else {
                return Err(ApiError::BadRequest(
                    "Clé Infomaniak manquante — configurez-la dans Settings".into(),
                ));
            };
            let mut m = OpenAiModel::new(key);
            let base = if llm_cfg.infomaniak_base_url.is_empty() {
                "https://api.infomaniak.com/1/ai".to_string()
            } else {
                llm_cfg.infomaniak_base_url.clone()
            };
            m = m.with_base_url(base);
            let model = model_override.or_else(|| {
                if llm_cfg.infomaniak_model.is_empty() {
                    None
                } else {
                    Some(llm_cfg.infomaniak_model.clone())
                }
            });
            if let Some(model) = model {
                m = m.with_model(model);
            }
            Ok(Arc::new(m))
        }
        other => Err(ApiError::BadRequest(format!("unknown provider: {other}"))),
    }
}

fn resolve_ref(
    r: &str,
    refs: &HashMap<String, ConceptId>,
    graph: &OntologyGraph,
) -> Option<ConceptId> {
    if let Some(id) = refs.get(r) {
        return Some(*id);
    }
    if let Some((ty, name)) = r.split_once(':') {
        return graph.find_by_name(ty, name);
    }
    None
}

