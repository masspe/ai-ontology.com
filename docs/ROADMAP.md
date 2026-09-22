# Feuille de route — où on en est, ce qui vient

**Ce document est le point d'entrée d'une session de travail.** Il dit
l'état exact du projet, les décisions prises et leur motif, les prochaines
étapes dans l'ordre avec leurs critères d'acceptation, et le procédé à
suivre. Il est mis à jour à chaque fusion dans `main`. Le détail technique
vit dans [`STORAGE.md`](./STORAGE.md) (format, règles R7–R17),
[`STORAGE-PLAN.md`](./STORAGE-PLAN.md) (phases, mesures, décisions
datées) et [`PERFORMANCE.md`](./PERFORMANCE.md) (H1–H10, R1–R6).

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

## 3. Prochaines étapes, dans l'ordre

Chaque étape est une branche, revue indépendante, suite complète verte avec
les seuils, fusion `--no-ff` dans `main`, CI verte, puis mise à jour de ce
document.

### 3.1 Rust ≥ 90 % par crate, puis seuil en CI

- `server` est à 78,5 % : `ingest_review.rs` (relecture d'ingestion par LLM)
  à 53 %. Tester avec un modèle simulé : JSON tronqué, `null` à la place
  d'une chaîne, variantes inconnues, erreurs fournisseur, réparation.
- `cli` est à 76,9 % : `ask` et `retrieve` sur `EchoModel`, `migrate` sur un
  `graph.log` de fixture, `serve` lancé puis arrêté dans un test avec
  quelques appels HTTP réels, `--ns` bout en bout.
- Puis `--fail-under-lines 90` dans le job `coverage` de `ci.yml`.
- Critère : chaque crate ≥ 90 % de lignes sur Linux (`cargo llvm-cov`), CI
  verte avec le seuil.

### 3.2 T1 — pagination par curseur

- `GET /concepts|/relations|/rules|/actions?after=<curseur opaque>&limit=`
  avec `next_cursor` ; `offset` toléré pendant une transition, documenté
  comme déprécié ; web (listes, « page suivante ») et CLI adaptés.
- Prérequis de P2–P4 (index triés sur disque : `offset` deviendrait
  O(offset)). Spécification : STORAGE-PLAN §8 T1.
- Critère : équivalence offset/curseur testée ; curseur stable sous
  insertions et suppressions concurrentes ; P95 du listing inchangé.

### 3.3 Preuve de capacité à 5×10⁶ sur 64 Go

- Sur une machine de 64 Go (à prévoir, pas disponible ici) :
  `bench gen --concepts 5000000 --relations 25000000 --ns 5 --payload 1300`
  puis `bench hydrate --json` avec `--memory-mode strict`.
- Critère : hydratation acceptée en `strict`, écart estimation / tas
  consigné dans STORAGE.md §7.8. C'est la seule preuve qui autorise à
  promettre 10⁷ à un client.

### 3.4 T6 — profil d'`apply` par échantillonnage

- Avant de toucher aux structures : où passent les 73 à 91 % du temps
  d'hydratation. Méthode et sortie attendue : STORAGE-PLAN §8 T6.

### 3.5 P1 — payloads sur disque

- `DashMap<ConceptId, Concept>` devient `DashMap<ConceptId, Slot>` (32 o +
  `Loc`) ; `get_concept` relit le payload hors verrou (R12) depuis le
  segment scellé (`mmap`) ou actif (`pread`) ; payload du segment actif
  gardé en tas jusqu'au scellement. Spécification : STORAGE.md §6.2,
  STORAGE-PLAN §7.2.
- Critère J5 : sur le store 5×10⁵ / 2,5×10⁶, tas divisé par ≥ 1,4, P95
  `GET /concepts/{id}` < 2× P0, hydratation ≤ 1,2× P0 ; capacité vérifiée
  par 3.3.

### 3.6 Contrôleur

- Seuils 70/85/55 % avec hystérésis 60 s, transitions journalisées en
  `warn`, test par budget artificiel sans oscillation. STORAGE.md §8.5,
  STORAGE-PLAN §7.4.

### 3.7 Sur besoin client uniquement

- **P2–P4, CSR des relations** : déclencheur = un tenant au-delà de 5×10⁶
  concepts sur un nœud contraint, ou une latence d'expansion que seul le CSR
  résout. STORAGE-PLAN §7.3.
- Compaction par domaine, group commit inter-requêtes, maintenance à chaud
  des `.xref`, `ns` dans l'interface web, limite Job Object Windows,
  métrique `majflt/s`.

### 3.8 Dette, en parallèle quand une session le permet

- Migration `react-router` 7 (deux vulnérabilités npm modérées).
- Modules d'authentification `.jsx` (`Login`, `Signup`, `OAuthCallback`,
  `ProtectedRoute`, `msBE`, `Toast`, `ConfirmDialog`) en TypeScript ; ils
  sont testés à 97–100 % depuis le 2026-09-22.
- Retrieval (STORAGE-PLAN §8 R) : `reindex_all` au démarrage et cosinus en
  O(N) sont les vrais murs avant 10⁷, indépendants du stockage.

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
