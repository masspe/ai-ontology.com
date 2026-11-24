// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Write-back with approvals: an action of the graph changes the
//! database it came from. Off unless the source says `allow_writes`.
//! An administrator binds an action type to a named statement of the
//! source — one `UPDATE`, `INSERT` or `DELETE` with blanks. Running an
//! action fills the blanks from the action and its sheets and files the
//! result as a pending approval; another administrator approves it, and
//! only then does the statement run, in a transaction rolled back when
//! it would change more rows than the statement allows. Each step is in
//! the audit log.
//!
//! Routes: `POST /sources/:id/statements`,
//! `PATCH|DELETE /sources/:id/statements/:sid`, `POST /actions/:id/run`,
//! `GET /approvals`, `POST /approvals/:id/approve`,
//! `POST /approvals/:id/reject`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ontology_graph::{Action, ActionId, Concept, PropertyValue};
use ontology_io::connect::sql_literal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::audit::AuditNote;
use crate::auth::now_rfc3339;
use crate::sources::{admin_only, connector_for, safe_param};
use crate::{ApiError, AppState, AuthContext};

/// Decided approvals kept in the file; the audit log keeps them all.
const KEPT: usize = 200;

/// What an approved action of `action_type` runs on its source.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NamedStatement {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub action_type: String,
    /// `UPDATE … SET state = {state} WHERE code = {subject.code}`: a
    /// `{parameter}` of the action, `{subject}` / `{object}` (the sheet's
    /// name) or `{subject.property}`, each written as a literal.
    pub sql: String,
    /// More rows than this would change: rolled back.
    pub max_rows: u64,
    pub enabled: bool,
}

/// A run of an action, waiting for — or past — another administrator.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Approval {
    pub id: u64,
    pub action_id: u64,
    pub action_type: String,
    pub action_name: String,
    pub source_id: u64,
    pub source_name: String,
    pub statement_id: u64,
    /// The statement as it was when asked: changed since, nothing runs.
    pub template: String,
    /// What runs, blanks filled: what the approver reads is what runs.
    pub sql: String,
    pub max_rows: u64,
    pub requested_by: String,
    pub requested_by_id: String,
    pub requested_at: String,
    /// `pending`, `running`, `done`, `failed`, `rejected`.
    pub status: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub rows_affected: Option<u64>,
    pub error: Option<String>,
}

/// `sql` with every `{blank}` replaced by the literal of its value: a
/// short text without a quote, a number, or yes/no.
fn render(sql: &str, value: impl Fn(&str) -> Option<PropertyValue>) -> Result<String, String> {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(open) = rest.find('{') {
        let close = open
            + rest[open..]
                .find('}')
                .ok_or("a `{` is never closed".to_string())?;
        let name = rest[open + 1..close].trim();
        let literal = match value(name) {
            Some(PropertyValue::Text(t)) if safe_param(t.trim()) => {
                sql_literal(&Value::String(t.trim().to_string()))
            }
            // In parentheses: `a - {n}` must not read `a --1`, a comment.
            Some(PropertyValue::Number(n)) if n.is_finite() && n.is_sign_negative() => {
                format!("({n})")
            }
            Some(PropertyValue::Number(n)) if n.is_finite() => n.to_string(),
            Some(PropertyValue::Bool(b)) => format!("'{b}'"),
            Some(_) => {
                return Err(format!(
                    "the value of {{{name}}} cannot be written: a short text without quotes, a number or yes/no can"
                ))
            }
            None => return Err(format!("{{{name}}} has no value on this action")),
        };
        out.push_str(&rest[..open]);
        out.push_str(&literal);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    // A literal never holds a brace: one left over was in the statement.
    if out.contains('}') {
        return Err("a `}` closes nothing".into());
    }
    Ok(out)
}

/// The value of a blank: `subject`, `object` (the sheet's name),
/// `subject.property`, else the action's parameter of that name.
fn blank(
    action: &Action,
    subject: &Concept,
    object: Option<&Concept>,
    name: &str,
) -> Option<PropertyValue> {
    let (who, property) = match name.split_once('.') {
        Some((who, property)) => (who, Some(property)),
        None => (name, None),
    };
    let sheet = match who {
        "subject" => subject,
        "object" => object?,
        _ => return action.parameters.get(name).cloned(),
    };
    match property {
        None => Some(PropertyValue::Text(sheet.name.clone())),
        Some(p) => sheet.properties.get(p).cloned(),
    }
}

/// Pending approvals all stay; of the decided ones, the last [`KEPT`].
fn forget_old(approvals: &mut Vec<Approval>) {
    let mut extra = approvals
        .iter()
        .filter(|a| a.status != "pending")
        .count()
        .saturating_sub(KEPT);
    approvals.retain(|a| {
        a.status == "pending" || extra == 0 || {
            extra -= 1;
            false
        }
    });
}

/// Asking and deciding are for signed-in administrator accounts: a key
/// or the service token is nobody in particular, and two of them are
/// not two people. Returns the account's id and how the log names it.
fn account(ctx: &Option<axum::Extension<AuthContext>>) -> Result<(String, String), ApiError> {
    match ctx {
        Some(c) if !c.service && c.clearances.is_none() && !c.read_only => {
            Ok((c.subject.clone(), admin_only(ctx)?))
        }
        _ => Err(ApiError::Forbidden(
            "write-back is for signed-in administrator accounts".into(),
        )),
    }
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/sources/:id/statements", post(create_statement))
        .route(
            "/sources/:id/statements/:sid",
            axum::routing::patch(update_statement).delete(delete_statement),
        )
        .route("/actions/:id/run", post(run_action))
        .route("/approvals", get(list_approvals))
        .route("/approvals/:id/approve", post(approve))
        .route("/approvals/:id/reject", post(reject))
        .with_state(state)
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct StatementPatch {
    name: Option<String>,
    description: Option<String>,
    action_type: Option<String>,
    sql: Option<String>,
    max_rows: Option<u64>,
    enabled: Option<bool>,
}

/// `p` over `st`, and the whole checked: the caller keeps the result
/// only when it is valid.
fn apply_statement_patch(
    s: &AppState,
    st: &mut NamedStatement,
    p: StatementPatch,
) -> Result<(), ApiError> {
    if let Some(v) = p.name {
        st.name = v.trim().to_string();
    }
    if let Some(v) = p.description {
        st.description = v;
    }
    if let Some(v) = p.action_type {
        st.action_type = v.trim().to_string();
    }
    if let Some(v) = p.sql {
        st.sql = v.trim().trim_end_matches(';').trim_end().to_string();
    }
    if let Some(v) = p.max_rows {
        st.max_rows = v;
    }
    if let Some(v) = p.enabled {
        st.enabled = v;
    }
    if st.name.is_empty() || st.name.chars().count() > 80 {
        return Err(ApiError::BadRequest(
            "a name of 1 to 80 characters is required".into(),
        ));
    }
    if !s
        .graph
        .ontology()
        .action_types
        .contains_key(&st.action_type)
    {
        return Err(ApiError::BadRequest(format!(
            "no action type named {:?}",
            st.action_type
        )));
    }
    let verb = st.sql.split_whitespace().next().unwrap_or_default();
    if !["update", "insert", "delete"].contains(&verb.to_ascii_lowercase().as_str())
        || st.sql.contains(';')
        || st.sql.len() > 4000
    {
        return Err(ApiError::BadRequest(
            "one UPDATE, INSERT or DELETE statement, without `;`, of at most 4000 characters"
                .into(),
        ));
    }
    render(&st.sql, |_| Some(PropertyValue::Number(0.0))).map_err(ApiError::BadRequest)?;
    if !(1..=10_000).contains(&st.max_rows) {
        return Err(ApiError::BadRequest("max_rows: between 1 and 10000".into()));
    }
    Ok(())
}

async fn create_statement(
    State(s): State<AppState>,
    ctx: Option<axum::Extension<AuthContext>>,
    Path(id): Path<u64>,
    Json(p): Json<StatementPatch>,
) -> Result<Response, ApiError> {
    admin_only(&ctx)?;
    let mut st = NamedStatement {
        max_rows: 1,
        enabled: true,
        ..Default::default()
    };
    apply_statement_patch(&s, &mut st, p)?;
    let created = s.sources.update(id, |src| {
        st.id = src.statements.iter().map(|x| x.id).max().unwrap_or(0) + 1;
        src.statements.push(st.clone());
    })?;
    Ok((
        StatusCode::CREATED,
        Json(created.statements.last().cloned().unwrap_or_default()),
    )
        .into_response())
}

async fn update_statement(
    State(s): State<AppState>,
    ctx: Option<axum::Extension<AuthContext>>,
    Path((id, sid)): Path<(u64, u64)>,
    Json(p): Json<StatementPatch>,
) -> Result<Json<NamedStatement>, ApiError> {
    admin_only(&ctx)?;
    let mut out = Err(ApiError::NotFound(format!("statement {sid}")));
    s.sources.update(id, |src| {
        if let Some(st) = src.statements.iter_mut().find(|x| x.id == sid) {
            let mut next = st.clone();
            out = apply_statement_patch(&s, &mut next, p).map(|()| {
                *st = next.clone();
                next
            });
        }
    })?;
    out.map(Json)
}

async fn delete_statement(
    State(s): State<AppState>,
    ctx: Option<axum::Extension<AuthContext>>,
    Path((id, sid)): Path<(u64, u64)>,
) -> Result<StatusCode, ApiError> {
    admin_only(&ctx)?;
    let mut found = false;
    s.sources.update(id, |src| {
        let before = src.statements.len();
        src.statements.retain(|x| x.id != sid);
        found = src.statements.len() != before;
    })?;
    if !found {
        return Err(ApiError::NotFound(format!("statement {sid}")));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The approval, and its line in the audit log.
fn noted(status: StatusCode, a: &Approval, note: String) -> Response {
    let mut resp = (status, Json(a)).into_response();
    resp.extensions_mut().insert(AuditNote(note));
    resp
}

/// Ask for the action to be written back: nothing runs yet.
async fn run_action(
    State(s): State<AppState>,
    ctx: Option<axum::Extension<AuthContext>>,
    Path(id): Path<u64>,
) -> Result<Response, ApiError> {
    let (who_id, who) = account(&ctx)?;
    let action = crate::lookup(s.graph.get_action(ActionId(id)), "action", id)?;
    let sources = s.sources.list();
    let mut bound = sources
        .iter()
        .filter(|src| src.allow_writes)
        .flat_map(|src| src.statements.iter().map(move |st| (src, st)))
        .filter(|(_, st)| st.enabled && st.action_type == action.action_type);
    let (src, st) = match (bound.next(), bound.next()) {
        (Some(one), None) => one,
        (None, _) => {
            return Err(ApiError::BadRequest(format!(
                "no source writes back actions of type {}",
                action.action_type
            )))
        }
        _ => {
            return Err(ApiError::BadRequest(format!(
                "several statements are bound to actions of type {}: disable all but one",
                action.action_type
            )))
        }
    };
    let subject = s.graph.get_concept(action.subject)?;
    let object = action.object.map(|o| s.graph.get_concept(o)).transpose()?;
    let sql = render(&st.sql, |name| {
        blank(&action, &subject, object.as_ref(), name)
    })
    .map_err(ApiError::Unprocessable)?;
    let approval = {
        let mut inner = s.sources.inner.lock();
        if inner
            .approvals
            .iter()
            .any(|a| a.action_id == id && a.status == "pending")
        {
            return Err(ApiError::Unprocessable(format!(
                "action {id} already awaits an approval"
            )));
        }
        inner.next_approval += 1;
        let a = Approval {
            id: inner.next_approval,
            action_id: id,
            action_type: action.action_type.clone(),
            action_name: action.name.clone(),
            source_id: src.id,
            source_name: src.name.clone(),
            statement_id: st.id,
            template: st.sql.clone(),
            sql,
            max_rows: st.max_rows,
            requested_by: who,
            requested_by_id: who_id,
            requested_at: now_rfc3339(),
            status: "pending".into(),
            ..Default::default()
        };
        inner.approvals.push(a.clone());
        forget_old(&mut inner.approvals);
        s.sources.persist(&inner)?;
        a
    };
    let note = format!(
        "write-back asked: approval {} for action {} ({}) on {}: {}",
        approval.id, id, approval.action_type, approval.source_name, approval.sql
    );
    Ok(noted(StatusCode::CREATED, &approval, note))
}

/// Newest first.
async fn list_approvals(
    State(s): State<AppState>,
    ctx: Option<axum::Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    admin_only(&ctx)?;
    let mut approvals = s.sources.inner.lock().approvals.clone();
    approvals.reverse();
    Ok(Json(serde_json::json!({ "approvals": approvals })))
}

fn pending(approvals: &mut [Approval], id: u64) -> Result<&mut Approval, ApiError> {
    let a = approvals
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("approval {id}")))?;
    if a.status != "pending" {
        return Err(ApiError::Unprocessable(format!(
            "approval {id} is already {}",
            a.status
        )));
    }
    Ok(a)
}

/// Another administrator says yes: the statement runs, once. The answer
/// is the approval, `done` or `failed` — a failure is an outcome to
/// log, not an error of this request.
async fn approve(
    State(s): State<AppState>,
    ctx: Option<axum::Extension<AuthContext>>,
    Path(id): Path<u64>,
) -> Result<Response, ApiError> {
    let (who_id, who) = account(&ctx)?;
    // Pending → running under the lock and on disk before anything runs:
    // neither a second approval nor a restart runs it again.
    let (mut approval, src) = {
        let mut inner = s.sources.inner.lock();
        let file = &mut *inner;
        let a = pending(&mut file.approvals, id)?;
        if a.requested_by_id == who_id {
            return Err(ApiError::Forbidden(
                "another administrator must approve what you asked for".into(),
            ));
        }
        let src = file
            .sources
            .iter()
            .find(|x| x.id == a.source_id && x.allow_writes)
            .filter(|x| {
                x.statements.iter().any(|st| {
                    st.id == a.statement_id
                        && st.enabled
                        && st.action_type == a.action_type
                        && st.sql == a.template
                        && st.max_rows == a.max_rows
                })
            })
            .cloned()
            .ok_or_else(|| {
                ApiError::Unprocessable(
                    "the source or its statement changed since this was asked: reject it and run the action again"
                        .into(),
                )
            })?;
        let before = a.clone();
        a.status = "running".into();
        a.decided_by = Some(who);
        a.decided_at = Some(now_rfc3339());
        let running = a.clone();
        if let Err(e) = s.sources.persist(file) {
            *pending(&mut file.approvals, id)? = before;
            return Err(e);
        }
        (running, src)
    };
    let outcome = match connector_for(&s.sources, &src) {
        Ok(c) => c
            .execute(&approval.sql, approval.max_rows)
            .await
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    let result = match outcome {
        Ok(rows) => {
            approval.status = "done".into();
            approval.rows_affected = Some(rows);
            format!("{rows} row(s) changed")
        }
        Err(e) => {
            approval.status = "failed".into();
            approval.error = Some(e.clone());
            format!("failed: {e}")
        }
    };
    {
        let mut inner = s.sources.inner.lock();
        if let Some(a) = inner.approvals.iter_mut().find(|a| a.id == id) {
            *a = approval.clone();
        }
        // The statement ran or failed either way: the answer and the
        // audit log still say which.
        if let Err(e) = s.sources.persist(&inner) {
            tracing::error!(approval = id, error = %e, "write-back outcome not saved");
        }
    }
    let note = format!(
        "write-back approved: approval {} for action {} ({}) on {}, asked by {}: {} — {}",
        approval.id,
        approval.action_id,
        approval.action_type,
        approval.source_name,
        approval.requested_by,
        approval.sql,
        result
    );
    Ok(noted(StatusCode::OK, &approval, note))
}

/// No: by another administrator, or by the one who asked.
async fn reject(
    State(s): State<AppState>,
    ctx: Option<axum::Extension<AuthContext>>,
    Path(id): Path<u64>,
) -> Result<Response, ApiError> {
    let (_, who) = account(&ctx)?;
    let rejected = {
        let mut inner = s.sources.inner.lock();
        let file = &mut *inner;
        let a = pending(&mut file.approvals, id)?;
        a.status = "rejected".into();
        a.decided_by = Some(who);
        a.decided_at = Some(now_rfc3339());
        let a = a.clone();
        s.sources.persist(file)?;
        a
    };
    let note = format!(
        "write-back rejected: approval {} for action {} ({}) on {}, asked by {}",
        rejected.id,
        rejected.action_id,
        rejected.action_type,
        rejected.source_name,
        rejected.requested_by
    );
    Ok(noted(StatusCode::OK, &rejected, note))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Option<PropertyValue> {
        Some(PropertyValue::Text(s.into()))
    }

    #[test]
    fn a_blank_becomes_a_literal_or_the_statement_is_refused() {
        let value = |name: &str| match name {
            "state" => text(" done "),
            "n" => Some(PropertyValue::Number(42.0)),
            "ratio" => Some(PropertyValue::Number(-0.5)),
            "yes" => Some(PropertyValue::Bool(true)),
            "quote" => text("x' OR 1=1 --"),
            "nan" => Some(PropertyValue::Number(f64::NAN)),
            "list" => Some(PropertyValue::List(Vec::new())),
            _ => None,
        };
        assert_eq!(
            render(
                "update t set s = {state}, r = { ratio }, ok = {yes} where id = {n}",
                value
            )
            .unwrap(),
            "update t set s = 'done', r = (-0.5), ok = 'true' where id = 42"
        );
        // A negative number after a minus sign does not open a comment.
        assert_eq!(
            render("update t set r = r -{ratio} where id = {n}", value).unwrap(),
            "update t set r = r -(-0.5) where id = 42"
        );
        assert_eq!(render("delete from t", value).unwrap(), "delete from t");
        for (sql, why) in [
            ("update t set s = {quote}", "cannot be written"),
            ("update t set s = {nan}", "cannot be written"),
            ("update t set s = {list}", "cannot be written"),
            ("update t set s = {missing}", "{missing} has no value"),
            ("update t set s = {state", "never closed"),
            ("update t set s = state}", "closes nothing"),
        ] {
            let e = render(sql, value).unwrap_err();
            assert!(e.contains(why), "{sql}: {e}");
        }
    }

    #[test]
    fn a_blank_reads_the_action_and_its_sheets() {
        let sheet = |name: &str, code: &str| Concept {
            id: ontology_graph::ConceptId(1),
            concept_type: "Order".into(),
            name: name.into(),
            description: String::new(),
            properties: [("code".to_string(), PropertyValue::from(code))]
                .into_iter()
                .collect(),
            labels: Vec::new(),
        };
        let (order, customer) = (sheet("Order 7", "O-7"), sheet("Acme", "C-1"));
        let action = Action {
            id: ActionId(1),
            action_type: "close".into(),
            name: "close order 7".into(),
            subject: order.id,
            object: None,
            parameters: [("state".to_string(), PropertyValue::from("done"))]
                .into_iter()
                .collect(),
            effect: String::new(),
            description: String::new(),
        };
        let get = |object: Option<&Concept>, name: &str| blank(&action, &order, object, name);
        assert_eq!(get(None, "state"), text("done"));
        assert_eq!(get(None, "subject"), text("Order 7"));
        assert_eq!(get(None, "subject.code"), text("O-7"));
        assert_eq!(get(None, "subject.absent"), None);
        assert_eq!(get(None, "object"), None);
        assert_eq!(get(None, "object.code"), None);
        assert_eq!(get(Some(&customer), "object"), text("Acme"));
        assert_eq!(get(Some(&customer), "object.code"), text("C-1"));
        assert_eq!(get(None, "absent"), None);
    }

    #[test]
    fn decided_approvals_past_the_last_ones_are_forgotten_pending_ones_never() {
        let mut all: Vec<Approval> = (1..=KEPT as u64 + 3)
            .map(|id| Approval {
                id,
                status: if id == 2 { "pending" } else { "done" }.into(),
                ..Default::default()
            })
            .collect();
        forget_old(&mut all);
        let ids: Vec<u64> = all.iter().map(|a| a.id).collect();
        // 202 decided, 200 kept: the two oldest decided go, the pending stays.
        assert_eq!(ids.len(), KEPT + 1);
        assert_eq!(&ids[..3], &[2, 4, 5]);
        forget_old(&mut all);
        assert_eq!(all.len(), KEPT + 1);
    }
}
