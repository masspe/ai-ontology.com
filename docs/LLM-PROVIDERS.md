# Language-model providers


`ontology-rag` ships these `LanguageModel` implementations:

| Backend             | Constructor                             | Default model     |
| ------------------- | --------------------------------------- | ----------------- |
| Echo (offline fake) | `EchoModel`                             | `echo`            |
| Anthropic Messages  | `AnthropicModel::new(key)`              | `claude-opus-4-7` |
| OpenAI Chat         | `OpenAiModel::new(key)`                 | `gpt-4o-mini`     |
| DeepSeek Chat       | `OpenAiModel::deepseek(key)`            | `deepseek-chat`   |
| Infomaniak AI Tools | `OpenAiModel::infomaniak(key, product)` | *(none — pick one)* |

DeepSeek and Infomaniak are OpenAI-compatible byte-for-byte (streaming SSE
and the `usage` block included), so they share `OpenAiModel` with a different
base URL. Every HTTP client supports streaming, retries 408 / 409 / 429 / 5xx
with full-jitter exponential backoff (3 retries by default), and honors
server-sent `retry-after`.

`v1_api_url(base, path)` builds every endpoint and inserts the `/v1` segment
only when the base URL does not already carry one, so both documented
spellings of a base URL work.

## Configuring an LLM provider

**Credentials live in the settings file, never in the environment.** There is
no `*_API_KEY` variable and no provider flag on `ontology serve`.

* The file is `<data>/settings.json`, or `./.ontology/settings.json` when no
  `--data` directory is given. Override with `--settings <path>`.
* It is created on the first save and holds the raw keys, so it survives
  restarts. On Unix it is created mode 0600; on Windows it inherits the
  directory, so keep the data directory out of shared locations.
* Keys never travel back out: `GET /settings` strips every secret-shaped
  field and returns a masked `*_api_key_hint` instead.
* Applying a provider or model takes effect on the next request — `/ask`,
  `/ask/stream` and `/ingest/analyze` all resolve it per call.

Configure it in the UI (**Settings → Configuration**), or over the API:

```bash
curl -s -XPATCH localhost:5000/settings -H 'content-type: application/json' -d '{
  "llm": {
    "active_provider": "infomaniak",
    "infomaniak_api_key": "…",
    "infomaniak_product_id": "101112",
    "infomaniak_model": "mixtral"
  }
}'
```

Supporting endpoints, all of which accept unsaved credentials in the body so
you can validate before saving:

| Endpoint | Purpose |
| -------- | ------- |
| `POST /settings/llm/test` | Probe the provider; echoes the resolved endpoint. |
| `GET  /settings/llm/models?provider=…` | Model catalogue for the stored config. |
| `POST /settings/llm/models` | Same, with credentials supplied in the body. |
| `POST /settings/llm/infomaniak/products` | Resolve the AI Tools `product_id`. |

### Infomaniak AI Tools

Swiss-hosted open-source models behind an OpenAI-compatible API.

1. Create an API token in the Infomaniak manager with the **`ai-tools`**
   scope.
2. Paste it into **Settings → Configuration** and click **Détecter** to read
   the `product_id` from `GET https://api.infomaniak.com/1/ai`.
3. Click **Charger les modèles** (`GET {base}/models`), pick one, then
   **Appliquer**.

Requests then go to
`https://api.infomaniak.com/2/ai/{product_id}/openai/v1/chat/completions`.
The base URL is derived from the product id; the "base URL" field is an
advanced override for proxies and staging hosts, and is normally left empty.

The CLI uses whatever the settings file selects; `--model` overrides the
model for one run:

```bash
ontology --data $DATA ask                    "Who wrote about RAG?"
ontology --data $DATA ask --model gpt-4o     "Who wrote about RAG?"
ontology --settings ./staging/settings.json ask "Who wrote about RAG?"
```

## Prompt caching

The Anthropic client routes the ontology (stable per knowledge base) into a
separately-cached `system` block via `cache_control: {"type": "ephemeral"}`,
so repeated queries against the same KB pay roughly 10% of the input price
for the cached prefix on subsequent requests within the TTL (5 min default).
Verify hits via `RagAnswer.usage.cache_read_input_tokens`. The minimum
cacheable prefix on Claude Opus 4.7 is 4096 tokens; below that the
breakpoint is silently ignored — no error.

`temperature` is automatically omitted on Claude Opus 4.7 (the API rejects
it with a 400). Older models still receive it.

OpenAI and DeepSeek both perform **automatic** server-side prefix caching
on identical leading content — the `OpenAiModel` folds `cached_context`
into the leading `system` message so byte-stable prefixes pay the cached
rate without any client opt-in. Cache hits are exposed via
`usage.cache_read_input_tokens` (mapped from OpenAI's
`prompt_tokens_details.cached_tokens` and DeepSeek's
`prompt_cache_hit_tokens`).

All HTTP clients retry 408 / 409 / 429 / 5xx with full-jitter
exponential backoff (default 3 retries; configurable via
`with_max_retries`). When the server sends a `retry-after`
header it's honored verbatim.

