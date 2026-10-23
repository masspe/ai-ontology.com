// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Integration tests for the ABL source scanner (`scan_dir`, `AblSource`):
//! a real directory of `.p` / `.w` / `.i` fixtures, scanned and then
//! imported after an xref directory so `Table` and `Program` sheets exist
//! for the joins and screens to attach to.

mod hardening_common;

use hardening_common::{abl_fixtures_dir, xref_fixtures_dir, xref_roots, UPSERT};
use ontology_graph::{Ontology, OntologyGraph, PropertyValue};
use ontology_io::{
    ingest_records_with, scan_abl, scan_dir, AblFile, AblSource, ProgramKind, Record, Source,
    TableIndex, XrefFile, XrefSource,
};
use ontology_storage::FlakyStore;
use std::collections::BTreeSet;

fn core_tables() -> TableIndex {
    TableIndex::new(
        ["appdb.payvar", "appdb.schedule"]
            .iter()
            .map(|s| s.to_string()),
    )
}

fn text(s: &str) -> PropertyValue {
    PropertyValue::Text(s.to_string())
}

fn list(items: &[&str]) -> PropertyValue {
    PropertyValue::List(items.iter().map(|s| text(s)).collect())
}

/// `scan_dir` walks every `.p`/`.w`/`.i` fixture, drops the files that
/// carry no join or screen information, and reduces `pr/main.p` to the one
/// join pair it encodes (a buffer alias, `EQ` and `=` across lines, and the
/// implicit `FOR EACH ... OF ...`).
#[test]
fn scan_dir_reads_every_source_file() {
    let tables = core_tables();
    let scanned = scan_dir(&abl_fixtures_dir(), &tables, Some(2)).unwrap();

    assert_eq!(
        scanned.file_count, 5,
        "pr/main.p, pr/nested.p, pr/local.i, gl/b-gl100.w, gl/w-gl100.w"
    );
    assert!(scanned.errors.is_empty(), "{:?}", scanned.errors);

    let names: Vec<&str> = scanned.files.iter().map(|f| f.program.as_str()).collect();
    assert_eq!(
        names,
        ["gl/b-gl100.w", "gl/w-gl100.w", "pr/main.p"],
        "nested.p and local.i carry no join or screen info and are dropped"
    );
    for f in &scanned.files {
        assert_eq!(
            f.program,
            f.program.to_lowercase(),
            "program identity is lower-case"
        );
        assert!(
            !f.program.contains('\\'),
            "program identity uses / separators"
        );
    }

    let main = scanned
        .files
        .iter()
        .find(|f| f.program == "pr/main.p")
        .unwrap();
    let pair = ("appdb.payvar".to_string(), "appdb.schedule".to_string());
    let u = main
        .joins
        .get(&pair)
        .unwrap_or_else(|| panic!("no join for {pair:?}: {:?}", main.joins));
    assert_eq!(u.count, 2, "cono=cono (EQ) and period=period (=)");
    assert_eq!(u.implicit, 1, "FOR EACH schedule OF payvar");
    assert_eq!(u.keys.get("cono=cono"), Some(&1));
    assert_eq!(u.keys.get("period=period"), Some(&1));
    assert_eq!(main.joins.len(), 1, "{:?}", main.joins);
}

/// The full pipeline: an xref directory import creates the `Table` and
/// `Program` sheets, then the ABL import adds the `gl100` screen (folded
/// from its browser and window parts) and the `joined_on` link between the
/// two tables `pr/main.p` joins.
#[tokio::test]
async fn abl_import_adds_screens_and_joins_to_an_xref_graph() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();

    let (mut xref_src, _) =
        XrefSource::from_path(&xref_fixtures_dir(), &xref_roots(), Some(2)).unwrap();
    ingest_records_with(&mut xref_src, &graph, Some(&store), UPSERT)
        .await
        .unwrap();

    // Register `gl/w-gl100.w` as a compiled Program so one half of the
    // gl100 screen resolves to a real Program sheet while its browser
    // (`gl/b-gl100.w`, never compiled in this fixture set) does not.
    let mut extra = XrefSource::from_files(vec![XrefFile {
        program: "gl/w-gl100.w".to_string(),
        kind: Some(ProgramKind::Window),
        ..Default::default()
    }]);
    ingest_records_with(&mut extra, &graph, Some(&store), UPSERT)
        .await
        .unwrap();

    let tables: BTreeSet<String> = graph
        .concepts_of_type("Table", false)
        .into_iter()
        .map(|c| c.name)
        .collect();
    let programs: BTreeSet<String> = graph
        .concepts_of_type("Program", false)
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert!(programs.contains("gl/w-gl100.w"));
    assert!(!programs.contains("gl/b-gl100.w"));
    let table_index = TableIndex::new(tables);

    let (mut abl_src, scanned) =
        AblSource::from_dir(&abl_fixtures_dir(), &table_index, programs, Some(2)).unwrap();
    assert!(scanned.errors.is_empty(), "{:?}", scanned.errors);

    let stats = ingest_records_with(&mut abl_src, &graph, Some(&store), UPSERT)
        .await
        .unwrap();
    assert!(stats.concepts > 0 && stats.relations > 0, "{stats:?}");

    let screen_id = graph
        .find_by_name("Screen", "gl100")
        .expect("gl100 screen exists");
    let screen = graph.get_concept(screen_id).unwrap();
    assert_eq!(
        screen.properties.get("title"),
        Some(&text("gl100 - Grand livre"))
    );
    assert_eq!(
        screen.properties.get("tabs"),
        Some(&list(&["Liste", "Compte", "Cumuls"])),
        "browser tabs first, then the window's own"
    );
    assert_eq!(
        screen.properties.get("menu_items"),
        Some(&list(&["Masquer"]))
    );
    assert_eq!(
        screen.properties.get("programs"),
        Some(&list(&["gl/b-gl100.w", "gl/w-gl100.w"]))
    );

    let w_id = graph.find_by_name("Program", "gl/w-gl100.w").unwrap();
    assert!(
        graph
            .relation_between(screen_id, w_id, "implemented_by")
            .is_some(),
        "the window program is registered: implemented_by links to it"
    );
    assert!(
        graph.find_by_name("Program", "gl/b-gl100.w").is_none(),
        "the browser was never compiled: no Program sheet, so no edge to it either"
    );

    let payvar = graph.find_by_name("Table", "appdb.payvar").unwrap();
    let schedule = graph.find_by_name("Table", "appdb.schedule").unwrap();
    let rid = graph
        .relation_between(schedule, payvar, "joined_on")
        .expect("joined_on schedule -> payvar");
    let rel = graph.get_relation(rid).unwrap();
    assert_eq!(rel.weight, 3.0, "count(2) + implicit(1)");
    assert_eq!(
        rel.properties.get("keys"),
        Some(&list(&["cono=cono", "period=period"])),
        "most-used-first, ties broken alphabetically"
    );
    assert_eq!(
        rel.properties.get("programs"),
        Some(&PropertyValue::Number(1.0))
    );
    assert_eq!(
        rel.properties.get("implicit"),
        Some(&PropertyValue::Number(1.0))
    );

    let inv_id = graph
        .relation_between(payvar, schedule, "joined_on")
        .expect("the symmetric inverse exists");
    let inv = graph.get_relation(inv_id).unwrap();
    assert_eq!(inv.weight, rel.weight);
    assert_eq!(inv.properties, rel.properties);
}

/// A second run of the same ABL import over the same graph journals
/// nothing and changes no counts: the import is idempotent, like the xref
/// one it follows.
#[tokio::test]
async fn abl_import_is_idempotent() {
    let graph = OntologyGraph::with_arc(Ontology::new());
    let store = FlakyStore::new();
    let (mut xref_src, _) =
        XrefSource::from_path(&xref_fixtures_dir(), &xref_roots(), Some(2)).unwrap();
    ingest_records_with(&mut xref_src, &graph, Some(&store), UPSERT)
        .await
        .unwrap();

    let tables: BTreeSet<String> = graph
        .concepts_of_type("Table", false)
        .into_iter()
        .map(|c| c.name)
        .collect();
    let programs: BTreeSet<String> = graph
        .concepts_of_type("Program", false)
        .into_iter()
        .map(|c| c.name)
        .collect();
    let table_index = TableIndex::new(tables);

    let (mut first_src, _) =
        AblSource::from_dir(&abl_fixtures_dir(), &table_index, programs.clone(), Some(2)).unwrap();
    let first = ingest_records_with(&mut first_src, &graph, Some(&store), UPSERT)
        .await
        .unwrap();
    assert!(first.concepts > 0 || first.relations > 0, "{first:?}");
    let (c1, r1) = (graph.concept_count(), graph.relation_count());
    let written = store.records_written();

    let (mut second_src, _) =
        AblSource::from_dir(&abl_fixtures_dir(), &table_index, programs, Some(2)).unwrap();
    let second = ingest_records_with(&mut second_src, &graph, Some(&store), UPSERT)
        .await
        .unwrap();
    assert_eq!(
        (second.concepts, second.relations, second.updated),
        (0, 0, 0),
        "{second:?}"
    );
    assert_eq!((graph.concept_count(), graph.relation_count()), (c1, r1));
    assert_eq!(
        store.records_written(),
        written,
        "an identical re-import journals nothing"
    );
}

async fn drain(src: &mut AblSource) -> Vec<Record> {
    let mut out = Vec::new();
    while let Some(r) = src.next().await.unwrap() {
        out.push(r);
    }
    out
}

/// `group_screens`/`from_files` directly: a `.w` whose file name carries no
/// ERP code keeps its full path as the screen name; a file with neither
/// title, tabs nor menu produces no screen at all; `implemented_by` is
/// emitted only for the programs the caller says exist.
#[tokio::test]
async fn screens_without_a_code_keep_their_path() {
    let no_code = AblFile {
        program: "ba/w-menu.w".into(),
        title: Some("Menu".into()),
        ..Default::default()
    };
    let silent = AblFile {
        program: "ba/v-silent.w".into(),
        ..Default::default()
    };
    let coded = AblFile {
        program: "gl/w-ca200.w".into(),
        title: Some("ca200 - Autre ecran".into()),
        ..Default::default()
    };

    let mut programs = BTreeSet::new();
    programs.insert("ba/w-menu.w".to_string());
    // "gl/w-ca200.w" is deliberately left out of `programs`.

    let mut src = AblSource::from_files(vec![no_code, silent, coded], programs);
    let records = drain(&mut src).await;

    let screen_names: Vec<&str> = records
        .iter()
        .filter_map(|r| match r {
            Record::Concept(c) if c.concept_type == "Screen" => Some(c.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        screen_names.contains(&"ba/w-menu.w"),
        "no code in the file name: the path is the screen name; {screen_names:?}"
    );
    assert!(screen_names.contains(&"ca200"), "{screen_names:?}");
    assert!(
        !screen_names.contains(&"ba/v-silent.w"),
        "no title, tabs or menu: no screen at all; {screen_names:?}"
    );
    assert_eq!(screen_names.len(), 2);

    let implemented: Vec<(&str, &str)> = records
        .iter()
        .filter_map(|r| match r {
            Record::NamedRelation {
                relation_type,
                source_name,
                target_name,
                ..
            } if relation_type == "implemented_by" => {
                Some((source_name.as_str(), target_name.as_str()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        implemented,
        [("ba/w-menu.w", "ba/w-menu.w")],
        "only the program present in `programs` gets an implemented_by edge"
    );
}

/// `TableIndex::new`: a bare name shared by several databases resolves to
/// whichever database has more tables; a name with no `.` is accepted
/// without panicking; `is_empty` reports on the whole index. `resolve`
/// itself is private, so this is observed through `scan_abl`'s join
/// resolution rather than called directly.
#[test]
fn table_index_resolves_bare_names_by_database_majority() {
    let idx = TableIndex::new(
        ["dba.x", "dbz.x", "dbz.y", "dbz.z", "orphan"]
            .iter()
            .map(|s| s.to_string()),
    );
    assert!(!idx.is_empty());
    assert!(TableIndex::new(std::iter::empty()).is_empty());

    // "x" is ambiguous (both dba and dbz have it); dbz has more tables and
    // wins. "y" exists only in dbz. The join pair proves both bare names
    // resolved through the majority rule rather than being left unknown.
    let f = scan_abl("FIND a WHERE x.f EQ y.f NO-LOCK.", "pr/bare.p", &idx);
    let pair = ("dbz.x".to_string(), "dbz.y".to_string());
    assert_eq!(f.joins[&pair].keys.get("f=f"), Some(&1), "{:?}", f.joins);
}

/// Edge cases of `scan_abl`'s alias resolution: a buffer defined `FOR
/// TEMP-TABLE` shadows the alias itself; an alias whose target is
/// separately shadowed resolves to nothing; the join pair is sorted the
/// same way regardless of which table is named first; and two aliases of
/// the same table never produce a self-join.
#[test]
fn scan_abl_handles_buffer_for_temp_table_reversed_pairs_and_self_joins() {
    let tables = core_tables();

    let shadow = scan_abl(
        "DEFINE BUFFER tt-plan FOR TEMP-TABLE schedule NO-UNDO.\n\
         FIND x WHERE x.a EQ tt-plan.a NO-LOCK.",
        "pr/shadow.p",
        &tables,
    );
    assert!(shadow.joins.is_empty(), "{:?}", shadow.joins);

    let shadow2 = scan_abl(
        "DEFINE BUFFER b-plan FOR schedule.\n\
         DEFINE TEMP-TABLE schedule NO-UNDO FIELD x AS INTEGER.\n\
         FIND payvar WHERE payvar.a EQ b-plan.a NO-LOCK.",
        "pr/shadow2.p",
        &tables,
    );
    assert!(
        shadow2.joins.is_empty(),
        "an alias whose target is shadowed resolves to nothing: {:?}",
        shadow2.joins
    );

    let reversed = scan_abl(
        "FIND x WHERE schedule.a EQ payvar.a NO-LOCK.",
        "pr/reversed.p",
        &tables,
    );
    let pair = ("appdb.payvar".to_string(), "appdb.schedule".to_string());
    assert_eq!(
        reversed.joins[&pair].keys.get("a=a"),
        Some(&1),
        "{:?}",
        reversed.joins
    );

    let self_join = scan_abl(
        "DEFINE BUFFER b-payvar FOR payvar.\n\
         FIND x WHERE payvar.a EQ b-payvar.b NO-LOCK.",
        "pr/self.p",
        &tables,
    );
    assert!(
        self_join.joins.is_empty(),
        "two aliases of the same table are not a join: {:?}",
        self_join.joins
    );
}

/// A join text buried in a correctly nested comment never surfaces, and a
/// pair where one side is a shadowed temp-table or a table the schema does
/// not know never becomes a `joined_on` link.
#[test]
fn unknown_tables_and_shadowed_names_produce_no_joins() {
    let tables = core_tables();

    let nested = std::fs::read_to_string(abl_fixtures_dir().join("pr/nested.p")).unwrap();
    let f = scan_abl(&nested, "pr/nested.p", &tables);
    assert!(
        f.joins.is_empty(),
        "the nested comment hides the join text entirely: {:?}",
        f.joins
    );

    let local = std::fs::read_to_string(abl_fixtures_dir().join("pr/local.i")).unwrap();
    let f2 = scan_abl(&local, "pr/local.i", &tables);
    assert!(
        f2.joins.is_empty(),
        "a shadowed temp-table and an unknown table both resolve to nothing: {:?}",
        f2.joins
    );
}
