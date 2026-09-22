# Performance — règles, optimisations et hypothèses

Ce document décrit l'ensemble des optimisations de lecture mises en place sur
`OntologyGraph` et la couche HTTP, les **règles** auxquelles elles obéissent,
et les **hypothèses** sous-jacentes. Il sert de référence pour comprendre
pourquoi les choses sont structurées ainsi et pour éviter de casser les
invariants en ajoutant du code.

Document compagnon : [`STORAGE.md`](./STORAGE.md) couvre la couche
**stockage** (format binaire, partitionnement par domaine `ns`, hydratation,
recovery, mémoire contrainte) et continue la numérotation ici — H11+ et R7+.
Le plan d'implémentation correspondant est dans
[`STORAGE-PLAN.md`](./STORAGE-PLAN.md).

> **Mise à jour du 2026-09-08.** `STORAGE.md` lève le non-objectif
> « persistance » (§1) et assouplit H1 (§2). Tout le reste de ce document
> reste valide ; les points touchés sont signalés en ligne par un renvoi
> « → STORAGE.md ».

---

## 1. Objectifs et non-objectifs

### Objectifs

- Servir les endpoints de listing (`GET /concepts`, `GET /relations`) en
  temps quasi indépendant de la taille du graphe.
- Garder les chemins de traversée (`expand`, `shortest_path`) en temps
  proportionnel à ce qui est *réellement* visité, pas au graphe entier.
- Permettre au client (navigateur, React Query, etc.) de revalider sans
  payload via `ETag` / `304 Not Modified`.
- Garantir la **cohérence forte** : une écriture est visible par toute
  lecture ultérieure, sans fenêtre de staleness.

### Non-objectifs

- ~~Persistance optimisée (snapshot/WAL plus rapide, mmap) — hors scope.~~
  **Levé** : la persistance est désormais couverte par `STORAGE.md`. Ce
  document ne traite toujours que des index mémoire et de la couche HTTP.
- Recherche full-text avancée (BM25 multi-champ, stemming, fuzziness
  paramétrable) — un trigram index suffit pour la sous-chaîne sur les noms.
- Concurrence multi-writer haute fréquence — l'app est lecture-dominante,
  les locks d'écriture sont courts mais pas optimisés pour 100k writes/s.

---

## 2. Hypothèses fondatrices

Toutes les optimisations qui suivent supposent que ces faits restent vrais.
Si l'un change, il faut réévaluer.

| # | Hypothèse | Conséquence |
|---|---|---|
| H1 | Le graphe tient entièrement en RAM — **hypothèse de développement, pas propriété du système** (→ STORAGE.md H18, H21, §8) | Pas de pagination disque ; on peut maintenir des index secondaires sans souci de coût mémoire. Reformulée : « les *index mémoire* tiennent en RAM ». Tout ce document décrit le palier **P0** de STORAGE.md §8.2 ; si le budget ne couvre pas le graphe, les paliers P1-P5 s'appliquent par domaine et certaines complexités de §5 changent (§8.8 de STORAGE.md) |
| H2 | Les lectures dominent largement les écritures | On accepte de payer un peu plus à l'écriture (maintenance des index, bump de génération) pour gagner beaucoup en lecture |
| H3 | Le nombre de `concept_type` et `relation_type` est petit (≤ quelques centaines) | Les index par type (label index, typed adjacency) ont une cardinalité de clé bornée |
| H4 | Les noms de concepts sont courts (≤ quelques dizaines de chars) | Le trigram index a un nombre raisonnable de trigrammes par concept |
| H5 | Le `concept_type` d'un concept est immutable après création | Les index par type n'ont pas à gérer la mutation du type d'un concept. Avec le partitionnement par `ns`, c'est aussi ce qui garantit qu'une entité ne change jamais de domaine (→ STORAGE.md R13 : à honorer si H5 tombe) |
| H6 | Les champs `source`, `target`, `relation_type` d'une relation sont immutables | L'index d'adjacence (typée ou non) n'a jamais à être déplacé après création |
| H7 | Toute écriture passe par les méthodes publiques de `OntologyGraph` (champs privés) | Pas de chemin d'écriture qui contourne la mise à jour des index |
| H8 | Le client HTTP respecte `Cache-Control` / `If-None-Match` (ou peut être configuré pour) | L'ETag a une utilité réelle ; sinon il n'y a que le cache server-side |
| H9 | Les pages renvoyées sont petites (typiquement K ≤ quelques milliers) | Le clone d'une `Vec<Concept>` de taille K est acceptable ; on n'a pas besoin de `Arc<Concept>` |
| H10 | Les query params des listings sont peu variés en pratique (l'UI répète souvent les mêmes filtres) | Le query cache a un hit rate exploitable |

---

## 3. Règles d'invariance

Ces règles doivent être respectées par tout nouveau code qui touche au graphe.

### R1 — Une seule porte d'entrée en écriture

Toutes les mutations passent par les méthodes publiques (`upsert_concept`,
`update_concept`, `remove_concept`, `add_relation`, `remove_relation`,
`upsert_rule`, `update_rule`, `remove_rule`, `upsert_action`, `update_action`,
`remove_action`, `clear_instances`). **Il ne doit jamais exister de code qui
écrit directement dans les `DashMap` primaires.** Tous les champs sont privés
précisément pour rendre cette règle structurellement vraie.

### R2 — Une mutation = mise à jour de tous les index concernés + bump de génération

Cette règle se compose avec R8 de `STORAGE.md` (« disque avant mémoire ») :
la séquence complète d'une mutation est *append disque → DashMap primaire →
index dérivés → bump de génération*. Depuis la phase 1 de `STORAGE-PLAN.md`,
chaque méthode mutante existe en deux moitiés — `prepare_*` / `preview_*`
(validation et allocation d'id, sans effet) puis `apply_prepared_*` /
`apply_*_update` (insertion et maintenance des index) — et les handlers HTTP,
`ingest_records` et le CLI appellent l'append du store **entre** les deux. Les
formes en une pièce (`upsert_concept`, `add_relation`, `update_*`) restent la
composition des deux et servent au rejeu. Un `prepare_*` qui réussit garantit
que l'`apply` correspondant ne peut plus échouer pour une raison sémantique ;
c'est ce qui rend l'ordre disque-puis-mémoire sûr.

Chaque méthode mutante a pour responsabilité de :

1. Mettre à jour le `DashMap` primaire correspondant.
2. Mettre à jour **tous** les index dérivés impactés (sorted, by_type,
   trigrams, adjacency, typed adjacency).
3. Appeler `bump_concepts_gen()` et/ou `bump_relations_gen()` selon ce qui
   a changé.

Si une nouvelle méthode mutante est ajoutée, elle doit honorer ces trois
points sous peine de lectures incohérentes ou de cache stale.

### R3 — Pas de lock long-tenu pendant un `await`

Les `RwLock` (`parking_lot`) ne sont pas async-aware. Tout `RwLockReadGuard`
ou `RwLockWriteGuard` doit être libéré avant le prochain `await`. Idem pour
`Mutex` autour des caches.

### R4 — Les index sont vérité reconstructible, pas vérité stockée

Les index secondaires (sorted, by_type, trigrams, typed adjacency) ne sont
**jamais persistés**. Ils se reconstruisent automatiquement lors du
`Store::load_into` qui rejoue le WAL/snapshot via les méthodes publiques.
**Ne jamais essayer de les sérialiser.**

Précision apportée par `STORAGE.md` R9 : cette interdiction vise les index
**sémantiques** (ceux du tableau §4.1). Les index de **stockage** (`.idx`,
`.ent`, `.xref`, et en mémoire contrainte `.adj`, `.srt`) sont eux persistés,
mais ce sont des tables d'offsets dérivées d'un fichier immuable, elles ne
connaissent ni noms, ni types, ni ordre d'affichage. R4 n'est pas affaiblie :
un `.srt` qui reproduit l'ordre de `concepts_sorted` sur disque n'est licite
que parce qu'il est reconstructible par scan du `.data` (R7) et n'est jamais
la source de vérité.

### R5 — Le cache est invalidé par génération, pas par TTL

Les query caches n'ont pas de TTL. La cohérence vient uniquement du
compteur de génération. Ne jamais introduire un cache avec TTL sans
mécanisme d'invalidation explicite — cela violerait l'objectif de
cohérence forte.

### R6 — Toute clé d'index reflète exactement l'ordre de la sortie utilisateur

L'index `concepts_sorted` est ordonné `(concept_type, name, id)` parce que
c'est l'ordre exact retourné par `GET /concepts`. Si on change l'ordre
exposé, l'index doit changer aussi, sinon on revient à un re-sort.

---

## 4. Optimisations implémentées

### 4.1 Index secondaires sur `OntologyGraph`

Localisés dans [`crates/graph/src/graph.rs`](../crates/graph/src/graph.rs).

| Champ | Type | Rôle |
|---|---|---|
| `concepts_sorted` | `RwLock<BTreeSet<(String, String, ConceptId)>>` | Itération ordonnée globale pour `GET /concepts` |
| `concepts_by_type` | `DashMap<String, BTreeSet<(String, ConceptId)>>` | Fast path pour `?type=X` — label index à la Neo4j |
| `name_trigrams` | `RwLock<AHashMap<[char; 3], BTreeSet<ConceptId>>>` | Inverted index trigrammes pour `?q=` (pg_trgm / ES ngram) |
| `relations_sorted` | `RwLock<BTreeSet<RelationId>>` | Itération ordonnée globale pour `GET /relations` |
| `rules_sorted` | `RwLock<BTreeSet<(String, String, RuleId)>>` | Listing trié des règles |
| `actions_sorted` | `RwLock<BTreeSet<(String, String, ActionId)>>` | Listing trié des actions |
| `out_edges_typed` | `DashMap<ConceptId, AHashMap<String, AdjList>>` | Adjacence sortante par type (Neo4j relationship-type chains) |
| `in_edges_typed` | `DashMap<ConceptId, AHashMap<String, AdjList>>` | Adjacence entrante par type |

**Hypothèses spécifiques** :

- Les `BTreeSet` ordonnés sont protégés par un seul `RwLock` global plutôt
  qu'un sharding — l'**hypothèse** est que la contention en écriture
  reste basse (H2). En cas de doute, profiler avant de sharder.
- `concepts_by_type` utilise `DashMap` car la clé (le `concept_type`) varie
  peu et l'accès est concurrent ; le `BTreeSet` interne est sérialisé par
  un seul writer par bucket via l'API DashMap.

### 4.2 Méthodes de listing paginées

| Méthode | Endpoint cible |
|---|---|
| `list_concepts_page(type?, needle?, offset, limit, track_total)` | `GET /concepts` |
| `list_relations_page(source?, target?, type?, offset, limit, track_total)` | `GET /relations` |
| `list_rules_page(offset, limit)` | (interne, non exposé via pagination) |
| `list_actions_page(offset, limit)` | (interne, non exposé via pagination) |

**Règles** :

- Le clone d'une `Concept` / `Relation` ne se fait **que** pour les entités
  qui finissent dans la page renvoyée. Le scan d'index, lui, ne clone rien.
- L'ordre de sortie est garanti par l'itération du `BTreeSet` ; aucun
  `sort()` post-fetch.
- `total` est calculé en pleine fidélité quand `track_total=true`, en
  lower-bound quand `track_total=false`.

**Pagination par `offset` → curseur** (→ STORAGE.md §8.8). L'`offset` est
O(1) sur un `BTreeSet` résident mais devient O(offset) dès que le listing
passe par une fusion k-way de `.srt` (palier P3). Comme c'est le seul point
d'API irréversible du passage en mémoire contrainte, `GET /concepts` et
`GET /relations` doivent exposer `cursor=` / `next_cursor` **avant** toute
implémentation des paliers, `offset` restant accepté mais déprécié. La clé
de curseur est exactement la clé de `concepts_sorted`, `(concept_type, name,
id)` — c'est R6 qui rend le curseur possible sans nouvel index. Chantier T1
de `STORAGE-PLAN.md`.

### 4.3 `track_total` (à la Elasticsearch)

Quand `track_total=false`, l'itération s'arrête dès la page pleine. Le `total`
renvoyé vaut alors `offset + page.len()` — une lower-bound.

**Quand l'utiliser** : scroll infini, lookahead, autocomplétion. Tout
contexte où l'UI n'affiche pas un compteur précis.

**Quand ne pas l'utiliser** : pagination "page X sur Y" où Y doit être exact.

### 4.4 Fast paths dans `list_relations_page`

Sélection automatique selon les filtres :

| `source` | `target` | `type` | Chemin utilisé |
|---|---|---|---|
| ✓ | – | ✓ | `out_edges_typed[source][type]` — degré-typé |
| – | ✓ | ✓ | `in_edges_typed[target][type]` — degré-typé |
| ✓ | ✓ | ✓ | min(out_edges_typed[s][t], in_edges_typed[t][T]) |
| ✓ | – | – | `out_edges[source]` — degré sortant |
| – | ✓ | – | `in_edges[target]` — degré entrant |
| ✓ | ✓ | – | min des deux degrés |
| – | – | – ou ✓ | scan `relations_sorted` |

C'est un **mini query planner** : sélectionner la plus petite source de
candidats avant de filtrer.

### 4.5 Fast path trigrammes dans `list_concepts_page`

Quand `needle.chars().count() >= 3` :

1. Tokeniser le needle en trigrammes codepoint-aware.
2. Récupérer le `BTreeSet<ConceptId>` de chaque trigramme via
   `name_trigrams`. Si **un seul** trigramme est absent, retourner vide
   immédiatement.
3. Trier les sets par taille (smallest first).
4. Intersecter en partant du plus petit.
5. Pour chaque candidat survivant, vérifier le `contains()` réel et le
   filtre `concept_type` éventuel.
6. Trier les survivants par `(concept_type, name)` et paginer.

**Hypothèse** : le needle a au moins 3 caractères. Pour des queries plus
courtes (typiquement abréviations type "fr"), fallback sur le scan
linéaire — accepté car ces cas sont rares et la sélectivité serait de
toute façon mauvaise.

### 4.6 Traversée zero-clone

Dans [`crates/graph/src/traversal.rs`](../crates/graph/src/traversal.rs) :

- `outgoing_typed(node, types)` / `incoming_typed(node, types)` — ne
  clonent que les relations dont le type est whitelisté.
- `for_each_neighbor(node, direction, f)` — itère
  `(neighbor, RelationId)` sans cloner `Relation`. Utilisé par
  `shortest_path` pour ne payer le clone qu'à la reconstruction du chemin
  (≤ `max_depth` clones).
- `expand()` bascule sur les variantes typées dès que
  `spec.relation_types` est non vide.

### 4.7 Query cache server-side

Deux compteurs `AtomicU64` :

- `concepts_gen` — bumpé par `upsert_concept`, `update_concept`,
  `remove_concept`, `clear_instances`.
- `relations_gen` — bumpé par `add_relation`, `remove_relation`,
  `remove_concept` (cascade), `clear_instances`.

Deux caches `Mutex<AHashMap>` cappés à 256 entrées :

- `list_concepts_cache` — clé `(type?, needle?, offset, limit, track_total)`.
- `list_relations_cache` — clé `(source?, target?, type?, offset, limit, track_total)`.

Chaque entrée stocke `(gen_au_moment_du_build, total, page)`. À la lecture :

1. Snapshot `gen` actuel.
2. Lookup. Si entrée présente et `entry.gen == gen` → renvoyer
   `(total, page.clone())`.
3. Sinon, recalculer via `list_*_page_uncached`, insérer sous le `gen`
   capturé.

**Eviction** : sur overflow (256 entrées), `clear()` complet — plus simple
qu'un LRU et acceptable car la reconstruction d'une entrée est rapide.

**Invalidation** : sur tout write, `bump_*_gen()` incrémente le compteur
**et** appelle `cache.lock().clear()`. Les entrées stale ne survivent jamais.

### 4.8 ETag / 304 Not Modified

Sur `GET /concepts` et `GET /relations` :

- `ETag: W/"c<gen>"` pour les concepts, `W/"r<gen>"` pour les relations.
- `If-None-Match: <etag>` reçu → comparaison ; si match → `304` sans body.

**Pourquoi l'ETag suffit même s'il ne hashe pas les query params** : la
clé de cache HTTP côté client est l'URL complète, query string incluse.
L'ETag varie quand la donnée varie ; tant que `gen` est inchangé, la
représentation que le client a pour cette URL précise est encore valide.

**Hypothèse client** : le client (browser, React Query, `reqwest` avec
cache) envoie effectivement `If-None-Match`. Sinon, on retombe sur le
cache server-side qui économise quand même le travail d'index.

### 4.9 Corrections de hot paths dans le serveur

Dans [`crates/server/src/lib.rs`](../crates/server/src/lib.rs) :

| Endpoint | Avant | Après |
|---|---|---|
| `list_concepts` / `list_relations` | `all_concepts()`/`all_relations()` + filter + sort + paginate | `list_*_page` direct |
| `subgraph_handler` (seeds par défaut) | `all_concepts()` puis filter | `list_concepts_page` par type ou global, capé à `limit` |
| `export?format=json` | `all_concepts().flat_map(outgoing)` | un seul `all_relations()` |

### 4.10 Bugfix annexe (lié à la perf indirectement)

[`crates/rag/src/prompt.rs:202`](../crates/rag/src/prompt.rs#L202) :
`String::truncate(max_context_chars)` paniquait quand l'offset tombait au
milieu d'un caractère UTF-8 multi-octets (accents, em-dash, ellipse…).
Corrigé en reculant à la frontière de codepoint la plus proche avant de
tronquer.

---

## 5. Complexité par endpoint

| Endpoint | Avant | Après |
|---|---|---|
| `GET /concepts` | O(N) clones + O(N log N) sort | O(K) clones (K = page) |
| `GET /concepts?type=X` | O(N) clone + filter | O(\|bucket\|) bucket scan |
| `GET /concepts?q=foo` | O(N) `to_lowercase().contains()` | O(\|candidats trigrammes\|) |
| `GET /relations` | O(M) clones + sort | O(M) iter trié, O(K) clones |
| `GET /relations?source=X` | O(M) scan | O(deg<sub>out</sub>(X)) |
| `GET /relations?source=X&type=T` | O(M) scan | O(deg<sub>typé</sub>) |
| `expand(seeds, types)` | O(somme des degrés) clones | O(somme des degrés-typés) clones |
| `shortest_path` BFS | 2 clones de `Vec<Relation>` par hop | 0 clone pendant BFS, ≤ depth clones à la reconstruction |
| `POST /subgraph` (seeds par défaut) | O(N) | O(limit) |
| `GET /export?json` | O(N + 2M) clones | O(N + M) clones |
| `GET /concepts` (hit cache server) | — | O(K) (juste le clone de la page) |
| `GET /concepts` (hit cache client via 304) | — | O(1) |

---

## 6. Coût en écriture

Chaque insertion ou suppression de concept paie en plus :

- 1 insertion/suppression dans `concepts_sorted` (O(log N)).
- 1 insertion/suppression dans `concepts_by_type[type]` (O(log |bucket|)).
- ~|nom| insertions/suppressions dans `name_trigrams` (O(|nom| log N)
  avec coefficient très petit).
- 1 `bump_concepts_gen()` → `AtomicU64::fetch_add` + `cache.lock().clear()`.

Chaque insertion/suppression de relation paie en plus :

- 1 insertion/suppression dans `relations_sorted` (O(log M)).
- 1 push/retain dans `out_edges`, `in_edges`, `out_edges_typed`, `in_edges_typed`.
- Si la relation est symétrique, le double (matérialisation de l'inverse).
- 1 `bump_relations_gen()`.

**Ordre de grandeur** : un insert de concept passe de ~5 µs à ~30 µs
(estimation, non mesurée). Acceptable selon H2.

Ce coût est celui de la **mémoire seule**. Le coût disque (sérialisation,
`write` bufferisé, `fsync` amorti par group commit) s'y ajoute et est chiffré
dans `STORAGE.md` §7.7 ; sans group commit, le `fsync` domine tout ce qui
précède d'un facteur 5. Depuis la phase 1 de `STORAGE-PLAN.md`, `FileStore`
fait un `fdatasync` par `append` et **un seul** par `append_batch` ; le chemin
HTTP unitaire paie donc un `fsync` par requête, la cascade d'un
`DELETE /concepts/{id}` en paie un pour tout le lot, et `ingest_records`
regroupe les concepts consécutifs par 256.

---

## 7. Limites connues et points de vigilance

### 7.1 Renommage de concept

Le `concept_type` étant immutable (H5), un renommage de concept ne déplace
que la position dans `concepts_by_type[type]` et regénère les trigrammes
du nom. Si jamais on autorise le changement de type, **toute la
maintenance d'index doit être revue**.

### 7.2 Cache invalidé sur n'importe quel write

Sur write-heavy workload, le hit-rate du cache tombe à zéro. C'est
volontaire (cohérence forte sur H2). Si l'app devient write-heavy, il
faudra envisager une invalidation plus fine (par exemple invalider
seulement les entrées dont le filtre matche la mutation), au prix d'une
complexité accrue.

### 7.3 Trigrammes < 3 caractères

Les queries de 1-2 caractères retombent sur le scan linéaire. Acceptable
sous H4 — les noms étant courts, le scan reste rapide.

### 7.4 Pas de cache pour `list_rules`/`list_actions`

Volontaire : le nombre de règles/actions est petit (H3) et l'endpoint
n'est pas paginé côté API. Pas de gain attendu.

### 7.5 Cache renvoie `Vec<Concept>` (clone)

Sur hit, on clone le `Vec<Concept>` pour le renvoyer. Passer à
`Arc<Vec<Concept>>` ou `Arc<[Concept]>` éliminerait ce clone, au prix de
changer la signature publique. À faire si le profil montre que c'est
significatif (cf. H9).

### 7.6 Locks globaux sur `BTreeSet`

`RwLock<BTreeSet<…>>` est un point de contention potentiel sous écriture
parallèle. Sous H2, ce n'est pas un problème. Si ça le devient, options :
sharder le `BTreeSet`, ou passer à `crossbeam-skiplist`.

### 7.7 `bump_*_gen` fait un `lock().clear()` synchrone

Sur un write, on prend brièvement le `Mutex` du cache pour le vider. Si
le cache est en train d'être lu, le write attend. C'est court (un `clear`
sur ≤ 256 entrées) mais non nul. Sous H2, OK.

### 7.8 L'hydratation rejoue mutation par mutation

`Store::load_into` applique chaque enregistrement via les méthodes publiques
(R1, R4). Chaque appel prend les write-locks de `concepts_sorted` et
`name_trigrams`, vide les caches et bumpe la génération : O(N log N) avec une
contention inutile puisque personne ne lit pendant le démarrage. Correct,
mais c'est le poste qui dominera le démarrage une fois le format binaire en
place (`STORAGE.md` §7.1 : le parsing domine, puis viennent les index). Un
`OntologyGraph::bulk_load` construisant les index en une passe est prévu en
phase 4 de `STORAGE-PLAN.md`, **seulement si la mesure le justifie**. Il
devra rester une méthode publique de `OntologyGraph` (R1) et bumper une fois
la génération (R2).

**Livré (phase 4)** : `OntologyGraph::begin_bulk()` rend une garde ; tant
qu'elle vit, les mutations publiques maintiennent les cartes primaires, l'index
des noms et l'adjacence (validation, upserts, suppressions et cascades
inchangés) mais laissent de côté les index dérivés — ensembles triés, seaux
par type, trigrammes, caches de pages, générations. `end_bulk()` (ou la chute
de la garde, y compris sur une erreur de rejeu) les reconstruit d'un bloc
depuis les cartes primaires et bumpe chaque génération une fois (R1, R2).
Les trois stores l'utilisent pendant `load_into`. Pendant la garde, les
lectures qui passent par les index dérivés (pages, `?q=`) sont périmées :
l'hydratation a lieu avant que le graphe soit servi, et c'est documenté.
Gain mesuré : 1,4–1,75× à 200 k / 1 M, 1,1–1,2× à 500 k / 2,5 M. La
reconstruction coûte 0,4 à 1,7 s ; la maintenance par mutation des index
dérivés coûtait donc (temps gagné + reconstruction) 45 à 65 % d'`apply` à
200 k et 15 à 26 % à 500 k ; le reste est dans les structures primaires des
relations (`STORAGE-PLAN.md` §6.6). Le profil par échantillonnage annoncé
ci-dessus n'a pas été fait ; l'imputation est une soustraction. Il est
planifié comme T6 dans `STORAGE-PLAN.md` §8, à faire avant le chantier
relations de la phase 5, parce que le remède (pré-dimensionnement, table de
symboles ou CSR) dépend de ce qu'il montrera. Effet de
bord : les `BTreeSet` bâtis d'un bloc sont plus denses (−3 % de tas mesuré à
200 k).

**Mesuré (phase 4, 2026-09-16, `STORAGE.md` §7.8)** : sur 3 M
d'enregistrements (500 k concepts, 2,5 M relations), l'hydratation prend
23 s en JSON dont 3,7 s de décodage et **19,4 s d'`apply`** — 6,5 µs par
enregistrement, soit l'ordre de grandeur estimé en §6 (~30 µs par concept
avec ses relations). `apply` représente 73 à 91 % de l'hydratation selon le
codec : la mesure justifie `bulk_load` — avec la réserve qu'une
soustraction n'est pas un profil ; un profil de `apply` précédera sa
conception. Coût mémoire mesuré en P0 (tas, store refermé) : ~2,75 Ko par
concept de 1,3 Ko et ~350 à 475 o par relation (structure + 4 index
d'adjacence + ensemble trié + chaîne du type).

### 7.9 Tout ce document décrit P0

Complexités (§5), coûts (§6) et fast paths (§4.4, §4.5) supposent les index
résidents. `STORAGE.md` §8.8 donne, section par section, ce qui change quand
un domaine descend de palier : trigrammes relâchés en P2 (`?q=` → scan
`.srt`), `concepts_sorted`/`by_type` relâchés en P3 (fusion k-way), adjacence
heap relâchée en P4 (`.adj`). Le query cache §4.7, borné, est conservé à tous
les paliers et sa valeur augmente quand ils descendent.

---

## 8. Comment ajouter une nouvelle optimisation sans casser ce qui existe

Checklist :

1. **Lire R1-R6**, et R7-R17 de `STORAGE.md` si la feature touche au
   stockage ou ajoute une structure dont la taille croît avec N. Toute
   violation = bug latent.
2. **Identifier la mutation** que la nouvelle feature implique. Quels
   `bump_*_gen` doivent être appelés ? L'append disque précède-t-il bien la
   mutation mémoire (R8) ?
3. **Identifier les index** impactés. Faut-il en ajouter un nouveau ? Si
   oui et qu'il est O(N) en heap, R15 exige sa forme disque, son chemin de
   lecture dégradé et le palier où il est relâché.
4. **Vérifier les hypothèses H1-H10** (et H11-H25) : la feature en
   suppose-t-elle de nouvelles ? H1 n'est plus qu'une hypothèse de
   développement.
5. **Tests d'invariance** : ajouter un test qui fait une mutation puis
   vérifie que la lecture suivante reflète bien le changement. Le cache
   doit servir de la donnée fraîche.
6. **Profiler en release** : pas en debug. Les `RwLock` parking_lot et
   les `BTreeSet` ont des perfs très différentes entre les deux.

---

## 9. Pointeurs vers le code

| Sujet | Fichier |
|---|---|
| Structure `OntologyGraph`, tous les index | [`crates/graph/src/graph.rs`](../crates/graph/src/graph.rs) |
| Méthodes `list_*_page` | idem, section "ordered listings" |
| `outgoing_typed` / `for_each_neighbor` | idem, section adjacence |
| `expand` / `shortest_path` | [`crates/graph/src/traversal.rs`](../crates/graph/src/traversal.rs) |
| Handlers HTTP avec ETag | [`crates/server/src/lib.rs`](../crates/server/src/lib.rs) — `list_concepts`, `list_relations` |
| `subgraph_handler` seeds fixés | idem |
| Bugfix UTF-8 truncate | [`crates/rag/src/prompt.rs`](../crates/rag/src/prompt.rs) |
| Couche stockage (`Store`, `FileStore`, `LogRecord`, `apply`) | [`crates/storage/src/`](../crates/storage/src/) — format cible dans [`STORAGE.md`](./STORAGE.md), plan dans [`STORAGE-PLAN.md`](./STORAGE-PLAN.md) |
| Attribution des ids (`IdAllocator`, compteur unique partagé) | [`crates/graph/src/id.rs`](../crates/graph/src/id.rs) |

---

## 10. Récap visuel

```
                    ┌─────────────────────────────────────────────────┐
                    │              GET /concepts?type=X&q=foo          │
                    └──────────────────────┬───────────────────────────┘
                                           │
                                           ▼
                           ┌───────────────────────────────┐
              304 ◄────────┤ If-None-Match == "c<gen>" ?   │
                           └───────────────┬───────────────┘
                                           │ non
                                           ▼
                           ┌───────────────────────────────┐
            hit ──────────►│  list_concepts_cache lookup   │
                           │  (gen-snapshot match ?)        │
                           └───────────────┬───────────────┘
                                           │ miss
                                           ▼
                           ┌───────────────────────────────┐
                           │   list_concepts_page_uncached │
                           ├───────────────────────────────┤
                           │ q ≥ 3 chars ?                  │
                           │   ├── oui ─► trigram intersect │
                           │   │           → contains() →   │
                           │   │           → type filter →  │
                           │   │           → sort           │
                           │   └── non ──► concepts_by_type │
                           │                ou concepts_sorted │
                           └───────────────┬───────────────┘
                                           │
                                           ▼
                                  insert(cache, gen)
                                           │
                                           ▼
                              Response + ETag: W/"c<gen>"
```

Toute écriture (`upsert_concept` &c.) :

```
   write → DashMap primaire
         → update concepts_sorted
         → update concepts_by_type
         → update name_trigrams
         → bump_concepts_gen():
              concepts_gen.fetch_add(1)
              list_concepts_cache.clear()
```
