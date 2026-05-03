# Finance demo — contracts + invoices + line items

A minimal end-to-end walkthrough that exercises the full stack:

1. an **ontology** for a finance/contracts domain,
2. **structured data** loaded from JSONL (companies, people, structural
   relations) and from XLSX (invoices, line items),
3. **free-text contracts** loaded as documents (each becomes a `Contract`
   concept whose `description` is the file body, fully searchable),
4. a **React UI** to upload everything via drag-and-drop and ask
   natural-language questions that the LLM answers using the ontology
   and the retrieved subgraph as context.

## Files in this directory

| File | Role |
|---|---|
| `ontology.json` | concept types (`Company`, `Person`, `Contract`, `Invoice`, `LineItem`) and relation types (`between`, `signed_by`, `employed_by`, `issued_by`, `issued_to`, `covers_contract`, `has_line_item`) |
| `seed.jsonl` | base concepts — Companies, Persons, and the `employed_by` relations between them |
| `invoices.xlsx` | one row per invoice (name, description, issued_by, issued_to, contract, issue_date, amount_eur) |
| `line_items.xlsx` | one row per line item (name, description, invoice, amount_eur, quantity) |
| `contracts/*.txt` | the actual contract bodies — three Master Services Agreements |
| `relations.jsonl` | cross-type links — `between` / `signed_by` / `issued_by` / `issued_to` / `covers_contract` / `has_line_item`. Loaded **last** so all endpoints already exist. |

## Run the whole thing in 4 commands

From the repo root:

```bash
# 1. build the binary
cargo build --release --bin ontology

# 2. start the server (echo LLM by default — no API key needed)
DATA=$(mktemp -d)
./target/release/ontology --data "$DATA" serve --bind 127.0.0.1:8080 &

# 3. start the React UI
cd web
npm install
npm run dev   # served at http://localhost:5173

# 4. switch to the live LLM (optional)
#    Stop the server above, then:
#    ANTHROPIC_API_KEY=sk-... ./target/release/ontology --data "$DATA" \
#        serve --bind 127.0.0.1:8080 --anthropic
```

Open http://localhost:5173, switch to the **Upload** tab and drop the
files in this order (the order matters — relations need their
endpoints to exist):

1. `ontology.json` — kind **Ontology JSON**.
2. `seed.jsonl` — kind **JSONL records**. Loads 3 companies + 3 people +
   their `employed_by` links.
3. `contracts/*.txt` — kind **Text document**, concept type `Contract`,
   multi-select all three files. Each file becomes one searchable
   `Contract` concept whose description is the full text.
4. `invoices.xlsx` — kind **XLSX**, concept type `Invoice`.
5. `line_items.xlsx` — kind **XLSX**, concept type `LineItem`.
6. `relations.jsonl` — kind **JSONL records**. The cross-type links
   (`between`, `signed_by`, `issued_by`, `issued_to`, `covers_contract`,
   `has_line_item`). Loaded **last** so every endpoint exists.

The **Stats** header at the top updates live so you can watch the
graph grow (you should land at 25 concepts, 36 relations).

## Sample questions

The UI ships four example questions in French wired to the live LLM
stream. They each demonstrate a different retrieval pattern:

| Question | What the system has to do |
|---|---|
| *"Quels contrats Acme Labs a-t-elle signés en 2025 ?"* | type-filter on `Contract`, walk `between` edges back to `Company:Acme Labs` |
| *"Quel est le montant total facturé à Initech ?"* | type-filter on `Invoice`, follow `issued_to` to `Initech`, sum `amount_eur` properties |
| *"Qui a signé le contrat C-2025-002 ?"* | direct lookup on the `Contract` concept by name, then walk `signed_by` |
| *"Quelle est la plus grosse ligne facturée et à quel contrat est-elle liée ?"* | rank `LineItem` by `amount_eur`, walk `has_line_item` ← `Invoice` → `covers_contract` |

The hybrid retrieval (TF-IDF + vector + 2-hop subgraph expansion)
surfaces the relevant concepts and edges; the LLM uses that subgraph
as grounded context. The system prompt instructs the model to cite
concept names verbatim and to refuse if the context doesn't support
the answer.

## What to watch for

* The **Citations** chips under each answer show which concepts the
  retrieval grounded the answer in. If a citation looks irrelevant,
  the retrieval has misranked — try adjusting `top_k` (in `api.ts`)
  or filtering by concept type.
* The **usage** line shows token counts. With prompt caching enabled
  (default for the Anthropic client), the second question against the
  same KB should show non-zero `cache_read` once the ontology block
  exceeds the model's minimum cacheable prefix (4096 tokens on Opus
  4.7 — the toy ontology here is too small to engage caching, but the
  numbers will appear once you scale the ontology up).
* Each request gets an `X-Request-Id` response header — useful for
  correlating server logs with UI behavior.

## Reset and reload

```bash
# Drop the data dir and start fresh
rm -rf "$DATA"
mkdir "$DATA"

# Or from the API: POST /compact takes a snapshot then truncates the WAL.
curl -XPOST localhost:8080/compact
```

For batch/CI ingest without the UI, the same data loads via the CLI:

```bash
ontology --data "$DATA" ingest --ontology ontology.json seed.jsonl
ontology --data "$DATA" ingest --text-type Contract contracts/
ontology --data "$DATA" ingest --xlsx-type Invoice  invoices.xlsx
ontology --data "$DATA" ingest --xlsx-type LineItem line_items.xlsx
ontology --data "$DATA" ingest                       relations.jsonl
ontology --data "$DATA" stats
ontology --data "$DATA" ask "Qui a signé C-2025-002 ?"
```
