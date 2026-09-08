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

---

## 2. Hypothèses fondatrices

| # | Hypothèse | Conséquence |
|---|---|---|
| H11 | Le log est strictement append-only ; un enregistrement scellé n'est jamais réécrit | Les segments scellés sont immuables → `mmap` sûr, index construit une fois, CRC vérifié une fois |
| H12 | `seq` est alloué par un compteur global unique, monotone, sans trou | L'entrée d'index est **adressée directement** (`base + (seq − base_seq) × 32`), pas recherchée |
| H13 | Le `ns` (domaine) d'un type de concept est déclaré dans l'ontologie et stable | Le routage d'écriture est statique : aucune résolution d'entité pour choisir la partition |
| H14 | `relation_type.domain` / `.range` déterminent le `ns` source et cible | Une relation inter-domaine est routable **avant** d'inspecter ses extrémités |
| H15 | Les `ConceptId` sont attribués séquentiellement | Les zone maps `entity_min`/`entity_max` du MANIFEST sont sélectives. **Faux si passage aux UUID** — cf. §9.4 |
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

### 4.3 Entrée d'index — 32 o, taille fixe

| Offset | Champ |
|---|---|
| 0 | `seq` u64 |
| 8 | `offset` u64 |
| 16 | `payload_len` u32 |
| 20 | `kind` u8, `flags` u8 |
| 22 | `ns_id` u16 |
| 24 | `entity_id` u64 |

`entity_id` s'interprète selon `kind` : `id` pour un `Concept`,
`(source << 32) | target` pour une `Relation`. C'est ce qui permet de
reconstruire toute l'adjacence **sans ouvrir un seul `.data`**.

Sous H12, l'adressage est direct : `entry = 32 + (seq − base_seq) × 32`.
Pas de recherche binaire, pas de comparaison.

### 4.4 Fichiers auxiliaires

- `.ent` — 16 o par entrée, `(entity_id, seq)` trié. Recherche binaire quand
  la `HashMap` mémoire d'un domaine froid a été relâchée (§6.3).
- `.xref` — 24 o par entrée, `(target_id, source_id, seq)`. Arêtes entrantes
  provenant d'un autre domaine, **sans duplication de payload** : le `ns`
  cible garde une adjacence entrante complète même chargé seul.
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
(R10) et, par partition, les zone maps : `base_seq`/`last_seq`,
`entity_min`/`entity_max`, bitmap des `kind`, compteur d'enregistrements.
Une requête bornée à un domaine élague ses partitions avant d'ouvrir un
fichier.

---

## 5. Hydratation

```
meta  → rejeu intégral (ontologie complète, toujours)
graph → lecture des .idx uniquement (32 o/enregistrement)
      + .xref (24 o/arête entrante inter-domaine)
      → aucun .data touché
```

Sous H16, c'est le gain principal du format. Sur le `graph.log` actuel
(39 enregistrements, 52 915 o) : **1,2 Ko lus** au lieu de 52 Ko. À 10⁶
enregistrements de taille comparable : ~32 Mo séquentiels au lieu de ~1,3 Go.

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

Le comportement sous contrainte est un choix explicite, pas une surprise :

```toml
[memory]
mode = "adaptive"   # "strict" | "adaptive"
heap_fraction = 0.6
```

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

Le surcoût est l'index (32 o/enregistrement) et le padding d'alignement. Il
s'inverse dès le passage à un codec binaire (§7.1).

### 10.3 Collision de vocabulaire `domain`

Dans l'ontologie, `relation_type.domain` / `.range` désignent le **type
source** et le **type cible**. Le partitionnement métier utilise donc `ns`,
jamais `domain`. Ne pas réintroduire le mot dans le code de stockage.

### 10.4 Zone maps et UUID

L'élagage par `entity_min`/`entity_max` suppose des ids séquentiels (H15).
Un passage aux UUID rend ces zone maps inutiles ; il ne resterait que
l'élagage par `ns` et la recherche binaire dans `.ent`. Décision à prendre
avant, pas après.

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
  ├─ .idx  ← [seq|off|len|kind|ns|ent]        │    .idx  → slots + adjacence
  │         (32 o, jamais fsyncé — R7)        │    .xref → arêtes entrantes
  │                                           │    (aucun .data lu — H16)
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
