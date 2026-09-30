# Déploiement — une image, un conteneur par client

Ce document est la page d'installation du produit (ROADMAP.md §3.8.1 à
3.8.3, décisions du 2026-09-29 : authentification dans le binaire, serveur
à nous, sauvegarde par copie des fichiers). Il suppose Docker sur un
serveur Linux que vous administrez.

## 1. Ce que contient l'image

Une seule image, `Dockerfile` à la racine, quatre étapes : construction de
l'interface web (Node), cache des dépendances Rust, compilation du binaire
`ontology`, image finale minimale (distroless) avec le binaire et
l'interface. Au démarrage, le binaire fait tout sur le port 5000 :

- l'API (`/concepts`, `/retrieve`, `/metrics`, …) ;
- l'interface web, servie pour toute adresse qui n'est pas une route d'API
  (`/`, `/graph`, `/concepts/…` renvoient `index.html`, le navigateur fait
  le reste) ;
- la connexion des utilisateurs (`/auth/login`, `/auth/signup`, `/auth/me`,
  `/auth/logout`, gestion des comptes par l'administrateur), avec les
  comptes dans `/data/users.json` et le secret des jetons dans
  `/data/jwt.secret`.

Un processus par client (STORAGE.md §10.8) : chaque client a son
conteneur, son volume de données, ses comptes, sa limite mémoire.

## 2. Démarrer un client

```sh
docker compose up -d --build          # deux clients d'exemple : acme, globex
open http://localhost:5001            # acme
```

`compose.yaml` montre le modèle : un service par client, un volume par
service, `mem_limit`, une sonde de santé (`ontology healthcheck`), un arrêt
propre (`stop_grace_period`). La limitation des tentatives de connexion est
par adresse IP : derrière un reverse proxy, c'est l'adresse du proxy que le
binaire voit, sauf à faire porter cette limite par le proxy. Pour un client de plus, copiez un bloc de
service et son volume ; le routage par nom d'hôte (TLS compris) est
l'affaire du reverse proxy devant les conteneurs (Caddy ou Traefik font
cela en quelques lignes).

Options utiles du binaire (dans `command:`) :

| Option | Rôle |
|---|---|
| `--memory-mode strict` (défaut de l'image) | un store qui ne tient pas dans la limite mémoire du conteneur refuse de démarrer, en donnant les deux chiffres, au lieu d'être tué plus tard |
| `--tier p1` | payloads sur disque : le réglage d'un nœud de 16 Go au-delà du million de concepts (STORAGE.md §7.8) |
| `--login` | connexion intégrée (activée par l'image) ; `--allow-signup` laisse l'inscription ouverte après le premier compte |
| `--jwt-secret-env NOM` | prendre le secret des jetons dans la variable d'environnement `NOM` plutôt que dans `/data/jwt.secret` |
| `--seed /chemin` | charger un exemple (schéma + JSONL) dans un store vide, une seule fois |
| `--web /srv/web` | dossier de l'interface construite (celui de l'image) |
| `--backup-dir /backups` | dossier où `POST /backup` copie le store (défaut de l'image ; §5) |

## 3. Premier compte et comptes suivants

Le **premier compte créé** sur un client devient son administrateur ; dès
lors l'inscription libre est fermée (réponse 403 « Sign-up is closed »).
L'administrateur crée les comptes suivants :

```sh
TOKEN=$(curl -s -X POST http://localhost:5001/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"admin@acme.ch","password":"…"}' | python3 -c 'import json,sys;print(json.load(sys.stdin)["token"])')
curl -s -X POST http://localhost:5001/auth/users -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"email":"jean@acme.ch","password":"Mot2Passe!","name":"Jean"}'
```

Règles : adresse valide, mot de passe de 8 caractères et trois classes
(majuscule, minuscule, chiffre, autre), nom non vide. Les mots de passe sont
hachés (bcrypt) ; un `users.json` écrit par l'ancien serveur Node reste
valable tel quel. Un tel fichier n'a pas d'administrateur et l'inscription y
est fermée : pour en désigner un, arrêtez le conteneur, ajoutez
`"role": "admin"` sur un compte du fichier, redémarrez. Supprimer un compte
révoque ses jetons immédiatement. L'écran de création de comptes dans Paramètres arrive
avec le lot A de l'interface (ROADMAP §3.9).

Google et Microsoft (OAuth) ne sont pas encore servis par le binaire : les
boutons renvoient vers la page de connexion avec `error=…_not_configured`.
C'est l'étape 3.8.3b ; le serveur Node `auth-server/` est conservé jusque-là
pour qui en a besoin, puis retiré.

## 4. Mémoire

La limite du conteneur (`mem_limit`) est ce que le binaire lit comme
budget (cgroup v2), fraction 0,6 par défaut (STORAGE.md §8.1). Repères
mesurés (§7.8) : 2×10⁶ concepts et 10⁷ relations tiennent sur 16 Go en P1
(6,4 Go de tas après hydratation) ; 10⁷ / 5×10⁷ demandent 64 Go.

## 5. Sauvegarde et restauration

Décision du 2026-09-29, livrée le 2026-09-30 (ROADMAP §3.8.4) : la
sauvegarde est une **copie des fichiers du store** (le `MANIFEST.json` et les
segments `.data` / `.idx`, dans la disposition du store), prise sous le
verrou d'écriture, donc cohérente. Les segments scellés sont immuables :
une sauvegarde répétée au même endroit ne recopie que le MANIFEST, les
segments actifs et ce qui a été scellé depuis, et retire les segments
qu'une compaction a remplacés. Le MANIFEST est retiré au début et réécrit
en dernier : une sauvegarde interrompue n'est pas restaurable (« not a
backup ») et se relance simplement ; elle n'est jamais restaurée
tronquée. Un dossier de sauvegarde sert un seul store (identité dans le
MANIFEST) : un second client, ou une copie restaurée, y est refusé.

**Sauvegarder un conteneur qui tourne** : `POST /backup` (appel
authentifié) copie le store dans le dossier fixé au démarrage par
`--backup-dir` (l'image le fixe à `/backups`, `compose.yaml` y monte un
volume par client). La destination n'est jamais choisie par l'appelant. La
réponse dit ce qui a été copié :

```sh
curl -X POST -H "authorization: Bearer $TOKEN" http://localhost:5001/backup
# {"files":13,"copied":3,"bytes":41290,"records":1284}
```

Une tâche planifiée (cron du serveur hôte) qui appelle cette route puis
synchronise le volume `backups` vers le stockage externe (partage monté,
`rclone` vers un stockage objet compatible S3, second serveur) fait une
sauvegarde quotidienne incrémentale. Rétention : garder les copies
externes 30 jours ; le volume `backups` lui-même ne contient que la
dernière.

**Sauvegarder un store à l'arrêt** : `ontology --data /data backup
/backups` fait la même copie depuis la ligne de commande (refusée tant
qu'un serveur tient le store).

**Restaurer** : `ontology --data <dossier> restore <sauvegarde>` reconstruit
`<dossier>/store` : copie dans un dossier de travail, ouverture, relecture
complète (une sauvegarde qui ne se relit pas est rejetée avant de toucher
à quoi que ce soit), puis mise en place. La commande refuse d'écraser un
store existant : déplacez-le d'abord. La relecture complète charge le
graphe en mémoire : la lancer dans un conteneur à part, pas à côté du
serveur qui tourne sous sa limite mémoire. Pour un client :

```sh
docker stop acme
docker run --rm -v acme-data:/data -v acme-backups:/backups --entrypoint sh ontology:local \
  -c 'mv /data/store /data/store.old && /usr/local/bin/ontology --data /data restore /backups'
docker start acme
```

Le store restauré a sa propre identité : sa prochaine sauvegarde va dans
un dossier vide (ou dans `/backups` vidé), jamais par-dessus la sauvegarde
d'origine.
Les petits fichiers à côté du store (`users.json`, `jwt.secret`,
`settings.json`) ne font pas partie de la sauvegarde : copiez-les avec le
volume, ou fournissez le secret par `--jwt-secret-env`. La CI exécute
sauvegarde par l'API, restauration par la commande et comparaison du graphe
à chaque changement (`scripts/e2e_image.sh`), et les tests du store
vérifient l'aller-retour, l'incrémental et les refus
(`crates/storage/tests/backup.rs`).

## 6. Vérification de bout en bout

`scripts/e2e_image.sh` est ce que la CI exécute à chaque changement (job
`image`) : construction de l'image, démarrage sur un volume neuf avec
l'exemple finance, création de l'administrateur, connexion, lecture du
graphe par l'API et de l'interface sur le même port, écriture, arrêt propre
(`docker stop`), redémarrage sur le même volume, données et compte
retrouvés, sonde de santé depuis l'intérieur du conteneur, sauvegarde par
l'API et restauration par la commande. Le même script
se lance à la main contre n'importe quelle image :

```sh
docker build -t ontology:local .
scripts/e2e_image.sh ontology:local
```
