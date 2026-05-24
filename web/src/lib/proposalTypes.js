// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// TypeScript mirror of `crates/io/src/proposal.rs`. Keep these definitions
// in lockstep with the Rust types so the wizard can edit the JSON payload
// in place between `/ingest/analyze` and `/ingest/apply`.
/** Every client_ref present in the proposal, in declaration order. */
export function iterRefs(p) {
    return [
        ...p.concept_types.map((x) => x.client_ref),
        ...p.relation_types.map((x) => x.client_ref),
        ...p.concepts.map((x) => x.client_ref),
        ...p.relations.map((x) => x.client_ref),
        ...p.rules.map((x) => x.client_ref),
        ...p.actions.map((x) => x.client_ref),
    ];
}
