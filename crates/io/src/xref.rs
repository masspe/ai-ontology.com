// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Progress OpenEdge `COMPILE … XREF-XML` reader.
//!
//! One `.xref.xml` describes one compiled program: a `<Source>` per file
//! that took part in the compile (the program, then each include), and a
//! flat list of `<Reference Reference-type=… Object-identifier=…>` with an
//! `<Object-context>` child. The file is read as a stream, one event at a
//! time, and reduced to an [`XrefFile`] summary: per table how the program
//! touches it, which programs it runs, which includes it pulls in. The raw
//! references (millions over a code base) never exist in memory at once.
//!
//! The full ontology (sheets and links) is built from these summaries by
//! the import; this module only parses and aggregates.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use std::path::Path;

/// What kind of compile unit the main `<Source>` is, by extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramKind {
    /// `.p`
    Procedure,
    /// `.w` (GUI window / SmartObject)
    Window,
    /// `.cls`
    Class,
    /// `.i` compiled on its own, or anything else
    Other,
}

impl ProgramKind {
    fn of(path: &str) -> Self {
        match path.rsplit('.').next() {
            Some("p") => Self::Procedure,
            Some("w") => Self::Window,
            Some("cls") => Self::Class,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Procedure => "procedure",
            Self::Window => "window",
            Self::Class => "class",
            Self::Other => "other",
        }
    }
}

/// How one program uses one database table, summed over the file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TableUse {
    /// `ACCESS`, `REFERENCE`, `SEARCH`, `SORT-ACCESS`.
    pub reads: u32,
    /// `UPDATE`.
    pub updates: u32,
    /// `CREATE`.
    pub creates: u32,
    /// `DELETE`.
    pub deletes: u32,
    /// Fields named by `ACCESS` references.
    pub fields_read: BTreeSet<String>,
    /// Fields named by `UPDATE` references.
    pub fields_updated: BTreeSet<String>,
}

/// The summary of one `.xref.xml`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct XrefFile {
    /// Normalised path of the main `<Source>` (see [`normalize_path`]).
    pub program: String,
    pub kind: Option<ProgramKind>,
    /// Fully qualified class name when the unit is a `.cls` (`CLASS` ref).
    pub class: Option<String>,
    /// Normalised include paths (`INCLUDE`).
    pub includes: BTreeSet<String>,
    /// `RUN <file>` targets (normalised path → count).
    pub runs: BTreeMap<String, u32>,
    /// `RUN <internal-procedure>` targets (bare name → count).
    pub calls: BTreeMap<String, u32>,
    /// `RUN VALUE(expr)`: targets unknown at compile time.
    pub dynamic_runs: u32,
    /// Classes used through `INVOKE`, `NEW`, `CAST` (class name → count).
    pub invokes: BTreeMap<String, u32>,
    /// Internal procedures the program defines (`PROCEDURE`).
    pub procedures: BTreeSet<String>,
    /// User-defined functions the program defines (`FUNCTION`).
    pub functions: BTreeSet<String>,
    /// `db.table` → use.
    pub tables: BTreeMap<String, TableUse>,
    /// `STRING` references (literals), counted only.
    pub strings: u32,
    /// Every `<Reference>` seen.
    pub references: u32,
}

/// Lower-cases, turns `\` into `/`, drops `./`, and strips the first of
/// `roots` that prefixes the path (compared case-insensitively, `\` or
/// `/`). Programs compiled from `D:\App\ERP\sd\x.p` and run as `sd/x.p`
/// then share one identity.
pub fn normalize_path(raw: &str, roots: &[String]) -> String {
    let mut p = raw.replace('\\', "/").to_lowercase();
    for root in roots {
        let r = root.replace('\\', "/").to_lowercase();
        let r = r.trim_end_matches('/');
        if let Some(rest) = p.strip_prefix(r) {
            if let Some(rest) = rest.strip_prefix('/') {
                p = rest.to_string();
                break;
            }
        }
    }
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    p
}

/// `db.table` when `s` names a database table: exactly one `.`, no `:`
/// (class members), no space (parameter signatures). Temp-tables have no
/// database prefix and are left out, as are .NET types (`System.Object`
/// has one dot: the known database names are checked by the import, not
/// here — the parser does not know the schema).
fn table_name(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    if trimmed.is_empty()
        || trimmed.contains(':')
        || trimmed.contains(' ')
        || trimmed.matches('.').count() != 1
    {
        return None;
    }
    Some(trimmed)
}

fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.as_ref() != key {
            return None;
        }
        // Paths and identifiers carry at most `&amp;`; strings are dropped
        // anyway, so a failed unescape keeps the raw text.
        Some(
            quick_xml::escape::unescape(&a.value)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| a.value.to_string()),
        )
    })
}

fn bump(map: &mut BTreeMap<String, u32>, key: String) {
    *map.entry(key).or_insert(0) += 1;
}

/// Parse one xref document from a buffered reader.
pub fn parse_xref<R: BufRead>(input: R, roots: &[String]) -> Result<XrefFile, String> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(true);
    let mut out = XrefFile::default();
    let mut buf = Vec::with_capacity(4096);

    // The <Reference> being read: its type, identifier and context.
    let mut ref_type: Option<String> = None;
    let mut ref_ident: Option<String> = None;
    let mut context = String::new();
    let mut in_context = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                "Source" if out.program.is_empty() => {
                    if let Some(f) = attr(&e, "File-name") {
                        out.program = normalize_path(&f, roots);
                        out.kind = Some(ProgramKind::of(&out.program));
                    }
                }
                "Reference" => {
                    ref_type = attr(&e, "Reference-type");
                    ref_ident = attr(&e, "Object-identifier");
                    context.clear();
                    out.references += 1;
                }
                "Object-context" if ref_type.is_some() => in_context = true,
                _ => {}
            },
            Ok(Event::Text(t)) if in_context => {
                context.push_str(&t.xml10_content());
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                "Object-context" => in_context = false,
                "Reference" => {
                    if let (Some(ty), Some(id)) = (ref_type.take(), ref_ident.take()) {
                        record(&mut out, &ty, id, &context, roots);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(format!(
                    "xml parse at byte {}: {e}",
                    reader.buffer_position()
                ))
            }
            _ => {}
        }
        buf.clear();
    }
    if out.program.is_empty() {
        return Err("no <Source> element".into());
    }
    Ok(out)
}

/// Parse the file at `path`.
pub fn parse_xref_file(path: &Path, roots: &[String]) -> Result<XrefFile, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    parse_xref(std::io::BufReader::with_capacity(1 << 16, f), roots)
}

fn record(out: &mut XrefFile, ty: &str, id: String, context: &str, roots: &[String]) {
    match ty {
        // Field-level: identifier is the field, context the table.
        "ACCESS" | "UPDATE" => {
            if let Some(t) = table_name(context) {
                let u = out.tables.entry(t.to_string()).or_default();
                if ty == "ACCESS" {
                    u.reads += 1;
                    u.fields_read.insert(id);
                } else {
                    u.updates += 1;
                    u.fields_updated.insert(id);
                }
            }
        }
        // Table-level: identifier is the table.
        "REFERENCE" | "SEARCH" | "SORT-ACCESS" | "CREATE" | "DELETE" => {
            if let Some(t) = table_name(&id) {
                let u = out.tables.entry(t.to_string()).or_default();
                match ty {
                    "CREATE" => u.creates += 1,
                    "DELETE" => u.deletes += 1,
                    _ => u.reads += 1,
                }
            }
        }
        "RUN" => {
            let lower = id.to_ascii_lowercase();
            if lower.starts_with("value(") {
                out.dynamic_runs += 1;
            } else if lower.ends_with(".p")
                || lower.ends_with(".w")
                || lower.ends_with(".r")
                || lower.ends_with(".cls")
            {
                bump(&mut out.runs, normalize_path(&id, roots));
            } else {
                bump(&mut out.calls, lower);
            }
        }
        "INCLUDE" => {
            out.includes.insert(normalize_path(&id, roots));
        }
        "PROCEDURE" => {
            out.procedures.insert(id.to_ascii_lowercase());
        }
        "FUNCTION" => {
            out.functions.insert(id.to_ascii_lowercase());
        }
        "INVOKE" | "NEW" | "CAST" => {
            let class = id.split(':').next().unwrap_or(&id).to_string();
            bump(&mut out.invokes, class);
        }
        "CLASS" => out.class = Some(id),
        "STRING" => out.strings += 1,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(ty: &str, id: &str, ctx: &str) -> String {
        format!(
            r#"<Reference Reference-type="{ty}" Object-identifier="{id}">
  <Source-guid>g</Source-guid><File-num>1</File-num><Ref-seq>1</Ref-seq><Line-num>3</Line-num>
  <Object-context>{ctx}</Object-context><Access-mode/><Detail/>
</Reference>"#
        )
    }

    fn doc(refs: &[String]) -> String {
        format!(
            r#"<?xml version="1.0"?>
<Cross-reference xmlns="uri:schemas-progress-com:XREFD:0005">
  <Source File-name="D:\Mediasoft\App\MS.ERP2\sd\Pcto.p"><Source-guid>g</Source-guid><File-num>1</File-num>
  {}
  </Source>
  <Source File-name=".\sd\inc\tt.i"><Source-guid>g</Source-guid><File-num>2</File-num></Source>
</Cross-reference>"#,
            refs.join("\n")
        )
    }

    #[test]
    fn summarises_one_program() {
        let roots = vec!["D:\\Mediasoft\\App\\MS.ERP2".to_string()];
        let xml = doc(&[
            reference("COMPILE", "D:\\Mediasoft\\App\\MS.ERP2\\sd\\Pcto.p", ""),
            reference("STRING", "Soci&amp;t&#233;", ""),
            reference("ACCESS", "nste", "msdb.sdva"),
            reference("ACCESS", "peri", "msdb.sdva"),
            reference("UPDATE", "nste", "msdb.sdva"),
            reference("ACCESS", "pAppl", "PUBLIC-PROPERTY"),
            reference("SEARCH", "msdb.batc", ""),
            reference("SEARCH", "Net.sd.Gui.Planning:ttSDBA", ""),
            reference("CREATE", "msdb.planning", ""),
            reference("DELETE", "msdb.planning", ""),
            reference("RUN", "sd/pcto2.p", ""),
            reference("RUN", "Net\\Sd\\Proc\\DrVacanGZ.p", ""),
            reference("RUN", "dispatch", ""),
            reference("RUN", "VALUE(cProg)", ""),
            reference("INCLUDE", "net/sd/inc/tt/ttsdba.i", ""),
            reference("PROCEDURE", "adm-create-controls", ""),
            reference("FUNCTION", "fGetDate", ""),
            reference("INVOKE", "Net.Class.Config:Get", ""),
            reference("NEW", "Net.Class.Config", ""),
        ]);
        let f = parse_xref(xml.as_bytes(), &roots).unwrap();

        assert_eq!(f.program, "sd/pcto.p");
        assert_eq!(f.kind, Some(ProgramKind::Procedure));
        assert_eq!(f.references, 19);
        assert_eq!(f.strings, 1);

        let sdva = &f.tables["msdb.sdva"];
        assert_eq!((sdva.reads, sdva.updates), (2, 1));
        assert_eq!(sdva.fields_read.len(), 2);
        assert!(sdva.fields_updated.contains("nste"));
        assert_eq!(f.tables["msdb.batc"].reads, 1);
        let planning = &f.tables["msdb.planning"];
        assert_eq!((planning.creates, planning.deletes), (1, 1));
        assert_eq!(
            f.tables.len(),
            3,
            "class property and temp-table are not tables"
        );

        assert_eq!(f.runs["sd/pcto2.p"], 1);
        assert_eq!(f.runs["net/sd/proc/drvacangz.p"], 1);
        assert_eq!(f.calls["dispatch"], 1);
        assert_eq!(f.dynamic_runs, 1);
        assert!(f.includes.contains("net/sd/inc/tt/ttsdba.i"));
        assert!(f.procedures.contains("adm-create-controls"));
        assert!(f.functions.contains("fgetdate"));
        assert_eq!(f.invokes["Net.Class.Config"], 2);
    }

    #[test]
    fn normalises_paths() {
        let roots = vec![
            "d:/mediasoft/app/ms.erp/".to_string(),
            "D:\\Mediasoft\\App\\MS.ERP2".to_string(),
        ];
        assert_eq!(
            normalize_path("D:\\Mediasoft\\App\\MS.ERP2\\Sd\\X.P", &roots),
            "sd/x.p"
        );
        assert_eq!(
            normalize_path("D:\\Mediasoft\\App\\MS.ERP\\sd\\x.p", &roots),
            "sd/x.p"
        );
        assert_eq!(
            normalize_path(".\\net\\web\\inc\\WebConfig.i", &roots),
            "net/web/inc/webconfig.i"
        );
        assert_eq!(normalize_path("sd/x.p", &roots), "sd/x.p");
        // A root that is only a prefix of a segment does not match.
        assert_eq!(
            normalize_path("D:\\Mediasoft\\App\\MS.ERP2x\\y.p", &roots),
            "d:/mediasoft/app/ms.erp2x/y.p"
        );
    }

    #[test]
    fn rejects_documents_without_source() {
        assert!(parse_xref(
            br#"<?xml version="1.0"?><Cross-reference/>"#.as_slice(),
            &[]
        )
        .is_err());
        assert!(parse_xref(b"<Cross-reference><Source".as_slice(), &[]).is_err());
    }
}
