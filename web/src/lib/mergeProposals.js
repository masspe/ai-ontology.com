// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Merge multiple per-file `OntologyProposal`s (each obtained from
// `POST /ingest/analyze`) into a single proposal that can be reviewed and
// applied in one shot by the `/builder` page.
//
// Steps:
// 1. Prefix every `client_ref` with `f${index}/` so refs from different
//    files never collide.
// 2. Rewrite intra-proposal references (`source_ref`, `target_ref`,
//    `subject_ref`, `object_ref`, `applies_to`) so they point at the new
//    prefixed refs. References of the form `"Type:Name"` (which target
//    pre-existing graph concepts) are left untouched.
// 3. Deduplicate by natural identity (concept (type+name), concept-type
//    name, relation-type name, relation (type+source+target), …),
//    keeping the first occurrence and remapping references from
//    subsequent files to the canonical ref.
/** Reference field shape: looks like `Type:Name` (graph ref) when it
 * contains a single `:` and neither side starts with `f<digits>/`. */
function isGraphRef(ref) {
    if (!ref)
        return false;
    if (ref.startsWith("f") && /^f\d+\//.test(ref))
        return false;
    return ref.includes(":");
}
/** Rewrite a single ref using a same-file map. Pass-through for graph refs
 * or refs not present in the map. */
function remap(ref, localMap) {
    const mapped = localMap.get(ref);
    return mapped ?? ref;
}
/** Apply the global canonicalization map (used for cross-file dedupe). */
function canonicalize(ref, canonical) {
    // Walk the chain in case of A→B→C (not expected but cheap to support).
    let cur = ref;
    for (let i = 0; i < 4; i++) {
        const next = canonical.get(cur);
        if (!next || next === cur)
            return cur;
        cur = next;
    }
    return cur;
}
/** Merge `parts` into one proposal. */
export function mergeProposals(parts) {
    const concept_types = [];
    const relation_types = [];
    const concepts = [];
    const relations = [];
    const rules = [];
    const actions = [];
    /** Maps a non-canonical client_ref to the canonical one (chosen on first
     * occurrence). Used to remap later items' refs. */
    const canonical = new Map();
    // Natural-identity indices for dedupe.
    const ctByName = new Map();
    const rtByName = new Map();
    const conceptByKey = new Map(); // `${type}::${name}`
    const relationByKey = new Map(); // `${type}::${src}::${tgt}` post-remap
    const ruleByKey = new Map();
    const actionByKey = new Map();
    // Track encodings across parts to surface a meaningful `source`.
    const encodings = new Set();
    const languages = new Set();
    let firstLanguage = null;
    parts.forEach((part, idx) => {
        const prefix = `f${idx}/`;
        const p = part.proposal;
        // Build per-file map of original→prefixed refs so we can rewrite
        // intra-proposal references safely.
        const localMap = new Map();
        const collect = (refs) => {
            for (const it of refs)
                localMap.set(it.client_ref, prefix + it.client_ref);
        };
        collect(p.concept_types);
        collect(p.relation_types);
        collect(p.concepts);
        collect(p.relations);
        collect(p.rules);
        collect(p.actions);
        // Helper to rewrite refs: local-prefix first, then canonicalize.
        const rewrite = (ref) => {
            if (isGraphRef(ref))
                return ref;
            const local = remap(ref, localMap);
            return canonicalize(local, canonical);
        };
        // ---- concept types ----
        for (const ct of p.concept_types) {
            const newRef = prefix + ct.client_ref;
            const dedupeKey = ct.name;
            const existing = ctByName.get(dedupeKey);
            if (existing) {
                canonical.set(newRef, existing);
                continue;
            }
            ctByName.set(dedupeKey, newRef);
            concept_types.push({ ...ct, client_ref: newRef });
        }
        // ---- relation types ----
        for (const rt of p.relation_types) {
            const newRef = prefix + rt.client_ref;
            const dedupeKey = rt.name;
            const existing = rtByName.get(dedupeKey);
            if (existing) {
                canonical.set(newRef, existing);
                continue;
            }
            rtByName.set(dedupeKey, newRef);
            relation_types.push({ ...rt, client_ref: newRef });
        }
        // ---- concepts ----
        for (const c of p.concepts) {
            const newRef = prefix + c.client_ref;
            const dedupeKey = `${c.concept_type}::${c.name.trim().toLowerCase()}`;
            const existing = conceptByKey.get(dedupeKey);
            if (existing) {
                canonical.set(newRef, existing);
                continue;
            }
            conceptByKey.set(dedupeKey, newRef);
            concepts.push({ ...c, client_ref: newRef });
        }
        // ---- relations ----
        for (const r of p.relations) {
            const newRef = prefix + r.client_ref;
            const src = rewrite(r.source_ref);
            const tgt = rewrite(r.target_ref);
            const dedupeKey = `${r.relation_type}::${src}::${tgt}`;
            const existing = relationByKey.get(dedupeKey);
            if (existing) {
                canonical.set(newRef, existing);
                continue;
            }
            relationByKey.set(dedupeKey, newRef);
            relations.push({
                ...r,
                client_ref: newRef,
                source_ref: src,
                target_ref: tgt,
            });
        }
        // ---- rules ----
        for (const ru of p.rules) {
            const newRef = prefix + ru.client_ref;
            const dedupeKey = `${ru.rule_type}::${ru.name}`;
            const existing = ruleByKey.get(dedupeKey);
            if (existing) {
                canonical.set(newRef, existing);
                continue;
            }
            ruleByKey.set(dedupeKey, newRef);
            rules.push({
                ...ru,
                client_ref: newRef,
                applies_to: ru.applies_to?.map(rewrite),
            });
        }
        // ---- actions ----
        for (const a of p.actions) {
            const newRef = prefix + a.client_ref;
            const subj = rewrite(a.subject_ref);
            const obj = a.object_ref ? rewrite(a.object_ref) : a.object_ref;
            const dedupeKey = `${a.action_type}::${a.name}`;
            const existing = actionByKey.get(dedupeKey);
            if (existing) {
                canonical.set(newRef, existing);
                continue;
            }
            actionByKey.set(dedupeKey, newRef);
            actions.push({
                ...a,
                client_ref: newRef,
                subject_ref: subj,
                object_ref: obj,
            });
        }
        if (p.source?.encoding)
            encodings.add(p.source.encoding);
        if (p.language) {
            languages.add(p.language.code);
            if (!firstLanguage)
                firstLanguage = p.language;
        }
    });
    // Some relations/rules/actions kept above may still reference a ref
    // that was later canonicalized (canonical entries added during the same
    // file iteration are picked up by `rewrite`, but if file B added a
    // canonical mapping, items from file A pushed earlier may still hold
    // the pre-canonical ref). Run a final canonicalization pass.
    const fixRel = (r) => ({
        ...r,
        source_ref: isGraphRef(r.source_ref) ? r.source_ref : canonicalize(r.source_ref, canonical),
        target_ref: isGraphRef(r.target_ref) ? r.target_ref : canonicalize(r.target_ref, canonical),
    });
    const fixRule = (r) => ({
        ...r,
        applies_to: r.applies_to?.map((x) => isGraphRef(x) ? x : canonicalize(x, canonical)),
    });
    const fixAction = (a) => ({
        ...a,
        subject_ref: isGraphRef(a.subject_ref)
            ? a.subject_ref
            : canonicalize(a.subject_ref, canonical),
        object_ref: a.object_ref
            ? isGraphRef(a.object_ref)
                ? a.object_ref
                : canonicalize(a.object_ref, canonical)
            : a.object_ref,
    });
    const source = {
        name: parts.length === 1
            ? parts[0].file
            : `${parts.length} files (${parts
                .slice(0, 3)
                .map((p) => p.file)
                .join(", ")}${parts.length > 3 ? ", …" : ""})`,
        encoding: encodings.size === 0
            ? undefined
            : encodings.size === 1
                ? [...encodings][0]
                : "mixed",
    };
    return {
        source,
        language: firstLanguage,
        concept_types,
        relation_types,
        concepts,
        relations: relations.map(fixRel),
        rules: rules.map(fixRule),
        actions: actions.map(fixAction),
    };
}
