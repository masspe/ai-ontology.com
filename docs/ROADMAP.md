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

Dernière mise à jour : **2026-09-22**.

---

## 1. État au 2026-09-22

| Chantier | État | Preuve |
|---|---|---|
| Stockage phases 1–4 (chemin d'écriture, conteneur binaire, partitionnement par `ns`, mesures et codec) | **Livré, dans `main`** | STORAGE-PLAN §3.6, §4.6, §5.6, §6.6 ; STORAGE.md §7.7–7.8 |
| Phase 5, socle mémoire (budget, estimation R14, plan `strict`/`adaptive`, `/stats.memory`, `/metrics`) | **Livré 2026-09-22** (`f5359b7`) | STORAGE.md §8.1 ; STORAGE-PLAN §7.1 |
| Couverture de tests | Rust **90,2 %** de lignes (576 tests) ; web **99,7 %** (687 tests), seuil 90 % imposé par la CI | README « Test coverage » ; badges vivants en tête du README |
| CI | rustfmt, clippy, tests Ubuntu + Windows, web (vitest avec seuils, tsc, build), `coverage` (publie les badges) | `.github/workflows/ci.yml` |
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

### 3.1 T1 — pagination par curseur (plan §0, §8 T1 : « avant toute phase 5 »)

- Tel que spécifié au plan : `GET /concepts?cursor=<base64(ctype, name, id)>&limit=`
  avec `next_cursor` (R6 : `concepts_sorted` supporte déjà `range(..)` sur
  cette clé), idem `GET /relations` sur `RelationId` ; `offset` conservé
  mais documenté déprécié ; web et OpenAPI à jour. Règles et actions par
  extension si le coût est trivial, sinon plus tard.
- Dépendance déclarée de 5a dans le tableau §0 ; indispensable à P3 (index
  triés sur disque, `offset` deviendrait O(offset)).
- Critère : équivalence offset/curseur testée ; curseur stable sous
  insertions et suppressions concurrentes ; P95 du listing inchangé.

### 3.2 Phase 5a — P1, payloads sur disque (plan §7.2 ; « suit la phase 4 sans attendre un incident client »)

- `DashMap<ConceptId, Concept>` devient `DashMap<ConceptId, Slot>` (32 o +
  `Loc`) ; `get_concept` relit le payload hors verrou (R12) depuis le
  segment scellé (`mmap`) ou actif (`pread`) ; payload du segment actif
  gardé en tas jusqu'au scellement. H7/R1 restent vrais (tests d'invariance
  de PERFORMANCE.md §8.5, risque listé au plan §10).
- Critère J5 (plan §9) : sur le store 5×10⁵ / 2,5×10⁶, tas divisé par
  ≥ 1,4, P95 `GET /concepts/{id}` < 2× P0, hydratation ≤ 1,2× P0 ;
  **capacité prouvée** par `bench gen 5×10⁶ / 2,5×10⁷` hydraté en
  `--memory-mode strict` sur une machine de 64 Go (à prévoir : pas
  disponible sur le poste de développement). Cette preuve est la seule qui
  autorise à promettre 10⁷ à un client (plan §6.6, décision 3).

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

- **R — index de retrieval** (plan §1 : « deux murs arrivent avant ceux du
  stockage », §8 R) : `reindex_all` au démarrage et cosinus en O(N). Pas
  commencé. Première action, une mesure : temps de réindexation et P95
  d'une recherche à 5×10⁵ et 2×10⁶ concepts avec `bench`, pour dater le
  mur par rapport à la garantie 16 Go affichée (2×10⁶). Puis persistance
  des vecteurs et index approximatif ; jalon JR.
- **T — transverse** (plan §0 ligne T : CI, métriques, docs) : couverture
  Rust ≥ 90 % par crate (`server` 78,5 %, `cli` 76,9 % : relecture
  d'ingestion avec LLM simulé ; `ask`/`retrieve` sur `EchoModel`,
  `migrate`, `serve` lancé et arrêté dans un test), puis
  `--fail-under-lines 90` dans le job `coverage` ; migration
  `react-router` 7 ; modules d'authentification `.jsx` en TypeScript.
  Ces travaux ne bloquent pas T1 ni P1 et ne les précèdent pas.

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
| `adaptive` charge ou non un domaine ; les paliers §8.2 n'existent pas encore | Attendu avant P1 | STORAGE.md §8.1 |
| Le chantier R n'est pas commencé alors que le plan le place avant les murs du stockage | Priorité à mesurer (§3.4) | plan §1, §8 R |

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
