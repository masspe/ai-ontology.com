// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// TypeScript mirror of `crates/io/src/proposal.rs`. Keep these definitions
// in lockstep with the Rust types so the wizard can edit the JSON payload
// in place between `/ingest/analyze` and `/ingest/apply`.

export type DecisionAction = "merge" | "create_new" | "skip";

export interface LangTag {
  code: string;
  script: string;
  confidence: number;
}

export interface ProposalSource {
  name: string;
  kind?: string;
  encoding?: string;
  had_bom?: boolean;
  provider?: string;
  model?: string;
}

export type ConflictKind =
  | { kind: "exists"; existing_id: string; existing_display: string }
  | { kind: "type_mismatch"; existing_type: string; existing_id: string }
  | { kind: "dangling_ref"; missing_ref: string };

export interface ConflictInfo {
  kind: ConflictKind;
  summary: string;
}

export interface ProposalConceptType {
  client_ref: string;
  name: string;
  description?: string;
  properties?: string[];
  parent?: string | null;
  confidence?: number;
  conflict?: ConflictInfo | null;
}

export interface ProposalRelationType {
  client_ref: string;
  name: string;
  domain: string;
  range: string;
  symmetric?: boolean;
  description?: string;
  confidence?: number;
  conflict?: ConflictInfo | null;
}

export interface ProposalConcept {
  client_ref: string;
  concept_type: string;
  name: string;
  description?: string;
  properties?: [string, string][];
  evidence?: string | null;
  confidence?: number;
  conflict?: ConflictInfo | null;
}

export interface ProposalRelation {
  client_ref: string;
  relation_type: string;
  source_ref: string;
  target_ref: string;
  weight?: number | null;
  evidence?: string | null;
  confidence?: number;
  conflict?: ConflictInfo | null;
}

export interface ProposalRule {
  client_ref: string;
  rule_type: string;
  name: string;
  when?: string;
  then?: string;
  applies_to?: string[];
  strict?: boolean;
  description?: string;
  evidence?: string | null;
  confidence?: number;
  conflict?: ConflictInfo | null;
}

export interface ProposalAction {
  client_ref: string;
  action_type: string;
  name: string;
  subject_ref: string;
  object_ref?: string | null;
  parameters?: [string, string][];
  effect?: string;
  description?: string;
  evidence?: string | null;
  confidence?: number;
  conflict?: ConflictInfo | null;
}

export interface OntologyProposal {
  source?: ProposalSource | null;
  language?: LangTag | null;
  concept_types: ProposalConceptType[];
  relation_types: ProposalRelationType[];
  concepts: ProposalConcept[];
  relations: ProposalRelation[];
  rules: ProposalRule[];
  actions: ProposalAction[];
}

export interface ApplyDecision {
  client_ref: string;
  action: DecisionAction;
}

export type ApplyOutcome =
  | { status: "created"; id: string }
  | { status: "merged"; id: string }
  | { status: "skipped" }
  | { status: "failed"; error: string };

export interface ApplyReport {
  concept_types: [string, ApplyOutcome][];
  relation_types: [string, ApplyOutcome][];
  concepts: [string, ApplyOutcome][];
  relations: [string, ApplyOutcome][];
  rules: [string, ApplyOutcome][];
  actions: [string, ApplyOutcome][];
  created: number;
  merged: number;
  skipped: number;
  failed: number;
}

/** Every client_ref present in the proposal, in declaration order. */
export function iterRefs(p: OntologyProposal): string[] {
  return [
    ...p.concept_types.map((x) => x.client_ref),
    ...p.relation_types.map((x) => x.client_ref),
    ...p.concepts.map((x) => x.client_ref),
    ...p.relations.map((x) => x.client_ref),
    ...p.rules.map((x) => x.client_ref),
    ...p.actions.map((x) => x.client_ref),
  ];
}
