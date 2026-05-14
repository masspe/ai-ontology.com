// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
//
// Hand-written OpenAPI 3.0 description of the ontology server, plus a
// Swagger UI page served from `/docs`. Kept manual to avoid pulling in
// proc-macro derive crates for every handler signature.

use axum::{response::Html, Json};
use serde_json::Value;
use std::sync::OnceLock;

/// Serve `/openapi.json` — the machine-readable spec.
pub async fn openapi_spec() -> Json<Value> {
    Json(spec().clone())
}

/// Serve `/docs` — Swagger UI bound to `/openapi.json`.
pub async fn swagger_ui() -> Html<&'static str> {
    Html(SWAGGER_UI_HTML)
}

fn spec() -> &'static Value {
    static SPEC: OnceLock<Value> = OnceLock::new();
    SPEC.get_or_init(|| {
        let raw = SPEC_JSON.replace("__VERSION__", env!("CARGO_PKG_VERSION"));
        serde_json::from_str(&raw).expect("OpenAPI spec must be valid JSON")
    })
}

const SWAGGER_UI_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <title>ai-ontology.com — API docs</title>
    <link
      rel="stylesheet"
      href="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui.css"
    />
    <style>body { margin: 0; } #swagger-ui { box-sizing: border-box; }</style>
  </head>
  <body>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui-bundle.js"></script>
    <script src="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui-standalone-preset.js"></script>
    <script>
      window.ui = SwaggerUIBundle({
        url: "./openapi.json",
        dom_id: "#swagger-ui",
        deepLinking: true,
        presets: [SwaggerUIBundle.presets.apis, SwaggerUIStandalonePreset],
        layout: "StandaloneLayout",
        persistAuthorization: true,
      });
    </script>
  </body>
</html>
"##;

const SPEC_JSON: &str = r##"{
    "openapi": "3.0.3",
    "info": {
        "title": "ai-ontology.com API",
        "version": "__VERSION__",
        "description": "Graph + RAG API for the ai-ontology.com stack. All endpoints except `/healthz`, `/openapi.json` and `/docs` require a bearer token when the server is started with `--bearer-token` or with `--jwt-*` flags.",
        "license": {
            "name": "AGPL-3.0-or-later OR LicenseRef-Winven-Commercial",
            "url": "https://www.gnu.org/licenses/agpl-3.0.html"
        }
    },
    "servers": [
        { "url": "/", "description": "This server" }
        ],
        "security": [{ "bearerAuth": [] }],
        "components": {
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "JWT or static token"
                }
            },
            "schemas": {
                "Error": {
                    "type": "object",
                    "properties": {
                        "error": { "type": "string" }
                    },
                    "required": ["error"]
                },
                "Stats": {
                    "type": "object",
                    "properties": {
                        "concepts": { "type": "integer" },
                        "relations": { "type": "integer" },
                        "rules": { "type": "integer" },
                        "actions": { "type": "integer" },
                        "concept_types": { "type": "integer" },
                        "relation_types": { "type": "integer" }
                    }
                },
                "PropertyValue": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "number" },
                        { "type": "boolean" }
                    ],
                    "description": "Free-form scalar property: text / number / boolean."
                },
                "Concept": {
                    "type": "object",
                    "required": ["concept_type", "name"],
                    "properties": {
                        "id": { "type": "integer", "format": "int64" },
                        "concept_type": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "properties": {
                            "type": "object",
                            "additionalProperties": { "$ref": "#/components/schemas/PropertyValue" }
                        }
                    }
                },
                "ConceptPatch": {
                    "type": "object",
                    "description": "Partial update for a concept. `concept_type` is immutable.",
                    "properties": {
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "properties": {
                            "type": "object",
                            "additionalProperties": { "$ref": "#/components/schemas/PropertyValue" }
                        }
                    }
                },
                "Relation": {
                    "type": "object",
                    "required": ["relation_type", "source", "target"],
                    "properties": {
                        "id": { "type": "integer", "format": "int64" },
                        "relation_type": { "type": "string" },
                        "source": { "type": "integer", "format": "int64" },
                        "target": { "type": "integer", "format": "int64" },
                        "weight": { "type": "number", "format": "float", "default": 1.0 },
                        "properties": {
                            "type": "object",
                            "additionalProperties": { "$ref": "#/components/schemas/PropertyValue" }
                        }
                    }
                },
                "RelationPatch": {
                    "type": "object",
                    "properties": {
                        "weight": { "type": "number", "format": "float" },
                        "properties": {
                            "type": "object",
                            "additionalProperties": { "$ref": "#/components/schemas/PropertyValue" }
                        }
                    }
                },
                "Rule": {
                    "type": "object",
                    "required": ["rule_type", "name", "when", "then"],
                    "properties": {
                        "id": { "type": "integer", "format": "int64" },
                        "rule_type": { "type": "string" },
                        "name": { "type": "string" },
                        "when": { "type": "string", "description": "Antecedent / condition expression." },
                        "then": { "type": "string", "description": "Consequent / conclusion expression." },
                        "applies_to": {
                            "type": "array",
                            "items": { "type": "integer", "format": "int64" }
                        },
                        "strict": { "type": "boolean" },
                        "description": { "type": "string" },
                        "properties": {
                            "type": "object",
                            "additionalProperties": { "$ref": "#/components/schemas/PropertyValue" }
                        }
                    }
                },
                "RulePatch": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "when": { "type": "string" },
                        "then": { "type": "string" },
                        "applies_to": {
                            "type": "array",
                            "items": { "type": "integer", "format": "int64" }
                        },
                        "strict": { "type": "boolean" },
                        "description": { "type": "string" },
                        "properties": {
                            "type": "object",
                            "additionalProperties": { "$ref": "#/components/schemas/PropertyValue" }
                        }
                    }
                },
                "Action": {
                    "type": "object",
                    "required": ["action_type", "name", "subject"],
                    "properties": {
                        "id": { "type": "integer", "format": "int64" },
                        "action_type": { "type": "string" },
                        "name": { "type": "string" },
                        "subject": { "type": "integer", "format": "int64" },
                        "object": { "type": "integer", "format": "int64", "nullable": true },
                        "parameters": {
                            "type": "object",
                            "additionalProperties": { "$ref": "#/components/schemas/PropertyValue" }
                        },
                        "effect": { "type": "string" },
                        "description": { "type": "string" }
                    }
                },
                "ActionPatch": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "subject": { "type": "integer", "format": "int64" },
                        "object": { "type": "integer", "format": "int64", "nullable": true },
                        "parameters": {
                            "type": "object",
                            "additionalProperties": { "$ref": "#/components/schemas/PropertyValue" }
                        },
                        "effect": { "type": "string" },
                        "description": { "type": "string" }
                    }
                },
                "Ontology": {
                    "type": "object",
                    "properties": {
                        "concept_types": { "type": "object", "additionalProperties": true },
                        "relation_types": { "type": "object", "additionalProperties": true },
                        "rule_types": { "type": "object", "additionalProperties": true },
                        "action_types": { "type": "object", "additionalProperties": true }
                    }
                },
                "RetrievalRequest": {
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": { "type": "string" },
                        "top_k": { "type": "integer", "default": 8 },
                        "depth": { "type": "integer", "default": 1 }
                    }
                },
                "RagAnswer": {
                    "type": "object",
                    "properties": {
                        "answer": { "type": "string" },
                        "citations": { "type": "array", "items": { "type": "object" } },
                        "subgraph": { "type": "object" }
                    }
                }
            }
        },
        "paths": {
            "/healthz": {
                "get": {
                    "summary": "Liveness probe",
                    "security": [],
                    "tags": ["meta"],
                    "responses": { "200": { "description": "ok" } }
                }
            },
            "/openapi.json": {
                "get": {
                    "summary": "Machine-readable OpenAPI spec",
                    "security": [],
                    "tags": ["meta"],
                    "responses": { "200": { "description": "OpenAPI 3.0 document" } }
                }
            },
            "/docs": {
                "get": {
                    "summary": "Swagger UI",
                    "security": [],
                    "tags": ["meta"],
                    "responses": { "200": { "description": "HTML page" } }
                }
            },
            "/stats": {
                "get": {
                    "summary": "Graph counts snapshot",
                    "tags": ["meta"],
                    "responses": {
                        "200": {
                            "description": "Counts",
                            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Stats" } } }
                        }
                    }
                }
            },
            "/stats/history": {
                "get": {
                    "summary": "Time-series of stats snapshots",
                    "tags": ["meta"],
                    "responses": { "200": { "description": "Array of snapshots" } }
                }
            },
            "/metrics": {
                "get": {
                    "summary": "Prometheus text metrics",
                    "tags": ["meta"],
                    "responses": { "200": { "description": "text/plain metrics" } }
                }
            },
            "/ontology": {
                "get": {
                    "summary": "Get the active ontology schema",
                    "tags": ["ontology"],
                    "responses": {
                        "200": {
                            "description": "Ontology",
                            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Ontology" } } }
                        }
                    }
                },
                "put": {
                    "summary": "Replace the active ontology schema",
                    "tags": ["ontology"],
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Ontology" } } }
                    },
                    "responses": { "200": { "description": "Updated" } }
                }
            },
            "/ontology/generate": {
                "post": {
                    "summary": "LLM-generate an ontology from a description",
                    "tags": ["ontology"],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "description": { "type": "string" },
                                        "apply": { "type": "boolean", "default": false }
                                    },
                                    "required": ["description"]
                                }
                            }
                        }
                    },
                    "responses": { "200": { "description": "Generated ontology" } }
                }
            },
            "/concepts": {
                "get": {
                    "summary": "List concepts",
                    "tags": ["concepts"],
                    "parameters": [
                        { "name": "type",   "in": "query", "schema": { "type": "string" } },
                        { "name": "q",      "in": "query", "schema": { "type": "string" } },
                        { "name": "limit",  "in": "query", "schema": { "type": "integer", "default": 50 } },
                        { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 } }
                    ],
                    "responses": { "200": { "description": "List response" } }
                },
                "post": {
                    "summary": "Create a concept",
                    "tags": ["concepts"],
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Concept" } } }
                    },
                    "responses": {
                        "200": { "description": "Created", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Concept" } } } }
                    }
                }
            },
            "/concepts/{id}": {
                "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } }],
                "get":    { "summary": "Get a concept",    "tags": ["concepts"], "responses": { "200": { "description": "Concept" }, "404": { "description": "Not found" } } },
                "patch":  {
                    "summary": "Update a concept", "tags": ["concepts"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ConceptPatch" } } } },
                    "responses": { "200": { "description": "Updated" } }
                },
                "delete": { "summary": "Delete a concept", "tags": ["concepts"], "responses": { "204": { "description": "Deleted" } } }
            },
            "/relations": {
                "get": {
                    "summary": "List relations",
                    "tags": ["relations"],
                    "parameters": [
                        { "name": "source", "in": "query", "schema": { "type": "integer" } },
                        { "name": "target", "in": "query", "schema": { "type": "integer" } },
                        { "name": "type",   "in": "query", "schema": { "type": "string" } },
                        { "name": "limit",  "in": "query", "schema": { "type": "integer", "default": 50 } },
                        { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 } }
                    ],
                    "responses": { "200": { "description": "List response" } }
                },
                "post": {
                    "summary": "Create a relation",
                    "tags": ["relations"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Relation" } } } },
                    "responses": { "200": { "description": "Created" } }
                }
            },
            "/relations/{id}": {
                "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } }],
                "get":    { "summary": "Get a relation",    "tags": ["relations"], "responses": { "200": { "description": "Relation" } } },
                "patch":  {
                    "summary": "Update a relation", "tags": ["relations"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RelationPatch" } } } },
                    "responses": { "200": { "description": "Updated" } }
                },
                "delete": { "summary": "Delete a relation", "tags": ["relations"], "responses": { "204": { "description": "Deleted" } } }
            },
            "/rules": {
                "get":  { "summary": "List rules",  "tags": ["rules"], "responses": { "200": { "description": "List" } } },
                "post": {
                    "summary": "Create a rule", "tags": ["rules"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Rule" } } } },
                    "responses": { "200": { "description": "Created" } }
                }
            },
            "/rules/{id}": {
                "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } }],
                "get":    { "summary": "Get a rule",    "tags": ["rules"], "responses": { "200": { "description": "Rule" } } },
                "patch":  {
                    "summary": "Update a rule", "tags": ["rules"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RulePatch" } } } },
                    "responses": { "200": { "description": "Updated" } }
                },
                "delete": { "summary": "Delete a rule", "tags": ["rules"], "responses": { "204": { "description": "Deleted" } } }
            },
            "/actions": {
                "get":  { "summary": "List actions",  "tags": ["actions"], "responses": { "200": { "description": "List" } } },
                "post": {
                    "summary": "Create an action", "tags": ["actions"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Action" } } } },
                    "responses": { "200": { "description": "Created" } }
                }
            },
            "/actions/{id}": {
                "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } }],
                "get":    { "summary": "Get an action",    "tags": ["actions"], "responses": { "200": { "description": "Action" } } },
                "patch":  {
                    "summary": "Update an action", "tags": ["actions"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ActionPatch" } } } },
                    "responses": { "200": { "description": "Updated" } }
                },
                "delete": { "summary": "Delete an action", "tags": ["actions"], "responses": { "204": { "description": "Deleted" } } }
            },
            "/retrieve": {
                "post": {
                    "summary": "Hybrid (lexical+vector) retrieval over the graph",
                    "tags": ["rag"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RetrievalRequest" } } } },
                    "responses": { "200": { "description": "Seeds + subgraph" } }
                }
            },
            "/subgraph": {
                "post": {
                    "summary": "Expand a subgraph around given seeds",
                    "tags": ["rag"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object" } } } },
                    "responses": { "200": { "description": "Subgraph" } }
                }
            },
            "/ask": {
                "post": {
                    "summary": "RAG question answering",
                    "tags": ["rag"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RetrievalRequest" } } } },
                    "responses": {
                        "200": { "description": "Answer", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RagAnswer" } } } }
                    }
                }
            },
            "/ask/stream": {
                "post": {
                    "summary": "RAG QA streamed as Server-Sent Events",
                    "tags": ["rag"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RetrievalRequest" } } } },
                    "responses": { "200": { "description": "text/event-stream" } }
                }
            },
            "/path": {
                "post": {
                    "summary": "Shortest path between two concepts",
                    "tags": ["rag"],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "source": { "type": "integer" },
                                        "target": { "type": "integer" },
                                        "max_depth": { "type": "integer", "default": 6 }
                                    },
                                    "required": ["source", "target"]
                                }
                            }
                        }
                    },
                    "responses": { "200": { "description": "Path or null" } }
                }
            },
            "/compact": {
                "post": {
                    "summary": "Snapshot the graph and truncate the WAL",
                    "tags": ["admin"],
                    "responses": { "200": { "description": "Compacted" } }
                }
            },
            "/upload": {
                "post": {
                    "summary": "Upload + ingest a CSV / JSONL / TXT / XLSX file",
                    "tags": ["ingest"],
                    "requestBody": {
                        "required": true,
                        "content": { "multipart/form-data": { "schema": { "type": "object", "properties": { "file": { "type": "string", "format": "binary" } } } } }
                    },
                    "responses": { "200": { "description": "Ingest report" } }
                }
            },
            "/export": {
                "get": {
                    "summary": "Export the graph",
                    "tags": ["ingest"],
                    "parameters": [
                        { "name": "format", "in": "query", "schema": { "type": "string", "enum": ["json", "jsonl", "csv"] } }
                    ],
                    "responses": { "200": { "description": "Exported payload" } }
                }
            },
            "/files":      { "get": { "summary": "List ingested files",  "tags": ["ingest"], "responses": { "200": { "description": "List" } } } },
            "/files/{id}": {
                "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } }],
                "get":    { "summary": "Get an ingested file's record", "tags": ["ingest"], "responses": { "200": { "description": "File" } } },
                "delete": { "summary": "Delete an ingested file",      "tags": ["ingest"], "responses": { "204": { "description": "Deleted" } } }
            },
            "/queries": {
                "get":  { "summary": "List saved queries",  "tags": ["queries"], "responses": { "200": { "description": "List" } } },
                "post": { "summary": "Create a saved query","tags": ["queries"], "responses": { "200": { "description": "Created" } } }
            },
            "/queries/{id}": {
                "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } }],
                "get":    { "summary": "Get a saved query",    "tags": ["queries"], "responses": { "200": { "description": "Query" } } },
                "patch":  { "summary": "Update a saved query", "tags": ["queries"], "responses": { "200": { "description": "Updated" } } },
                "delete": { "summary": "Delete a saved query", "tags": ["queries"], "responses": { "204": { "description": "Deleted" } } }
            },
            "/queries/{id}/run": {
                "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } }],
                "post": { "summary": "Execute a saved query", "tags": ["queries"], "responses": { "200": { "description": "Result" } } }
            },
            "/settings": {
                "get":   { "summary": "Get runtime settings",   "tags": ["admin"], "responses": { "200": { "description": "Settings" } } },
                "patch": { "summary": "Patch runtime settings", "tags": ["admin"], "responses": { "200": { "description": "Updated" } } }
            }
        }
    }"##;
