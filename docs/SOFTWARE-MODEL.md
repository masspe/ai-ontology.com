# A code base as a graph — how the software model is built, and why answers are what they are

This document explains the logic behind the software-model import: what
becomes a sheet, what becomes a link, where every piece of information
comes from, how a question travels through the graph to an answer — and,
honestly, what this design can and cannot answer today. The first reader
is for Progress OpenEdge ERPs; the model and the reasoning are the same for
any code base (the names in the examples are invented).

## 1. The idea

A code base is already a graph. A program reads tables, runs other
programs, includes shared files; a table has fields and keys; a screen is a
handful of programs. The import does not invent anything: it reads what the
compiler and the data dictionary already know and writes it down as sheets
(concepts) and links (relations), so that the same engine that answers
questions over a finance or a HR ontology can answer questions over the
software.

Two layers are needed, and only the first one exists today:

| Layer | Says | Source | State |
| ----- | ---- | ------ | ----- |
| **Structure** | *what touches what*: program → table, table → field, program → program, screen → programs | compiler cross-reference, sources, data dictionary | built (0.2.0 + `.df`) |
| **Meaning** | *what things are for*: what a screen does, which business function a table serves, the words users say | screen titles, menu tree, user help, documentation, people | mostly absent |

Questions people actually ask ("where do I enter a payroll variable?",
"what is the posting process?") live in the second layer. The first layer
answers "which programs write `emp`?" and "what breaks if `cono` changes?"
very well, and answers business questions only when the business words
happen to appear in a name or a label. That gap, not the graph itself, is
why answers feel off. Section 5 says what to add.

## 2. Sheets and links

Every sheet has a type, a unique name within the type, a one-line
description (what retrieval and the prompt read) and typed properties.
Every link has a type, a direction, a weight and optional properties.

| Sheet | Named | Description holds | Properties |
| ----- | ----- | ----------------- | ---------- |
| `Module` | the source directory (`pr`, `gl`) | — | `path`, `program_count` |
| `Program` | the source path (`pr/w-pr100.w`) | a short verb phrase of what it does to the data | `code` (screen code), `kind`, `missing`, `procedures`, `functions` |
| `Include` | the include path | — | `path` |
| `Class` | the class name | — | `name` |
| `Database` | `appdb` | — | — |
| `Table` | `appdb.emp` | its **label** from the dictionary (« Salariés ») | `db`, `table`, `label`, `dump_name`, `primary_key`, `unique_keys`, `field_count` |
| `Field` | `appdb.emp.cono` | its **label**, then the dictionary description | `table`, `field`, `label`, `column_label`, `help`, `data_type`, `format`, `decimals`, `extent`, `order` |
| `Screen` | the code users know (`gl100`) | the window title, the tabs, the menu entries | `title`, `tabs`, `menu_items`, `programs` |

| Link | From → to | Weight | Properties |
| ---- | --------- | ------ | ---------- |
| `belongs_to` | Program → Module | 1 | |
| `runs` | Program → Program | number of `RUN` statements | |
| `includes` | Program → Include | | |
| `invokes` | Program → Class | | |
| `reads` / `updates` / `creates` / `deletes` | Program → Table | number of references in the compiled code | |
| `reads_field` / `updates_field` | Program → Field | | |
| `in_database` | Table → Database | | |
| `has_field` | Table → Field | | |
| `joined_on` | Table ↔ Table (symmetric) | number of joins in the sources | `keys` (`cono=cono`), `programs`, `implicit` |
| `implemented_by` | Screen → Program | | |
| `fires_on` | Program → Table (database trigger) | | `event` (WRITE, DELETE) |

Weights matter: when a sheet has thousands of neighbours, the expansion
takes the heaviest first, so a program that reads a table in forty places
outranks one that reads it once.

## 3. Where each piece comes from

Three readers write into the same sheets; a re-import updates what changed
and journals nothing for what did not (`upsert` + `merge_properties`), so
the three can run in any order and repeatedly.

1. **The compiler cross-reference** (`--xref`, `*.xref.xml`). One file per
   compiled program, streamed one `<Reference>` at a time and reduced to a
   per-program summary. `ACCESS`/`UPDATE`/`SEARCH`/`CREATE`/`DELETE`
   references give the program → table and program → field links with
   their counts; `RUN` gives `runs` (resolved by path, then by unique file
   name as the PROPATH would, else a `missing` program so the link is
   kept); `INCLUDE`, `CLASS`, `PROCEDURE`/`FUNCTION` fill the rest. This
   is the backbone: exact, complete, cheap (8 GB in 20 s).
2. **The sources** (`--abl`, `.p`/`.w`/`.i`). No grammar — a few patterns
   over the comment-stripped text, for what the cross-reference cannot
   say: `WHERE a.f EQ b.g` and `FOR EACH a OF b` give the table ↔ table
   joins with their keys; `TITLE =`, `FOLDER-LABELS`, `MENU-ITEM … LABEL`
   give the screen's words; the file names (`w-gl100.w`, `b-gl100.w`)
   group the programs of one screen under its code.
3. **The data dictionary dump** (`--df`). The only source that knows what a
   table *means* to users: labels and descriptions become the sheets'
   descriptions, indexes become keys, triggers become `fires_on`, and the
   fields no program touches exist too.

Three rules travel with the model as prompt hints (not executed logic):
a table change impacts its writers first; an include change impacts every
program including it; a question about an area is answered at module
level, not as a list of programs.

## 4. How a question becomes an answer

```
question ──► retrieval (top-k seeds) ──► expansion (subgraph) ──► prompt ──► model ──► answer + citations
```

1. **Retrieval** scores every sheet on its name and description, twice:
   a lexical index (BM25-style, exact words after normalisation) and a
   vector index. The two scores are fused 50/50 and the best 8 become the
   seeds. Today the vector side is a **hashing embedder** — characters
   hashed into a fixed vector, no language model behind it. It makes
   near-identical strings close; it does **not** know that « salaire » and
   « paie » mean the same thing, nor that « saisir » relates to a screen.
   For business questions it is mostly noise; the lexical side carries
   the result, which is why a question works exactly when its words are
   in a label.
2. **Expansion** walks the links from the seeds, breadth-first, depth 2,
   at most 64 sheets, heaviest links first and round-robin across link
   types (so "who updates X" is answered with updaters, not drowned in
   readers). A hub — a company table read by 5 000 programs — brings in
   its heaviest neighbours, which are the generic utilities, not the
   specific screen.
3. **The prompt** carries the ontology (types, link types, rules — the
   stable, cached part), then the seeds with their properties, then the
   facts of the subgraph as `A —link→ B` lines with weights, under a
   character budget (facts and sheets share it; long lists are cut).
4. **The model** answers and cites sheet ids; the UI shows the citations.

## 5. What this design answers well, and what it cannot yet

**Good today** — anything structural: impact of a table or field change,
which programs read/write a table, how a screen is implemented, what runs
what, which tables are joined and on which keys, what a field is (from the
dictionary), database triggers.

**Weak today**, and the reasons are specific:

1. **The business vocabulary is not in the graph.** 3 350 of the 3 619
   windows in the first ERP have no title in their code; the menu tree,
   the screen catalogue and the user help are outside the sources. A
   question in users' words has nothing to match. Connecting to the
   *data* does not fix this — but two *metadata* sources do: the program
   catalogue table (titles, menu path) and the help pages.
2. **No semantic retrieval.** With the hashing embedder, synonyms and
   paraphrases do not match. Real embeddings (a provider's embedding API
   or a local multilingual model) are the single biggest lever for
   French business questions.
3. **The grain is wrong for the questions.** Fields are 18 000 sheets and
   1.2 M links; screens are 3 600 and thin. Retrieval and expansion are
   dominated by the level nobody asks about.
4. **Nothing is measured.** Without a set of real questions with expected
   answers, every tuning is a guess.

## 6. What to add — the meaning layer (general, not ERP-specific)

In order of leverage:

1. **Documents attached to sheets by key.** A generic importer: "this
   folder of text/HTML files, keyed by file name, describes sheets of type
   T" — user help pages become Screen descriptions; a README becomes a
   Module description. Same mechanism for any code base.
2. **The application's own catalogue** (for OpenEdge ERPs: the program
   table with titles and menu paths, through the ODBC connector of phase
   B): Screen titles and a `Menu` path. This is where « Écritures
   analytiques » gets attached to `gl100`.
3. **Real embeddings** for the vector index, multilingual, behind the
   existing `Embedder` trait; the hashing embedder stays for tests and
   offline use.
4. **Answer-level sheets and links.** Roll program → table up to
   screen → table (`uses`), module → table; make Screen/Module/Table the
   default seed types and hide field-level links from expansion unless
   the question is about a field.
5. **The question set** (owner): 10–15 real questions with the expected
   screen/table/program. It becomes a test; the pass count is the gauge
   for every change above.

The structure layer stays exactly as it is — it is what makes the answers
*verifiable* (every claim cites a program or a table that exists) and what
impact analysis needs. The meaning layer is what makes them *findable*.
