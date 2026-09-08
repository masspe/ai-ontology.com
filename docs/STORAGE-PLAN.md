# Plan de développement — stockage binaire partitionné

Plan d'implémentation de l'architecture décrite dans [`STORAGE.md`](./STORAGE.md).
Il part de l'état **réel** du code au 2026-09-08 (commit `f313b93`), pas de
l'état idéal ; la section 1 liste les écarts constatés, la section 2 les
décisions à prendre avant d'écrire une ligne, les sections 3 à 8 les phases.

Convention : `H*` / `R*` renvoient aux hypothèses et règles de
`PERFORMANCE.md` (H1-H10, R1-R6) et de `STORAGE.md` (H11-H25, R7-R17).

---

## 0. Résumé

| Phase | Contenu | Dépend de | Taille (j·h, indicative) | Livrable |
|---|---|---|---|---|
| 1 | Chemin d'écriture : R8, `fsync`, group commit, recovery tolérante | — | 4-6 | Store actuel durable et correct |
| 2 | Conteneur binaire mono-flux : `.data`/`.idx`, MANIFEST, CRC, recovery, migration | 1, décisions D1-D4 | 10-14 | `SegmentStore` remplace `FileStore` |
| 3 | Partitionnement par `ns` : routage, `.xref`, roulement, scellement `mmap`, compaction | 2, D2 | 10-14 | Store conforme à STORAGE.md §4-§7, §9 |
| 4 | Mesure et codec : générateur de store, benchs, `bincode`/`postcard` | 2 | 4-6 | Chiffres réels, codec binaire activé si gain mesuré |
| 5 | Mémoire contrainte P1-P5 : budget, `Loc`, `.adj`/`.srt`, paliers, hystérésis | 3, 4, D5 | 15-25 | Mode `adaptive` / `strict` |
| T | Transverse : curseur de pagination, CI Windows+Linux, métriques, docs | — (curseur avant 5) | 3-5 | — |

Les phases 1 à 3 s'exécutent **maintenant** (STORAGE.md §10.1 : « écrire le
format maintenant, garder l'index mémoire trivial »). Les phases 4 et 5 sont
**conditionnées à une mesure** sur un store de taille réaliste ; la phase 5
en particulier ne démarre que si un client réel dépasse la machine.

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

## 2. Décisions à prendre avant la phase 2

Chacune bloque une phase ; la recommandation est en gras.

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

Non fait, volontairement : le `seq` à 0 entre `open()` et `load_into()`
(§1.1) disparaît avec la phase 2 ; `spawn_snapshotter` reste non câblé
puisque le snapshot disparaît avec le format binaire.

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

---

## 7. Phase 5 — Mémoire contrainte (P1 → P5)

Conditionnée à un besoin client réel. Ordre imposé par le gain par palier
(§8.1) : P1 supprime ~90 % de l'empreinte, le reste est marginal.

### 7.1 Socle (avant P1)

- Budget (§8.1) : cgroup v2 → v1 → `MemAvailable` ; **Windows :
  `GlobalMemoryStatusEx` + limite de Job Object si présente** (absent de
  STORAGE.md, à ajouter).
- Estimation par domaine depuis les compteurs du MANIFEST (R14) ; ajouter au
  MANIFEST `payload_bytes` et `edges` par partition (ça grossit le plancher
  §8.7 de 16 o par partition — acceptable, à consigner).
- `--memory-mode strict` : refus au démarrage avec requis/disponible (R17).
  Livrable autonome et utile même sans P1.
- Métriques `/metrics` : budget, utilisé, palier par domaine, `majflt/s`
  (Linux seulement).

### 7.2 P1 — payloads sur disque

Le plus gros chantier : `DashMap<ConceptId, Concept>` devient
`DashMap<ConceptId, Slot>` avec `Slot { ctype: Sym, name: Sym, loc: Loc,
gen }` et `get_concept` relit le payload via `Loc` (mmap scellé ou `pread`
actif) **hors verrou** (R12). Touche `crates/graph` en profondeur ; H7/R1
restent vrais. Le payload d'un enregistrement dans le segment **actif** est
gardé en heap jusqu'au scellement (sa taille est bornée par le seuil de
roulement).

### 7.3 P2 → P4 — index dérivés sur disque

`.srt` et `.adj` construits au scellement (§8.3), R15 pour chacun ; fusion
k-way pour le listing ; **prérequis : curseur de pagination (T1)**, sinon
`GET /concepts?offset=` devient O(offset).

### 7.4 Contrôleur

Seuils 70/85/55 % avec hystérésis 60 s (§8.5), LRU par `AtomicU64`,
transitions journalisées en `warn`. Test : injection d'un budget artificiel,
vérifier absence d'oscillation sous charge constante.

---

## 8. Transverse

### T1 — Pagination par curseur (à faire **tôt**, avant toute phase 5)

`GET /concepts?cursor=<base64(ctype, name, id)>&limit=` avec `next_cursor`
dans la réponse ; `offset` conservé mais documenté déprécié. `concepts_sorted`
supporte déjà `range(..)` sur la clé `(type, name, id)` (R6). Idem
`GET /relations` sur `RelationId`. Web UI et OpenAPI à jour.

### T2 — CI

Matrice `ubuntu-latest` × `windows-latest` dès la phase 2 ; `cargo test
--workspace` ; job `bench` manuel (`workflow_dispatch`) en phase 4.

### T3 — Observabilité

`/metrics` : par flux, nombre de segments, taille, `next_seq`, nombre de
`sync_data` cumulés, durée de la dernière compaction ; par domaine, palier
courant (phase 5).

### T4 — Documentation

- STORAGE.md : amender H12/§4.3 (D1), §4.3 entrée 48 o (D2), §5 (D4), §8.0
  config (D5), §10.4 (D6), ajouter Windows en §7.6 et §8.1.
- PERFORMANCE.md : mis à jour le 2026-09-08 pour renvoyer à STORAGE.md (H1
  assouplie, R4↔R9, non-objectif levé, pagination par curseur).
- README : section « Stockage » remplaçant « WAL + bincode snapshots »
  (le README annonce déjà `bincode`, ce qui est faux aujourd'hui).

---

## 9. Ordre d'exécution et jalons

```
S1-S2   Phase 1  ─────────────┐
S2      Décisions D1-D6 ─────┤  (revue de STORAGE.md amendé)
S3-S5   Phase 2  ─────────────┤── T2 (CI Windows) en parallèle
S5      T1 curseur            │
S6-S8   Phase 3  ─────────────┘
S9      Phase 4 : générateur + benchs → GO / NO-GO codec, bulk_load
S10+    Phase 5 : uniquement sur besoin mesuré ; P1 d'abord
```

Jalons vérifiables :

| Jalon | Preuve |
|---|---|
| J1 (fin phase 1) | Test « append échoué → mémoire inchangée » vert ; `sync_data` présent |
| J2 (fin phase 2) | `data/graph.log` migré, `serve` démarre dessus, tests HTTP identiques sur les deux stores, CI verte sur 2 OS |
| J3 (fin phase 3) | Deux domaines réels dans `examples/finance` (ex. `modele` / `source`), hydratation sélective démontrée, 2 `fsync` pour 100 écritures sur 2 domaines |
| J4 (fin phase 4) | Tableau §7.7 de STORAGE.md rempli de chiffres mesurés à 10⁶ |
| J5 (P1) | RSS divisée par ≥ 5 sur le store généré, P95 `GET /concepts/{id}` < 2× P0 |

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
