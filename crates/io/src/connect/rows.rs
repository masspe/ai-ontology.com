// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Rows into sheets: a [`Mapping`] says which column is the key, which
//! columns become properties and which foreign keys become links;
//! [`RowSource`] feeds the regular ingester with the records.

use std::collections::VecDeque;

use ahash::AHashMap;
use async_trait::async_trait;
use ontology_graph::{Concept, ConceptId, ConceptType, PropertyValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ResultSet;
use crate::ingest::{IngestError, Source};
use crate::record::Record;

/// A foreign key in the rows: the column's value names a sheet of
/// `target_type` (by its key), and the row's sheet gets a link to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationRule {
    pub column: String,
    pub relation_type: String,
    pub target_type: String,
    /// Create the target as a bare sheet when it is not there yet (the
    /// table it comes from may sync later, and fills it in).
    #[serde(default = "default_true")]
    pub create_target: bool,
}

fn default_true() -> bool {
    true
}

/// How a table becomes sheets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mapping {
    /// The concept type the rows become (declared if needed).
    pub concept_type: String,
    /// The column whose value is the sheet's identity (its name, unless
    /// `name_column` says otherwise).
    pub key_column: String,
    /// The column shown as the sheet's name; the key stays a property.
    pub name_column: Option<String>,
    /// The column that becomes the description.
    pub description_column: Option<String>,
    /// The columns kept as properties; empty: every column.
    pub property_columns: Vec<String>,
    pub relations: Vec<RelationRule>,
    /// Storage domain of the type when it has to be declared.
    pub ns: Option<String>,
    /// Stamped on every sheet (`source = "<source>/<mapping>"`), so a
    /// later sync can tell its own sheets from the rest.
    pub source_tag: String,
}

/// A JSON cell as a property value; `Null` is no property.
pub fn property(v: &Value) -> Option<PropertyValue> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(PropertyValue::Bool(*b)),
        Value::Number(n) => n.as_f64().map(PropertyValue::Number),
        Value::String(s) => Some(PropertyValue::Text(s.clone())),
        other => Some(PropertyValue::Text(other.to_string())),
    }
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The records of one result set under a mapping, in the order the
/// ingester needs them: the type declaration, then per row the link
/// targets, the sheet, its links.
pub struct RowSource {
    queue: VecDeque<Record>,
    /// Rows whose key cell was empty: counted, never a sheet.
    pub skipped: usize,
    /// The key values seen, for a stale sweep after a full sync.
    pub keys: Vec<String>,
}

impl RowSource {
    pub fn new(rs: &ResultSet, m: &Mapping, declare_type: bool) -> Result<Self, IngestError> {
        let key = rs
            .column_index(&m.key_column)
            .ok_or_else(|| IngestError::Source(format!("no column `{}`", m.key_column)))?;
        let name = match &m.name_column {
            Some(c) => Some(
                rs.column_index(c)
                    .ok_or_else(|| IngestError::Source(format!("no column `{c}`")))?,
            ),
            None => None,
        };
        let description = match &m.description_column {
            Some(c) => Some(
                rs.column_index(c)
                    .ok_or_else(|| IngestError::Source(format!("no column `{c}`")))?,
            ),
            None => None,
        };
        let mut props: Vec<usize> = if m.property_columns.is_empty() {
            (0..rs.columns.len()).collect()
        } else {
            m.property_columns
                .iter()
                .map(|c| {
                    rs.column_index(c)
                        .ok_or_else(|| IngestError::Source(format!("no column `{c}`")))
                })
                .collect::<Result<_, _>>()?
        };
        props.retain(|i| Some(*i) != description);
        let rules: Vec<(usize, &RelationRule)> = m
            .relations
            .iter()
            .map(|r| {
                rs.column_index(&r.column)
                    .map(|i| (i, r))
                    .ok_or_else(|| IngestError::Source(format!("no column `{}`", r.column)))
            })
            .collect::<Result<_, _>>()?;

        let mut queue = VecDeque::new();
        if declare_type {
            queue.push_back(Record::ConceptTypeDecl(ConceptType {
                name: m.concept_type.clone(),
                properties: None,
                ns: m.ns.clone(),
                ..Default::default()
            }));
            for r in &m.relations {
                if r.create_target && r.target_type != m.concept_type {
                    queue.push_back(Record::ConceptTypeDecl(ConceptType {
                        name: r.target_type.clone(),
                        properties: None,
                        ns: m.ns.clone(),
                        ..Default::default()
                    }));
                }
            }
        }
        let mut skipped = 0;
        let mut keys = Vec::with_capacity(rs.rows.len());
        for row in &rs.rows {
            let k = text(row.get(key).unwrap_or(&Value::Null));
            if k.trim().is_empty() {
                skipped += 1;
                continue;
            }
            let sheet_name = name
                .and_then(|i| row.get(i))
                .map(text)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| k.clone());
            let mut c = Concept::new(ConceptId(0), m.concept_type.clone(), sheet_name);
            if let Some(d) = description.and_then(|i| row.get(i)) {
                c.description = text(d);
            }
            for &i in &props {
                if let Some(v) = row.get(i).and_then(property) {
                    c.properties.insert(rs.columns[i].name.clone(), v);
                }
            }
            c.properties
                .insert("source".into(), PropertyValue::Text(m.source_tag.clone()));
            c.properties
                .insert("stale".into(), PropertyValue::Bool(false));
            let mut links = Vec::new();
            for (i, rule) in &rules {
                let target = text(row.get(*i).unwrap_or(&Value::Null));
                if target.trim().is_empty() {
                    continue;
                }
                if rule.create_target {
                    let mut t =
                        Concept::new(ConceptId(0), rule.target_type.clone(), target.clone());
                    t.properties
                        .insert("source".into(), PropertyValue::Text(m.source_tag.clone()));
                    queue.push_back(Record::Concept(t));
                }
                links.push(Record::NamedRelation {
                    relation_type: rule.relation_type.clone(),
                    source_type: m.concept_type.clone(),
                    source_name: c.name.clone(),
                    target_type: rule.target_type.clone(),
                    target_name: target,
                    weight: 1.0,
                    properties: AHashMap::new(),
                });
            }
            keys.push(c.name.clone());
            queue.push_back(Record::Concept(c));
            queue.extend(links);
        }
        Ok(Self {
            queue,
            skipped,
            keys,
        })
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[async_trait]
impl Source for RowSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        Ok(self.queue.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect::ColumnInfo;
    use serde_json::json;

    fn rs() -> ResultSet {
        ResultSet {
            columns: ["id", "label", "amount", "customer", "note"]
                .iter()
                .map(|c| ColumnInfo {
                    name: c.to_string(),
                    data_type: "any".into(),
                })
                .collect(),
            rows: vec![
                vec![
                    json!("O-1"),
                    json!("First order"),
                    json!(12.5),
                    json!("C-7"),
                    json!("rush"),
                ],
                vec![
                    json!(""),
                    json!("no key"),
                    json!(1),
                    json!("C-7"),
                    Value::Null,
                ],
                vec![
                    json!("O-2"),
                    Value::Null,
                    json!(true),
                    Value::Null,
                    json!(""),
                ],
            ],
            truncated: false,
        }
    }

    #[tokio::test]
    async fn rows_become_typed_sheets_with_properties_and_links() {
        let m = Mapping {
            concept_type: "Order".into(),
            key_column: "ID".into(),
            name_column: Some("label".into()),
            description_column: Some("note".into()),
            property_columns: vec![],
            relations: vec![RelationRule {
                column: "customer".into(),
                relation_type: "placed_by".into(),
                target_type: "Customer".into(),
                create_target: true,
            }],
            ns: Some("erp".into()),
            source_tag: "crm/orders".into(),
        };
        let mut src = RowSource::new(&rs(), &m, true).unwrap();
        assert_eq!(src.skipped, 1);
        assert_eq!(src.keys, ["First order", "O-2"]);
        let mut out = Vec::new();
        while let Some(r) = src.next().await.unwrap() {
            out.push(r);
        }
        // Two type declarations, then: customer stub, order, link; then order 2.
        assert!(
            matches!(&out[0], Record::ConceptTypeDecl(t) if t.name == "Order" && t.ns.as_deref() == Some("erp"))
        );
        assert!(matches!(&out[1], Record::ConceptTypeDecl(t) if t.name == "Customer"));
        assert!(
            matches!(&out[2], Record::Concept(c) if c.concept_type == "Customer" && c.name == "C-7")
        );
        let Record::Concept(order) = &out[3] else {
            panic!("{:?}", out[3]);
        };
        assert_eq!(order.name, "First order");
        assert_eq!(order.description, "rush");
        assert_eq!(order.properties["id"], PropertyValue::Text("O-1".into()));
        assert_eq!(order.properties["amount"], PropertyValue::Number(12.5));
        assert_eq!(
            order.properties["source"],
            PropertyValue::Text("crm/orders".into())
        );
        assert_eq!(order.properties["stale"], PropertyValue::Bool(false));
        assert!(
            !order.properties.contains_key("note"),
            "the description is not a property too"
        );
        assert!(
            matches!(&out[4], Record::NamedRelation { relation_type, source_name, target_name, .. }
            if relation_type == "placed_by" && source_name == "First order" && target_name == "C-7")
        );
        let Record::Concept(second) = &out[5] else {
            panic!("{:?}", out[5]);
        };
        assert_eq!(second.name, "O-2", "no label: the key is the name");
        assert_eq!(second.properties["amount"], PropertyValue::Bool(true));
        assert!(
            !second.properties.contains_key("customer"),
            "null is no property"
        );
        assert_eq!(out.len(), 6, "no link without a target value");
    }

    #[test]
    fn a_mapping_names_a_column_that_is_not_there() {
        let bad = Mapping {
            concept_type: "Order".into(),
            key_column: "nope".into(),
            ..Default::default()
        };
        assert!(RowSource::new(&rs(), &bad, false).is_err());
        let bad_prop = Mapping {
            concept_type: "Order".into(),
            key_column: "id".into(),
            property_columns: vec!["ghost".into()],
            ..Default::default()
        };
        assert!(RowSource::new(&rs(), &bad_prop, false).is_err());
        let ok = RowSource::new(
            &rs(),
            &Mapping {
                concept_type: "Order".into(),
                key_column: "id".into(),
                property_columns: vec!["amount".into()],
                ..Default::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(ok.len(), 2, "no declaration, two sheets");
        assert!(!ok.is_empty());
        assert_eq!(
            property(&json!({"a": 1})),
            Some(PropertyValue::Text("{\"a\":1}".into()))
        );
    }
}
