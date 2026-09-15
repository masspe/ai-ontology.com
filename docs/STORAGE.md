# Persistance — format binaire, partitionnement et performances

Document compagnon de [`PERFORMANCE.md`](./PERFORMANCE.md). Celui-ci couvre
les index **mémoire** et la couche HTTP ; ce document couvre la couche
**stockage** : format des fichiers, partitionnement, hydratation, recovery,
et les considérations de performance propres au disque.

Les numérotations d'hypothèses (`H*`) et de règles (`R*`) continuent celles
de `PERFORMANCE.md` — H11+ et R7+ — pour qu'une référence croisée reste
sans ambiguïté dans le code et les commits.

> **Changement de scope explicite.** `PERFORMANCE.md` §1 classe
> « Persistance optimisée (snapshot/WAL plus rapide, mmap) » en
> **non-objectif**. Ce document lève ce non-objectif. Tout le reste de
> `PERFORMANCE.md` reste valide sans modification : R4 (« les index sont
> vérité reconstructible, pas vérité stockée ») en particulier est
> **renforcé**, pas contredit — voir R9.

---

## 1. Objectifs et non-objectifs

### Objectifs

- Remplacer le WAL JSON `graph.log` (`[u32 BE len][JSON]`) par un conteneur
  binaire où **l'index est séparé des données**.
- Rendre le démarrage proportionnel au **nombre d'entités**, pas au **volume
  de payload** : hydrater en ne lisant que les fichiers d'index.
- Permettre le chargement **partiel** du graphe, par domaine métier (`ns`),
  sans lire ni même ouvrir les fichiers des autres domaines.
- Faire du domaine l'unité de rétention, de compaction, de sauvegarde et de
  droits — pas seulement une optimisation de lecture.
- Garder la durabilité append-only : aucune écriture en place, un
  enregistrement acquitté est un enregistrement récupérable.

### Non-objectifs

- Transactions multi-enregistrements avec rollback. L'atomicité est
  l'enregistrement ; un lot est simplement une suite d'enregistrements.
- Réplication, consensus, multi-nœud.
- Compression par bloc (le champ `flags` la réserve ; l'implémentation
  attend un besoin mesuré).
- Écriture concurrente multi-processus sur le même store. Un seul processus
  écrivain, verrou de fichier au démarrage.

### Stratégie et cible de dimensionnement

**La mémoire d'abord, le disque quand il faut.** Le mode nominal est P0 :
le graphe entier, payloads compris, vit en RAM et le disque ne sert qu'à la
durabilité (§8.0). Les paliers P1 à P5 ne s'activent que si le budget
mémoire ne couvre plus le graphe, par domaine, sous pression, avec
hystérésis. Le format décrit ici a pour seule raison d'être de rendre cette
descente possible **sans changer de version de fichier** le jour où elle
devient nécessaire : tout ce qu'un palier relâche en mémoire doit déjà avoir
sa forme disque (R15).

Cible de dimensionnement retenue le 2026-09-08 : **un store = un tenant**,
et un store doit servir **10⁷ concepts et 5×10⁷ relations sur un nœud de
16 Go** — en P0 tant que les payloads le permettent, en P1 sinon. C'est le
jeu de données du banc de la phase 4 de `STORAGE-PLAN.md`. Le plafond dur
du format est 2³² concepts par store (§10.4).

Deux murs arrivent avant ceux du stockage et sont traités hors de ce
document : l'index de retrieval (`crates/index`, reconstruit à chaque
démarrage, recherche vectorielle en O(N)) et les payloads de plusieurs Mo
(documents entiers dans la description d'un concept). Voir
`STORAGE-PLAN.md` §8 (chantiers R et G).

---

## 2. Hypothèses fondatrices

| # | Hypothèse | Conséquence |
|---|---|---|
| H11 | Le log est strictement append-only ; un enregistrement scellé n'est jamais réécrit | Les segments scellés sont immuables → `mmap` sûr, index construit une fois, CRC vérifié une fois |
| H12 | `seq` est alloué par un compteur global unique, monotone, sans trou | Ordre total sur tous les flux : le recovery et `next_seq` en dépendent. **Pas** d'adressage direct par `seq` dans un `.idx` : une partition ne reçoit qu'une fraction des `seq` (les autres vont dans d'autres `ns`), son index est donc **positionnel** (l'entrée *i* est le *i*-ème enregistrement de la partition) et une recherche par `seq` est une recherche binaire — cf. §4.3, décision D1 |
| H13 | Le `ns` (domaine) d'un type de concept est déclaré dans l'ontologie et stable | Le routage d'écriture est statique : aucune résolution d'entité pour choisir la partition |
| H14 | `relation_type.domain` / `.range` déterminent le `ns` source et cible | Une relation inter-domaine est routable **avant** d'inspecter ses extrémités |
| H15 | Les `ConceptId` sont attribués séquentiellement, par un compteur **propre aux concepts** | Les zone maps `entity_min`/`entity_max` du MANIFEST sont sélectives. Le compteur unique partagé entre concepts, relations, règles et actions (état du code avant la phase 2) laisse des trous et avance quatre fois trop vite vers 2³² ; il est scindé par famille en phase 2. **Faux si passage aux UUID** — décision D6, §10.4 : on reste en `u64` |
| H16 | Le payload domine le volume, l'entité domine le nombre | Séparer index et données fait gagner un ordre de grandeur à l'hydratation |
| H17 | Un seul processus écrit dans le store | Pas de coordination inter-processus ; le `fsync` seul assure la durabilité |
| H18 | Les segments scellés tiennent dans l'espace d'adressage, pas nécessairement en RAM | `mmap` + page cache OS ; H1 de `PERFORMANCE.md` s'assouplit en « les *index mémoire* tiennent en RAM » |
| H19 | Le disque cible fait un `fsync` en 50–200 µs (NVMe) | Le group commit est nécessaire dès quelques milliers d'écritures/s — cf. §7.2 |
| H20 | Le nombre de domaines reste petit (≤ quelques dizaines par tenant) | Le nombre de segments actifs, donc de `fsync` par ronde, reste borné |
| H21 | Le graphe **peut** dépasser la mémoire disponible | H1 de `PERFORMANCE.md` ne tient plus ; il faut des paliers de dégradation explicites — §8 |
| H22 | Le budget mémoire est connaissable et exécutoire (cgroup v2 `memory.max`, `MemAvailable`) | On peut décider *avant* de charger, plutôt que découvrir en OOM (R14) |
| H23 | Les pages `mmap` sont récupérables par l'OS et ne comptent pas dans le budget heap | Déplacer une structure du heap vers un fichier `mmap` la retire du budget, même si la RSS n'en témoigne pas |
| H24 | Le working set est très inférieur au graphe : les accès se concentrent sur quelques domaines | La dégradation par domaine est efficace ; si l'accès est uniforme, seul le palier global sauve |
| H25 | Ontologie + zone maps du MANIFEST tiennent toujours en mémoire | C'est le plancher incompressible : s'il ne tient pas, le store est inexploitable (R17) |

---

## 3. Règles d'invariance

### R7 — Le `.data` est la vérité, le `.idx` est dérivé

Tout fichier d'index (`.idx`, `.ent`, `.xref`) doit être **intégralement
reconstructible** par un scan séquentiel du `.data` correspondant. Aucune
information ne doit exister uniquement dans un index. Corollaire direct :
un `.idx` n'a **jamais** besoin d'être `fsync`é (§7.2).

### R8 — Disque avant mémoire

Une mutation écrit d'abord l'enregistrement dans le segment actif, puis met
à jour les structures mémoire. Jamais l'inverse : un crash entre les deux
laisse un enregistrement durable que l'hydratation retrouvera, alors que
l'ordre inverse laisserait un état mémoire que rien ne justifie.

Cette règle compose avec R2 de `PERFORMANCE.md` : la séquence complète d'une
mutation est *append disque → DashMap primaire → index dérivés → bump de
génération*.

### R9 — Deux familles d'index, deux régimes

| Famille | Exemples | Persisté ? |
|---|---|---|
| Index de **stockage** | `.idx`, `.ent`, `.xref` | Oui — ce sont des tables d'offsets, dérivées d'un fichier immuable |
| Index **sémantiques** | `concepts_sorted`, `concepts_by_type`, `name_trigrams`, adjacence typée | **Non, jamais** (R4) |

R4 interdit de sérialiser les index sémantiques. Cette interdiction reste
entière. Un `.idx` n'est pas un index sémantique : il ne connaît ni les
noms, ni les types, ni l'ordre d'affichage — uniquement `(seq, offset, len,
kind, ns, entity_id)`.

### R10 — Un `ns_id` n'est jamais réutilisé

Un `ns_id` u16 est attribué une fois et gelé dans le MANIFEST. Après
suppression d'un domaine, son identifiant est retiré, jamais recyclé —
sinon d'anciens segments scellés pointeraient vers le mauvais domaine.

### R11 — Ne jamais `mmap` le segment actif

Le segment actif grandit ; un `mmap` sur un fichier qui change de taille
sous le mapping est un `SIGBUS` en attente. Segments scellés en `mmap`,
segment actif en `pread`. Le passage de l'un à l'autre se fait au moment du
scellement, via `ArcSwap`.

### R12 — Aucun défaut de page sous un verrou

Résoudre la `Loc` sous le `RwLock` du shard, **relâcher**, puis lire le
payload dans le `mmap`. Un défaut de page froid coûte ~100 µs ; le prendre
sous verrou bloque tous les lecteurs du shard. C'est l'équivalent stockage
de R3 (« pas de lock long-tenu pendant un `await` »).

### R13 — Un changement de `ns` s'écrit des deux côtés

Si une entité change de type au point de traverser une frontière de
domaine : enregistrement complet dans le nouveau `ns` **et** pierre tombale
(`kind = TOMBSTONE`) dans l'ancien. Sans elle, une hydratation du seul
ancien domaine ressusciterait l'entité.

Note : H5 de `PERFORMANCE.md` (type de concept immutable) rend ce cas
inatteignable aujourd'hui. R13 est la condition à honorer **si** H5 tombe.

### R14 — Estimer avant de charger

L'hydratation calcule le coût mémoire de chaque domaine depuis les compteurs
du MANIFEST **avant** d'ouvrir un fichier, et choisit son palier (§8.2) en
fonction du budget restant. Il ne doit exister aucun chemin qui charge
d'abord et découvre ensuite qu'il n'y a plus de place.

### R15 — Toute structure O(N) en mémoire a un équivalent disque et un fallback

Ajouter un index heap dont la taille croît avec le nombre d'entités est
autorisé **à condition** de fournir : (a) sa forme persistée dérivée
(`.adj`, `.srt`, `.ent`…), (b) le chemin de lecture dégradé qui l'utilise,
(c) le palier auquel la structure heap est relâchée.

Sans les trois, l'index est une bombe à retardement : il fonctionne jusqu'au
jour où le graphe d'un client dépasse la machine.

### R16 — La dégradation est par domaine, avec hystérésis

On ne dégrade jamais globalement quand un seul domaine est en cause, et on
ne remonte jamais d'un palier au même seuil que celui de la descente. Deux
seuils distincts (§8.5), sinon le système oscille entre paliers sous charge
constante.

### R17 — Échouer explicitement plutôt que se faire tuer

Si le plancher incompressible (H25) ne tient pas dans le budget, le
démarrage échoue avec le chiffre requis et le chiffre disponible. Être tué
par l'OOM killer après trois minutes d'hydratation n'est pas un mode de
défaillance acceptable : il ne dit rien de la cause et perd le segment actif
en cours d'écriture.

---

## 4. Disposition sur disque

```
store/
  MANIFEST.json                    # partitions, zone maps, ns_id gelés
  meta/000001.data|.idx            # flux ontologie — global, jamais partitionné
  graph/modele/000001.data|.idx|.ent|.xref|.adj|.srt
  graph/source/000002.data|.idx|.ent|.xref|.adj|.srt
  graph/restitution/...
```

`.adj` et `.srt` n'existent que pour les segments **scellés** : ce sont les
formes disque des structures mémoire, construites au scellement, et elles
n'ont d'utilité qu'en mémoire contrainte (§8.3).

Deux flux parce que les usages diffèrent : `meta` est rejoué intégralement
au démarrage, `graph` n'est jamais lu en entier. Les segments roulent
indépendamment par domaine (64 Mo ou 100k enregistrements), et un segment
scellé est immuable (H11).

### 4.1 En-tête de fichier — 32 o

| Fichier | Contenu |
|---|---|
| `.data` | magic `GRFD`, version u16, codec u8, flags u8, partition_id u32, base_seq u64, réservé |
| `.idx` | magic `GRFI`, version u16, entry_size u16, partition_id u32, base_seq u64, count u32, réservé |

### 4.2 Enregistrement — en-tête 32 o + payload aligné 8

| Offset | Champ |
|---|---|
| 0 | `seq` u64 |
| 8 | `ts_micros` u64 |
| 16 | `payload_len` u32 |
| 20 | `crc32c(payload)` u32 |
| 24 | `kind` u8, `codec` u8, `flags` u8, réservé u8 |
| 28 | réservé u32 |

L'octet `codec` rend le conteneur agnostique au format de payload : JSON
aujourd'hui, `bincode` demain, sans casser les fichiers existants (§7.1).
`flags` réserve la compression par enregistrement.

### 4.3 Entrée d'index — 48 o, taille fixe

| Offset | Champ |
|---|---|
| 0 | `seq` u64 |
| 8 | `offset` u64 |
| 16 | `payload_len` u32 |
| 20 | `kind` u8, `flags` u8 |
| 22 | `ns_id` u16 |
| 24 | `entity_id` u64 — l'id de l'entité (`ConceptId`, `RelationId`, `RuleId`, `ActionId` selon `kind`) |
| 32 | `endpoints` u64 — `(source << 32) \| target` pour une relation, 0 sinon |
| 40 | `rtype_sym` u32 — symbole du type de relation (table gelée du MANIFEST), 0 sinon |
| 44 | `target_ns_id` u16 — domaine de la **cible** d'une relation inter-domaine, 0 sinon (phase 3 : c'est ce qui permet de reconstruire le `.xref` du domaine cible depuis les seuls index) |
| 46 | réservé u16 |

**Décision D2 (2026-09-08).** L'entrée porte l'id de l'entité **et**, pour
une relation, ses extrémités et son type. Sans l'id, `DeleteRelation` et
`UpdateRelation` ne se rejouent pas depuis l'index ; sans le type,
l'adjacence typée (`out_edges_typed`) ne se reconstruit pas. Or l'objectif
« hydrater sans ouvrir un `.data` » et les paliers P1-P4 en dépendent.
Seize octets de plus par enregistrement (16 Mo à 10⁶) évitent un changement
de version du fichier plus tard. Les symboles `rtype_sym` sont attribués
une fois et gelés dans le MANIFEST, jamais réutilisés — même règle que les
`ns_id` (R10).

**Décision D1 (2026-09-08).** L'index est **positionnel et dense** : l'entrée
*i* décrit le *i*-ème enregistrement de la partition, `entry = 32 + i × 48`.
Le `seq` de chaque entrée est croissant dans une partition ; le retrouver
est une recherche binaire, qui ne sert qu'au recovery et aux outils — le
chemin chaud ne cherche jamais par `seq`, la `Loc` d'un slot pointe
directement `(part, off, len)`.

### 4.3 bis Flux de destination par `kind` — décision D3

| `RecordKind` | Flux | Pourquoi |
|---|---|---|
| `Ontology` | `meta` | Rejoué intégralement, nécessaire au routage |
| `Concept`, `UpdateConcept`, `DeleteConcept` | `graph/<ns>` avec `ns = ns_of_type(concept_type)` | H13. Pour `DeleteConcept(id)`, le type est résolu en mémoire avant l'append (l'entité existe encore) |
| `Relation`, `UpdateRelation`, `DeleteRelation` | `graph/<ns_source>` + entrée `.xref` dans `ns_cible` si différent | H14 |
| `Rule`, `Action`, `DeleteRule`, `DeleteAction` | `meta` | Peu nombreux (H3), transverses aux domaines (`applies_to`, `subject`). Leur validation à l'hydratation tolère un id de concept d'un domaine non chargé |
| `Clear` | **supprimé** | Jamais écrit ; si `DELETE /graph` revient, c'est une compaction de chaque domaine vers un segment vide |

### 4.4 Fichiers auxiliaires

- `.ent` — 16 o par entrée, `(entity_id, seq)` trié. Recherche binaire quand
  la `HashMap` mémoire d'un domaine froid a été relâchée (§6.3).
- `.xref` — 24 o par entrée, `(target_id, source_id, seq)`. Arêtes entrantes
  provenant d'un autre domaine, **sans duplication de payload** : le `ns`
  cible garde une adjacence entrante complète même chargé seul. **Phase 3 :**
  fichier entièrement dérivé (R7), **reconstruit à chaque ouverture** et
  après compaction depuis les entrées `.idx` des autres domaines
  (`target_ns_id`), jamais synchronisé, en-tête `GRFX`. Il échappe à la règle
  d'immutabilité des segments scellés (`.data`/`.idx`), puisqu'il n'est
  qu'une vue. Une entrée est rangée dans la partition du domaine cible dont
  la plage de `seq` `[base_seq, base_seq suivant)` contient le `seq` de la
  relation. La maintenance à chaud (append au fil des écritures) arrive
  avec les lecteurs en cours de processus, en P1.
- `.adj` — CSR gelée sur disque : lignes `(node_id u64, off u32, len u32)`
  triées par `node_id`, suivies du tableau contigu des voisins. Recherche
  binaire sur les lignes, puis lecture séquentielle des voisins. Permet la
  traversée sans adjacence en heap (§8.3).
- `.srt` — clés de listing `(ctype_sym u32, name_key u64, id u64)` triées
  dans l'ordre exact de `GET /concepts` (R6 de `PERFORMANCE.md`). Permet la
  pagination ordonnée sans `BTreeSet` en heap, par fusion k-way des
  partitions (§8.3).

### 4.5 MANIFEST

JSON, petit, réécrit atomiquement (write + rename). Porte les `ns_id` gelés
(R10), la table des `rtype_sym` gelés (D2), les quatre compteurs d'ids par
famille (H15) et, par partition, les zone maps : `base_seq`/`last_seq`,
`entity_min`/`entity_max`, bitmap des `kind`, compteur d'enregistrements,
`payload_bytes` et `edges` (nécessaires à l'estimation R14). Une requête
bornée à un domaine élague ses partitions avant d'ouvrir un fichier.

---

## 5. Hydratation

```
meta  → rejeu intégral (ontologie, règles, actions — toujours)
graph → lecture des .idx (48 o/enregistrement) → slots + adjacence
      + .xref (24 o/arête entrante inter-domaine)
      puis, en P0 seulement : lecture séquentielle des .data → payloads
```

**Décision D4 (2026-09-08).** En P0 les payloads vivent en heap (§8.0), donc
l'hydratation P0 lit **aussi** les `.data`, en un seul passage séquentiel
par partition (`madvise(Sequential)`, CRC vérifié au passage — §7.3). Le
démarrage « index seul » est une propriété des paliers **P1 et au-delà**,
où les payloads restent sur disque et se relisent via `Loc`. Conséquence
honnête : en phases 2 et 3, le format n'accélère pas le démarrage — le
parsing des payloads domine (§7.1) — il apporte la durabilité, l'isolation
par domaine et la préparation de P1. Le gain de démarrage en P0 vient du
codec binaire (phase 4) ; en P1 le problème disparaît.

**Hydratation sélective en P0 (phase 3).** `load_domains(ns…)` charge
`meta` et les flux demandés, fusionnés par `seq`. Une relation, une règle
ou une action qui référence un concept d'un domaine non chargé est
**ignorée et comptée** (`HydrationReport.skipped_cross_domain`) : le
graphe mémoire P0 ne représente pas une arête pendante. La « résolution
à la demande » d'un voisin non chargé décrite plus haut est un contrat P1+.
Une règle de `meta` qui porte sur un domaine non chargé disparaît donc de la
vue partielle ; c'est la limite documentée de ce mode, réservé au
développement sur un sous-ensemble.

**Compaction (phase 3) : le store entier, pas un domaine.** Avec un rejeu
ordonné par `seq`, réécrire un seul domaine donnerait à ses enregistrements
des `seq` postérieurs aux règles de `meta` et aux relations inter-domaines
qui en dépendent, et le rejeu échouerait. La compaction réécrit donc
**tous** les flux depuis le graphe vivant, dans l'ordre des dépendances
(ontologie, concepts, relations — direction canonique des symétriques —,
règles, actions), avec des `seq` neufs, dans une nouvelle partition par
flux ; elle **vérifie** par rejeu dans un graphe de travail que le résultat
est sémantiquement identique au graphe vivant (concepts par id, multi-
ensemble d'arêtes, règles et actions), puis bascule et supprime les
anciennes partitions (renommées `.old` et balayées à l'ouverture suivante si
un mapping les retient encore, cas Windows). Un échec de vérification
laisse le store intact. La compaction **par domaine** de §1 reste
l'objectif ; elle exige un rejeu par passes de dépendance plutôt que par
`seq`, ce qui suppose de revalider les instances à chaque changement de
schéma — chantier lié à P1.

**L'ordre de rejeu est l'ordre global des `seq`, pas flux par flux.** Les
flux sont des fichiers indépendants, mais une règle écrite dans `meta`
référence des concepts écrits dans `graph/<ns>` juste avant elle, et un
changement d'ontologie dans `meta` doit précéder les concepts qui en
dépendent. L'hydratation fusionne donc les flux par `seq` (H12). Comme
`meta` est petit (H3), il est chargé en mémoire et fusionné au fil du scan
séquentiel des flux `graph`. Trouvé par test en phase 2 : le rejeu flux par
flux échouait sur `UnknownConcept` à la première règle.

Ordres de grandeur à 10⁶ enregistrements de ~1,3 Ko : index ~48 Mo
séquentiels ; payloads ~1,3 Go, parsés en ~30 s en JSON, ~2-3 s en codec
binaire. À 10⁷ : ~5 min en JSON en P0 — c'est là que P1 ou le codec cessent
d'être optionnels.

Le chargement est **sélectif** : `hydrate(&NsSet)` n'ouvre que les
partitions dont le `ns` est demandé. Un voisin vivant dans un domaine non
chargé apparaît comme un `id` non résolu — soit renvoyé tel quel (suffisant
pour une traversée d'un saut), soit résolu à la demande via
`hydrate(ns_of(id))`, le `ns` étant connu de l'ontologie sans accès disque
(H13).

---

## 6. Index mémoire dérivé du stockage

### 6.1 Sharding composite

Le domaine seul est un mauvais axe de concurrence : sur le jeu de données
actuel, 11 concepts sur 11 sont dans `modele`. Un verrou saturé, quatre
inutilisés. Les deux clés se composent :

```rust
#[inline]
fn shard_of(&self, ns: u16, id: u64) -> usize {
    let n = ns as usize;
    (self.base[n] + (fxhash(id) as u32 & self.mask[n])) as usize
}
```

Le **domaine** décide quoi charger, évincer, verrouiller ensemble et purger.
Le **hachage** décide comment paralléliser à l'intérieur. Le nombre de
shards par domaine est calculé à l'hydratation depuis les compteurs du
MANIFEST, pas figé à la compilation.

### 6.2 Slot de 32 o

Le slot porte de quoi router, filtrer et traverser — pas le payload :
`id u64`, `ctype: Sym`, `name: Sym`, `Loc { part u16, kind u8, off u32,
len u32 }`, `gen u32`. Les propriétés restent sur disque et se relisent à la
demande via `Loc` (R12 pour le verrouillage).

À ~90 o par concept (slot + entrée de hash) et 8 o par arête, 10⁶ concepts
et 5×10⁶ arêtes tiennent dans ~130 Mo résidents, **quel que soit le volume
de payload**. C'est ce qui rend H1 de `PERFORMANCE.md` tenable à mesure que
le graphe grossit (H18).

### 6.3 Adjacence : CSR + delta

La CSR est optimale en lecture et pathologique en insertion — or un log
append-only écrit en permanence. Structure à deux étages :

```rust
struct Adjacency {
    csr: Csr,                                              // gelée à l'hydratation
    delta: HashMap<u64, SmallVec<[u64; 4]>, FxBuildHasher>, // depuis
}
```

La CSR se reconstruit à la compaction du domaine, quand `delta` dépasse
quelques pour cent des arêtes. À l'hydratation, la construire en **deux
passes** (compter les degrés, puis remplir) : un seul `Vec` dimensionné
d'avance, zéro réallocation.

Cette structure alimente `out_edges` / `in_edges` de `PERFORMANCE.md` §4.1 ;
elle ne les remplace pas et ne change rien à leur contrat de lecture.

---

## 7. Considérations de performance

### 7.1 Le codec écrase tout le reste

À 1 352 o de payload moyen, `serde_json` désérialise à ~250 Mo/s, soit ~5 µs
par enregistrement. L'accès index + `mmap` coûte ~100 ns. **~98 % du temps de
lecture est dans le parsing**, pas dans la disposition disque.

Tant qu'on reste en JSON, optimiser l'alignement, le CRC ou le nombre de
`syscall` ne se mesure pas. Passer le payload en `bincode`/`postcard`
(l'octet `codec` est prévu pour) : 10 à 20× sur la désérialisation, ~40 % de
volume en moins sur les enregistrements `Ontology` qui répètent les mêmes
clés. **C'est la seule optimisation dont le gain se voit sans instrument.**

### 7.2 `fsync`, multiplié par le nombre de domaines

Un `fsync` coûte 50–200 µs (H19). Avec un `commit()` par écriture, on
plafonne à ~5 000 appends/s quel que soit le reste. Le partitionnement par
domaine aggrave mécaniquement le problème : *n* segments actifs, donc
potentiellement *n* `fsync` par ronde (H20 borne *n*).

Deux corrections, indissociables :

```rust
// (a) l'index n'est jamais fsyncé — il est reconstructible (R7)
self.data.sync_data()?;
self.idx.flush()?;

// (b) group commit : 1 fsync par ns *touché*, pas par écriture
pub fn commit_group(&self, batch: &[Event]) -> io::Result<u64> {
    let mut dirty: SmallVec<[Ns; 8]> = smallvec![];
    for ev in batch { dirty.push_unique(self.stream(ev.ns).append_buffered(ev)?); }
    for ns in dirty { self.stream(ns).sync()?; }
    Ok(self.next_seq.load(Ordering::SeqCst))
}
```

Un lot de 100 écritures réparties sur 2 domaines : 2 `fsync` au lieu de 200.
**Sans group commit, le partitionnement par domaine fait perdre en écriture
ce qu'il fait gagner en lecture.**

### 7.3 CRC : au scan, pas au chemin chaud

`crc32c` matériel tourne à plusieurs Go/s : le vérifier pendant
l'hydratation et le recovery est gratuit. Le revérifier à chaque lecture
d'un enregistrement chaud ne l'est pas. Vérification à l'hydratation et au
recovery uniquement.

### 7.4 Hachage

`HashMap` par défaut = SipHash, 3 à 5× trop cher sur des clés `u64`.
`FxHashMap` ou `ahash` partout, y compris dans la `SymbolTable` — cohérent
avec l'usage d'`AHashMap` déjà en place dans `PERFORMANCE.md` §4.1.

### 7.5 Petites partitions

Un domaine à 50 enregistrements produit un segment de 4 Ko. Multiplié par
tenant × domaine, cela donne des milliers de fichiers minuscules dont le
coût d'ouverture et de `mmap` dépasse celui des données. Rouler par taille
**ou** par âge, et compacter les segments scellés sous un seuil.

### 7.6 Compaction et mappings vivants

Réécrire un segment invalide son `mmap`. Il faut un `ArcSwap<Mmap>` par
partition pour que les lecteurs en cours terminent sur l'ancien mapping
avant que le fichier ne disparaisse. Même mécanisme qu'en R11 pour le
scellement.

### 7.7 Coût en écriture — ordres de grandeur

Un append de concept, en plus du coût mémoire déjà chiffré dans
`PERFORMANCE.md` §6 (~30 µs) :

| Poste | Coût |
|---|---|
| Sérialisation du payload | ~5 µs en JSON, ~0,3 µs en bincode |
| `write` bufferisé data + idx | ~0,2 µs (pas de `syscall` par enregistrement) |
| `fsync` | 50–200 µs, **amorti par le group commit** |
| Mise à jour `delta` d'adjacence | ~50 ns |

Sans group commit, le `fsync` domine tout le reste d'un facteur 5. Avec, en
lots de 100, le poste dominant redevient la sérialisation — donc §7.1.

---

## 8. Fonctionnement en mémoire contrainte

H1 de `PERFORMANCE.md` — « le graphe tient entièrement en RAM » — est une
hypothèse de développement, pas une propriété du système. Un client dont le
catalogue dépasse la machine ne doit pas provoquer un OOM kill : il doit
provoquer une **dégradation mesurable et annoncée**.

### 8.0 Le mode nominal reste « tout en mémoire »

**Si le budget couvre le graphe, rien de ce qui suit ne s'active.** P0 est
le comportement décrit dans `PERFORMANCE.md` sans modification : `DashMap`
primaires portant les entités **complètes, payloads désérialisés inclus**,
index secondaires résidents, aucun accès disque sur le chemin de lecture.
Le format binaire ne sert alors qu'à deux choses : la durabilité, et un
démarrage proportionnel au nombre d'entités plutôt qu'au volume.

La `Loc` du slot (§6.2) reste renseignée en P0 — elle ne coûte que 12 o et
c'est elle qui rend la descente vers P1+ possible sans reconstruire quoi que
ce soit — mais elle n'est **pas empruntée** tant qu'on est en P0.

Le comportement sous contrainte est un choix explicite, pas une surprise.
**Décision D5 (2026-09-08)** : le budget mémoire est une propriété du
déploiement, pas du store — le même répertoire peut tourner sur 2 Go ou
64 Go — il ne vit donc ni dans le MANIFEST ni dans `settings.json` (lu
*après* l'ouverture du store, modifiable à chaud). Il se donne au démarrage :

```
ontology serve --memory-mode adaptive --heap-fraction 0.6
ONTOLOGY_MEMORY_MODE=strict ONTOLOGY_HEAP_FRACTION=0.5   # équivalent Docker
```

Les valeurs effectives (budget calculé, mode, palier par domaine) sont
exposées dans `GET /metrics`.

- `strict` — le store refuse de démarrer si le graphe n'entre pas dans le
  budget (R17), avec le requis et le disponible. À utiliser sur un
  déploiement dimensionné, où une dégradation silencieuse serait un
  incident : on préfère un échec au démarrage qu'un P95 qui triple sans
  explication trois semaines plus tard.
- `adaptive` — les paliers §8.2 s'appliquent, par domaine.

Dans les deux modes, le franchissement d'un palier est un événement
journalisé au niveau `warn` avec le domaine, le palier et le poste mémoire
qui a déclenché. Une dégradation qui ne se voit pas dans les logs est un
bug.

### 8.1 Le budget, pas la RAM totale

Le budget se lit dans l'environnement d'exécution, jamais dans
`/proc/meminfo` `MemTotal` — un conteneur avec `memory.max = 2 Gi` sur un
hôte à 64 Gi verra 64 Gi et se fera tuer.

```rust
fn budget() -> u64 {
    let avail = cgroup_v2_max()                  // /sys/fs/cgroup/memory.max
        .or_else(cgroup_v1_limit)
        .unwrap_or_else(mem_available);          // /proc/meminfo MemAvailable
    (avail as f64 * CONFIG.heap_fraction) as u64 // 0.6 par défaut
}
```

L'estimation du coût d'un domaine se calcule depuis le MANIFEST, avant toute
ouverture de fichier (R14) :

```
coût(ns) ≈ payload_moyen × concepts   // P0 uniquement — entités complètes en heap
         +  90 o × concepts           // slot 32 o + entrée de hash + symboles
         +   8 o × arêtes             // CSR out + in
         +  60 o × concepts           // trigrammes du nom (H4)
```

Le premier terme est de loin le plus gros (~1,4 Ko par entité sur le jeu
actuel) et c'est le seul que P1 supprime : passer de P0 à P1 divise
l'empreinte par un ordre de grandeur, au prix d'une lecture `mmap` sur les
endpoints qui renvoient les propriétés. C'est pour ça que c'est la première
marche.

Parmi les index, le poste dominant est le troisième : les trigrammes coûtent
à eux seuls plus que le graphe qu'ils indexent. C'est la marche suivante.

### 8.2 Quatre paliers, décidés par domaine

| Palier | Résident | `?type=` / `?q=` | Listing ordonné | Traversée 1 saut |
|---|---|---|---|---|
| **P0** | tout, **payloads inclus** (état actuel) | O(bucket) / trigrammes | O(K) | O(deg) heap |
| **P1** | payloads relâchés (relus via `Loc`) | O(bucket) / trigrammes | O(K) | O(deg) heap |
| **P2** | sans trigrammes | O(bucket) / **scan `.srt`** | O(K) | O(deg) heap |
| **P3** | sans `by_id` ni `*_sorted` | binaire `.ent` / scan `.srt` | fusion k-way `.srt` | O(deg) heap |
| **P4** | sans adjacence | binaire `.ent` | fusion k-way `.srt` | binaire `.adj` + lecture contiguë |
| **P5** | domaine déchargé | résolu à la demande | exclu du listing sauf demande explicite | rehydratation à la volée |

Le palier est choisi **par domaine** (R16), pas globalement : le domaine
chaud reste en P0 pendant que les froids descendent. Sous H24, c'est ce qui
rend la dégradation quasi invisible en pratique ; si les accès sont
uniformes, seule la descente globale sauve, et elle se paie.

Un accès en P4 coûte typiquement 2 à 5 µs contre ~200 ns en P0 — un facteur
20, sur un chemin qui n'était pas le goulot (§7.1 : le parsing domine
toujours). C'est le bon ordre de dégradation.

### 8.3 Les structures qui rendent la dégradation possible

Descendre de palier n'a de sens que si la structure relâchée a un
substitut disque (R15). D'où deux fichiers supplémentaires, construits au
scellement d'un segment, donc payés une fois sur un fichier immuable (H11) :

```rust
// .adj — CSR gelée, mmap, zéro heap
struct AdjHeader { magic: [u8;4], version: u16, rows: u32, edges: u64 }
struct AdjRow    { node_id: u64, off: u32, len: u32 }   // trié par node_id
// suivi de: [u64; edges]  — voisins, contigus par ligne

fn neighbors<'a>(map: &'a Mmap, node: u64) -> &'a [u64] {
    match rows(map).binary_search_by_key(&node, |r| r.node_id) {
        Ok(i) => { let r = &rows(map)[i]; &edges(map)[r.off as usize..][..r.len as usize] }
        Err(_) => &[],
    }
}
```

`.srt` suit le même principe pour le listing : les clés
`(ctype_sym, name_key, id)` triées dans l'ordre exact de `GET /concepts`
(R6). La pagination devient une fusion k-way des `.srt` des partitions du
domaine — un tas binaire de taille « nombre de partitions », pas un
`BTreeSet` de taille N.

Les deux fichiers sont dérivés (R7) : perdus, ils se reconstruisent par un
scan du `.idx` et du `.data`. Ils n'existent pas pour le segment actif, dont
les structures restent en heap — leur taille est bornée par le seuil de
roulement, donc connue d'avance.

### 8.4 `mmap` n'est pas du heap, mais ce n'est pas gratuit

Sous H23, déplacer une structure vers un fichier `mmap` la retire du budget :
le kernel récupère ces pages sous pression au lieu d'invoquer l'OOM killer.
La RSS ne le reflète pas — elle compte les pages mappées présentes — donc
**piloter sur la RSS seule fait descendre de palier sans raison**.

L'indicateur utile est le taux de défauts de page majeurs
(`/proc/self/stat`, champ `majflt`) : il dit si le working set déborde
réellement. Deux appels à ne pas oublier :

```rust
map.advise(Advice::Sequential)?;  // pendant l'hydratation d'un .idx
map.advise(Advice::Random)?;      // ensuite, accès .adj / .data
map.advise(Advice::DontNeed)?;    // à l'éviction d'un domaine
```

### 8.5 Seuils et hystérésis

| Événement | Seuil | Action |
|---|---|---|
| Descente | budget utilisé > 70 % **ou** majflt/s > seuil | Domaine LRU descend d'un palier |
| Urgence | > 85 % | Domaines froids directement en P5 |
| Remontée | < 55 % pendant 60 s | Domaine chaud remonte d'un palier |

Les seuils de descente et de remontée sont volontairement écartés (R16). Un
seuil unique produit un système qui monte et descend en boucle sous charge
constante, en payant la reconstruction à chaque oscillation — coûteuse pour
les trigrammes, qui se reconstruisent en lisant tous les noms.

Le « LRU » est un simple `AtomicU64` par domaine portant le dernier `seq`
d'accès. Pas de liste chaînée, pas de verrou.

### 8.6 Le chemin d'écriture aussi

La pression mémoire ne vient pas que de la lecture :

- Le `delta` d'adjacence (§6.3) croît sans borne entre deux compactions.
  Déclencher la compaction sur **taille absolue** en plus du ratio, sinon un
  domaine peu volumineux mais très écrit gonfle indéfiniment.
- Les buffers d'écriture sont proportionnels au nombre de segments actifs,
  donc de domaines (H20). Sous pression, **réduire les buffers avant de
  dégrader les lectures** : c'est moins cher, et l'effet est immédiat.
- Le segment actif n'est jamais `mmap` (R11), donc son index vit en heap.
  Baisser le seuil de roulement sous pression le borne plus tôt, au prix de
  plus de fichiers (§7.5).

### 8.7 Le plancher incompressible

Ne peuvent jamais être évincés : l'ontologie (nécessaire pour router, donc
pour tout), les zone maps du MANIFEST (~64 o par partition), `next_seq`, le
verrou d'écriture.

```
plancher ≈ taille_ontologie + 64 o × partitions
```

Ce plancher **croît avec le nombre de partitions**, ce qui en fait le seul
poste vraiment dangereux à long terme : un store jamais compacté finit par
ne plus pouvoir démarrer. Compacter les petites partitions (§7.5) n'est donc
pas une optimisation, c'est une condition de survie.

Si le plancher ne tient pas dans le budget, R17 s'applique : refus explicite
au démarrage, avec le requis et le disponible.

### 8.8 Conséquences sur `PERFORMANCE.md`

| Section | Impact en mémoire contrainte |
|---|---|
| §4.1 `name_trigrams` | Première structure d'index relâchée (P2). `?q=` retombe sur un scan `.srt` — acceptable sous H4, les noms étant courts |
| §4.1 `concepts_sorted` / `by_type` | Relâchés en P3, remplacés par la fusion k-way des `.srt` |
| §4.2 pagination par `offset` | **Devient O(offset)** sur une fusion k-way : il n'y a plus de saut direct. Passer à une pagination par curseur `(ctype, name, id)` dès P3 |
| §4.3 `track_total` | Encore plus utile : le `total` exact impose de dérouler toute la fusion |
| §4.7 query cache (256 entrées) | Borné, donc conservé à tous les paliers. Sa valeur **augmente** quand les paliers descendent |
| §5 tableau de complexité | Valable en P0 ; ajouter une colonne P4 si le mode contraint devient nominal |

Le point d'API est le seul irréversible : **la pagination par offset ne
survit pas à la mémoire contrainte**. Si le mode dégradé est un scénario
réel, il vaut mieux exposer un curseur maintenant que casser les clients
plus tard.

---

## 9. Recovery

Le segment actif est le seul qui puisse être incohérent (H11 protège les
autres) :

1. Scan de la queue du dernier `.data` de chaque flux.
2. Troncature au dernier enregistrement dont le CRC et la longueur sont
   valides.
3. Reconstruction de la fin du `.idx` correspondant (R7).
4. Reprise de `next_seq` au `seq` du dernier enregistrement valide + 1.

Un `.idx` entièrement perdu se régénère par un scan séquentiel complet du
segment. C'est le prix de la séparation index/données, et il ne se paie que
sur incident.

---

## 10. Limites connues et points de vigilance

### 10.1 Le format ne devient rentable qu'à l'échelle

À 39 enregistrements et 52 Ko, un `Vec<Event>` chargé intégralement et
scanné linéairement répond en microsecondes. Le format devient rentable vers
10⁵–10⁶ enregistrements — ou plus tôt si l'isolation par domaine sert aux
droits et à la rétention plutôt qu'à la vitesse.

Stratégie recommandée : **écrire le format maintenant** (migrer un format
sur disque plus tard est douloureux), et **garder l'index mémoire trivial**
(une `HashMap`, pas de shards, pas de CSR) jusqu'à ce qu'une mesure justifie
le contraire. Le disque est difficile à migrer, la mémoire se réécrit en un
après-midi.

### 10.2 Surcoût de format

Conversion du `graph.log` actuel, vérifiée (CRC + round-trip sur les 39
enregistrements) :

```
source                52 915 o
format binaire        55 696 o   (+5,3 %)
  dont index            1 984 o
  dont MANIFEST           960 o
```

Le surcoût est l'index (48 o/enregistrement depuis D2, 32 o lors de cette
mesure) et le padding d'alignement. Il
s'inverse dès le passage à un codec binaire (§7.1).

### 10.3 Collision de vocabulaire `domain`

Dans l'ontologie, `relation_type.domain` / `.range` désignent le **type
source** et le **type cible**. Le partitionnement métier utilise donc `ns`,
jamais `domain`. Ne pas réintroduire le mot dans le code de stockage.

### 10.4 Zone maps et UUID — décision D6 : `u64` séquentiels

L'élagage par `entity_min`/`entity_max` suppose des ids séquentiels (H15).
Un passage aux UUID rendrait ces zone maps inutiles, casserait l'encodage
`(source << 32) | target` et les lignes CSR du `.adj`. **Tranché le
2026-09-08 : on reste en `u64` séquentiels.** Un besoin de fusion de stores
ou de fédération se traite par une propriété métier stable sur le concept,
pas par l'identifiant interne.

Deux corollaires :
- un compteur **par famille** (concepts, relations, règles, actions) au lieu
  du compteur unique actuel — sûr puisque les quatre types d'ids sont
  distincts, et quatre fois plus de marge sous 2³² ;
- `ConceptId < 2³²` est une **invariante vérifiée** dans `prepare_relation`
  et au démarrage, pas une hypothèse. 4,29 milliards de concepts par store
  est le plafond dur du format.

### 10.7 Gros payloads

H16 suppose des payloads d'environ 1,3 Ko. L'ingest de documents texte met
aujourd'hui le texte entier du fichier dans la description du concept : un
corpus de documents de quelques Mo sature la heap en P0 bien avant le
million d'entités, et l'index lexical indexe ce texte. Ce n'est pas le
format qui est en cause mais le modèle : découper les documents en fragments
ou stocker le texte hors du graphe avec une référence. À trancher avant la
phase 3 (chantier G de `STORAGE-PLAN.md`), car le choix influence le seuil de
roulement des segments et l'estimation R14.

### 10.8 Un store par tenant — décision T5 : un processus par tenant

Le `ns` sert la localité, la rétention et les droits, pas la répartition
horizontale (H20 le borne à quelques dizaines). Le multi-tenant est **un
store par tenant, servi par un processus par tenant** (`ontology serve
--data <tenant>`), tranché le 2026-09-15. Raisons : le verrou `LOCK`, le
budget mémoire (§8.1) et l'isolation des pannes sont naturellement par
processus ; un processus multi-store multiplierait fichiers ouverts (§7.5)
et plancher (§8.7) par le nombre de tenants sans rien simplifier. Le
routage des tenants vers leur processus est l'affaire du reverse proxy ou
de l'`auth-server`, pas du store. À revoir si le nombre de tenants dépasse
ce qu'une orchestration de processus gère confortablement (centaines).

### 10.9 Gros documents — décision G : fragments

Tranché le 2026-09-15 : un document texte plus long que 4 000 caractères
(`--chunk-chars`, 0 pour désactiver) est découpé en **fragments** aux
frontières de paragraphes. Le concept document garde un extrait de 600
caractères et les propriétés `fragments` et `chars` ; chaque fragment est un
concept de type `<Type>Fragment` (déclaré à la volée, domaine `default` sauf
déclaration explicite dans l'ontologie) relié au document par
`fragment_of` (ManyToOne). Les payloads restent de l'ordre du Ko (H16) et le
texte complet reste indexé par le retrieval, au lieu d'être tronqué à 64 Ko
comme avant. Les fragments sont l'unité naturelle du RAG (chantier R).

### 10.5 Déséquilibre entre domaines

Rien ne garantit l'équilibre : un domaine peut porter 95 % des entités.
C'est précisément pourquoi §6.1 compose domaine et hachage. Surveiller la
distribution des compteurs du MANIFEST ; si un `ns` dépasse ~70 % du total,
c'est le découpage de l'ontologie qu'il faut revoir, pas le sharding.

### 10.6 Amplification d'écriture des relations inter-domaines

Une relation inter-domaine écrit un enregistrement + une entrée `.xref`
(24 o) et touche deux segments, donc potentiellement deux `fsync` (§7.2). Si
le graphe est majoritairement inter-domaine, le découpage choisi est
mauvais : le bon découpage minimise les arêtes qui traversent.

---

## 11. Checklist d'évolution

1. **Lire R7-R17** en plus de R1-R6.
2. La nouvelle donnée est-elle reconstructible depuis le `.data` ? Si non,
   elle ne peut pas vivre dans un index (R7).
3. Le nouveau champ tient-il dans les octets réservés des en-têtes existants ?
   Sinon, incrémenter la `version` du fichier et gérer les deux formats en
   lecture — jamais réécrire un segment scellé pour migrer.
4. La feature ajoute-t-elle un `fsync` sur un chemin chaud ? Si oui, elle
   passe par `commit_group`.
5. La feature tient-elle un verrou pendant une lecture `mmap` ? Si oui,
   R12 est violée.
6. Les hypothèses H11-H25 tiennent-elles encore ? H15 (ids séquentiels) et
   H5 de `PERFORMANCE.md` (type immutable, cf. R13) sont les plus fragiles.
   `ConceptId < 2³²` (D6) est une invariante, pas une hypothèse : la
   vérifier, pas la supposer.
7. La feature ajoute-t-elle une structure dont la taille croît avec N ? Si
   oui, R15 exige sa forme disque, son chemin dégradé et son palier.
8. Le plancher (§8.7) augmente-t-il ? Une nouvelle donnée par partition dans
   le MANIFEST se paie sur chaque store, pour toujours.
9. **Mesurer sur un store de taille réaliste.** Sur 39 enregistrements, tout
   est rapide et aucune mesure n'a de sens.

---

## 12. Récap visuel

```
ÉCRITURE                                    HYDRATATION
────────                                    ───────────
append_concept(id, payload)                 hydrate(NsSet)
  │                                           │
  ├─ seq = next_seq.fetch_add(1)              ├─ meta/*.data  → rejeu ontologie
  ├─ ns  = ns_of_type(concept_type)   (H13)   │                 (intégral)
  │                                           │
  ├─ .data ← [hdr 32 o][payload][pad]         ├─ pour chaque ns demandé :
  ├─ .idx  ← [seq|off|len|kind|ns|ent|ends|rt] │    .idx  → slots + adjacence
  │         (48 o, jamais fsyncé — R7)        │    .xref → arêtes entrantes
  │                                           │    (.data lus en P0 seulement — D4)
  ├─ commit_group : 1 fsync / ns touché       │
  │                 (§7.2)                    └─ CSR construite en 2 passes
  │                                              (§6.3)
  └─ PUIS mémoire (R8) :
       shard_of(ns, id) → slot + delta
       → index sémantiques (R2)
       → bump_*_gen()
```

Paliers en mémoire contrainte (par domaine, §8.2) :

```
budget          P0 ─────► P1 ─────► P2 ─────► P3 ─────► P4 ─────► P5
utilisé        tout    − payloads  − trigr.  − by_id  − adjac.  déchargé
              (nominal)  (Loc)      (.srt)   (.ent)   (.adj)
  > 70 % ───────┴─────────┴──────────┴────────┴────────┘   descente LRU
  > 85 % ───────────────────────────────────────────────►  froids en P5
  < 55 % / 60 s ◄──────────────────────────────────────    remontée (R16)

  plancher jamais évincé : ontologie + zone maps  ──► sinon refus (R17)
```

Relation inter-domaine (`ModèleSémantique → SourceDeDonnées`) :

```
   ns_src=modele          .data : enregistrement complet
                          .idx  : entity_id = (src << 32) | dst
   ns_dst=source          .xref : (dst, src, seq)   ← 24 o, aucun payload
```
