# Plan de développement — stockage binaire partitionné

Plan d'implémentation de l'architecture décrite dans [`STORAGE.md`](./STORAGE.md).
Il part de l'état **réel** du code au 2026-09-08 (commit `f313b93`), pas de
l'état idéal ; la section 1 liste les écarts constatés, la section 2 les
décisions à prendre avant d'écrire une ligne, les sections 3 à 8 les phases.

Convention : `H*` / `R*` renvoient aux hypothèses et règles de
`PERFORMANCE.md` (H1-H10, R1-R6) et de `STORAGE.md` (H11-H25, R7-R17).

> **Où on en est et ce qui vient** : [`ROADMAP.md`](./ROADMAP.md) est le
> point d'entrée d'une session (état, décisions datées, prochaines étapes
> avec leurs critères, procédé). Ce plan garde le détail par phase.

---

## 0. Résumé

| Phase | Contenu | Dépend de | Taille (j·h, indicative) | Livrable |
|---|---|---|---|---|
| 1 | Chemin d'écriture : R8, `fsync`, group commit, recovery tolérante | — | 4-6 | **Livré** (§3.6) |
| 2 | Conteneur binaire mono-flux : `.data`/`.idx` 48 o, MANIFEST, CRC, recovery, migration, compteurs d'ids par famille | 1, D1-D6 | 10-14 | **Livré** (§4.6) — `SegmentStore` remplace `FileStore` |
| G | Décision gros documents : fragments ou texte hors graphe | — | 1 | **Tranché et livré** : fragments (STORAGE.md §10.9) |
| 3 | Partitionnement par `ns` : routage, `.xref`, roulement, scellement `mmap`, compaction, group commit inter-requêtes | 2, G | 10-14 | **Livré** (§5.6) — compaction store entier, group commit inter-requêtes reporté à la mesure |
| T1 | Pagination par curseur | — | 2 | **Livré** (2026-09-22, §8 T1) — API prête pour P3 |
| 4 | Mesure et codec : générateur 10⁷ / 5×10⁷, benchs, `postcard` | 2 | 4-6 | Chiffres réels sur la cible ; codec activé si gain mesuré |
| 5a | **Socle** (livré 2026-09-22) puis **P1** (livré 2026-09-23, §7.2) : slot + `Loc`, payloads relus depuis le disque | 3, 4, T1 | 8-12 | **Livré** — J5 mesuré (STORAGE.md §7.8) : mémoire privée ÷1,57, hydratation 1,19× |
| 5b | P2-P5 : `.adj`/`.srt` (CSR), paliers, hystérésis | 5a | 8-12 | **Sur besoin client** : un tenant au-delà de 5×10⁶ concepts sur un nœud contraint |
| R | Index de retrieval : persistance des vecteurs, index approximatif | indépendant | 8-12 | Retrieval en O(log N), démarrage sans réindexation |
| T | CI Windows+Linux (**livré**), métriques (**partiel** : mémoire et paliers livrés ; T3 par flux — segments, taille, `next_seq`, `sync_data`, dernière compaction — **à faire**), job `bench` manuel T2 (**à faire**), docs, position un store par tenant (**livré** : STORAGE.md §10.8, README « Deployment model ») | — | 2-3 | Couverture ≥ 90 % par crate imposée en CI (2026-09-23) ; correction du 2026-09-25 : la mention « métriques (livré) » du 2026-09-24 surévaluait T3 |

**Stratégie (validée le 2026-09-08)** : la mémoire d'abord, le disque quand
il faut. P0 reste le mode nominal ; le format est écrit maintenant pour que
la descente vers P1+ soit possible sans changer de version de fichier.
Cible de dimensionnement : 10⁷ concepts et 5×10⁷ relations par store,
**sur un nœud de 64 Go avec P1** depuis la révision du 2026-09-22
(STORAGE.md §1 et §8.1 : la phase 4 a mesuré 45 à 50 Go en P0, et P1 seul
ne loge pas 10⁷ sur 16 Go) ; sur 16 Go la garantie est 2×10⁶ / 10⁷.
**P1 (5a) reste le prochain palier** : il retire le payload du tas sur
tout store, quelle que soit la cible, et c'est la marche prévue par le
format (slot + `Loc`). Le CSR (5b) n'est engagé que sur besoin client.

---

## 1. Écarts entre le code et STORAGE.md

Constatés par lecture du code, chacun avec son emplacement.

### 1.1 Durabilité et ordre d'écriture

| Écart | Où | Impact |
|---|---|---|
| **Aucun `fsync`** dans tout le workspace : `append` fait un `flush()` du `BufWriter`, jamais `sync_data` | `crates/storage/src/file.rs:76` | Un enregistrement « acquitté » peut disparaître au crash OS. Contredit l'objectif « un enregistrement acquitté est un enregistrement récupérable » |
| **Ordre mémoire → disque**, inverse de R8 : les handlers mutent le graphe, puis appellent `store.append`, et ne défont rien si l'append échoue | `crates/server/src/lib.rs:1502-1515` (`create_concept`), idem `create_relation`, `put_ontology`, `crates/io/src/ingest.rs:168+` | Une 500 est renvoyée mais le graphe mémoire a déjà changé ; au redémarrage l'état diverge |
| **Recovery non tolérante** : un JSON tronqué en queue est une erreur `Decode` fatale au démarrage | `file.rs:127-128` | Un crash pendant l'écriture rend le store inouvrable |
| `seq` vaut 0 entre `open()` et `load_into()` ; un `append` dans cet intervalle réutilise des `seq` | `file.rs:56`, `file.rs:139` | Fragile ; disparaît si `open` hydrate |
| **Amplification ontologie** : `ingest_records` réémet l'ontologie complète à chaque type ajouté | `crates/io/src/ingest.rs:178,234,252,267,287` | 21 des 39 enregistrements de `data/graph.log` sont des `Ontology` (≈ 50 Ko sur 53) |
| `spawn_snapshotter` n'est câblé nulle part ; `graph.snap` n'existe jamais sans `compact` manuel | `crates/storage/src/periodic.rs`, `crates/cli/src/main.rs` | Le WAL croît sans borne ; sans conséquence directe pour le plan puisque le snapshot disparaît avec le nouveau format |

### 1.2 Modèle et ontologie

| Écart | Où | Impact |
|---|---|---|
| **Aucune notion de `ns`** sur `ConceptType`, `Concept`, `Relation` ni `LogRecord` | `crates/graph/src/schema.rs:26-46` | H13 (« le `ns` est déclaré dans l'ontologie ») n'a pas de support ; prérequis de la phase 3 |
| `RelationType.domain` / `.range` = types source/cible (H14 s'appuie dessus) | `schema.rs:52-53` | Compatible ; attention à la collision de vocabulaire (STORAGE.md §10.3) |
| `IdAllocator` : **un seul compteur** partagé entre concepts, relations, règles, actions | `crates/graph/src/id.rs:70-93` | Les `ConceptId` sont séquentiels mais **avec trous** (les ids des relations s'intercalent). Les zone maps `entity_min`/`entity_max` (H15) restent sélectives mais moins denses |
| `RecordKind::Clear` est du code mort côté écriture ; aucune route `DELETE /graph` | `crates/storage/src/log.rs:33-35` | À décider : supprimer, ou définir sa sémantique multi-partition (tombstone de domaine) |
| Règles et actions ne sont ni ontologie ni entités de domaine évident (`Rule.applies_to: Vec<ConceptId>` peut traverser des `ns`) | `crates/graph/src/model.rs:138-221` | Il faut leur attribuer un flux (décision D3) |

### 1.3 API et index

| Écart | Où | Impact |
|---|---|---|
| **Pagination par `offset`** uniquement, pas de curseur | `crates/server/src/lib.rs:1531-1554` | STORAGE.md §8.8 : « la pagination par offset ne survit pas à la mémoire contrainte » ; irréversible côté clients → à exposer tôt (transverse T1) |
| Les index mémoire (`concepts_sorted`, trigrammes, adjacence) vivent dans `OntologyGraph`, pas dans `crates/index` (qui est le retrieval RAG) | `crates/graph/src/graph.rs:88-136` | Le plan parle de `OntologyGraph` ; `crates/index` (`HybridIndex`) n'est pas concerné, il se reconstruit à chaque démarrage |
| L'hydratation rejoue **mutation par mutation** via `apply()` : chaque appel prend les write-locks des `BTreeSet`, vide le cache, bumpe la génération | `crates/storage/src/memory.rs:54-122` | Correct (R1/R2) mais O(N log N) avec contention inutile ; un chemin de chargement en masse est un gain de phase 4, pas un prérequis |
| Aucun test de corruption, de troncature, de redémarrage à chaud ; aucun test HTTP sur `FileStore` (tous sur `MemoryStore`) | `crates/storage/tests/`, `crates/server/tests/http.rs:24` | La phase 2 doit livrer ces tests avant le code qui les rend nécessaires |
| CI Linux uniquement ; développement sur Windows | `.github/workflows/ci.yml` | `mmap` + suppression de fichier mappé, `FlushFileBuffers`, chemins : comportements différents → matrice OS obligatoire dès la phase 2 |

### 1.4 Dépendances absentes

`memmap2`, `crc32c` (ou `crc32fast`, présent seulement en transitif via `zip`),
`arc-swap`, `bincode`/`postcard`, `criterion`, `proptest` : aucune n'est dans
`Cargo.toml`. `ahash` et `smallvec` sont déjà là.

---

## 2. Décisions prises avant la phase 2

**Toutes validées le 2026-09-08 dans le sens de la recommandation** (en
gras ci-dessous) et reportées dans STORAGE.md : D1 → H12 et §4.3, D2 → §4.3
(entrée 48 o, `rtype_sym` gelé dans le MANIFEST), D3 → §4.3 bis, D4 → §5,
D5 → §8.0, D6 → H15 et §10.4 (compteur par famille, `ConceptId < 2³²`
vérifié). Le texte des six analyses est conservé ci-dessous comme
justification.

### D1 — Adressage direct du `.idx` sous un `seq` global

STORAGE.md H12 pose un `seq` **global** monotone sans trou, et §4.3 en déduit
un adressage direct `entry = 32 + (seq − base_seq) × 32` dans le `.idx` d'une
partition. Or une partition ne reçoit qu'une fraction des `seq` (les autres
vont dans d'autres `ns`) : son `.idx` a des trous, l'adressage direct est faux
dès qu'il existe deux domaines.

Options :
- (a) un `seq` **par flux/partition** en plus du `seq` global (le global sert
  à l'ordre total et au recovery, le local à l'adressage) ;
- (b) garder le `.idx` dense et positionnel (l'entrée *i* est le *i*-ème
  enregistrement de la partition), retrouver un `seq` par recherche binaire
  (les `seq` sont croissants dans une partition) ;
- (c) un `.idx` creux dimensionné sur la plage globale (gaspillage
  proportionnel au nombre de domaines).

**Recommandation : (b).** Le chemin chaud ne cherche jamais par `seq` — la
`Loc` d'un slot pointe `(part, off, len)` directement. La recherche binaire ne
sert qu'au recovery et aux outils. Aucun champ supplémentaire dans les en-têtes.
À consigner dans STORAGE.md en amendant H12/§4.3.

### D2 — Ce que porte l'entrée d'index pour une relation

L'entrée `.idx` d'une `Relation` met `(source << 32) | target` dans
`entity_id` et rien d'autre. Deux choses manquent pour « reconstruire toute
l'adjacence sans ouvrir un seul `.data` » (§4.3) :
- le `RelationId`, sans lequel `DeleteRelation(id)` et `UpdateRelation` ne
  sont pas applicables depuis l'index seul, et sans lequel `IdAllocator` ne
  peut pas remonter son watermark ;
- le `relation_type`, sans lequel l'adjacence **typée** (`out_edges_typed`)
  ne se reconstruit pas.

Options :
- (a) passer l'entrée à 48 o : `+ relation_id u64`, `+ rtype_sym u32`,
  `+ réservé u32` ;
- (b) garder 32 o et accepter que l'hydratation P0 lise les payloads (ce
  qu'elle fait de toute façon, voir D4) ; l'index seul suffit alors pour
  P4-P5 seulement, avec l'adjacence non typée ;
- (c) mettre `relation_id` dans `entity_id` et déporter `(src, dst, rtype)`
  dans un fichier auxiliaire `.edg` par partition.

**Recommandation : (a).** 16 o de plus par enregistrement sont négligeables
(à 10⁶ enregistrements, +16 Mo lus au démarrage), et c'est la seule option
qui rend le `.idx` autosuffisant pour toute la sémantique de replay. Le
`rtype_sym` renvoie à une table de symboles portée par le MANIFEST (ou par
l'ontologie, qui est toujours rejouée avant).

### D3 — Flux de destination par `RecordKind`

| `RecordKind` | Flux | Justification |
|---|---|---|
| `Ontology` | `meta` | Rejoué intégralement, nécessaire au routage |
| `Concept`, `UpdateConcept`, `DeleteConcept` | `graph/<ns>` | `ns = ns_of_type(concept_type)` (H13). Pour `DeleteConcept(id)` le type n'est pas dans l'enregistrement : le résoudre en mémoire avant l'append (l'entité existe encore) |
| `Relation`, `UpdateRelation`, `DeleteRelation` | `graph/<ns_source>` + `.xref` dans `ns_cible` si différent | H14 ; pour `DeleteRelation(id)` même remarque |
| `Rule`, `Action`, `DeleteRule`, `DeleteAction` | **`meta`** | Peu nombreux (H3), transverses aux domaines (`applies_to`), pas de payload lourd |
| `Clear` | **supprimer** la variante, ou l'écrire comme tombstone de domaine dans chaque `ns` + `meta` | Code mort aujourd'hui ; la garder impose une sémantique multi-partition non triviale |

**Recommandation : supprimer `Clear`** ; si le besoin `DELETE /graph`
revient, il s'implémente par compaction de chaque domaine vers un segment vide.

### D4 — Ce que l'hydratation P0 lit vraiment

STORAGE.md §5 dit « aucun `.data` touché » ; §8.0 dit que P0 garde les
payloads désérialisés en heap. Les deux ne sont vrais ensemble que si P0
charge les payloads **paresseusement** après le démarrage, ou si l'on
accepte que le gain de §5 n'apparaisse qu'à partir de P1.

**Recommandation :** en phase 2-3, l'hydratation P0 lit `.idx` **puis** les
`.data` séquentiellement (`madvise(Sequential)`, CRC vérifié au passage —
§7.3). C'est déjà proportionnel au volume mais en un seul passage séquentiel
sans parsing de framing. Le démarrage « index seul » devient une propriété de
P1+ (phase 5). À écrire noir sur blanc dans STORAGE.md §5.

### D5 — Emplacement de la configuration mémoire

Il n'existe **aucun fichier TOML** : la configuration vient des flags CLI et
de `data/settings.json`, lequel est lu *après* l'ouverture du store et
modifiable à chaud par `PATCH /settings`. Le budget mémoire doit être connu
**avant** d'ouvrir un fichier (R14).

**Recommandation :** flags CLI `--memory-mode strict|adaptive` et
`--heap-fraction 0.6`, doublés des variables `ONTOLOGY_MEMORY_MODE` /
`ONTOLOGY_HEAP_FRACTION` pour Docker. Le bloc `[memory]` de STORAGE.md §8.0
est à réécrire dans ces termes.

### D6 — Ids : rester en `u64` séquentiel

Trancher **avant** la phase 2 (STORAGE.md §10.4). **Recommandation : rester
en `u64` séquentiel** ; noter dans STORAGE.md que le compteur est partagé
entre les quatre familles d'ids (§1.2 ci-dessus), et que `(source << 32) |
target` impose `ConceptId < 2³²` — ajouter l'assertion à l'append et la
vérification au démarrage.

---

## 3. Phase 1 — Chemin d'écriture correct (indépendant du format)

Objectif : que le store actuel soit **durable et cohérent** avant de changer
son format. Tout ce qui suit survit à la phase 2 tel quel.

### 3.1 R8 — disque avant mémoire

Le nœud : `upsert_concept` alloue l'id **et** insère. Pour écrire d'abord sur
disque il faut l'id avant l'insertion.

1. Dans `OntologyGraph`, ajouter `prepare_concept(&mut Concept) -> GraphResult<()>`
   qui valide (schéma, unicité de nom, propriétés requises) et alloue l'id
   sans insérer ; idem `prepare_relation`. `upsert_concept` / `add_relation`
   deviennent `prepare_*` + insertion. R1 est préservée : l'insertion reste
   la seule porte.
2. Dans `crates/server` et `crates/io/src/ingest.rs`, inverser l'ordre :
   `prepare → store.append → graph.insert → index.index_concept`. Une erreur
   d'append renvoie 500 **sans** état mémoire résiduel.
3. Cas `Delete*` : l'enregistrement ne dépend pas d'une allocation ; append
   puis `remove_*`.
4. Test : un `Store` de test qui échoue sur `append` ; vérifier que le graphe
   est inchangé et que `concepts_generation()` n'a pas bougé.

### 3.2 `fsync` et group commit

1. Étendre le trait `Store` :
   ```rust
   async fn append_batch(&self, records: &[LogRecord]) -> StoreResult<u64>; // seq du dernier
   ```
   avec `append` = `append_batch(&[r])`. `FileStore::append_batch` écrit tout,
   puis **un** `sync_data`. C'est l'embryon de `commit_group` (§7.2), le
   « par `ns` touché » arrive en phase 3.
2. `ingest_records` et `upload` utilisent `append_batch` par lot (taille de
   lot : 256, à ajuster) ; sur le chemin HTTP unitaire, un `sync_data` par
   requête est accepté pour l'instant (H19 : 50-200 µs).
3. `snapshot` / `compact` : `sync_data` sur le fichier temporaire **avant**
   le `rename`, puis `fsync` du répertoire (Unix ; no-op documenté sur
   Windows).
4. Mesure avant/après sur `ingest` du jeu `examples/finance` — c'est le
   premier chiffre du projet.

### 3.3 Recovery tolérante

`load_into` : un enregistrement dont la longueur dépasse la fin du fichier,
ou dont le JSON est invalide **en dernière position**, est traité comme une
queue tronquée → `warn!`, troncature du fichier à l'offset du dernier
enregistrement valide, poursuite. Un JSON invalide **au milieu** reste fatal.
Test : tronquer `graph.log` à chaque offset possible d'un log de 10
enregistrements et vérifier qu'on redémarre avec un préfixe cohérent.

### 3.4 Amplification ontologie

`ingest_records` : accumuler les modifications de schéma et émettre **un**
`Ontology` par appel (ou un `Ontology` par lot de `append_batch`). Divise par
~20 la taille du log actuel. Indépendant du format.

### 3.5 Critères de sortie

- Aucun handler ne mute la mémoire avant l'acquittement disque.
- `grep sync_data crates/` ≠ vide ; chaque `rename` est précédé d'un sync.
- Tests : append échoué, troncature à tout offset, lot de 1 000 concepts
  ingérés en < N `fsync` (N = nombre de lots).

### 3.6 État — livré le 2026-09-08 (branche `feat/storage-phase1`)

Ce qui a été fait, et où le plan a été précisé en cours de route :

| Point | Réalisation |
|---|---|
| R8 | `OntologyGraph` expose `prepare_concept` / `apply_prepared_concept`, idem `relation`, `rule`, `action` ; `preview_*_update` / `apply_*_update` pour les patchs ; `incident_relation_ids` pour journaliser une cascade avant de supprimer. Les formes historiques (`upsert_*`, `add_relation`, `update_*`) sont la composition des deux et servent au rejeu. Tous les handlers HTTP, `ingest_review::apply`, `ingest_records` et le CLI suivent `prepare → append → apply`. |
| Sérialisation des écrivains | `AppState.writer` (`tokio::sync::Mutex<()>`) : un seul écrivain à la fois dans le processus, ce qui rend `prepare → apply` atomique vis-à-vis des autres requêtes. Cohérent avec H17. |
| `fsync` | `FileStore::append_batch` écrit le lot puis un seul `sync_data` ; `append` = lot de 1 ; `sync_count()` exposé. Snapshot et compaction : `sync_all` du fichier temporaire avant le `rename`, `fsync` du répertoire sous Unix. |
| Group commit | `Store::append_batch` ajouté au trait (défaut : boucle). Utilisé pour la cascade de `DELETE /concepts/{id}` et par `ingest_records`. **Restriction volontaire** : seuls les *concepts consécutifs* sont regroupés (par 256), avec une vérification des doublons et des types disjoints *à l'intérieur du lot* ; tout autre enregistrement vide le lot d'abord. Regrouper des relations exigerait de modéliser la cardinalité des relations en attente — reporté au group commit inter-requêtes de la phase 3. |
| Recovery | Queue tronquée à n'importe quel octet (préfixe de longueur coupé, payload trop court, JSON invalide en dernière position) → `warn`, troncature au dernier enregistrement complet, reprise. Corruption *avant* la queue → erreur explicite, fichier intact. |
| Amplification ontologie | `ingest_records` et `ingest_review::apply` journalisent **un** `Ontology` par flux, placé avant la première instance qui pourrait en dépendre. Compromis documenté dans le code : un échec du store à cet instant laisse des *types* (jamais des instances) en mémoire sans équivalent disque, et l'ingest s'arrête sur l'erreur. |
| CI | Matrice `ubuntu-latest` × `windows-latest` (T2 avancé). |
| Tests | `graph` : 10 tests unitaires sur prepare/apply/preview. `storage/tests/recovery.rs` : lots, seqs, troncature à tout offset, garbage final, corruption médiane, snapshot + queue tronquée. `io/tests/write_ahead.rs` : lots, un seul `Ontology`, doublons et disjoints intra-lot, store en échec, rejeu, ids explicites. `server/tests/write_ahead.rs` : les 13 endpoints mutants sur un store en échec laissent le graphe et ses générations intacts ; 404 sans toucher au store ; cascade = un lot ; redémarrage sur `FileStore` après écritures HTTP. `FlakyStore` (`ontology_storage::testing`) partagé par ces tests. |

Revue avant fusion (2026-09-15), corrections apportées sur la branche :
- **Schéma et R8** : les déclarations de types sont appliquées en mémoire
  avant d'être journalisées (un seul `Ontology` par flux). Tout chemin
  d'erreur de l'ingest (doublon, store en échec, `UnknownNamed`) et les
  sorties `strict` de `/ingest/apply` **restaurent l'ontologie d'avant**
  tant que le schéma n'a pas été flushé ; aucune instance d'un type non
  journalisé ne peut exister à ce moment, le retour arrière est donc sûr.
- **`FileStore` empoisonné** après un `write`/`fsync` en échec : tout
  append suivant est refusé (`StoreError::Poisoned`) jusqu'au redémarrage,
  où la recovery tronque la queue déchirée. Sans cela, un client qui
  réessaie après une 500 pouvait produire deux enregistrements durables
  pour la même entité et rendre le rejeu impossible.
- **Upsert par id explicite** : l'ancien nom est retiré de l'index de noms
  lors d'un renommage, et un changement de `concept_type` est refusé dès
  `prepare_concept` (H5).
- Une erreur de store pendant `/upload` répond désormais 500, pas 400.

Non fait, volontairement : le `seq` à 0 entre `open()` et `load_into()`
(§1.1) disparaît avec la phase 2 ; `spawn_snapshotter` reste non câblé
puisque le snapshot disparaît avec le format binaire. Relevé mais laissé
tel quel (préexistant, sémantique du graphe à trancher) : une mise à jour
de règle ou d'action dont un concept `applies_to` / `subject` a été supprimé
entre-temps est acceptée en direct mais refusée au rejeu, car
`remove_concept` ne nettoie pas ces références.

---

## 4. Phase 2 — Conteneur binaire, flux unique

Objectif : le format de STORAGE.md §4, avec un **seul** domaine `graph/default`
et le flux `meta`, pour que toute la mécanique fichier soit testée avant que
le partitionnement n'y ajoute sa combinatoire.

### 4.1 Découpage du crate `ontology-storage`

```
src/
  segment/
    header.rs      # en-têtes 32 o GRFD / GRFI, version, codec, flags
    record.rs      # en-tête d'enregistrement 32 o, padding 8, crc32c
    idx.rs         # entrée 48 o (D2), lecture positionnelle, recherche binaire par seq (D1)
    writer.rs      # segment actif : BufWriter data + idx, pread, sync_data data / flush idx (R7)
    reader.rs      # segment scellé : mmap (memmap2) + ArcSwap<Mmap>
    recover.rs     # §9 : scan de queue, troncature, régénération .idx
  manifest.rs      # MANIFEST.json, write+rename, ns_id gelés (R10), zone maps
  codec.rs         # trait Codec { encode, decode }, JsonCodec = 0 ; bincode = 1 en phase 4
  stream.rs        # un flux = Vec<segment scellé> + 1 segment actif ; roulement 64 Mo / 100k
  segment_store.rs # impl Store : meta + graph/default
  migrate.rs       # graph.log (+ graph.snap) → nouveau format
  file.rs          # FileStore conservé en lecture seule pour la migration
```

Dépendances à ajouter : `memmap2`, `crc32c` (repli logiciel inclus),
`arc-swap`, `proptest` (dev), `tempfile` (dev — déjà présent côté server).

### 4.2 Sémantique

- `kind` u8 : une valeur par `RecordKind` restant (11 après D3), figée dans
  `record.rs` avec un test qui casse si l'enum change sans mise à jour.
- `entity_id` selon `kind` (D2) ; `ns_id` = 0 pour `meta`, 1 pour `default`.
- Écriture : `append_batch` → pour chaque enregistrement, encoder, écrire
  `data` puis `idx` en bufferisé ; en fin de lot `sync_data(data)`,
  `flush(idx)` ; **puis** mise à jour du MANIFEST si un segment a roulé
  (un roulement force un sync + rename).
- Lecture (hydratation P0, D4) : `meta` intégral ; `graph/default` : lire le
  `.idx`, puis parcourir le `.data` séquentiellement en vérifiant chaque CRC
  (§7.3), appliquer via `apply()` inchangé. Le chemin de chargement en masse
  attendra la phase 4.
- Recovery (§9) au `open()` : pour chaque flux, dernier `.data` ; scan de
  queue ; troncature ; comparaison avec le `.idx` et régénération de sa fin ;
  `.idx` absent → régénération complète ; `next_seq` = dernier valide + 1.
- Verrou d'écriture (H17) : fichier `store/LOCK` avec `try_lock` exclusif
  (`fs2` ou `std::fs::File::try_lock` selon MSRV) ; refus explicite si pris.

### 4.3 Migration

- `ontology migrate --data <dir>` : lit `graph.snap` puis `graph.log` via
  `FileStore`, écrit le nouveau format dans `<dir>/store/`, vérifie par
  rechargement complet (nombre de concepts, relations, règles, actions,
  `high_water`), renomme les anciens fichiers en `graph.log.migrated`.
- Au `serve` / `ingest`, si `graph.log` existe et `store/` n'existe pas :
  migration automatique avec le même chemin, journalisée. Jamais de
  suppression des anciens fichiers par le code.
- Test : la migration de `data/graph.log` (39 enregistrements) doit donner
  les 55 696 o annoncés en STORAGE.md §10.2, à ±(16 o × 39) près après D2.

### 4.4 Tests exigés avant fusion

| Test | Ce qu'il prouve |
|---|---|
| Round-trip `proptest` sur `Vec<LogRecord>` aléatoires | Encodage/décodage, padding, CRC |
| Troncature du `.data` actif à **tout** offset | §9 étapes 1-2 ; jamais de panique, préfixe cohérent |
| Suppression du `.idx` puis `open()` | R7 : régénération complète identique octet à octet |
| Corruption d'un octet dans un payload scellé | CRC détecte à l'hydratation ; erreur nommant partition + seq |
| Deux `open()` concurrents | Verrou : le second échoue avec un message |
| Migration de `data/graph.log` | Équivalence graphe avant/après |
| Tests HTTP `http.rs` rejoués sur `SegmentStore` | Un `make_state_with_segment_store()` ; aucun test ne doit changer |
| CI `windows-latest` + `ubuntu-latest` | Chemins, `FlushFileBuffers`, `mmap` |

### 4.5 Critères de sortie

- `FileStore` n'est plus instancié que par `migrate`.
- Démarrage sur le store migré : même `concept_count`, `relation_count`,
  même résultat de `GET /concepts` (ETag égal à génération égale).
- Aucune écriture en place hors MANIFEST (write+rename) et troncature de
  recovery.

### 4.6 État — livré le 2026-09-08 (branche `feat/storage-phase2`)

| Point | Réalisation |
|---|---|
| Format (§4.1-4.3 de STORAGE.md) | `segment/format.rs` : en-têtes `GRFD`/`GRFI` 32 o, en-tête d'enregistrement 32 o avec `crc32c`, payload aligné 8, entrée d'index **48 o positionnelle** (D1, D2) portant `entity_id`, `endpoints`, `rtype_sym`. Octets de `kind` gelés et testés. Little-endian. |
| Segment actif / scellé | `segment/active.rs` : écriture bufferisée, **un `fdatasync` par commit**, entrées d'index écrites après le sync et jamais synchronisées (R7), jamais `mmap` (R11). `segment/sealed.rs` : `memmap2` sur les deux fichiers, accès positionnel, recherche binaire par `seq`, scan séquentiel avec CRC optionnel. Le scellement estampille le compteur et synchronise une fois. |
| Recovery (§9) | `segment/recover.rs` : scan CRC de la queue, troncature au premier enregistrement tronqué/corrompu/indécodable, index complété ou réécrit depuis le premier désaccord, index perdu régénéré à l'identique. Testé à tout offset. |
| MANIFEST (§4.5) | `manifest.rs` : `ns_id` et `rtype_sym` gelés (R10, D2), allocateur de partitions, zone maps par partition scellée (`entity_min/max`, `kinds`, `edges`, `payload_bytes`). Réécrit par write + fsync + rename. **Jamais source de vérité des données** : les partitions sont redécouvertes depuis le répertoire à l'ouverture. Un symbole neuf est persisté **avant** le premier enregistrement qui l'utilise. |
| Flux (§4) | `stream.rs` : partitions scellées + une active ; roulement par taille ou nombre d'enregistrements (64 Mo / 100 000, configurable) ; un scellement interrompu avant l'écriture du MANIFEST est terminé à l'ouverture. |
| `SegmentStore` | `segment_store.rs` : deux flux (`meta` = ontologie, règles, actions — D3 ; `graph/default` = concepts, relations), `seq` global monotone, **un sync par flux touché par lot** (§7.2), verrou `LOCK` exclusif (H17, `File::try_lock`), hydratation P0 lisant index puis données avec CRC (D4), E/S bloquantes en `spawn_blocking`. `snapshot`/`compact` : no-op jusqu'à la phase 3. |
| Migration (§4.3) | `migrate.rs` : `graph.snap` déroulé puis `graph.log` au-delà du watermark, écriture par lots de 1 000, **vérification entité par entité** (ontologie, concepts, relations par id, règles, actions) entre le rejeu legacy et le rejeu du nouveau store, puis renommage en `*.migrated`. Automatique au démarrage du CLI si `graph.log` existe sans `store/` ; commande `ontology migrate` explicite. Refuse un store non vide. |
| Compteurs d'ids (D6) | `IdAllocator` par famille ; `ConceptId::fits_storage` vérifié dans `prepare_concept` et `prepare_relation`. |
| Toolchain | `Dockerfile` passe à `rust:1.98-slim` (`File::try_lock` exige ≥ 1.89). |
| Tests | `format_props.rs` (6 propriétés proptest), `segment_io.rs` (10), `segment_store.rs` (9 : routage et syncs, roulement + redémarrage, queue tronchée sur les deux flux + MANIFEST perdu, scellement interrompu, verrou, CRC nommant la partition, migration vérifiée, migration refusée, store vide), `manifest` (3 unitaires), `format` (8 unitaires), variante `SegmentStore` du test de redémarrage HTTP côté serveur. |

Écarts par rapport au plan initial, assumés : pas d'`ArcSwap` (il n'y a
pas encore de lecteurs concurrents du store — les lectures passent par la
mémoire en P0 ; il arrive en phase 3 avec la compaction) ; l'hydratation
rejoue toujours mutation par mutation via `apply()` (le chargement en masse
reste un chantier de phase 4, sur mesure). Le `.idx` de `DeleteRelation` ne
porte pas les extrémités (l'enregistrement ne les contient pas) : sans
conséquence en P0, à traiter avant P1 si l'hydratation « index seul » doit
rejouer les suppressions sans ouvrir le `.data`.

Revue avant fusion (2026-09-15), corrections apportées sur la branche :
- **Lot en échec** : le lot est entièrement encodé et routé avant la
  première écriture ; toute erreur ensuite **empoisonne** le store
  (`StoreError::Poisoned`) jusqu'au redémarrage. Sans cela, des
  enregistrements refusés au client restaient dans le tampon et devenaient
  durables au lot suivant, jusqu'à rendre l'hydratation impossible
  (`DuplicateConcept` au rejeu après un retry).
- **Ordre des syncs** : un lot qui touche plusieurs flux les synchronise
  dans l'ordre du premier `seq` touché, de sorte qu'un crash entre deux
  syncs laisse un préfixe du lot, jamais un trou.
- **Recovery** : un enregistrement invalide (CRC, `kind` inconnu, payload
  indécodable, `seq` non croissant) n'est tronqué que s'il est le
  **dernier** ; s'il est suivi d'octets valides, l'ouverture échoue en
  nommant la partition et l'offset et le fichier reste intact. Un
  `.data` sans en-tête (crash entre création et sync) est recréé.
- **Scellement** : un `.idx` dont les entrées ne couvrent pas exactement le
  `.data` (compteur à 0 après perte du page cache) n'est pas accepté comme
  scellé ; la partition est récupérée et rescellée.
- **Migration** : construite et vérifiée dans `store.migrating/` puis
  renommée ; un `store/` existant est refusé, un staging orphelin est
  jeté et refait ; les fichiers legacy ne sont plus ouverts en écriture
  (plus de troncature ni de `graph.log` créé) ; règles et actions sont
  comparées par contenu. Le CLI refuse de démarrer si `graph.log` et
  `store/` coexistent.
- `fsync` du répertoire après création d'une partition (Unix).

Limites connues laissées telles quelles : `sync_count` compte les commits
(pas les `fsync` de scellement ni du MANIFEST) ; après perte du MANIFEST,
les `rtype_sym` sont réattribués dans l'ordre de scan — égal à l'ordre
d'écriture tant qu'il n'y a qu'un flux graphe (voir phase 3) ; un
`graph.log` contenant un `Clear` (jamais écrit par le code) n'est pas
migrable.

Mesure sur le jeu de test de `segment_store.rs` (8 requêtes HTTP mutantes) :
8 syncs, 8 enregistrements, aucun overhead de format visible à cette taille
— conforme à STORAGE.md §10.1, la mesure utile attend la phase 4.

---

## 5. Phase 3 — Partitionnement par `ns`

### 5.1 Ontologie

1. `ConceptType.ns: Option<String>` (`#[serde(default)]`) ; `None` →
   `"default"`. Validation : un `ns` est un identifiant `[a-z0-9_-]{1,32}`.
   Ne **pas** nommer le champ `domain` (§10.3).
2. Un type hérite le `ns` de son `parent` s'il ne le déclare pas ; un enfant
   ne peut pas déclarer un `ns` différent de son parent (sinon `?type=Parent`
   avec `include_subtypes` traverserait les domaines sans le savoir).
3. Router dans `Ontology` : `ns_of_type(&str) -> &str`,
   `ns_of_relation_type(&str) -> (src_ns, dst_ns)` via `domain` / `range`
   (H14). Cache dans `descendants_cache`-like `OnceLock`.
4. Changement de `ns` d'un type existant : **refusé** par `extend_ontology`
   tant que des instances existent (l'équivalent de H5 au niveau du domaine ;
   évite R13 pour l'instant). Documenter la levée future.
5. `PUT /ontology`, ingest et web UI : exposer le champ ; OpenAPI.

### 5.2 Store

1. MANIFEST : table `ns → ns_id` gelée (R10) ; `ns_id` alloué au premier
   enregistrement du domaine, jamais recyclé ; suppression = marquage
   `retired`.
2. `stream(ns)` : un flux par domaine, créé paresseusement ; `meta` reste
   `ns_id = 0`.
3. Routage à l'append : `Concept*` → `ns_of_type` ; `Relation*` →
   `ns_source` + entrée `.xref (target, source, seq)` dans `ns_target` si
   différent ; `Delete*` résolus en mémoire avant l'append (D3).
4. `commit_group` : `SmallVec<[ns_id; 8]>` des flux touchés, un `sync_data`
   par flux (§7.2). C'est `append_batch` de la phase 1 étendu.
5. Roulement : 64 Mo **ou** 100 000 enregistrements **ou** âge > 24 h si non
   vide (§7.5). Scellement : `sync`, écriture finale du `.idx`, `mmap` du
   couple, `ArcSwap` (R11), MANIFEST mis à jour.
6. `.xref` : écrit comme le `.idx` (bufferisé, jamais `fsync`, dérivé — R7) ;
   reconstruit à l'hydratation depuis les `.idx` des autres domaines si
   absent.
7. Compaction par domaine : réécrit les enregistrements vivants d'un ensemble
   de segments scellés dans un nouveau segment, supprime les tombstones et
   les versions écrasées, met à jour MANIFEST, **puis** libère les anciens
   mappings via `ArcSwap` (§7.6). Sur Windows un fichier mappé ne se supprime
   pas : renommer en `.old`, supprimer au prochain démarrage. Fusion des
   petites partitions sous 1 Mo (§7.5, §8.7).
8. `POST /compact` et `ontology compact` : compactent **tous** les domaines ;
   ajouter `?ns=` pour un seul. `snapshot` disparaît du trait `Store`.

### 5.3 Hydratation sélective

- `hydrate(&NsSet)` dans `SegmentStore` ; `load_into` = tous les domaines.
- Exposition minimale : flag `serve --ns a,b` pour ne charger que certains
  domaines (cas d'usage : environnement de dev sur un sous-ensemble). Un
  voisin non chargé est renvoyé comme id nu (§5) ; la résolution à la
  demande attend un besoin.
- L'ontologie reste toujours chargée (H25).

### 5.4 Tests

| Test | Ce qu'il prouve |
|---|---|
| Deux domaines, relation inter-domaine, hydratation du seul domaine cible | `.xref` donne l'arête entrante sans le payload |
| Hydratation du seul domaine source | La relation est complète ; le concept cible est un id non résolu |
| 1 000 appends sur 2 domaines en lots de 100 | ≤ 2 `sync_data` par lot (compteur instrumenté) |
| Roulement forcé (seuil abaissé à 10 enregistrements) pendant des lectures concurrentes | `ArcSwap` : aucune lecture ne voit un mapping libéré ; pas de `SIGBUS` |
| Compaction pendant lecture, Linux et Windows | §7.6 ; le `.old` est nettoyé au redémarrage sur Windows |
| Suppression d'un domaine puis recréation du même nom | `ns_id` différent (R10) |
| `ns` déclaré sur un type enfant ≠ parent | Refus de validation |

### 5.5 Critères de sortie

- STORAGE.md §4, §5 (amendé par D4), §7.2, §7.5, §7.6, §9 implémentés.
- Un store à un seul domaine se comporte exactement comme la phase 2 (mêmes
  fichiers, un `ns_id` de plus dans le MANIFEST).

### 5.6 État — livré le 2026-09-15 (branche `feat/storage-phase3`)

| Point du plan | Réalisation |
|---|---|
| 5.1 Ontologie | `ConceptType.ns: Option<String>` (`[a-z0-9_-]{1,32}`, hérité du parent, `default` sinon), `Ontology::ns_of_type`, `ns_of_relation_type`, `namespaces`, `validate_namespaces`. `extend_ontology` est devenu **atomique** : la fermeture travaille sur une copie, validée (domaines, enfant ≠ parent refusé, changement de domaine d'un type ayant des instances refusé — `NamespaceChangeWithInstances`), puis substituée. `ns` omis en JSON quand absent : les ontologies existantes se chargent inchangées. |
| 5.2.1-2 MANIFEST, flux | `Manifest::intern_ns` (id gelé, jamais réutilisé, `retire_ns`), un flux `graph/<ns>` créé paresseusement au premier enregistrement du domaine, MANIFEST sauvé **avant** ce premier enregistrement. |
| 5.2.3 Routage | Le store garde l'ontologie courante (dernier `Ontology` de `meta`, mis à jour au fil des lots) : concepts → `ns_of_type`, relations → domaine source, `target_ns_id` dans l'entrée d'index pour les relations inter-domaines. Les tombstones portent un **`RouteHint`** (`LogRecord::delete_concept(id, type)`, `delete_relation(id, type)`), jamais persisté ; un tombstone sans indice est refusé avant toute écriture. La migration legacy déduit les indices en rejouant le journal dans un graphe de travail. |
| 5.2.4 Group commit | Un `sync_data` par domaine touché par lot (testé : 1 000 concepts sur 2 domaines en 10 lots = 20 syncs). Le group commit **inter-requêtes** n'est pas fait : `AppState.writer` sérialise les requêtes mutantes, il n'y a donc jamais deux lots en vol ; le batching inter-requêtes exigerait de relâcher ce verrou entre `prepare` et `apply`, ce qui rouvre les conflits de validation — à mesurer d'abord (phase 4). |
| 5.2.5 Roulement | Par taille ou nombre d'enregistrements ; pas de roulement par âge. `ArcSwap` non introduit : en P0 aucun lecteur ne lit le store en cours de processus, le swap se fait sous le mutex du store. |
| 5.2.6 `.xref` | Fichier dérivé reconstruit à chaque ouverture et après compaction depuis les `target_ns_id` des autres flux ; rangé par plage de `seq` de partition ; en-tête `GRFX`. Pas d'append à chaud (P1). |
| 5.2.7-8 Compaction | **Store entier**, pas par domaine (voir STORAGE.md §5 pour la raison : le rejeu par `seq` et les dépendances inter-flux). Réécriture depuis le graphe vivant dans l'ordre des dépendances, vérification par rejeu et comparaison sémantique, bascule, suppression (`.old` + balayage à l'ouverture). `POST /compact` et `ontology compact` compactent tout ; `snapshot` reste un no-op. |
| 5.3 Hydratation sélective | `Store::load_domains`, `SegmentStore::load_domains_report`, `serve --ns a,b`. Fusion par `seq` sur N flux (k-way sur les têtes décodées). Relations, règles, actions référençant un concept non chargé : ignorées et comptées. |
| G | Fragments : `extract_from_text_chunked`, `chunk_text`, `TextDocumentSource::with_chunk_chars`, `--chunk-chars` (défaut 4 000). |
| T5 | Un processus par tenant (STORAGE.md §10.8). |
| Exemple | `examples/finance/ontology.json` déclare trois domaines : `parties`, `contrats`, `facturation`. |
| Tests | `graph` : 6 unitaires sur les domaines. `storage/tests/domains.rs` (8) : routage et tombstones, tombstone non routé refusé, 20 syncs pour 10 lots sur 2 domaines, `.xref` reconstruit à l'ouverture et rangé par partition, hydratation sélective (3 combinaisons + domaine inconnu), compaction complète (réduction, partitions, aucun `.old`, rejeu, écriture après, redémarrage avec xref), reroutage après changement de schéma dans le même lot, layout mono-domaine identique à la phase 2. `io` : 3 unitaires sur le découpage. |

Revue avant fusion (2026-09-15), quatre décisions validées par le
propriétaire et corrections apportées sur la branche :
1. **Ids de relations stables après compaction** : nouveau `kind` 12
   `RelationExact`, écrit par la compaction pour chaque relation vivante
   (les deux sens d'une paire symétrique), rejoué sans matérialiser
   d'inverse ; la vérification compare désormais concepts, relations
   (par id, contenu compris), règles et actions. Sans cela, un tombstone
   émis après compaction visait un id absent du disque et la relation
   réapparaissait au redémarrage.
2. **Gardes de schéma** : refus de supprimer un type (concept, relation,
   règle, action) ayant des instances et de changer `domain`/`range` d'un
   type de relation ayant des relations ; `check_ontology` appelé **avant**
   d'écrire l'`Ontology` dans `PUT /ontology`, `/upload` et le seed (un
   schéma refusé arrivait sur disque et bloquait le redémarrage).
3. **Déclarations de types à l'ingest** : `merge_concept_type` — une
   redéclaration ne touche ni `ns` ni `parent` ni ce qu'elle ne mentionne
   pas ; l'exemple finance produit bien ses trois domaines.
4. **Fragments** : `fragment_type_decl` résolu par l'ingesteur (type
   `<Type>Fragment` dans le domaine du document, relation par type
   `fragment_of_<type>`), fragments émis avant leurs relations, plafond de
   2 000 fragments par document, fins de ligne CRLF normalisées.

Autres corrections de la revue : bascule de compaction résistante au
crash (staging `compacting/` + marqueur MANIFEST, reprise à l'ouverture) ;
lot encodé et routé avant la première écriture, empoisonnement sur échec
(hérité de la phase 2) ; `meta` réservé comme nom de domaine ; hydratation
sélective n'ignorant qu'une cible manquante (une source manquante est une
corruption) ; balayage des `.tmp` ; mappings du staging relâchés avant
toute suppression.

Non fait, volontairement : compaction par domaine et roulement par âge
(ci-dessus), maintenance à chaud du `.xref` (et prise en compte des
`DeleteRelation` dans le `.xref`, qui garde des arêtes obsolètes jusqu'à
la compaction), exposition du champ `ns` dans l'UI web (l'API
`PUT /ontology` et les fichiers JSON le prennent déjà).

---

## 6. Phase 4 — Mesurer, puis codec

Rien dans cette phase n'est décidé d'avance : STORAGE.md §11.9 (« sur 39
enregistrements, aucune mesure n'a de sens »).

1. **Générateur** : `ontology bench gen --concepts 1e6 --relations 5e6
   --ns 5 --payload 1300` produisant un store réaliste (distribution de
   noms courts H4, propriétés ~1,3 Ko).
2. **Benchs `criterion`** dans `crates/storage/benches/` : hydratation
   (temps, octets lus, RSS max), append unitaire, `append_batch` ×100,
   compaction ; et dans `crates/server` : `GET /concepts` page 200,
   `?q=`, `expand` profondeur 2. Publier les chiffres dans STORAGE.md §7.7
   en remplacement des estimations.
3. **Codec** : `codec = 1` `postcard` (ou `bincode` 2 — choisir sur la
   stabilité du format sérialisé de `PropertyValue` untagged, qui est le
   point délicat : `postcard` n'a pas de `deserialize_any`, donc
   `#[serde(untagged)]` ne fonctionne pas — probable passage à une enum
   taguée pour le stockage, avec conversion). Les segments existants
   restent lisibles (`codec` par enregistrement). Activer par défaut si le
   gain d'hydratation mesuré dépasse 3×.
4. **Chargement en masse** (`OntologyGraph::bulk_load(iter)`), seulement si
   le profil montre `apply()` dominant : construit `concepts_sorted`,
   trigrammes et adjacence en une passe, un seul bump de génération. R1/R2
   respectées : c'est une méthode publique de `OntologyGraph`.
5. Sharding composite (§6.1), `FxHash` dans une `SymbolTable` (§7.4) :
   **différés** jusqu'à mesure de contention. Ouvrir un ticket, pas une
   branche.

### 6.6 État — phase close le 2026-09-17 (branches `feat/storage-phase4`, `feat/bulk-load`, fusionnées dans `main`)

Livré :

- **Générateur** `ontology bench gen` : écrit directement sur disque (sans
  graphe en mémoire), N concepts / M relations / K domaines, payload cible,
  codec au choix, barrières par lots de 5 000 ; refuse un store non vide.
  10⁶ concepts + 5×10⁶ relations se génèrent en ~20 s ; la cible 10⁷ /
  5×10⁷ (~22 Go en JSON) en ~4 min — mais ne s'hydrate pas sur cette
  machine (voir mémoire ci-dessous).
- **Benchs** `bench hydrate | append | query | compact` (résultats en JSON
  avec `--json`) et le micro-bench criterion `crates/storage/benches/codec.rs`.
  Les figures sont dans `STORAGE.md` §7.7–7.8.
- **Codec 1 `postcard`** via un miroir tagué (`storage::codec`), octet codec
  par enregistrement, changement par `compact --codec`, store mixte lisible.
  Correction collatérale : `serde_json` en `float_roundtrip` (le parseur par
  défaut n'était pas correctement arrondi ; trouvé par un test de propriété).
- Tests : codec (unitaires, intégration, propriété), bench (unitaires + bout
  en bout CLI), résolveurs de recovery conscients du codec.

Mesuré (détail dans `STORAGE.md` §7.7–7.8) :

| Question de la phase | Réponse |
|---|---|
| Le parsing domine-t-il l'hydratation ? | **Non** : 9–27 %. `apply` (index mémoire) : 73–91 %. |
| Gain du codec binaire | décodage 3× par concept (micro-bench), 1,6–2,2× in situ, disque −24 %, **hydratation 0,93–1,24×** |
| Seuil « activer par défaut si > 3× » | **Non atteint → NO-GO comme défaut** ; codec 1 disponible en option (`compact --codec postcard`), lisible et testé |
| `bulk_load` justifié ? | **Oui** : `apply` domine. **Livré** (`OntologyGraph::begin_bulk` / `end_bulk`, mode « en masse » de l'hydratation) : hydratation **1,4 à 1,75× plus rapide à 200 k / 1 M** (10,7 s → 6,1–7,6 s), **1,1 à 1,2× à 500 k / 2,5 M** (23,1 s → 19,7–21,5 s). En dessous de la cible 2–3× : voir le profil ci-dessous |
| Empreinte P0 (tas, store refermé) | ~2,75 Ko / concept (1,3 Ko de payload), ~350–475 o / relation → **45 à 50 Go pour la cible**, 16 Go visés |
| Compaction complète | 30–54 k enr./s → 20–30 min à la cible ; compaction par domaine et vérification sans rejeu à prévoir |
| Page à offset | O(offset) : 13 ms à 500 k → T1 (curseur) avant la phase 5, comme prévu |

**Ce que la mesure dit du coût d'`apply` (item 4).** Le mode en masse
laisse les index dérivés (ensembles triés, seaux par type, trigrammes,
caches, générations) de côté pendant le rejeu et les reconstruit une fois.
La reconstruction coûte **0,36–0,53 s à 200 k / 1 M et 1,2–1,7 s à 500 k /
2,5 M** (ensemble trié des concepts 0,8–1,0 s, relations + règles + actions
0,2–0,3 s, seaux par type 0,1–0,2 s, trigrammes 0,1 s ; deux exécutions par
point, base « avant » à une exécution). Le coût de leur maintenance mutation
par mutation est le temps gagné plus la reconstruction : **3,5 à 5,1 s à
200 k, soit 45 à 65 % d'`apply` (7,8 s)** ; **2,8 à 5,1 s à 500 k, soit 15 à
26 % d'`apply` (19,4 s)**. Le poids des index dérivés décroît donc avec
l'échelle, et à 500 k le gros d'`apply` (~15 s) est ailleurs : dans les
structures **primaires**, et d'abord dans les relations — ~3 s pour 500 k
concepts (~6 µs chacun : validation du schéma, clé du nom en minuscules,
insertion `DashMap`) contre ~12 s pour 2,5 M relations (**~5 µs chacune** :
deux `contains_key`, quatre entrées d'adjacence dans des `DashMap` —
sortante, entrante et leurs variantes typées, avec deux clones de `String` —,
une insertion dans la table des relations, plus les redimensionnements des
tables). La prochaine marche n'est plus dans l'hydratation elle-même mais
dans la représentation des relations : table de symboles pour les noms de
types (§7.4 de STORAGE.md, différée) et adjacence CSR (§6.3) — les mêmes
chantiers que ceux qu'exige la mémoire. **Réserve** : ceci est une
imputation par soustraction entre mesures (hydratation moins parcours
décodé, moins reconstruction), pas un échantillonnage ; le profil
`WPA`/`perf` promis avant `bulk_load` n'a pas été fait (pas d'outil
disponible sur la machine de mesure) et reste dû avant d'engager ces
chantiers — mode opératoire et remèdes possibles en **T6** (§8).

Effets collatéraux : les `BTreeSet` construits d'un bloc sont plus denses
que ceux remplis mutation par mutation — tas après hydratation **−3 %
mesuré à 200 k / 1 M** (978 → 950 Mio) et 2,38 Go à 500 k / 2,5 M contre
~2,6 Go extrapolés (non mesurés avant). La reconstruction a un pic
transitoire : les paires (trigramme, id), les clés triées et les seaux par
type coexistent avec les index fraîchement bâtis — ~60 Mo à 500 k, de
l'ordre de 1,2 Go à 10⁷ concepts, à compter dans le budget de la phase 5.

Décisions prises le 2026-09-17 (validées par le propriétaire du produit) :

1. **Codec — tranché : JSON par défaut**, postcard en option (lisible, aucun outil à adapter),
   postcard en option documentée. Réévaluer après `bulk_load` : in situ le
   décodage coûte 1,23 µs par enregistrement en JSON contre 0,57 en
   postcard (3 M enr.) ; si `apply` tombe à ~2 µs par enregistrement, le
   codec 1 gagnera ~1,25× sur l'hydratation totale, ~1,5× si `apply` tombe
   à 1 µs. Le codec ne devient décisif qu'une fois `apply` optimisé.
   Restriction retenue : le flux `meta` (schéma, règles, actions) reste en
   JSON quel que soit le codec du store — les types du schéma ne sont pas un
   contrat disque gelé ; un store non-JSON déclare `format_version = 2`, que
   les builds antérieurs refusent à l'ouverture au lieu de tronquer.
2. **`bulk_load`** : **fait** (item 4). Méthode publique de `OntologyGraph`
   (R1), un seul bump de génération par famille (R2), garde qui rebâtit les
   index même si le rejeu échoue ; mêmes vues observables qu'en mode normal
   (test d'égalité sur des scripts aléatoires : listes triées, par type,
   trigrammes, relations, règles, actions, traversées). Gain 1,1 à 1,75×,
   sous la cible 2–3× : le coût restant est dans les structures primaires des
   relations, pas dans les index dérivés.
3. **Phase 5 — tranchée le 2026-09-22** : la cible 10⁷ / 5×10⁷ sur 16 Go
   exigeait P1 **et** le CSR (P2–P4), le chantier le plus long et le plus
   risqué du plan, pour un client qui n'existe pas encore. Décision (choix
   produit, validé par le propriétaire du projet) : **la cible est portée
   par le nœud**. 10⁷ / 5×10⁷ sur **64 Go avec P1** (`--heap-fraction 0.8`
   sur un nœud dédié : ~1,3×10⁷ / 6,3×10⁷ estimés) ; sur **16 Go**, la
   garantie est **2×10⁶ / 10⁷** (P0 dès aujourd'hui, ~2,4×10⁶ avec P1). Le
   **CSR est reporté** à un besoin client au-delà de 5×10⁶ concepts sur un
   nœud contraint. Motifs : le matériel est le levier le moins cher ; P1 a
   une valeur garantie quelle que soit la cible et ne change pas le format ;
   le socle (§7.1, livré) refuse ou réduit explicitement un chargement qui
   ne tient pas, ce qui rend la garantie vérifiable ; rien dans P1 ne
   contredit un CSR ultérieur. Tableau de capacité : STORAGE.md §8.1.
   **Preuve exigée avant toute promesse contractuelle à 10⁷** : `bench gen`
   à 5×10⁶ / 2,5×10⁷ puis `bench hydrate` sur une machine de 64 Go, écart
   estimation / tas consigné en §7.8 (une heure, VM louée si besoin).
   Décision du 2026-09-22 : pas faite tant que personne ne demande 10⁷ ;
   elle ne conditionne pas P1 (J5 se mesure sur le store 5×10⁵).

**Sortie de phase.** Générateur et benchs livrés, chiffres mesurés
(STORAGE.md §7.7–7.8), codec tranché, `bulk_load` livré, mesuré et relu ;
jalon J4 atteint pour ce qui est mesurable sur 16 Go (§9).

---

## 7. Phase 5 — Mémoire contrainte (P1 → P5)

Ordre, celui du tableau §0 et de §8 : **socle (§7.1, livré) → T1
(curseur, avant toute suite de la phase 5) → 5a = P1 (§7.2) → 5b sur
besoin client = T6 (profil d'`apply`, §8) puis P2–P4 (CSR, §7.3) puis
contrôleur (§7.4)**. Le plan supposait que P1 supprimait ~90 % de
l'empreinte ; la phase 4 a mesuré 25 à 30 % pour un ratio de 5 relations
par concept et des payloads de 1,3 Ko (§6.6, STORAGE.md §8.1) : c'est
pourquoi la cible 10⁷ est désormais portée par un nœud de 64 Go et non par
le CSR (§6.6, décision 3 du 2026-09-22).

### 7.1 Socle (avant P1) — **livré (2026-09-22, branche `feat/phase5-socle`)**

Ce qui a été fait, et où c'est testé :

- **Budget** (STORAGE.md §8.1) : explicite → cgroup v2 (cgroup du
  processus et ses parents, pas seulement la racine) → cgroup v1 →
  `MemAvailable` → Windows `GlobalMemoryStatusEx` → inconnu. Fraction
  `--heap-fraction` (0,6) refusée par la CLI hors de ]0 ; 1]. Sources
  dures (limite) / molles (mémoire libre) distinguées : `adaptive` ne
  retranche que sous une limite dure, sinon tout est chargé avec
  `over_budget` en `warn` (le comportement d'avant le socle est conservé
  sur un hôte nu — revue du 2026-09-22). La limite de Job Object
  Windows n'est **pas** lue (décision : `--memory-budget-mb` couvre ce cas).
  Tests : analyseurs sur fixtures (`max`, sentinelles v1, `MemAvailable`
  absent), borne de la fraction — `budget.rs` unitaires.
- **Estimation par domaine** depuis les zone maps du MANIFEST (`records`,
  `edges`, `payload_bytes` étaient déjà présents depuis la phase 3 : rien
  à ajouter au plancher §8.7) **plus** les compteurs du segment actif
  (`Active::counters`). Formule et coefficients en STORAGE.md §8.1. Tests :
  égalité stricte estimation ↔ compteurs écrits sur un store à partitions
  multiples, avant et après réouverture — `crates/storage/tests/budget.rs`.
- **Plan de chargement** `plan_load(estimations, budget, mode, --ns)` :
  `strict` refuse avec requis / budget / disponible / source / plus gros
  domaines (R17) ; `adaptive` prend les domaines du plus petit au plus
  grand ; `--ns` gagne, nom inconnu refusé ; budget inconnu → tout, avec
  `warn`. Tests : trois modes, budget exact, un octet de moins, store vide,
  hydratation effective des seuls domaines retenus — storage unitaires +
  intégration ; bout en bout par le binaire (`stats` refusé en strict à 1
  Mio avec code de sortie non nul et message ; charge partielle en adaptive
  = multiple entier de domaines ; budget 0 → 0 concept mais démarre ;
  variables d'environnement, priorité du drapeau, mode et fraction
  invalides refusés par `clap` avant d'ouvrir le store) —
  `crates/cli/tests/memory_socle.rs`.
- **Exposition** : `GET /stats.memory` et gauges `/metrics` (liste en
  STORAGE.md §8.1), `null` / absentes sans store disque. Tests : plan
  partiel, plan complet, budget inconnu, sans plan —
  `crates/server/tests/memory_socle.rs`.
- **Bench** : `bench hydrate --json` imprime `estimate_mib` et
  `estimate_vs_heap_pct` ; écart consigné en STORAGE.md §7.8.

Hors socle, reporté : `majflt/s` (n'a de sens qu'avec des payloads sur
disque, P1) ; palier par domaine dans `/metrics` (n'existe qu'à partir de
P1) ; Job Object Windows.

### 7.2 P1 — payloads sur disque — **livré 2026-09-23** (branche `feat/p1`)

Le plus gros chantier : `DashMap<ConceptId, Concept>` devient
`DashMap<ConceptId, Slot>` avec `Slot { ctype: Sym, name: Sym, loc: Loc,
gen }` et `get_concept` relit le payload via `Loc` depuis le segment scellé
(mmap) **hors verrou** (R12). Touche `crates/graph` en profondeur ; H7/R1
restent vrais. Le payload d'un enregistrement dans le segment **actif** est
gardé en heap jusqu'au scellement (sa taille est bornée par le seuil de
roulement) : le segment actif n'est jamais lu par le graphe (R11), ce qui
lève la mention initiale d'un `pread` sur l'actif.

Tel que livré :

- **Slot** : `{ concept_type: String, name: String, loc: Option<Loc>,
  payload: Option<(description, properties)> }` ; `Loc { ns_id: u16,
  partition: u32, offset: u64 }`. Les `Sym` (table de symboles §7.4) et le
  `gen` du §6.2 de STORAGE.md restent la cible ; le slot livré tient en
  ~80 o hors chaînes. `Option<payload>` est le drapeau de résidence.
- **Lecture** : `get_concept`, les listings (`list_concepts_page`,
  `list_concepts_after`, `all_concepts`, `concepts_of_type`) et `expand`
  clonent le payload s'il est résident, sinon copient la `Loc`, relâchent
  le shard, puis lisent via `PayloadSource` (le `SealedIndex` du store :
  `RwLock<HashMap<(ns, partition), Arc<SealedSegment>>>`, l'`Arc` cloné
  sous un verrou de lecture court, décodage hors verrou ; jamais le
  `Mutex` du store). L'id du record est vérifié. Échec →
  `GraphError::PayloadUnavailable` → HTTP 500 explicite (R17), jamais un
  404 ni un panic ; les listings faillibles (`try_*`) sont utilisés partout
  où un panic serait remonté à un client ou tenu sous le verrou du store
  (compaction, vérification de migration, graines du sous-graphe).
- **Écriture** (R8) : le store écrit le record, puis appelle
  `set_loc(id, loc, resident = true)` **avant** que le graphe applique ;
  la position est mise en attente et rattachée à l'apply. Une position
  résidente arrivant pour un slot déjà évincé reste en attente (le record
  est dans l'actif, illisible par le lecteur) : le slot garde sa position
  scellée jusqu'à l'apply. Au roulement, le store appelle
  `partition_sealed(ns, partition)` : les payloads résidents de cette
  partition sont libérés (index latéral partition → ids ; une position
  arrivant après le scellement de sa partition est évincée aussitôt).
- **Hydratation** : `resident = record dans la partition active` ; les
  records scellés d'un domaine P1 ne retiennent donc jamais leur payload.
  Le rejeu d'un `UpdateConcept` remplace en place sans relire l'ancien
  payload.
- **Compaction** : nouveaux segments dans l'index → relocalisation de
  chaque concept depuis les entrées `.idx` (`set_loc(.., false)`) →
  retrait des anciennes partitions → suppression des fichiers. Un lecteur
  tenant encore un `Arc` d'un ancien segment pendant quelques
  microsecondes fait échouer la suppression sous Windows ; le fichier est
  alors renommé `.old` et balayé à l'ouverture suivante (ceiling consigné).
- **Palier par domaine** : `SegmentStore::set_tiers(HashMap<ns_id, Tier>)`
  avant l'hydratation ; défaut P0 partout. Le plan de chargement
  (STORAGE.md §8.1) choisit P1 pour un domaine qui ne tient pas en P0 mais
  tient sans ses payloads ; `--tier auto|p0|p1` (`ONTOLOGY_TIER`) force ;
  `/stats.memory.p1_domains`, `.resident_payloads`, gauges
  `ontology_domains_p1`, `ontology_resident_payloads`.
- **Tests** : graphe (P0 inchangé ; éviction et relecture ; ordre
  write-ahead ; scellement par partition ; mise à jour d'un concept
  évincé ; scripts aléatoires P0 = P1 sur listings, curseur, `expand`,
  `all_concepts` ; suppression ; H7/R1), store (lecteur sur chaque entrée
  de chaque partition scellée, refus des offsets faux et des relations ;
  hydratation P1 = P0 ; éviction réelle ; roulement ; relocalisation par
  compaction ; **écrivain et quatre lecteurs concurrents** sur un domaine
  P1), serveur (P1 à travers HTTP : GET, listing, PATCH puis roulement ;
  payload illisible → 500), CLI (`--tier p1` bout en bout).
- **Mesure J5** : STORAGE.md §7.8.

### 7.3 P2 → P4 — index dérivés sur disque (**sur besoin client**)

`.srt` et `.adj` construits au scellement (§8.3), R15 pour chacun ; fusion
k-way pour le listing ; **prérequis : curseur de pagination (T1)**, sinon
`GET /concepts?offset=` devient O(offset). Déclencheur (décision du
2026-09-22, §6.6) : un tenant au-delà de 5×10⁶ concepts sur un nœud
contraint, ou une latence d'expansion de graphe que le CSR seul résoudrait ;
T6 précède dans tous les cas.

### 7.4 Contrôleur

Seuils 70/85/55 % avec hystérésis 60 s (§8.5), LRU par `AtomicU64`,
transitions journalisées en `warn`. Test : injection d'un budget artificiel,
vérifier absence d'oscillation sous charge constante.

---

## 8. Transverse

### T1 — Pagination par curseur (à faire **tôt**, avant toute phase 5) — **livré 2026-09-22**

`GET /concepts?cursor=<base64(ctype, name, id)>&limit=` avec `next_cursor`
dans la réponse ; `offset` conservé mais documenté déprécié. `concepts_sorted`
supporte déjà `range(..)` sur la clé `(type, name, id)` (R6). Idem
`GET /relations` sur `RelationId`. Web UI et OpenAPI à jour.

Tel que livré (branche `feat/t1-cursor`) :

- **Curseur** = clé de tri de la ligne, sérialisée JSON puis base64url sans
  padding : opaque pour le client, autonome pour le serveur (la suppression
  du concept nommé par le curseur n'invalide pas le curseur), sûre dans une
  URL. Concepts : `(concept_type, name, id)` ; relations : `RelationId`.
- **Graphe** : `list_concepts_after` / `list_relations_after` entrent dans
  les ensembles triés par `range((Excluded(clé), Unbounded))` sur chacun des
  chemins existants (ensemble global, seau d'un type, fusion k-way des
  sous-types, intersection de trigrammes filtrée après coup, adjacence des
  relations) : une page coûte O(log N + page) où qu'elle commence, contre
  O(offset) avant. `list_*_page` (offset) est inchangé et délègue à la même
  fonction interne avec `after = None`.
- **API** : `cursor=` sur `GET /concepts` et `GET /relations` ; la réponse
  porte `next_cursor` (`null` sur la dernière page, une page courte termine
  la marche) **y compris sur une page par offset**, pour qu'un client
  commence à `offset=0` et poursuive par curseur. Sur une page par curseur,
  `offset` et `track_total` sont ignorés et `total` vaut `null` (compter,
  c'est parcourir). Curseur illisible → 400 `invalid cursor`, curseur vide →
  absent. `offset` reste servi, marqué `deprecated` dans `/openapi.json`.
  L'ETag ne change pas (génération des données).
- **Web** : la page Concepts pagine par pile de curseurs (`›` empile
  `next_cursor`, `‹` dépile) ; les sauts numérotés disparaissent, ce qui est
  la propriété même d'un curseur ; le total affiché vient de la première
  page. Le client `api.ts` expose `cursor` et `next_cursor`.
- **Règles et actions** : non concernés (`STORAGE.md` §8 ne les met pas sur
  disque ; leurs listes restent petites) — YAGNI, à faire le jour où un
  index trié sur disque les touche.
- **Tests** : graphe — la marche par curseur reproduit exactement le
  listing par offset sur les huit chemins de balayage et cinq tailles de
  page, un curseur survit à la suppression du concept qu'il nomme et aux
  insertions de part et d'autre ; relations, cinq chemins ; serveur —
  contrat complet (première page par offset puis curseur, filtres
  transportés, `total` nul, dernière page, ETag, 400) pour les deux
  ressources ; web — pile de curseurs, retour arrière, page vidée par une
  suppression ; `bench query` mesure la page par curseur à côté de la page
  par offset (chiffres en STORAGE.md §7.8).

### T2 — CI

Matrice `ubuntu-latest` × `windows-latest` dès la phase 2 ; `cargo test
--workspace` ; job `bench` manuel (`workflow_dispatch`) en phase 4.

État 2026-09-25 : matrice et `cargo test --workspace` livrés (phase 2),
plus `coverage` avec seuil 90 % par crate ; **le job `bench` manuel manque**
(`ROADMAP.md` §3.7 bis).

### T3 — Observabilité

`/metrics` : par flux, nombre de segments, taille, `next_seq`, nombre de
`sync_data` cumulés, durée de la dernière compaction ; par domaine, palier
courant (phase 5).

État 2026-09-25 : livré pour la mémoire (`ontology_memory_*`,
`ontology_domains_*`, `ontology_resident_payloads`, socle §7.1 et P1) ;
**manque** tout le par-flux (segments, taille, `next_seq`, `sync_data`,
dernière compaction) et le palier par domaine en étiquette
(`ontology_domains_p1` est un total). Le store expose déjà tout cela
(`OpenReport`, MANIFEST, `syncs`, `CompactionReport`) : quelques jauges à
brancher (`ROADMAP.md` §3.7 bis).

### G — Gros documents (décision avant la phase 3)

L'ingest texte met le fichier entier dans la description du concept
(STORAGE.md §10.7). Options : (a) découper en fragments liés au document
par une relation `part_of`, chaque fragment restant un payload de l'ordre
du Ko ; (b) stocker le texte hors du graphe (fichier ou flux `blob` par
domaine) et ne garder qu'une référence et un extrait. **Recommandation :
(a)**, qui ne demande aucun nouveau type de fichier et améliore le
retrieval (les fragments sont l'unité naturelle du RAG). À trancher avant
la phase 3 : le choix fixe le seuil de roulement et l'estimation R14.

### R — Index de retrieval (indépendant du stockage, en parallèle)

`HybridIndex` est reconstruit à chaque démarrage (`reindex_all`) et la
recherche vectorielle est un cosinus brute force en O(N). À la cible de
10⁷ concepts c'est des secondes par requête et des minutes au démarrage,
avant que l'embedding lui-même ne coûte quand un vrai modèle remplacera
l'embedder par défaut. Chantier : (1) persister les vecteurs dans un fichier
dérivé par domaine, reconstructible (R7), invalidé par le hash du texte
indexé ; (2) index approximatif (HNSW) construit au scellement d'un segment,
`mmap` ; (3) index lexical incrémental plutôt que reconstruit. Peut avancer
en parallèle de la phase 2, il ne touche pas au format des `.data`.

**Première tranche livrée le 2026-09-23 (branche `feat/r-retrieval`) —
mesurer, puis corriger les causes avant d'ajouter des structures.**
Mesuré sur le store 5×10⁵ / 2,5×10⁶ (`bench query`, STORAGE.md §7.8) :

- Le mur du démarrage n'était pas l'embedding mais un **O(N²)** :
  `VectorIndex::insert` retrouvait l'id par balayage linéaire des lignes,
  soit 1,25×10¹¹ comparaisons à 5×10⁵. Une table id → position (O(1)) et,
  côté lexical, la liste des termes de chaque document pour ne toucher que
  ses propres postings : `reindex_all` passe de **~7 minutes à 17–28 s**,
  moins que l'hydratation elle-même (32 s).
- La recherche hybride était à **465 ms p50 / 2,1 s p99** : tri complet des
  N scores dans les deux index, parcours des postings des termes présents
  dans presque tout le corpus (poids IDF ≈ 0), produit scalaire non
  vectorisé. Sélection top-k en O(N), saut des termes présents dans plus
  de la moitié des documents (au-delà de mille postings), huit
  accumulateurs dans `cosine` : **70 ms p50 / 100 ms p99**, aucune
  dépendance ajoutée, résultats identiques (les termes sautés pesaient ≈ 0).
- Ce qui reste est le balayage O(N·dim) attendu : ~70 ms à 5×10⁵, donc
  ~0,3 s à 2×10⁶ (garantie 16 Go) et ~1,4 s à 10⁷ (cible 64 Go).

**Décisions datées (2026-09-23), amendement du chantier** :

1. **(1) persistance des vecteurs — reportée** tant que l'embedder est le
   hachage : recalculer 5×10⁵ vecteurs coûte des secondes, un fichier
   dérivé n'en économiserait pas plus. Elle redevient nécessaire le jour
   où un modèle d'embedding réel (secondes par millier de textes) remplace
   `HashEmbedder` ; à faire alors avec l'invalidation par hash du texte,
   comme prévu.
2. **(2) HNSW — reporté sur mesure** : déclencheur = un P95 de `/retrieve`
   au-delà de 200 ms, soit ~1,5×10⁶ concepts avec l'embedder actuel, ou
   l'arrivée d'un modèle d'embedding (dimension et coût de la distance
   supérieurs). Construit au scellement, `mmap`, comme prévu.
3. **(3) index lexical incrémental** : il l'était déjà par insertion ; le
   coût de mise à jour est désormais borné aux termes du document.
4. **Jalon JR** reformulé : « `reindex_all` sous la durée de
   l'hydratation ; `/retrieve` P95 < 200 ms jusqu'à 2×10⁶ » — atteint à
   5×10⁵, à mesurer à 2×10⁶ ; l'ancienne formulation (« supprimé du
   démarrage », « O(log N) à 10⁷ ») reste l'objectif de la tranche HNSW.

### T5 — Un store par tenant — **tranché 2026-09-15, README écrit 2026-09-24**

Position à écrire dans le README et à honorer dans le code : le `ns` n'est
pas une clé de répartition horizontale ; le multi-tenant est un store par
tenant (STORAGE.md §10.8). Décider processus par tenant ou processus
multi-store avant la phase 3 (impact : fichiers ouverts, plancher §8.7).

Décision : **un processus par tenant** (STORAGE.md §10.8). Le code
l'honore (un `LOCK` par store, budget mémoire par processus §8.1, `serve
--data <tenant>`), le README porte la position depuis le 2026-09-24
(section « Deployment model »). Le chantier production de `ROADMAP.md`
§3.8 en est l'application : une image, un conteneur par client, la limite
cgroup du conteneur étant le budget que le socle lit.

### T6 — Profil d'`apply` par échantillonnage (à faire **avant** le chantier relations de la phase 5)

**Pourquoi.** Toutes les imputations de coût de la phase 4 (§6.6, STORAGE.md
§7.8) sont des **soustractions** : hydratation totale moins parcours décodé,
moins reconstruction des index dérivés. Ce qui reste (~15 s sur 20 à 500 k /
2,5 M) est attribué **par élimination** aux structures primaires des
relations (~5 µs par relation : quatre entrées d'adjacence dans des
`DashMap`, deux clones de `String` du nom de type, une insertion dans la
table des relations, les redimensionnements des tables). C'est plausible,
pas observé. Or le remède dépend de la vraie cause :

| Si le temps est surtout dans… | Remède | Effort |
|---|---|---|
| les redimensionnements des tables de hachage au fil des insertions | pré-dimensionner les `DashMap` depuis les compteurs du MANIFEST (R14) au début du `bulk_load` | ~1 jour |
| les clones de `String` des noms de types de relation | table de symboles (STORAGE.md §7.4), `Sym` à la place de `String` dans l'adjacence typée | quelques jours |
| les quatre insertions d'adjacence elles-mêmes | adjacence CSR (STORAGE.md §6.3), c'est-à-dire le palier P2–P4 | plusieurs semaines |

Engager le CSR sur une déduction alors qu'une demi-journée de mesure peut
la confirmer ou l'infirmer contredirait la règle « mesurer, puis décider ».

**Quoi mesurer.** `ontology bench hydrate` (release) sur le store `bench gen
--concepts 500000 --relations 2500000 --ns 5 --payload 1300`, profil par
échantillonnage sur toute la durée, répartition par fonction et par pile
d'appels (flame graph) ; en particulier la part de `insert_relation_exact`,
`upsert_concept` / `prepare_concept`, `DashMap::entry` / rehash, allocations
(`String::clone`), `end_bulk`. Une seconde capture sur `bench append --n
2000 --batch 100` donne le même profil pour le chemin d'écriture live.

**Comment, au choix.**
1. Linux : `perf record -g` + `cargo flamegraph` (`cargo install flamegraph`),
   `[profile.release] debug = 1` le temps de la capture pour avoir les
   symboles. À faire sur le nœud cible : cela répond en même temps à la
   question de la machine représentative (§6.6, décision 3).
2. Windows : Windows Performance Toolkit (WPR pour capturer avec pile
   d'appels en mode administrateur, WPA pour analyser), symboles via
   `debug = 1` également. Non installé sur la machine de développement.
3. À défaut d'outil externe : instrumentation interne derrière une
   fonctionnalité de compilation (`--features profile-apply`) — compteurs
   `Instant` cumulés autour de chaque étape de `apply_prepared_concept` et
   `insert_relation_exact` (validation, index des noms, adjacence, table
   primaire), remontés dans `BulkLoadReport`. Moins précis (le chronométrage
   perturbe des étapes de la microseconde), mais sans dépendance.

**Sortie attendue.** Un tableau « fonction → % du temps d'`apply` » dans
STORAGE.md §7.8, et le choix motivé entre les trois remèdes ci-dessus,
consigné en §6.6 / §7 avant d'ouvrir le chantier relations.

### T4 — Documentation

- STORAGE.md : D1-D6 reportés le 2026-09-08 (H12, H15, §1, §4.3, §4.3 bis,
  §4.5, §5, §8.0, §10.4, §10.7, §10.8). Reste : Windows en §7.6 et §8.1.
- PERFORMANCE.md : mis à jour le 2026-09-08 pour renvoyer à STORAGE.md (H1
  assouplie, R4↔R9, non-objectif levé, pagination par curseur).
- README : section « Stockage » remplaçant « WAL + bincode snapshots »
  (le README annonce déjà `bincode`, ce qui est faux aujourd'hui).

---

## 9. Ordre d'exécution et jalons

```
S1-S2   Phase 1 ─ livré ──────┐
S2      Décisions D1-D6 ─ validées ┤
S3-S5   Phase 2  ─────────────┤── T2 (CI Windows, livré) ; R en parallèle
S5      G (gros documents) + T1 curseur
S6-S8   Phase 3  ─────────────┘
S9      Phase 4 : générateur 10⁷ + benchs → GO / NO-GO codec, bulk_load
S10     Phase 5a : socle (budget, R14, strict/adaptive) ─ livré 2026-09-22
S10     T1 curseur (prérequis de 5a, §0) ; T en parallèle : couverture Rust ≥ 90 % par crate puis seuil CI
S11-S13 Phase 5a : P1 (slot + Loc, payloads relus hors verrou) ; J5 prouvé à 5×10⁶ sur 64 Go
S14+    Phase 5b, uniquement sur besoin client : T6 profil d'apply → P2-P4 (CSR) → contrôleur ; R en parallèle
```

Jalons vérifiables :

| Jalon | Preuve |
|---|---|
| J1 (fin phase 1) | **Atteint** : test « append échoué → mémoire inchangée » vert sur les 13 endpoints ; `sync_data` présent ; 136 tests, CI 2 OS |
| J2 (fin phase 2) | **Atteint** sur la branche : migration automatique de `graph.log` au démarrage, redémarrage HTTP sur `SegmentStore` testé, CI 2 OS à confirmer par la PR |
| J3 (fin phase 3) | **Atteint** : trois domaines dans `examples/finance` (`parties`, `contrats`, `facturation`), hydratation sélective testée, 2 syncs par lot de 100 sur 2 domaines |
| J4 (fin phase 4) | **Atteint pour ce qui est mesurable ici** : §7.7–7.8 de STORAGE.md remplis de chiffres mesurés à 2×10⁵ / 10⁶ et 5×10⁵ / 2,5×10⁶, codec tranché (JSON par défaut, postcard en option), `bulk_load` livré et mesuré ; la cible 10⁷ / 5×10⁷ (45 à 50 Go en P0) n'est pas hydratable sur 16 Go — c'est la mesure elle-même qui le montre ; extrapolation linéaire documentée |
| J5 (P1) | **Atteint 2026-09-23** (STORAGE.md §7.8) : sur le store synthétique 5×10⁵ / 2,5×10⁶, mémoire privée ÷ 1,57 (critère ≥ 1,4), hydratation 1,19× (≤ 1,2×), P95 `GET /concepts/{id}` 1,0× (< 2×) ; payloads résidents 0 après hydratation ; preuve de capacité 5×10⁶ sur 64 Go reportée à une demande client (§6.6) |
| J5b (CSR, sur besoin) | 10⁷ / 5×10⁷ hydraté sur 16 Go en `strict` ; ~16 o par arête mesurés |
| JR (retrieval) | **Tranche 1 atteinte 2026-09-23** (§8 R) : `reindex_all` 17–28 s à 5×10⁵ (< hydratation 32 s ; était ~7 min), `/retrieve` p50 70 ms / p99 100 ms (étaient 465 ms / 2,1 s). Tranche 2, sur mesure : `reindex_all` supprimé du démarrage (persistance, dès un modèle d'embedding) ; O(log N) par HNSW quand le P95 dépasse 200 ms (~1,5×10⁶ concepts) |

---

## 10. Risques

| Risque | Mitigation |
|---|---|
| `mmap` sur Windows : fichier mappé non supprimable, `FlushViewOfFile` | Renommage différé (§5.2.7), CI Windows dès la phase 2 |
| Refactor `Concept` → `Slot` (P1) casse H7/R1 par inadvertance | Champs privés, `prepare_*` comme unique chemin, tests d'invariance de PERFORMANCE.md §8.5 |
| `#[serde(untagged)]` sur `PropertyValue` incompatible avec `postcard` | Enum de stockage taguée + conversion ; décision en phase 4 sur mesure |
| Le `seq` global et le `.idx` positionnel divergent après un recovery partiel | Recovery régénère toujours le `.idx` depuis le `.data` (R7) ; jamais l'inverse |
| Plancher §8.7 qui grossit avec les partitions | Compaction des petites partitions livrée en phase 3, pas en phase 5 |
| Le partitionnement est fait avant que le découpage métier de l'ontologie soit stable | `ns` optionnel, `default` par défaut ; changer de `ns` refusé tant qu'il y a des instances (§5.1.4) |
