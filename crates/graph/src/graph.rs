use ahash::AHashMap;
use dashmap::DashMap;
use parking_lot::RwLock;
use smallvec::SmallVec;
use std::sync::Arc;

use crate::error::{GraphError, GraphResult};
use crate::id::{ConceptId, IdAllocator, RelationId};
use crate::model::{Concept, Relation};
use crate::schema::Ontology;

type AdjList = SmallVec<[RelationId; 4]>;

/// In-memory ontology graph. Built for high read concurrency: lookups go
/// through `DashMap`s (sharded, lock-free reads) while edge index updates
/// take a single short write lock.
#[derive(Debug)]
pub struct OntologyGraph {
    ontology: RwLock<Ontology>,
    concepts: DashMap<ConceptId, Concept>,
    relations: DashMap<RelationId, Relation>,
    /// (concept_type, lowercased name) -> id, for natural-language lookup.
    name_index: DashMap<(String, String), ConceptId>,
    out_edges: DashMap<ConceptId, AdjList>,
    in_edges: DashMap<ConceptId, AdjList>,
    ids: IdAllocator,
}

impl OntologyGraph {
    pub fn new(ontology: Ontology) -> Self {
        Self {
            ontology: RwLock::new(ontology),
            concepts: DashMap::new(),
            relations: DashMap::new(),
            name_index: DashMap::new(),
            out_edges: DashMap::new(),
            in_edges: DashMap::new(),
            ids: IdAllocator::new(1),
        }
    }

    pub fn with_arc(ontology: Ontology) -> Arc<Self> { Arc::new(Self::new(ontology)) }

    pub fn ontology(&self) -> Ontology { self.ontology.read().clone() }

    pub fn extend_ontology<F>(&self, f: F) -> GraphResult<()>
    where F: FnOnce(&mut Ontology) -> GraphResult<()>,
    {
        let mut g = self.ontology.write();
        f(&mut g)
    }

    // ---------- concepts ----------

    /// Insert a concept. Allocates an id if `concept.id == ConceptId(0)`.
    pub fn upsert_concept(&self, mut concept: Concept) -> GraphResult<ConceptId> {
        {
            let onto = self.ontology.read();
            let ct = onto.concept_type(&concept.concept_type)?;
            if let Some(allowed) = &ct.properties {
                for k in concept.properties.keys() {
                    if !allowed.iter().any(|a| a == k) {
                        return Err(GraphError::InvalidProperty {
                            property: k.clone(),
                            concept_type: ct.name.clone(),
                        });
                    }
                }
            }
        }
        if concept.id.0 == 0 {
            concept.id = self.ids.next_concept();
        } else {
            self.ids.observe(concept.id.0);
        }
        let key = (concept.concept_type.clone(), concept.name.to_lowercase());
        if let Some(existing) = self.name_index.get(&key) {
            if *existing != concept.id {
                return Err(GraphError::DuplicateConcept(
                    concept.name.clone(),
                    concept.concept_type.clone(),
                ));
            }
        }
        self.name_index.insert(key, concept.id);
        let id = concept.id;
        self.concepts.insert(id, concept);
        Ok(id)
    }

    pub fn get_concept(&self, id: ConceptId) -> GraphResult<Concept> {
        self.concepts
            .get(&id)
            .map(|c| c.clone())
            .ok_or(GraphError::UnknownConcept(id))
    }

    pub fn find_by_name(&self, concept_type: &str, name: &str) -> Option<ConceptId> {
        self.name_index
            .get(&(concept_type.to_string(), name.to_lowercase()))
            .map(|v| *v)
    }

    pub fn concept_count(&self) -> usize { self.concepts.len() }
    pub fn relation_count(&self) -> usize { self.relations.len() }

    pub fn all_concepts(&self) -> Vec<Concept> {
        self.concepts.iter().map(|e| e.value().clone()).collect()
    }

    // ---------- relations ----------

    pub fn add_relation(&self, mut rel: Relation) -> GraphResult<RelationId> {
        let src = self.concepts.get(&rel.source).ok_or(GraphError::UnknownConcept(rel.source))?;
        let tgt = self.concepts.get(&rel.target).ok_or(GraphError::UnknownConcept(rel.target))?;
        {
            let onto = self.ontology.read();
            onto.validate_edge(&rel.relation_type, &src.concept_type, &tgt.concept_type)?;
        }
        let symmetric = self
            .ontology
            .read()
            .relation_type(&rel.relation_type)
            .map(|rt| rt.symmetric)
            .unwrap_or(false);

        if rel.id.0 == 0 {
            rel.id = self.ids.next_relation();
        } else {
            self.ids.observe(rel.id.0);
        }
        drop(src); drop(tgt);

        let id = rel.id;
        let (s, t) = (rel.source, rel.target);
        self.out_edges.entry(s).or_default().push(id);
        self.in_edges.entry(t).or_default().push(id);
        self.relations.insert(id, rel);

        if symmetric && s != t {
            // Materialize the inverse so traversals are direction-agnostic.
            let inverse = Relation {
                id: self.ids.next_relation(),
                relation_type: self.relations.get(&id).unwrap().relation_type.clone(),
                source: t,
                target: s,
                weight: self.relations.get(&id).unwrap().weight,
                properties: AHashMap::new(),
            };
            let inv_id = inverse.id;
            self.out_edges.entry(t).or_default().push(inv_id);
            self.in_edges.entry(s).or_default().push(inv_id);
            self.relations.insert(inv_id, inverse);
        }
        Ok(id)
    }

    pub fn get_relation(&self, id: RelationId) -> GraphResult<Relation> {
        self.relations
            .get(&id)
            .map(|r| r.clone())
            .ok_or(GraphError::UnknownRelation(id))
    }

    pub fn outgoing(&self, id: ConceptId) -> Vec<Relation> {
        self.out_edges
            .get(&id)
            .map(|adj| {
                adj.iter()
                    .filter_map(|rid| self.relations.get(rid).map(|r| r.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn incoming(&self, id: ConceptId) -> Vec<Relation> {
        self.in_edges
            .get(&id)
            .map(|adj| {
                adj.iter()
                    .filter_map(|rid| self.relations.get(rid).map(|r| r.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Concept;
    use crate::schema::{ConceptType, RelationType};

    fn toy_ontology() -> Ontology {
        let mut o = Ontology::new();
        o.add_concept_type(ConceptType {
            name: "Person".into(), parent: None,
            properties: None, description: "a human".into(),
        });
        o.add_concept_type(ConceptType {
            name: "Paper".into(), parent: None,
            properties: None, description: "research paper".into(),
        });
        o.add_relation_type(RelationType {
            name: "authored".into(),
            domain: "Person".into(),
            range: "Paper".into(),
            cardinality: Default::default(),
            symmetric: false,
            description: "authorship".into(),
        }).unwrap();
        o
    }

    #[test]
    fn insert_and_traverse() {
        let g = OntologyGraph::new(toy_ontology());
        let alice = g.upsert_concept(Concept::new(Default::default(), "Person", "Alice")).unwrap();
        let paper = g.upsert_concept(Concept::new(Default::default(), "Paper", "On RAG")).unwrap();
        g.add_relation(Relation::new(Default::default(), "authored", alice, paper)).unwrap();
        assert_eq!(g.outgoing(alice).len(), 1);
        assert_eq!(g.incoming(paper).len(), 1);
    }

    #[test]
    fn schema_violation_rejected() {
        let g = OntologyGraph::new(toy_ontology());
        let a = g.upsert_concept(Concept::new(Default::default(), "Paper", "P1")).unwrap();
        let b = g.upsert_concept(Concept::new(Default::default(), "Paper", "P2")).unwrap();
        let res = g.add_relation(Relation::new(Default::default(), "authored", a, b));
        assert!(res.is_err());
    }
}
