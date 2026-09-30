# Feuille de route — où on en est, ce qui vient

**Ce document est le point d'entrée d'une session de travail**, et rien de
plus : l'état exact du projet, les décisions prises et leur motif, les
prochaines étapes **dans l'ordre du plan** avec leurs critères, le procédé.
**La référence est [`STORAGE-PLAN.md`](./STORAGE-PLAN.md)** (phases,
dépendances, mesures, décisions datées), adossé à
[`STORAGE.md`](./STORAGE.md) (format, règles R7–R17) et
[`PERFORMANCE.md`](./PERFORMANCE.md) (H1–H10, R1–R6). Cette feuille de
route s'ajuste au plan, jamais l'inverse : toute évolution se décide et se
date dans le plan, puis se reflète ici. Mise à jour à chaque fusion dans
`main`.

Dernière mise à jour : **2026-09-28**.

---

## 1. État au 2026-09-28

| Chantier | État | Preuve |
|---|---|---|
| Stockage phases 1–4 (chemin d'écriture, conteneur binaire, partitionnement par `ns`, mesures et codec) | **Livré, dans `main`** | STORAGE-PLAN §3.6, §4.6, §5.6, §6.6 ; STORAGE.md §7.7–7.8 |
| T1, pagination par curseur (`cursor=` / `next_cursor` sur `/concepts` et `/relations`, web par pile de curseurs, `offset` déprécié) | **Livré 2026-09-22** | STORAGE-PLAN §8 T1 ; STORAGE.md §7.8 |
| Phase 5, socle mémoire (budget, estimation R14, plan `strict`/`adaptive`, `/stats.memory`, `/metrics`) | **Livré 2026-09-22** (`f5359b7`) | STORAGE.md §8.1 ; STORAGE-PLAN §7.1 |
| Phase 5a, **P1** payloads sur disque (slot + `Loc`, lecteur sans verrou, éviction au scellement, relocalisation à la compaction, `--tier`, plan P1 en adaptive) | **Livré 2026-09-23** | STORAGE-PLAN §7.2 ; STORAGE.md §7.8 (J5), §8.1, §8.2 |
| Couverture de tests | Rust **95,2 %** de lignes (CI Linux du 2026-09-28 : cli 95,2, graph 97,0, index 99,3, io 93,4, rag 96,8, server 96,6, storage 91,8) ; web **99,7 %** ; **seuil 90 % par crate et pour le web imposé par la CI** (2026-09-23) | README « Test coverage » ; badges vivants ; `scripts/coverage_badges.py --fail-under 90` |
| CI | huit jobs : `repo guard` (premier, les autres l'attendent), rustfmt, clippy, tests Ubuntu + Windows, web (vitest avec seuils, tsc, build), `audit` (cargo, npm), `auth-server` (node --test), `coverage` (publie les badges, échoue sous 90 % sur un crate ou le web) ; job manuel `bench` (T2) | `.github/workflows/ci.yml`, `bench.yml` |
| Cible de dimensionnement | **Révisée 2026-09-22, précisée 2026-09-28** : 10⁷ / 5×10⁷ sur 64 Go avec P1 ; 2×10⁶ / 10⁷ garantis sur 16 Go **en P1** ; CSR sur besoin client | STORAGE-PLAN §6.6 décision 3 ; STORAGE.md §1, §7.8 et §8.1 |
| T2, job `bench` manuel | **Livré 2026-09-28** (`bench.yml`, `workflow_dispatch`) | STORAGE-PLAN §8 T2 ; ici §3.7 bis |
| Mesure à 2×10⁶ / 10⁷ (§3.7) | **Faite 2026-09-28** sur le runner : P0 9,9 Go, P1 6,4 Go, `/retrieve` p99 230 ms | STORAGE.md §7.8 ; ici §3.7 |
| R, retrieval | Tranche 1 livrée 2026-09-23 ; **tranche 2a livrée 2026-09-28** (scan parallèle, réindexation parallèle, termes internés) : à 2×10⁶ `/retrieve` p99 230 → 66 ms, `reindex_all` 61,6 → 37,9 s, **critère JR tenu** ; 2b (persistance, index approximatif) conditionnelle | STORAGE-PLAN §8 R décisions 5-7 ; STORAGE.md §7.8 ; ici §3.7 ter |
| T3, métriques par flux | **Livré 2026-09-28** : `ontology_stream_*{ns}`, `ontology_domain_tier{ns,tier}`, `ontology_store_*` | STORAGE-PLAN §8 T3 ; README « Observability » |

Chiffres à garder en tête (mesurés phase 4, portable 16 Go) : un concept de
1,3 Ko coûte ~2,75 Ko de tas en P0, une relation 350 à 475 o ; hydratation
100 à 160 k enregistrements/s ; l'estimateur du socle majore le tas de 17 à
32 %.

## 2. Décisions qui structurent la suite

| Date | Décision | Pourquoi | Où |
|---|---|---|---|
| 2026-09-08 | D1–D6 : mémoire d'abord, disque quand il faut ; budget mémoire = propriété du déploiement (drapeaux + env, jamais dans le store) | Un même store tourne sur 2 ou 64 Go | STORAGE.md §8.0, STORAGE-PLAN §2 |
| 2026-09-17 | JSON reste le codec par défaut, `postcard` en option (`compact --codec postcard`) | Décodage = 9 à 27 % de l'hydratation ; le binaire ne fait gagner que −24 % de disque | STORAGE-PLAN §6.6 |
| 2026-09-17 | `bulk_load` livré | 1,4 à 1,75× à 200 k, 1,1 à 1,2× à 500 k ; le reste est dans les structures primaires des relations | STORAGE-PLAN §6.6 |
| 2026-09-22 | `adaptive` ne retranche des domaines que sous une **limite dure** (cgroup, `--memory-budget-mb`) ; sous une lecture de mémoire libre il charge tout et avertit (`over_budget`) | Retrancher sur la mémoire libre chargerait un jeu de domaines différent à chaque démarrage | STORAGE.md §8.1 |
| 2026-09-22 | Cible portée par le nœud : **64 Go + P1** pour 10⁷ / 5×10⁷ ; **CSR reporté** à un client au-delà de 5×10⁶ concepts sur un nœud contraint | Le matériel est le levier le moins cher ; P1 a une valeur garantie et ne change pas le format ; le socle rend la garantie vérifiable | STORAGE-PLAN §6.6 décision 3 |
| 2026-09-22 | Couverture **≥ 90 % minimum partout**, imposée par la CI (web fait ; Rust dès que `server` et `cli` y sont) | Règle du propriétaire du projet | README « Test coverage » |

## 3. Prochaines étapes, dans l'ordre du plan

**`STORAGE-PLAN.md` est la référence ; ce document en est l'index
d'exécution.** L'ordre ci-dessous est celui du tableau §0 et du calendrier
§9 du plan. En cas de divergence, le plan prime ; une évolution de l'ordre
ou d'une phase se décide et se date **dans le plan** (comme la décision 3 de
§6.6), puis se reflète ici, jamais l'inverse. Seule exception admise : un
ajustement qui améliore le plan à la lumière de ce qui a été construit, lui
aussi consigné dans le plan d'abord.

Chaque étape est une branche, revue indépendante, suite complète verte avec
les seuils, fusion `--no-ff` dans `main`, CI verte, puis mise à jour de ce
document et des sections datées du plan.

### 3.1 T1 — pagination par curseur — **livré 2026-09-22** (plan §0, §8 T1)

- Tel que spécifié au plan : `GET /concepts?cursor=<base64(ctype, name, id)>&limit=`
  avec `next_cursor` (R6 : `concepts_sorted` supporte déjà `range(..)` sur
  cette clé), idem `GET /relations` sur `RelationId` ; `offset` conservé
  mais documenté déprécié ; web et OpenAPI à jour. Règles et actions par
  extension si le coût est trivial, sinon plus tard.
- Dépendance déclarée de 5a dans le tableau §0 ; indispensable à P3 (index
  triés sur disque, `offset` deviendrait O(offset)).
- Critère : équivalence offset/curseur testée ; curseur stable sous
  insertions et suppressions concurrentes ; P95 du listing inchangé.

### 3.2 Phase 5a — P1, payloads sur disque — **livré 2026-09-23** (plan §7.2)

- `DashMap<ConceptId, Concept>` devient `DashMap<ConceptId, Slot>` (32 o +
  `Loc`) ; `get_concept` relit le payload hors verrou (R12) depuis le
  segment scellé (`mmap`) ou actif (`pread`) ; payload du segment actif
  gardé en tas jusqu'au scellement. H7/R1 restent vrais (tests d'invariance
  de PERFORMANCE.md §8.5, risque listé au plan §10).
- Critère J5 (plan §9) : sur le store 5×10⁵ / 2,5×10⁶, tas divisé par
  ≥ 1,4, P95 `GET /concepts/{id}` < 2× P0, hydratation ≤ 1,2× P0 ;
  **capacité prouvée** par `bench gen 5×10⁶ / 2,5×10⁷` hydraté en
  `--memory-mode strict` sur une machine de 64 Go. **Décision du
  2026-09-22** : aucune machine de 64 Go n'est disponible et personne ne
  demande 10⁷ ; cette preuve n'est faite que le jour où une promesse
  contractuelle l'exige (une VM louée une heure suffit), et la formulation
  actuelle — garantie 16 Go, cible 64 Go estimée — reste telle quelle.
  Elle ne conditionne pas P1, dont le critère J5 se mesure sur le store
  5×10⁵ du poste de développement.

### 3.3 Phase 5b — uniquement sur besoin client (plan §0, §7.3, §7.4, §8 T6)

Déclencheur : un tenant au-delà de 5×10⁶ concepts sur un nœud contraint, ou
une latence d'expansion de graphe que seul le CSR résout. Dans l'ordre du
plan :

1. **T6**, profil d'`apply` par échantillonnage, avant de toucher aux
   structures des relations (plan §8 T6 : méthode, sortie attendue).
2. **P2–P4**, `.srt` et `.adj` construits au scellement, CSR des relations,
   fusion k-way pour le listing (plan §7.3, STORAGE.md §6.3, §8.3).
3. **Contrôleur**, seuils 70/85/55 % avec hystérésis 60 s, transitions en
   `warn`, test par budget artificiel sans oscillation (plan §7.4,
   STORAGE.md §8.5). Critère J5b : 10⁷ / 5×10⁷ hydraté sur 16 Go en
   `strict`, ~16 o par arête mesurés.

### 3.4 Chantiers du plan menés en parallèle

- **R — index de retrieval** (plan §1, §8 R) — **tranche 1 livrée
  2026-09-23** : le mur du démarrage était un O(N²) dans l'insertion
  vectorielle (7 min → 17–28 s à 5×10⁵) ; la recherche hybride passe de
  465 ms à 70 ms p50 (top-k O(N), termes à IDF ≈ 0 sautés, cosinus
  vectorisé). Reste sur mesure (décisions datées au plan §8 R) :
  persistance des vecteurs dès un modèle d'embedding réel ; HNSW quand le
  P95 de `/retrieve` dépasse 200 ms (~1,5×10⁶ concepts). **Mesuré le
  2026-09-28 à 2×10⁶** (§3.7) : p99 230 ms, `reindex_all` 61,6 s — tranche 2
  déclenchée, à planifier après T3 (plan §8 R décision 5, ici §3.7 ter).
- **T — transverse** (plan §0 ligne T : CI, métriques, docs) —
  **couverture livrée 2026-09-23** : `server` 78,9 → 96,6 % (relecture
  d'ingestion avec un LLM scripté, plan de contrôle des fournisseurs sur
  un serveur local), `cli` 76,9 → 95,2 % (`ask`/`retrieve` sur
  `EchoModel`, tous les formats d'ingestion, `serve` lancé sur un port
  libre et arrêté proprement) ; le job `coverage` échoue sous 90 % sur un
  crate ou le web. Au passage, `serve` gère SIGTERM / Ctrl+C (arrêt propre
  pour `docker stop`). Reste : migration `react-router` 7 ; modules
  d'authentification `.jsx` en TypeScript.

### 3.5 Reports connus (plan §5.6, §7.1)

Compaction par domaine, group commit inter-requêtes, maintenance à chaud
des `.xref`, `ns` dans l'interface web, limite Job Object Windows, métrique
`majflt/s`, palier par domaine dans `/metrics` (n'existe qu'à partir de P1).

### 3.6 Écarts constatés entre l'exécution et le plan (à garder visibles)

| Écart | Nature | Où c'est consigné |
|---|---|---|
| Le socle (§7.1) a été livré avant T1, que le tableau §0 donne comme dépendance de 5a | De lettre : le socle ne pagine rien ; T1 reste devant P1 | plan §7 (ordre), ici §3.1 |
| Les chiffres de la phase 4 sont mesurés à 2×10⁵ / 5×10⁵ et extrapolés, pas « sur la cible » | Contrainte matérielle (16 Go) ; partiellement fermé par la mesure à 2×10⁶ sur runner (§3.7) ; preuve 5×10⁶ / 64 Go reportée à une demande client (§3.2) | plan §9 J4, §6.6 |
| Le socle laisse de côté Job Object Windows, `majflt/s`, palier par domaine | Sans objet avant P1 | plan §7.1 |
| Des paliers §8.2, seuls P0 et P1 existent ; `adaptive` choisit P1 pour un domaine qui tient sans ses payloads, pas de bascule à chaud | P2–P5 et le contrôleur sont en 5b, sur besoin client | STORAGE.md §8.1, §8.2 |
| Le chantier R a démarré après les phases 1–4 alors que le plan le plaçait en parallèle | Tranche 1 livrée 2026-09-23 ; tranche 2 déclenchée 2026-09-28 (§3.7 ter) | plan §1, §8 R |

### 3.7 Mesure à 2×10⁶ — **livré 2026-09-28** (runner CI, workflow `bench`)

Faite sur le runner (T2) et non sur le portable (2 à 5 Go libres) : résultats
dans STORAGE.md §7.8 (tableau « Mesure à 2×10⁶ / 10⁷ »), §8.1 et README
« Memory budget » (Sizing). Retenu : la garantie 16 Go s'entend **en P1**
(tas 9,9 Go en P0 et l'index de retrieval ne tient plus ; 6,4 Go après
hydratation en P1, l'index tient ensuite) ; **déclencheur HNSW atteint**
(`/retrieve` p99 230 ms, `reindex_all` 61,6 s > hydratation) → chantier R
tranche 2 à planifier (plan §8 R, décision datée) ; curseur T1 ×32 sur
l'offset. Le texte d'origine suit pour mémoire.

Générer un store à la taille de la garantie 16 Go (`bench gen --concepts
2000000 --relations 10000000 --ns 5 --payload 1300`, ~1,8 Go sur disque —
mesuré : 4,6 Go)
et mesurer ce que le README affiche par extrapolation : hydratation P0 et
P1, mémoire privée, `reindex_all`, P50/P95 de la recherche hybride et de
`GET /concepts/{id}`. Sortie : une ligne mesurée dans STORAGE.md §7.8 et
§8.1 à la place de « estimé », et la décision HNSW (plan §8 R) confirmée
ou avancée si le P95 dépasse 200 ms. Aucun code : `bench` suffit.

### 3.7 bis Reliquats du plan — T2 et T3 (avant la production, une branche)

Relecture du plan le 2026-09-25 : deux livrables transverses attendus
depuis les phases 4 et 5 manquent, et ne figuraient pas ici.

- **T3, observabilité par flux** (plan §8 T3) — **livré 2026-09-28** (état
  détaillé au plan §8 T3, README « Observability ») : `/metrics` gagne, par
  domaine (étiquette `ns`), le nombre de segments scellés, les octets sur
  disque, `next_seq`, les `sync_data` cumulés, la durée de la dernière
  compaction, et le palier courant (`p0`/`p1`) en étiquette plutôt qu'en
  total. Tout existe déjà côté store (`OpenReport`, MANIFEST, compteur de
  syncs, `CompactionReport`) : brancher, tester les noms et valeurs des
  jauges sur un store à deux domaines, documenter dans le README.
- **T2, job `bench` manuel** (plan §8 T2) — **livré 2026-09-28**
  (`.github/workflows/bench.yml`, lancé depuis l'onglet Actions ou l'API) :
  un `workflow_dispatch` qui génère un store de taille paramétrable, lance
  `hydrate` (P0 et `--p1`) et `query --p1`, et publie le JSON en artefact —
  la mesure §3.7 a été faite avec lui sur un runner plutôt que sur le poste.

Non consigné auparavant, aussi relevé : le **group commit inter-requêtes**
(plan §5.6, « à mesurer d'abord, phase 4 ») a été mesuré en phase 4
(STORAGE.md §7.7 : appends unitaires vs par lot) mais la décision de le
faire ou d'y renoncer n'est pas écrite ; à trancher dans le plan §5.6 avec
ces chiffres. La **compaction par domaine** (plan §6.6, « à prévoir ») reste
un report daté, sans déclencheur : à lier au chantier 5b.

### 3.7 ter Chantier R, tranche 2 — **2a livrée 2026-09-28**, 2b conditionnelle

Découpage daté au plan §8 R décision 6 : **2a** (livrée, mesurée au plan
décision 7 : JR tenu à 2×10⁶) corrige les causes mesurées sans nouvelle structure — balayage vectoriel parallèle sur
lignes contiguës, tokenisation et embedding parallèles, termes internés
dans l'index lexical ; critère JR mesuré à 2×10⁶ sur le runner. **2b**
(persistance des deux index, index approximatif) seulement si 2a ne tient
pas le critère ou dès un modèle d'embedding réel. Texte d'origine :

La mesure §3.7 a atteint les deux déclencheurs datés du plan (§8 R décision
5) : `/retrieve` p99 230 ms (> 200 ms) et `reindex_all` 61,6 s au-delà de
l'hydratation. Contenu : HNSW construit au scellement d'un segment (`mmap`)
et persistance des vecteurs par domaine pour sortir `reindex_all` du
démarrage. Place : après T3, **avant la mise en production d'un tenant
au-delà de ~10⁶ concepts** ; en dessous, la tranche 1 suffit (70 ms à
5×10⁵). Décision du propriétaire attendue sur l'ordre R tranche 2 / §3.8.

### 3.8 Production — **chantier principal, validé le 2026-09-24** — **prochaine étape (reprise du 2026-09-29)**

**Décisions du propriétaire, 2026-09-29** : (1) **authentification en Rust**
dans le binaire, le serveur Node est retiré une fois le remplacement en
place ; (2) **déploiement sur un serveur à nous** (image + compose, pas
d'hébergeur managé) ; (3) **sauvegarde par copie des segments immuables et
du MANIFEST vers un stockage externe** (objet compatible S3 ou second
serveur), incrémentale, rétention 30 jours, restauration exercée en CI ;
les instantanés disque restent un complément optionnel de l'hébergeur.
Ordre d'exécution retenu : passe 1 = 3.8.1 + 3.8.2 (image, compose, web
servie par le binaire, test de bout en bout) — **livrée 2026-09-29**
(`276d22e`) ; passe 2 = lot A de l'interface (§3.9) — **livrée
2026-09-29** ; puis, **ordre révisé le 2026-09-30 avec le propriétaire** :
3.8.4 sauvegarde (**livrée 2026-09-30**), lot B (première tranche,
**livrée 2026-09-30**), 3.8.5 mise à jour (**livrée 2026-09-30**), lot B
(tranches 2a et 2b **livrées 2026-09-30**, traduction du corps des pages
au fil de l'eau)
(seconde tranche), 3.8.6 clés d'API et audit, et **3.8.3b OAuth reporté**
(avec le retrait d'`auth-server/` et la dette web) jusqu'à la demande d'un
client : le login intégré couvre le besoin, OAuth n'apporte que la
connexion par compte Google ou Microsoft.

État à la fin du 2026-09-28 : tous les reliquats du plan sont livrés (T1,
T2, T3, mesure §3.7, R tranche 2a) ; `main` = 44a47fd, CI verte, incident
clos pour ce dépôt. Trois décisions du propriétaire ouvraient ce chantier
(tranchées ci-dessus), dans cet ordre : (1) auth en Rust dans le binaire (recommandé : un
processus par tenant, T5, plus de surface npm) ou conservation du serveur
Node ; (2) cible de déploiement (image + compose sur un serveur géré, ou
hébergeur managé) ; (3) sauvegarde (copie des segments immuables et du
MANIFEST vers un stockage objet, ou snapshot disque). L'image et le compose
n'en dépendent pas et démarrent en premier. Le lot A de l'interface (§3.9,
premier jour, boutons inertes) s'intercale juste après, avant toute
démonstration ; le lot B (quotidien) suit la sauvegarde. Points d'hygiène hors chantier :
jeton GitHub du poste à révoquer (Settings, Applications), protection de
`main` à durcir pour imposer les PR, scan `claude-security` en attente de
l'accord sur le coût.

Hors du plan de stockage, dont il est le débouché et dont il applique les
décisions : **T5** (un store et un processus par tenant, STORAGE.md §10.8),
**D5** (le budget mémoire est une propriété du déploiement : la limite
cgroup du conteneur, lue par le socle §8.1), **R17** (refus explicite au
démarrage plutôt qu'un OOM), la migration automatique de format (phase 2)
pour la mise à jour, les segments immuables et le `LOCK` (phases 2–3) pour
la sauvegarde, l'arrêt propre sur SIGTERM (chantier T). Direction arrêtée
avec le propriétaire : **une seule image de conteneur, un conteneur par
client**
(limite mémoire cgroup lue par le socle §8.1, volume par client, isolation
par construction), la même image livrée en auto-hébergement sous licence
commerciale aux clients qui ne peuvent pas sortir leurs données. Pas de
processus multi-tenant : l'isolation que le conteneur donne gratuitement
n'est pas à réécrire. Le produit est l'API REST (`/openapi.json`) ;
l'interface web est la console d'administration et d'analyse.

Ce que chaque étape applique du plan de stockage (vérifié le 2026-09-24) :

| Étape production | Ce qu'elle applique du plan de stockage |
|---|---|
| Une image, un conteneur par client | **T5**, tranchée le 2026-09-15 (STORAGE.md §10.8) : un store par tenant, un processus par tenant, le routage au reverse proxy |
| Limite mémoire du conteneur | **D5** (le budget est une propriété du déploiement) et le socle §8.1 (cgroup v2 lu en premier) ; `--memory-mode strict` par défaut en conteneur = **R17** |
| `docker stop` propre | Arrêt sur SIGTERM livré avec le chantier T (2026-09-23) |
| Retrieval d'un tenant > ~10⁶ concepts | **R tranche 2a livrée** (§3.7 ter) : JR tenu à 2×10⁶ ; 2b (persistance, index approximatif) conditionnelle, plan §8 R décision 6 |
| Sauvegarde et restauration | Segments immuables et MANIFEST (phase 2), `LOCK` (H17), compaction comme condition de survie (§8.7) |
| Mise à jour exercée en CI | Migration automatique de format (phase 2), `format_version` 1 et 2 (phase 4) |
| Interface servie par le binaire, clés d'API, audit | Hors plan de stockage, sans contradiction avec lui |
| Phase 5b (T6, CSR, contrôleur) | **Pas dans ce chantier** : sur besoin client, comme le plan l'écrit |

Étapes, chacune sa branche, testée, fusionnée, dans cet ordre :

1. **Image et déploiement de référence** — **livré 2026-09-29** (passe 1 :
   `Dockerfile` avec étape web, `compose.yaml` deux clients, `scripts/e2e_image.sh`,
   job CI `image`, sonde `ontology healthcheck`, `docs/DEPLOIEMENT.md`). Le `Dockerfile` existant
   (cargo-chef, distroless, `serve` sur 5000) complété d'un `compose.yml`
   avec la limite mémoire, le volume de données, `--memory-mode strict`
   par défaut en conteneur ; un test CI construit l'image et vérifie
   `/healthz` et `/stats.memory.budget_source == "cgroup-v2"` dans le
   conteneur. Critère : `docker compose up` sert l'API sur un store vide,
   `docker stop` s'arrête proprement (fait : SIGTERM géré).
2. **Interface servie par le binaire** — **livré 2026-09-29** (`serve --web`,
   repli `index.html`, navigation navigateur distinguée de l'API par `Accept`).
   Les fichiers construits de `web/`
   servis par `serve` sur `/` (Vite reste l'outil de développement, le
   proxy disparaît en production) ; TLS terminé devant par un reverse
   proxy, jamais dans le binaire. Critère : l'image seule sert l'API et la
   console ; la suite web passe toujours.
3. **Authentification** — **tranchée et livrée en Rust le 2026-09-29** pour
   inscription, connexion, session, comptes (premier compte = administrateur,
   inscription fermée ensuite, `users.json` du Node repris tel quel, jetons
   révoqués à la suppression d'un compte) ; **reste 3.8.3b : OAuth Google et
   Microsoft** dans le binaire, puis retrait de `auth-server/`. Texte d'origine :
   décision à prendre : le service Node
   (`auth-server`) embarqué dans l'image, ou réécrit comme module Rust du
   serveur (un seul processus, une seule surface). Recommandation : Rust.
   Critère : inscription, connexion, JWT et OAuth couverts à ≥ 90 %.
4. **Sauvegarde et restauration** — **livré 2026-09-30** : `POST /backup`
   vers le dossier fixé par `serve --backup-dir` (image : `/backups`, un
   volume par client dans `compose.yaml`), `ontology backup <dest>` hors
   serveur, `ontology restore <src>` (copie en zone de travail, relecture
   complète, mise en place, jamais par-dessus un store existant). Copie
   sous le verrou d'écriture du MANIFEST et des segments `.data`/`.idx` ;
   incrémentale par immuabilité des segments scellés (fichier de même
   taille déjà présent = ignoré, partitions retirées par une compaction
   supprimées à destination) ; MANIFEST retiré au début et réécrit en
   dernier (une sauvegarde interrompue est refusée, jamais restaurée
   courte) ; identité du store dans le MANIFEST (un dossier de sauvegarde
   par store, une copie restaurée en reçoit une nouvelle).
   Vérifié par `compare_graphs` (`crates/storage/tests/backup.rs`), par
   l'API et la commande (`crates/server/tests/hardening_meta.rs`,
   `crates/cli/tests/commands.rs`) et de bout en bout dans l'image
   (`scripts/e2e_image.sh` : sauvegarde par l'API, restauration dans le
   conteneur, même graphe). Texte d'origine : `ontology backup <dest>` (copie des
   segments scellés et du MANIFEST après compaction, actif inclus sous
   verrou) et `restore`, testés par un aller-retour vérifié par
   `compare_graphs`. Critère : restauration d'un store 5×10⁵ identique au
   fingerprint — exercé à la main le 2026-09-30 sur le poste de
   développement (Windows, SSD) : store `bench gen` de 5×10⁵ concepts et
   10⁶ relations (819 Mo, 45 fichiers) ; première sauvegarde 30 s (dont
   l'ouverture du store), seconde sauvegarde sans changement 15 fichiers
   et 45 Mo copiés (les actifs et le MANIFEST), restauration 21 s avec
   relecture complète, mêmes comptes (500 000 / 1 000 000).
5. **Mise à jour** — **livré 2026-09-30** : job `upgrade` dans `ci.yml`
   (`scripts/upgrade_check.sh`). La version précédente (`HEAD~1` sur
   `main`, sinon la base de fusion avec `origin/main`) est construite depuis
   son propre checkout et écrit deux stores (exemple finance complet,
   20 000 concepts générés) ; la version courante les ouvre (`stats` et
   `export` canonique identiques), y écrit, compacte, rouvre, sauvegarde et
   restaure. Une rupture de format que la migration automatique ne couvre
   pas fait échouer la CI avant la fusion. Texte d'origine : la CI ouvre un
   store produit par la version précédente de `main` (artefact conservé)
   avec la version courante : migration de format et hydratation vertes.
   Critère : job `upgrade` dans `ci.yml`.
6. **Clés d'API par client et journal d'audit** — **livré 2026-09-30** :
   clés nommées (`POST /auth/keys` par un administrateur, secret `ok_…`
   montré une fois, haché SHA-256 dans `users.json`, `GET` liste, `DELETE`
   révoque sur-le-champ ; une clé est un appelant machine : elle ouvre
   l'API, pas `/auth/me` ni la gestion des comptes) ; journal d'audit
   (`<data>/audit.jsonl`, une ligne JSON par écriture réussie — POST, PUT,
   PATCH, DELETE — avec l'heure, l'appelant (courriel, nom de clé ou
   jeton de service), la méthode, le chemin, le statut et l'identifiant
   de requête ; `GET /audit?limit=` le relit, administrateurs seulement
   avec la connexion intégrée ; `serve --audit-log`, `--no-audit`).
   Non couvert : l'identifiant de la fiche créée par un `POST` (le chemin
   ne le porte pas ; l'identifiant de requête permet de le retrouver dans
   le journal du serveur) ; rotation du fichier ; un écran Réglages pour
   les clés et le journal (API seulement pour l'instant, `curl` ou
   `/docs`). Texte d'origine : clés
   d'API par client et journal d'audit des écritures (qui, quoi, quand,
   sur quel record), en dernier : nécessaires à la facturation et au
   support, pas au premier déploiement.

Deux décisions reviennent au propriétaire avant l'étape 3 : l'hébergeur
cible (Infomaniak Public Cloud par cohérence avec le fournisseur LLM, ou
autre) et l'authentification en Rust ou Node.

### 3.9 Interface — deux publics, deux lots (plan du 2026-09-29) — **lot A livré 2026-09-29**

L'interface a deux publics : la personne qui ouvre l'application pour la
première fois (tableau de bord vide, dix entrées de menu sans ordre, tout
en anglais et en jargon) et celle qui y travaille tous les jours (cherche,
importe, relit, corrige). Deux lots, à des moments différents du calendrier
§3.8 ; le vocabulaire et le menu se font une fois pour les deux.

**Lot A — premier jour** (2 à 3 jours ; **après 3.8.1 image + compose et
avant 3.8.3**, c'est-à-dire avant toute démonstration à un prospect) :

1. Parcours guidé à la place du tableau de bord vide : trois étapes
   numérotées, « Décrivez vos données » (modèle prêt à l'emploi : contrats
   et factures, personnes et organisations, équipements et procédures, ou
   vide), « Déposez vos fichiers », « Posez une question ». Chaque étape
   s'ouvre quand la précédente est faite ; le parcours disparaît une fois
   l'ontologie en place.
2. « Essayer avec l'exemple finance » en un clic (`examples/finance`
   existe) : un graphe rempli, une question, une réponse sourcée.
3. États vides orientés vers l'action, sur chaque page : la phrase dit
   quoi faire et le bouton le fait (« Aucun fichier. Déposez un Word, un
   Excel ou un PDF ici, ou essayez l'exemple »).
4. Menu par flux et vocabulaire sans jargon (vaut pour les deux lots) :
   *Préparer* (Modèle de données, Fichiers, Importer des documents),
   *Explorer* (Graphe, Fiches, Questions), *Automatiser* (Règles,
   Actions) ; vraies icônes ; une langue par défaut (français) ; aucun
   terme technique hors Paramètres (pas de `ns`, `P1`, `payload`).
5. Nettoyage des boutons qui ne mènent nulle part (tableau ci-dessous) :
   chacun est branché ou retiré, aucun ne reste inerte.
6. **Guide interactif** derrière le « ? » de la barre du haut (demandé le
   2026-09-29). Une visite pas à pas de chaque partie de l'application :
   à chaque étape, l'écran s'assombrit sauf l'élément expliqué (entrée du
   menu, barre de recherche, bouton), une carte dit à quoi il sert en deux
   phrases, et l'application navigue d'elle-même vers la page concernée.
   Boutons Précédent / Suivant / Terminer, touche Échap, et un sommaire
   pour sauter directement à une partie. Le guide s'ouvre seul à la
   première visite, et **reste accessible à tout moment** par le « ? » ; ce
   qui a déjà été vu est mémorisé dans le navigateur. Une étape par
   partie : tableau de bord, modèle de données, fichiers, import, graphe,
   fiches, règles, questions, actions, paramètres, recherche, retour
   d'expérience. Sans dépendance (une superposition et une carte), textes
   en français, testé comme le reste (rendu, navigation, mémorisation,
   réouverture). Il remplace le parcours guidé du point 1 pour la partie
   « découvrir », le point 1 garde la partie « faire » (les trois étapes).

**Livraison du lot A (2026-09-29, branche `feat/ui-lot-a`)** : les six
points sont en place dans `web/src` — parcours en trois étapes à la place
du tableau de bord vide (`components/Onboarding.tsx`, affiché tant que le
graphe n'a aucune fiche : l'étape 2 s'ouvre dès que le modèle existe) ; « Essayer avec
l'exemple finance » envoie les huit fichiers d'`examples/finance` par
l'API d'import existante, dans l'ordre du README (`lib/example.ts`, fichiers
embarqués dans le bundle, copiés dans l'image) ; états vides orientés vers
l'action sur le graphe, les fiches, les fichiers et les questions ; menu
par flux avec icônes et libellés français (`layout/nav.tsx`) ; boutons
inertes traités (tableau ci-dessous) ; guide interactif (`components/Tour.tsx`,
une étape par entrée du menu plus accueil, recherche et retour
d'expérience, spot sur l'élément, navigation automatique, sommaire,
clavier, mémorisé sous `tour.v1`, rouvert par le « ? »). Couverture web
99,5 % lignes. **Reporté au lot B** : les modèles prêts à l'emploi de
l'étape 1 (seul l'exemple finance existe ; l'étape renvoie au Modèle de
données) et la traduction du contenu des pages (titres, tableaux,
formulaires restent en anglais ; seuls le menu, la barre du haut, le
parcours, le guide et les états vides sont en français).

**Lot B — quotidien** (au fil de l'eau, **après 3.8.4 sauvegarde**, guidé
par ce que les premiers utilisateurs font réellement) :

1. Recherche unique en haut, toujours visible, avec réponse sourcée
   (aujourd'hui elle renvoie vers la page Questions).
2. La fiche d'un concept comme centre : sur un écran, ses liens, les
   documents d'origine, les règles qui s'appliquent, correction sur place.
3. Accueil orienté activité : importé cette semaine, en attente de
   relecture, exceptions signalées par les règles, dernières questions ; à
   la place des courbes et de l'aperçu décoratif du réseau.
4. Relecture d'import proposée d'office après un dépôt : « 12 personnes,
   4 organisations, 9 relations trouvées. Vérifier et ajouter ».
5. Questions enregistrées, rejouables en un clic, épinglables sur
   l'accueil (l'écran Questions les stocke déjà).
6. Graphe comme outil de réponse : part d'une fiche ou d'une question,
   voisinage à deux pas, filtre par type (le parcours existe côté serveur).
7. Évolution du modèle sans peur : ajouter un type ou une relation dit ce
   que ça touche (« 3 400 fiches concernées, rien à migrer »).

**Livraison du lot B, tranche 1 (2026-09-30, branche `feat/ui-lot-b1`)** :
point 4 — un document (texte, Word, PDF) déposé sur Fichiers part dans
l'assistant d'import, qui l'analyse aussitôt et dit ce qu'il a trouvé
(« 1 Contract, 1 Clause et 1 lien(s) trouvés dans contrat.pdf. Vérifiez,
corrigez si besoin, puis ajoutez. ») ; les fichiers structurés (CSV, Excel,
JSONL) se chargent toujours directement ; les fiches issues d'une relecture
portent le document d'origine (`source_file`). Point 2 — la fiche
(`/concepts/:id`, `pages/ConceptSheet.tsx`) : nom et description corrigés
sur place, liens dans les deux sens avec renvoi vers chaque fiche voisine,
informations structurées, documents d'origine, règles (générales ou
ciblées) et actions qui la concernent ; ouverte depuis l'inspecteur du
graphe. Point 3 — l'accueil (`pages/Dashboard.tsx`) : question en tête de
page, tuiles, « À faire » (relecture en cours, imports en échec), « Cette
semaine » (documents importés, fiches et liens ajoutés), dernières
questions, règles ; les courbes et l'aperçu décoratif ont disparu. Reports
du lot A — modèles prêts à l'emploi au premier jour (`examples/models` :
chantiers et sous-traitants, contrats et factures, personnes et
organisations) ; la traduction du contenu des pages reste partielle
(accueil, fiche, assistant d'import et premiers pas en français ; Fichiers,
Fiches, Règles, Actions, Graphe, Modèle de données et Paramètres encore en
anglais). Couverture web 99,4 % lignes. **Reste pour la tranche 2** :
points 5, 6 et 7 ; « exceptions signalées par les règles » sur l'accueil
attend un moteur d'évaluation des règles côté serveur (aucun aujourd'hui :
l'accueil compte les règles, il ne les évalue pas) ; navigation réduite à
quatre entrées (Accueil, Importer, Explorer, Réglages) proposée par le
propriétaire le 2026-09-30 à la place du menu latéral, à faire une fois la
fiche et l'accueil validés à l'usage ; traduction des pages restantes.

**Livraison du lot B, tranche 2a (2026-09-30, branche `feat/ui-lot-b2a`)** :
navigation à **quatre entrées** en haut de page à la place du menu latéral
(`layout/TopNav.tsx`, `layout/nav.tsx`) — Accueil et Réglages ouvrent une
page, Importer (Fichiers, Relire un document, Modèle de données) et
Explorer (Fiches, Graphe, Questions, Règles, Actions) ouvrent un menu
natif (`<details>`), l'entrée de la section courante est marquée ; le
guide interactif suit (une étape par page, le projecteur sur le menu qui
la contient). Point 6 (début) — le graphe s'ouvre sur une fiche :
`/graph?focus=<id>` amorce le voisinage sur cette fiche, la sélectionne et
la cadre ; la fiche y renvoie. Point 5 — questions épinglables à l'accueil
depuis la page Questions (★, mémorisé dans le navigateur), rejouées en un
clic depuis l'accueil (`/queries?run=<id>`). Couverture web 99,4 %.
**Tranche 2b livrée le 2026-09-30 (branche `feat/ui-lot-b2b`)** : point 7 —
avant d'enregistrer un modèle, l'interface compte les fiches dont le type
disparaît (`lib/impact.ts`, une requête `limit=1` par type retiré) et le
dit (« 12 fiche(s) concernée(s) : Person (12). Un type encore utilisé ne
peut pas être retiré : réaffectez ou supprimez ces fiches d'abord. »,
l'enregistrement est arrêté là, le brouillon reste ; sinon « Rien à
migrer », répété après l'enregistrement ; si le comptage échoue, le
serveur tranche). Traduction : titres et sous-titres de toutes les pages
en français ; **le corps des pages Fichiers, Fiches, Règles, Actions,
Graphe, Modèle de données et Réglages (tableaux, formulaires, boutons,
messages) reste en anglais** — à traduire page par page au fil de l'eau,
chacune avec ses tests. Le lot B est clos hors cette traduction.
**Traduction livrée le 2026-09-30 (branche `feat/ui-french`)** : corps
des sept pages, dialogue de confirmation, panneau de relecture d'import,
assistant d'import, Questions, réponse en direct ; dates en `fr-CH`.
**Retours du premier essai du propriétaire (2026-09-30, branche
`feat/ui-feedback-1`)** : (1) un modèle installé sans fiche laissait
Fiches et Graphe vides sans explication — les deux disent maintenant que le
modèle est en place et que les fiches viendront des fichiers ; (2) un
Excel ou CSV déposé sans type de fiche était refusé (« demande un type de
fiche ») alors que le sélecteur était caché — la page demande le type sur
place (« Chaque ligne de « invoices.xlsx » devient une fiche : de quel
type ? ») puis importe ; (3) « Relire un document » n'était pas compris —
renommé « Relecture d'un document » avec une phrase d'explication (un
document déposé dans Fichiers y arrive de lui-même ; un fichier structuré se
charge directement) ; (4) le graphe : jusqu'à 300 fiches il est dessiné en
entier, au-delà il se charge uniquement autour d'une sélection (un ou
plusieurs types de fiche, sélection multiple, ou une recherche, ou une
fiche), 250 fiches au plus, sans rechargement automatique ; les types de
lien se choisissent aussi à plusieurs. Restent en anglais, à dessein : les identifiants techniques (types de
fiches et de liens venant des données, identifiants de modèles, `Top-K`,
`BM25`), les messages d'erreur renvoyés par le serveur, et les nombres au
format `1,234` (locale `en-US` dans `fmtNum`, à passer en `fr-CH` avec les
tests qui les vérifient si souhaité). Le lot B est clos.

**Dette technique** (avec 3.8.3, inchangé) : migration `react-router` 7
(deux vulnérabilités npm modérées) ; modules d'authentification `.jsx` en
TypeScript (testés à 97–100 %, conversion mécanique).

**Boutons et liens qui ne mènent nulle part** (inventaire du 2026-09-29,
`web/src`) — traités dans le lot A, décision par ligne :

| Où | Élément | Aujourd'hui | Décision |
|---|---|---|---|
| Barre du haut | « ? » Aide | rien | **Brancher** sur le guide interactif (lot A.6) |
| Barre du haut | « ⚑ » Notifications | rien | **Retirer** jusqu'à ce qu'il existe des événements à notifier (lot B.3) |
| Barre du haut | Avatar « U » | rien | **Brancher** : nom de l'utilisateur, déconnexion, Paramètres |
| Tableau de bord | « View Analytics » | ouvre le Graphe | **Renommer** « Ouvrir le graphe » ou retirer (doublon de la carte voisine) |
| Tableau de bord | Aperçu du réseau | décoratif | **Retirer** (lot B.3 le remplace) |
| Fiches | icône « Expand » de la carte Détails | rien | **Retirer** (ou ouvrir la fiche en plein écran, lot B.2) |
| Fiches | « Export » dans les détails | rien | **Brancher** sur l'export existant du serveur (JSON), sinon retirer |
| Fiches | « Validate Definitions » | message « coming soon » | **Retirer** tant que la validation n'existe pas |
| Actions | icône « Expand » | rien | **Retirer** |
| Actions | « Run Now » | rien | **Brancher** sur l'exécution d'une action, sinon retirer |
| Actions | « Import Actions », « Generate with AI », « Bulk Edit », « Run All Active » | liens `href="#"` | **Retirer** les quatre ; « Generate with AI » revient quand la génération existe |
| Actions | bouton Supprimer de la liste | icône « … » (more) pour une suppression | **Corriger l'icône** (corbeille) |
| Actions | « Delete » des détails | icône « télécharger » | **Corriger l'icône** |
| Règles | icône « Expand » | rien | **Retirer** |
| Modèle de données | « More options ▾ » à côté de Générer | rien | **Retirer** |
| Modèle de données | « Layout ▾ », zoom +, zoom −, « Fit », plein écran | rien | **Brancher** sur la vue (le Graphe a déjà ces commandes) ou retirer |
| Modèle de données | « View Details › » | rien | **Retirer** ou ouvrir le type sélectionné |

État au 2026-09-29 : toutes les lignes traitées ; là où la décision
laissait le choix : « View Analytics » retiré (doublon), « Run Now » retiré
(pas d'exécution côté serveur), « Export » branché sur l'export JSONL du
serveur, commandes de vue du Modèle de données retirées, « View Details »
retiré, « Expand » des fiches retiré.

Règle pour la suite : un bouton n'entre dans l'interface qu'avec son
action et son test de rendu ; « coming soon » n'est pas un état.

## 4. Procédé

1. Une étape à la fois, sur sa branche (`feat/…`, `test/…`, `docs/…`).
2. Tests d'abord au niveau où le comportement se voit : unitaires,
   intégration, bout en bout (le binaire est lancé dans les tests CLI).
3. Vérification locale : `cargo fmt --all -- --check`, `cargo clippy
   --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
   `cd web && npm run test:coverage && npx tsc --noEmit`.
4. Revue indépendante contre le plan (un agent relecteur en lecture seule),
   corrections sur la branche.
5. `git merge --no-ff` dans `main`, push, CI verte sur les huit jobs de `ci.yml` ; les
   badges de couverture se republient seuls.
6. Mise à jour de ce document (état, décisions, étape suivante) et des
   sections datées de `STORAGE-PLAN.md`.

Contraintes de travail :

- Docs en français, `H*`/`R*` renvoient aux règles de `PERFORMANCE.md` et
  `STORAGE.md`. Réponses au propriétaire en français.
- Toute décision de modèle ou d'architecture est soumise avant d'être
  codée ; un défaut de comportement choisi en revue est signalé.
- **Deux ou trois agents en parallèle au maximum**, nombre et coût annoncés
  avant tout lancement au-delà d'un seul.
- Pas de `gh` sur la machine de développement : fusions locales, CI suivie
  via l'API publique (`api.github.com/repos/masspe/ai-ontology.com/actions/runs`).

## 5. Reproduire les mesures

```bash
# Couverture
cargo llvm-cov --workspace --summary-only
cd web && npm run test:coverage

# Benchs de stockage (répertoire dédié, jamais des données réelles)
ontology --data $DATA bench gen --concepts 200000 --relations 1000000 --ns 5 --payload 1300
ontology --data $DATA bench hydrate --json        # hydrate_ms, heap_delta_mib, estimate_mib, estimate_vs_heap_pct
ontology --data $DATA --memory-mode strict --memory-budget-mb 1024 stats   # refus explicite attendu
```

Sur la machine de développement (portable Windows, 16 Go, 14 threads) : la
compilation `release` de la CLI prend ~13 min, la suite web complète ~3 min
avec la moitié des cœurs, la suite Rust instrumentée ~15 min.
