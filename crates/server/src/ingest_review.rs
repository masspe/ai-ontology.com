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
    Action, ActionId, Concept, ConceptId, Ontology, OntologyGraph, Relation, RelationId, Rule,
    RuleId,
};
use ontology_io::{
    decode_to_utf8, detect_language, ApplyDecision, ApplyOutcome, ApplyReport, DecisionAction,
    OntologyProposal, ProposalSource,
};
use ontology_rag::{attach_conflicts, extract_proposal, LanguageModel};
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

    // 1a. Office formats are flattened to text server-side, so a spreadsheet
    //     or a .docx dropped on the analyzer works like a .txt would.
    let bytes = flatten_office_formats(form.file_name.as_deref(), bytes).await?;

    // 1. Decode + normalize (BOM strip, NFC).
    let decoded = decode_to_utf8(&bytes);

    // 1a'. Nothing to analyze. An empty upload must neither cost an LLM call
    // nor come back as a proposal grounded in no text at all.
    if decoded.text.trim().is_empty() {
        return Err(ApiError::Unprocessable(
            "file is empty; nothing to analyze".into(),
        ));
    }

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
            ontology_rag::ExtractError::Parse(msg) => {
                ApiError::Unprocessable(format!("LLM returned unparseable JSON: {msg}"))
            }
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

/// Spreadsheets (`.xlsx`, `.xlsm`, `.xls`, `.ods`) become one text line per
/// row (`header: value; …`), `.docx` becomes its paragraph text; anything
/// else passes through untouched. A zip container without a telling
/// extension is tried as docx, then as a spreadsheet.
async fn flatten_office_formats(
    file_name: Option<&str>,
    bytes: Vec<u8>,
) -> Result<Vec<u8>, ApiError> {
    let ext = file_name
        .map(std::path::Path::new)
        .and_then(|p| p.extension().and_then(|e| e.to_str()))
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some(e @ ("xlsx" | "xlsm" | "xls" | "ods")) => {
            spreadsheet_text(&bytes, e).await.map(String::into_bytes)
        }
        Some("docx") => ontology_io::extract_docx_text(&bytes)
            .map(String::into_bytes)
            .map_err(ApiError::Unprocessable),
        _ if ontology_io::is_zip(&bytes) => {
            if let Ok(t) = ontology_io::extract_docx_text(&bytes) {
                if !t.trim().is_empty() {
                    return Ok(t.into_bytes());
                }
            }
            spreadsheet_text(&bytes, "xlsx")
                .await
                .map(String::into_bytes)
        }
        _ => Ok(bytes),
    }
}

async fn spreadsheet_text(bytes: &[u8], ext: &str) -> Result<String, ApiError> {
    // calamine reads from a path; keep the tempfile alive until it is done.
    let tmp = crate::persist_temp(bytes, ext).await?;
    let path = tmp.path().to_path_buf();
    let text = tokio::task::spawn_blocking(move || {
        ontology_io::spreadsheet_to_text(&path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| ApiError::Store(e.to_string()))?
    .map_err(|e| ApiError::Unprocessable(format!("spreadsheet: {e}")))?;
    drop(tmp);
    if text.trim().is_empty() {
        return Err(ApiError::Unprocessable(
            "spreadsheet has no data rows".into(),
        ));
    }
    Ok(text)
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
        decisions.get(client_ref).copied().unwrap_or(default_action)
    };

    let mut report = ApplyReport::default();
    // Maps every proposal `client_ref` to a resolved live ConceptId so
    // later relations / actions can target newly-created concepts.
    let mut concept_refs: HashMap<String, ConceptId> = HashMap::new();

    // One write transaction for the whole proposal (single-writer store).
    // Every instance below follows `STORAGE.md` R8: prepare → append → apply.
    let _w = s.writer.lock().await;
    // Type declarations are applied to the live ontology as they come and
    // journaled as a single `Ontology` record before the first instance.
    let mut schema_changed = false;
    // Snapshot to restore if the declarations cannot be journaled: the live
    // schema must never be ahead of the store (R8).
    let schema_baseline = s.graph.ontology();

    // ---- concept types ----
    for ct in &proposal.concept_types {
        let action = decide(&ct.client_ref);
        if action == DecisionAction::Skip {
            report.skipped += 1;
            report
                .concept_types
                .push((ct.client_ref.clone(), ApplyOutcome::Skipped));
            continue;
        }
        let res = s.graph.extend_ontology(|onto| {
            // Refresh, never overwrite: an existing type keeps its domain
            // and whatever the proposal does not set.
            onto.merge_concept_type(ontology_graph::ConceptType {
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
                schema_changed = true;
                report.created += 1;
                report.concept_types.push((
                    ct.client_ref.clone(),
                    ApplyOutcome::Created {
                        id: ct.name.clone(),
                    },
                ));
            }
            Err(e) => {
                report.failed += 1;
                report.concept_types.push((
                    ct.client_ref.clone(),
                    ApplyOutcome::Failed {
                        error: e.to_string(),
                    },
                ));
                if strict {
                    rollback_schema(&s, &schema_baseline, schema_changed);
                    return Ok(Json(report));
                }
            }
        }
    }

    // ---- relation types ----
    for rt in &proposal.relation_types {
        let action = decide(&rt.client_ref);
        if action == DecisionAction::Skip {
            report
                .relation_types
                .push((rt.client_ref.clone(), ApplyOutcome::Skipped));
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
                schema_changed = true;
                report.created += 1;
                report.relation_types.push((
                    rt.client_ref.clone(),
                    ApplyOutcome::Created {
                        id: rt.name.clone(),
                    },
                ));
            }
            Err(e) => {
                report.failed += 1;
                report.relation_types.push((
                    rt.client_ref.clone(),
                    ApplyOutcome::Failed {
                        error: e.to_string(),
                    },
                ));
                if strict {
                    rollback_schema(&s, &schema_baseline, schema_changed);
                    return Ok(Json(report));
                }
            }
        }
    }

    // Schema lives in the ontology, which has no per-item log record: one
    // full snapshot makes every accepted type durable before any instance
    // that may reference it is journaled. A failure here aborts the apply —
    // continuing would journal concepts whose types are not on disk, and the
    // next replay would reject them.
    if schema_changed {
        if let Err(e) = s
            .store
            .append(&LogRecord::ontology(s.graph.ontology()))
            .await
        {
            warn!(error=%e, "wal append failed for ontology; type declarations rolled back");
            rollback_schema(&s, &schema_baseline, true);
            return Err(ApiError::Store(e.to_string()));
        }
    }

    // ---- concepts ----
    for c in &proposal.concepts {
        let action = decide(&c.client_ref);
        if action == DecisionAction::Skip {
            report
                .concepts
                .push((c.client_ref.clone(), ApplyOutcome::Skipped));
            report.skipped += 1;
            continue;
        }

        // Merge maps to an existing concept (if any); otherwise behave like CreateNew.
        // Each arm returns the fully validated concept and whether it was a
        // merge; nothing has been applied yet.
        let existing_id = s.graph.find_by_name(&c.concept_type, &c.name);
        let prepared: Result<(bool, Concept), ontology_graph::GraphError> =
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
                                m.insert(
                                    k.clone(),
                                    ontology_graph::PropertyValue::from_loose_text(v),
                                );
                            }
                            Some(m)
                        },
                    };
                    s.graph
                        .preview_concept_update(id, &patch)
                        .map(|updated| (true, updated))
                }
                _ => {
                    let mut concept =
                        Concept::new(ConceptId(0), c.concept_type.clone(), c.name.clone())
                            .with_description(c.description.clone());
                    // Stamp the detected language onto the concept so search
                    // and downstream filters can disambiguate by locale.
                    if let Some(lang) = proposal.language.as_ref() {
                        concept.properties.insert(
                            "lang".into(),
                            ontology_graph::PropertyValue::Text(lang.code.clone()),
                        );
                    }
                    // Amounts and flags arrive as text from the model; type them
                    // so they sort and filter like the CSV/XLSX import does.
                    for (k, v) in &c.properties {
                        concept
                            .properties
                            .insert(k.clone(), ontology_graph::PropertyValue::from_loose_text(v));
                    }
                    s.graph
                        .prepare_concept(&mut concept)
                        .map(|()| (false, concept))
                }
            };

        match prepared {
            Ok((merged, concept)) => {
                // Persist before applying or registering the ref: if the WAL
                // write fails the graph is untouched and the concept must not
                // be advertised as created (dependent relations dangle rather
                // than target an unpersisted node).
                let record = if merged {
                    LogRecord::update_concept(concept.clone())
                } else {
                    LogRecord::concept(concept.clone())
                };
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
                let applied = if merged {
                    s.graph.apply_concept_update(concept).map(|c| c.id)
                } else {
                    s.graph.apply_prepared_concept(concept)
                };
                let id = match applied {
                    Ok(id) => id,
                    Err(e) => {
                        // Cannot happen after a successful prepare/preview
                        // under the writer lock; reported rather than hidden.
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
                };
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
            report
                .relations
                .push((r.client_ref.clone(), ApplyOutcome::Skipped));
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
        let outcome: Result<RelationId, String> = async {
            s.graph
                .prepare_relation(&mut rel)
                .map_err(|e| e.to_string())?;
            s.store
                .append(&LogRecord::relation(rel.clone()))
                .await
                .map_err(|e| {
                    warn!(error=%e, "wal append failed for relation");
                    e.to_string()
                })?;
            s.graph
                .apply_prepared_relation(rel)
                .map_err(|e| e.to_string())
        }
        .await;
        match outcome {
            Ok(id) => {
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
            report
                .rules
                .push((r.client_ref.clone(), ApplyOutcome::Skipped));
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
        let outcome: Result<RuleId, String> = async {
            s.graph.prepare_rule(&mut rule).map_err(|e| e.to_string())?;
            s.store
                .append(&LogRecord::rule(rule.clone()))
                .await
                .map_err(|e| {
                    warn!(error=%e, "wal append failed for rule");
                    e.to_string()
                })?;
            s.graph.apply_prepared_rule(rule).map_err(|e| e.to_string())
        }
        .await;
        match outcome {
            Ok(id) => {
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
            report
                .actions
                .push((a.client_ref.clone(), ApplyOutcome::Skipped));
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
            act.parameters
                .insert(k.clone(), ontology_graph::PropertyValue::Text(v.clone()));
        }
        let outcome: Result<ActionId, String> = async {
            s.graph
                .prepare_action(&mut act)
                .map_err(|e| e.to_string())?;
            s.store
                .append(&LogRecord::action(act.clone()))
                .await
                .map_err(|e| {
                    warn!(error=%e, "wal append failed for action");
                    e.to_string()
                })?;
            s.graph
                .apply_prepared_action(act)
                .map_err(|e| e.to_string())
        }
        .await;
        match outcome {
            Ok(id) => {
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

/// Undo type declarations applied to the live ontology but never journaled.
/// Only instances of journaled types can exist, so this cannot orphan data.
fn rollback_schema(s: &AppState, baseline: &Ontology, changed: bool) {
    if !changed {
        return;
    }
    if let Err(e) = s.graph.extend_ontology(|o| {
        *o = baseline.clone();
        Ok(())
    }) {
        warn!(error=%e, "could not roll back un-journaled schema changes");
    }
}

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
/// * `"default"` / unset → the pipeline's own LLM. Tests inject a
///   deterministic fake there, and it is also what a plain `ontology serve`
///   with no provider configured falls back to.
/// * anything else → built from the persisted settings by
///   [`crate::model_for`], the single resolution path shared with `/ask`.
///
/// No environment variable is consulted: credentials live in the settings
/// store and nowhere else.
fn pick_model(
    s: &AppState,
    provider: Option<&str>,
    model_override: Option<String>,
) -> Result<Arc<dyn LanguageModel>, ApiError> {
    let llm = s.settings.read().llm.clone();
    let overrides = crate::LlmOverrides {
        provider: provider.map(str::to_string),
        model: model_override,
        ..Default::default()
    };
    if crate::effective_provider(&llm, &overrides) == "default" {
        return Ok(s.pipeline.llm.clone());
    }
    crate::model_for(&llm, &overrides).map_err(ApiError::BadRequest)
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
