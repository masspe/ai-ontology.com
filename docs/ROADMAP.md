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

Dernière mise à jour : **2026-09-25**.

---

## 1. État au 2026-09-22

| Chantier | État | Preuve |
|---|---|---|
| Stockage phases 1–4 (chemin d'écriture, conteneur binaire, partitionnement par `ns`, mesures et codec) | **Livré, dans `main`** | STORAGE-PLAN §3.6, §4.6, §5.6, §6.6 ; STORAGE.md §7.7–7.8 |
| T1, pagination par curseur (`cursor=` / `next_cursor` sur `/concepts` et `/relations`, web par pile de curseurs, `offset` déprécié) | **Livré 2026-09-22** | STORAGE-PLAN §8 T1 ; STORAGE.md §7.8 |
| Phase 5, socle mémoire (budget, estimation R14, plan `strict`/`adaptive`, `/stats.memory`, `/metrics`) | **Livré 2026-09-22** (`f5359b7`) | STORAGE.md §8.1 ; STORAGE-PLAN §7.1 |
| Phase 5a, **P1** payloads sur disque (slot + `Loc`, lecteur sans verrou, éviction au scellement, relocalisation à la compaction, `--tier`, plan P1 en adaptive) | **Livré 2026-09-23** | STORAGE-PLAN §7.2 ; STORAGE.md §7.8 (J5), §8.1, §8.2 |
| Couverture de tests | Rust **95,2 %** de lignes (cli 95,2, graph 97,0, index 99,3, io 93,7, rag 96,8, server 96,6, storage 91,8) ; web **99,7 %** ; **seuil 90 % par crate et pour le web imposé par la CI** (2026-09-23) | README « Test coverage » ; badges vivants ; `scripts/coverage_badges.py --fail-under 90` |
| CI | rustfmt, clippy, tests Ubuntu + Windows, web (vitest avec seuils, tsc, build), `coverage` (publie les badges, échoue sous 90 % sur un crate ou le web) | `.github/workflows/ci.yml` |
| Cible de dimensionnement | **Révisée 2026-09-22** : 10⁷ / 5×10⁷ sur 64 Go avec P1 ; 2×10⁶ / 10⁷ garantis sur 16 Go ; CSR sur besoin client | STORAGE-PLAN §6.6 décision 3 ; STORAGE.md §1 et §8.1 |

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
  P95 de `/retrieve` dépasse 200 ms (~1,5×10⁶ concepts). Mesure à 2×10⁶ à
  faire quand un store de cette taille est généré.
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
| Les chiffres de la phase 4 sont mesurés à 2×10⁵ / 5×10⁵ et extrapolés, pas « sur la cible » | Contrainte matérielle (16 Go) ; fermé par la preuve à 5×10⁶ de §3.2 | plan §9 J4 |
| Le socle laisse de côté Job Object Windows, `majflt/s`, palier par domaine | Sans objet avant P1 | plan §7.1 |
| Des paliers §8.2, seuls P0 et P1 existent ; `adaptive` choisit P1 pour un domaine qui tient sans ses payloads, pas de bascule à chaud | P2–P5 et le contrôleur sont en 5b, sur besoin client | STORAGE.md §8.1, §8.2 |
| Le chantier R n'est pas commencé alors que le plan le place avant les murs du stockage | Priorité à mesurer (§3.4) | plan §1, §8 R |

### 3.7 Mesure à 2×10⁶ — **prochaine étape** (une heure de machine)

Générer un store à la taille de la garantie 16 Go (`bench gen --concepts
2000000 --relations 10000000 --ns 5 --payload 1300`, ~1,8 Go sur disque)
et mesurer ce que le README affiche par extrapolation : hydratation P0 et
P1, mémoire privée, `reindex_all`, P50/P95 de la recherche hybride et de
`GET /concepts/{id}`. Sortie : une ligne mesurée dans STORAGE.md §7.8 et
§8.1 à la place de « estimé », et la décision HNSW (plan §8 R) confirmée
ou avancée si le P95 dépasse 200 ms. Aucun code : `bench` suffit.

### 3.7 bis Reliquats du plan — T2 et T3 (avant la production, une branche)

Relecture du plan le 2026-09-25 : deux livrables transverses attendus
depuis les phases 4 et 5 manquent, et ne figuraient pas ici.

- **T3, observabilité par flux** (plan §8 T3) : `/metrics` gagne, par
  domaine (étiquette `ns`), le nombre de segments scellés, les octets sur
  disque, `next_seq`, les `sync_data` cumulés, la durée de la dernière
  compaction, et le palier courant (`p0`/`p1`) en étiquette plutôt qu'en
  total. Tout existe déjà côté store (`OpenReport`, MANIFEST, compteur de
  syncs, `CompactionReport`) : brancher, tester les noms et valeurs des
  jauges sur un store à deux domaines, documenter dans le README.
- **T2, job `bench` manuel** (plan §8 T2) : `workflow_dispatch` dans
  `ci.yml` qui génère un store de taille paramétrable, lance `hydrate`
  (P0 et `--p1`) et `query`, et publie le JSON en artefact — la mesure
  §3.7 devient reproductible sur un runner plutôt que sur le poste.

Non consigné auparavant, aussi relevé : le **group commit inter-requêtes**
(plan §5.6, « à mesurer d'abord, phase 4 ») a été mesuré en phase 4
(STORAGE.md §7.7 : appends unitaires vs par lot) mais la décision de le
faire ou d'y renoncer n'est pas écrite ; à trancher dans le plan §5.6 avec
ces chiffres. La **compaction par domaine** (plan §6.6, « à prévoir ») reste
un report daté, sans déclencheur : à lier au chantier 5b.

### 3.8 Production — **chantier principal, validé le 2026-09-24**

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
| Sauvegarde et restauration | Segments immuables et MANIFEST (phase 2), `LOCK` (H17), compaction comme condition de survie (§8.7) |
| Mise à jour exercée en CI | Migration automatique de format (phase 2), `format_version` 1 et 2 (phase 4) |
| Interface servie par le binaire, clés d'API, audit | Hors plan de stockage, sans contradiction avec lui |
| Phase 5b (T6, CSR, contrôleur) | **Pas dans ce chantier** : sur besoin client, comme le plan l'écrit |

Étapes, chacune sa branche, testée, fusionnée, dans cet ordre :

1. **Image et déploiement de référence.** Le `Dockerfile` existant
   (cargo-chef, distroless, `serve` sur 5000) complété d'un `compose.yml`
   avec la limite mémoire, le volume de données, `--memory-mode strict`
   par défaut en conteneur ; un test CI construit l'image et vérifie
   `/healthz` et `/stats.memory.budget_source == "cgroup-v2"` dans le
   conteneur. Critère : `docker compose up` sert l'API sur un store vide,
   `docker stop` s'arrête proprement (fait : SIGTERM géré).
2. **Interface servie par le binaire.** Les fichiers construits de `web/`
   servis par `serve` sur `/` (Vite reste l'outil de développement, le
   proxy disparaît en production) ; TLS terminé devant par un reverse
   proxy, jamais dans le binaire. Critère : l'image seule sert l'API et la
   console ; la suite web passe toujours.
3. **Authentification.** Décision à prendre : le service Node
   (`auth-server`) embarqué dans l'image, ou réécrit comme module Rust du
   serveur (un seul processus, une seule surface). Recommandation : Rust.
   Critère : inscription, connexion, JWT et OAuth couverts à ≥ 90 %.
4. **Sauvegarde et restauration.** `ontology backup <dest>` (copie des
   segments scellés et du MANIFEST après compaction, actif inclus sous
   verrou) et `restore`, testés par un aller-retour vérifié par
   `compare_graphs`. Critère : restauration d'un store 5×10⁵ identique au
   fingerprint.
5. **Mise à jour.** La CI ouvre un store produit par la version précédente
   de `main` (artefact conservé) avec la version courante : migration de
   format et hydratation vertes. Critère : job `upgrade` dans `ci.yml`.
6. **Clés d'API par client et journal d'audit** des écritures (qui, quoi,
   quand, sur quel record), en dernier : nécessaires à la facturation et
   au support, pas au premier déploiement.

Deux décisions reviennent au propriétaire avant l'étape 3 : l'hébergeur
cible (Infomaniak Public Cloud par cohérence avec le fournisseur LLM, ou
autre) et l'authentification en Rust ou Node.

### 3.9 Dette, entre deux branches

Migration `react-router` 7 (deux vulnérabilités npm modérées) ; modules
d'authentification `.jsx` en TypeScript (testés à 97–100 %, la conversion
est mécanique) ; ces deux points se règlent naturellement avec l'étape 3.8.3.

## 4. Procédé

1. Une étape à la fois, sur sa branche (`feat/…`, `test/…`, `docs/…`).
2. Tests d'abord au niveau où le comportement se voit : unitaires,
   intégration, bout en bout (le binaire est lancé dans les tests CLI).
3. Vérification locale : `cargo fmt --all -- --check`, `cargo clippy
   --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
   `cd web && npm run test:coverage && npx tsc --noEmit`.
4. Revue indépendante contre le plan (un agent relecteur en lecture seule),
   corrections sur la branche.
5. `git merge --no-ff` dans `main`, push, CI verte sur les six jobs ; les
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
