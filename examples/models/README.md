# Modèles prêts à l'emploi

Schémas (types de fiches et de liens, sans données) proposés au premier
jour par l'étape « Décrivez vos données » de l'interface.
L'exemple finance complet (schéma et données) est dans `../finance`.

| Fichier | Pour |
|---|---|
| `chantiers.json` | chantiers, entreprises, personnes, contrats, factures, équipements |
| `personnes-organisations.json` | personnes, organisations, documents |

Chargement : `ontology --data <dir> ingest --ontology examples/models/chantiers.json`
ou par l'interface (`PUT /ontology`).
