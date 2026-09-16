// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Per-record codec cost, the figure `STORAGE.md` §7.1 is built on: encode
//! and decode of a realistic ~1.3 KB concept and of a relation, JSON versus
//! postcard. Run with `cargo bench -p ontology-storage --bench codec`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ontology_graph::{Concept, ConceptId, PropertyValue, Relation, RelationId};
use ontology_storage::codec::{decode, encode};
use ontology_storage::{RecordKind, CODEC_JSON, CODEC_POSTCARD};

fn concept_1300b() -> RecordKind {
    let mut c = Concept::new(ConceptId(123_456), "Invoice", "INV-2025-000123")
        .with_description("Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(18));
    c.properties
        .insert("amount_eur".into(), PropertyValue::Number(18_000.5));
    c.properties.insert(
        "issue_date".into(),
        PropertyValue::Text("2025-03-31".into()),
    );
    c.properties
        .insert("paid".into(), PropertyValue::Bool(false));
    c.properties.insert(
        "tags".into(),
        PropertyValue::List(vec![
            PropertyValue::Text("q1".into()),
            PropertyValue::Text("pilot".into()),
        ]),
    );
    c.properties
        .insert("issued_by".into(), PropertyValue::Text("Acme Labs".into()));
    RecordKind::Concept(c)
}

fn relation() -> RecordKind {
    let mut r = Relation::new(RelationId(77), "issued_to", ConceptId(5), ConceptId(9));
    r.weight = 1.0;
    RecordKind::Relation(r)
}

fn bench_codecs(c: &mut Criterion) {
    for (label, rec) in [("concept_1300b", concept_1300b()), ("relation", relation())] {
        let mut g = c.benchmark_group(label);
        for (name, codec) in [("json", CODEC_JSON), ("postcard", CODEC_POSTCARD)] {
            let bytes = encode(codec, &rec).unwrap();
            g.throughput(Throughput::Bytes(bytes.len() as u64));
            g.bench_with_input(BenchmarkId::new("encode", name), &rec, |b, rec| {
                b.iter(|| encode(codec, black_box(rec)).unwrap())
            });
            g.bench_with_input(BenchmarkId::new("decode", name), &bytes, |b, bytes| {
                b.iter(|| decode(codec, black_box(bytes)).unwrap())
            });
        }
        g.finish();
    }
}

criterion_group!(benches, bench_codecs);
criterion_main!(benches);
