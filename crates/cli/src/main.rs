// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ontology_graph::{Ontology, OntologyGraph};
use ontology_index::{HybridIndex, RetrievalRequest};
use ontology_io::{
    export_graph, ingest_records, CsvSource, JsonlSink, JsonlSource, TextDocumentSource,
    TripleSource, XlsxSource,
};
use ontology_rag::{AnthropicModel, EchoModel, OpenAiModel, RagPipeline};
use ontology_server::AppState;
use ontology_storage::{FileStore, MemoryStore, Store};
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser, Debug)]
#[command(name = "ontology", version, about = "Ontology graph + RAG CLI")]
struct Cli {
    /// Persistent data directory. Omit for an ephemeral in-memory store.
    #[arg(long, global = true)]
    data: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Ingest data from a file or directory.
    ///
    /// Format is inferred from the path's extension:
    /// - `.jsonl` / `.ndjson` — tagged Records (Concept / Relation / Ontology / NamedRelation).
    /// - `.triples` / `.txt`  — `Type:Name predicate Type:Name` lines.
    /// - `.csv`               — concept rows; requires `--csv-type`.
    /// - `.xlsx` / `.xls` / `.ods` — concept rows; requires `--xlsx-type`.
    /// - `-`                   — read JSONL from stdin.
    /// - directory             — every regular file becomes a Concept whose
    ///   description is the file's text. Requires `--text-type` and accepts
    ///   `--text-ext` (default: `txt,md`).
    Ingest {
        /// Optional path to an ontology JSON file applied before records.
        #[arg(long)]
        ontology: Option<PathBuf>,
        /// For CSV inputs: the concept type to assign every row.
        #[arg(long)]
        csv_type: Option<String>,
        /// For XLSX/XLS/ODS inputs: the concept type to assign every row.
        #[arg(long)]
        xlsx_type: Option<String>,
        /// For directory inputs: the concept type to assign each text file.
        #[arg(long)]
        text_type: Option<String>,
        /// File extensions (comma-separated, no dot) to pick up when
        /// ingesting a directory. Default: `txt,md`.
        #[arg(long, default_value = "txt,md")]
        text_ext: String,
        path: PathBuf,
    },
    /// Print summary statistics.
    Stats,
    /// Run a retrieval-only query and print the ranked seeds + subgraph.
    Retrieve {
        query: String,
        #[arg(long, default_value_t = 8)]
        top_k: usize,
        #[arg(long, default_value_t = 2)]
        depth: u32,
    },
    /// Run the full RAG pipeline. Uses the EchoModel by default; pass
    /// --anthropic, --openai, or --deepseek to call a live provider
    /// (each requires its own API-key env var).
    Ask {
        query: String,
        #[arg(long, default_value_t = 8)]
        top_k: usize,
        #[arg(long, default_value_t = 2)]
        depth: u32,
        /// Use the Anthropic Messages API. Requires `ANTHROPIC_API_KEY`.
        #[arg(long, conflicts_with_all = ["openai", "deepseek"])]
        anthropic: bool,
        /// Use the OpenAI chat completions API. Requires `OPENAI_API_KEY`.
        #[arg(long, conflicts_with_all = ["anthropic", "deepseek"])]
        openai: bool,
        /// Use the DeepSeek chat completions API (OpenAI-compatible).
        /// Requires `DEEPSEEK_API_KEY`.
        #[arg(long, conflicts_with_all = ["anthropic", "openai"])]
        deepseek: bool,
        /// Override the model name. Defaults: claude-opus-4-7 (anthropic),
        /// gpt-4o-mini (openai), deepseek-chat (deepseek).
        #[arg(long)]
        model: Option<String>,
    },
    /// Take a durable snapshot of the current graph (only with --data).
    Snapshot,
    /// Snapshot then truncate the WAL. Bounds disk usage on busy stores.
    Compact,
    /// Export the entire graph as a JSONL stream of tagged records.
    Export { path: PathBuf },
    /// Find a shortest path between two named concepts.
    Path {
        #[arg(long)]
        from_type: String,
        #[arg(long)]
        from_name: String,
        #[arg(long)]
        to_type: String,
        #[arg(long)]
        to_name: String,
        #[arg(long, default_value_t = 6)]
        max_depth: u32,
    },
    /// Run the HTTP server.
    Serve {
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
        /// Use the Anthropic Messages API. Requires `ANTHROPIC_API_KEY`.
        #[arg(long, conflicts_with_all = ["openai", "deepseek"])]
        anthropic: bool,
        /// Use the OpenAI chat completions API. Requires `OPENAI_API_KEY`.
        #[arg(long, conflicts_with_all = ["anthropic", "deepseek"])]
        openai: bool,
        /// Use the DeepSeek chat completions API. Requires `DEEPSEEK_API_KEY`.
        #[arg(long, conflicts_with_all = ["anthropic", "openai"])]
        deepseek: bool,
        /// Override the model name. Defaults: claude-opus-4-7 (anthropic),
        /// gpt-4o-mini (openai), deepseek-chat (deepseek).
        #[arg(long)]
        model: Option<String>,
        /// Optional bearer token. When set, every route except /healthz
        /// requires `Authorization: Bearer <token>`. Reads from the env
        /// var named here, NOT the literal value, so the secret never
        /// appears in process listings or shell history.
        #[arg(long)]
        auth_env: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,ontology=debug")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    let store: Arc<dyn Store> = match &cli.data {
        Some(dir) => Arc::new(FileStore::open(dir).await?),
        None => Arc::new(MemoryStore::new()),
    };

    let graph = OntologyGraph::with_arc(Ontology::new());
    store
        .load_into(&graph)
        .await
        .context("loading existing data")?;

    let index = Arc::new(HybridIndex::with_default_embedder(graph.clone()));
    index.reindex_all();

    match cli.cmd {
        Cmd::Ingest {
            ontology,
            csv_type,
            xlsx_type,
            text_type,
            text_ext,
            path,
        } => {
            if let Some(p) = ontology {
                let raw = tokio::fs::read_to_string(&p).await?;
                let onto: Ontology = serde_json::from_str(&raw)?;
                graph.extend_ontology(|target| {
                    *target = onto.clone();
                    Ok(())
                })?;
                store
                    .append(&ontology_storage::LogRecord::ontology(onto))
                    .await?;
            }

            let is_dir = tokio::fs::metadata(&path)
                .await
                .map(|m| m.is_dir())
                .unwrap_or(false);

            // `-` reads JSONL from stdin (handy for piping).
            let stats = if path.as_os_str() == "-" {
                let mut src = JsonlSource::stdin();
                ingest_records(&mut src, &graph, Some(store.as_ref())).await?
            } else if is_dir {
                let ty = text_type.context("--text-type required when ingesting a directory")?;
                let exts: Vec<&str> = text_ext.split(',').map(str::trim).collect();
                let mut src = TextDocumentSource::from_dir(ty, &path, &exts).await?;
                ingest_records(&mut src, &graph, Some(store.as_ref())).await?
            } else {
                match path.extension().and_then(|s| s.to_str()) {
                    Some("jsonl") | Some("ndjson") => {
                        let mut src = JsonlSource::open(&path).await?;
                        ingest_records(&mut src, &graph, Some(store.as_ref())).await?
                    }
                    Some("triples") | Some("txt") => {
                        let mut src = TripleSource::open(&path).await?;
                        ingest_records(&mut src, &graph, Some(store.as_ref())).await?
                    }
                    Some("csv") => {
                        let ty = csv_type.context("--csv-type required for CSV input")?;
                        let mut src = CsvSource::open(&path, ty).await?;
                        ingest_records(&mut src, &graph, Some(store.as_ref())).await?
                    }
                    Some("xlsx") | Some("xls") | Some("ods") => {
                        let ty = xlsx_type.context("--xlsx-type required for spreadsheet input")?;
                        let mut src = XlsxSource::open(&path, ty)?;
                        ingest_records(&mut src, &graph, Some(store.as_ref())).await?
                    }
                    _ => anyhow::bail!(
                        "unsupported extension; use .jsonl, .triples, .csv, .xlsx, '-' for stdin, \
                         or pass a directory with --text-type"
                    ),
                }
            };
            index.reindex_all();
            println!(
                "ingested: {} concepts, {} relations, {} ontology updates",
                stats.concepts, stats.relations, stats.ontology_updates,
            );
        }
        Cmd::Stats => {
            let onto = graph.ontology();
            println!(
                "concepts: {}\nrelations: {}\nconcept_types: {}\nrelation_types: {}",
                graph.concept_count(),
                graph.relation_count(),
                onto.concept_types.len(),
                onto.relation_types.len(),
            );
        }
        Cmd::Retrieve {
            query,
            top_k,
            depth,
        } => {
            let mut req = RetrievalRequest {
                query,
                top_k,
                ..Default::default()
            };
            req.expansion.max_depth = depth;
            let (scored, subgraph) = index.retrieve(&req);
            println!("# top-{} concepts", scored.len());
            for s in &scored {
                if let Ok(c) = graph.get_concept(s.id) {
                    println!(
                        "{:>6.3}  ({}) {}  [lex={:.2} vec={:.2}]",
                        s.score, c.concept_type, c.name, s.lexical, s.vector,
                    );
                }
            }
            println!(
                "\n# subgraph: {} concepts, {} edges",
                subgraph.concepts.len(),
                subgraph.relations.len()
            );
            for r in &subgraph.relations {
                let s = subgraph.concepts.iter().find(|c| c.id == r.source);
                let t = subgraph.concepts.iter().find(|c| c.id == r.target);
                if let (Some(s), Some(t)) = (s, t) {
                    println!("  {} -[{}]-> {}", s.name, r.relation_type, t.name);
                }
            }
        }
        Cmd::Ask {
            query,
            top_k,
            depth,
            anthropic,
            openai,
            deepseek,
            model,
        } => {
            let llm = build_llm(anthropic, openai, deepseek, model.as_deref())?;
            let pipe = RagPipeline::new(index.clone(), llm);
            let mut req = RetrievalRequest {
                query,
                top_k,
                ..Default::default()
            };
            req.expansion.max_depth = depth;
            let answer = pipe.answer_with(req).await?;
            println!("--- answer ---\n{}\n", answer.answer);
            println!("--- citations ---");
            for s in &answer.retrieved {
                if let Ok(c) = graph.get_concept(s.id) {
                    println!("  ({}) {}  score={:.3}", c.concept_type, c.name, s.score);
                }
            }
            let u = &answer.usage;
            if u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens > 0 {
                println!(
                    "--- usage --- in={} out={} cache_write={} cache_read={}",
                    u.input_tokens,
                    u.output_tokens,
                    u.cache_creation_input_tokens,
                    u.cache_read_input_tokens,
                );
            }
        }
        Cmd::Snapshot => {
            store.snapshot(&graph).await?;
            println!("snapshot written");
        }
        Cmd::Compact => {
            store.compact(&graph).await?;
            println!("compacted: snapshot written and WAL truncated");
        }
        Cmd::Export { path } => {
            let mut sink = JsonlSink::create(&path).await?;
            let stats = export_graph(&graph, &mut sink).await?;
            println!(
                "exported: {} concepts, {} relations -> {}",
                stats.concepts,
                stats.relations,
                path.display(),
            );
        }
        Cmd::Path {
            from_type,
            from_name,
            to_type,
            to_name,
            max_depth,
        } => {
            let src = graph
                .find_by_name(&from_type, &from_name)
                .ok_or_else(|| anyhow::anyhow!("no concept ({from_type}) {from_name}"))?;
            let tgt = graph
                .find_by_name(&to_type, &to_name)
                .ok_or_else(|| anyhow::anyhow!("no concept ({to_type}) {to_name}"))?;
            match graph.shortest_path(src, tgt, max_depth)? {
                None => println!("no path within depth {max_depth}"),
                Some(path) => {
                    print!("{}", path.start.name);
                    for step in &path.steps {
                        print!(
                            " -[{}]-> {}",
                            step.relation.relation_type, step.concept.name
                        );
                    }
                    println!("\n({} hops)", path.len());
                }
            }
        }
        Cmd::Serve {
            bind,
            anthropic,
            openai,
            deepseek,
            model,
            auth_env,
        } => {
            let llm = build_llm(anthropic, openai, deepseek, model.as_deref())?;
            let pipeline = Arc::new(RagPipeline::new(index.clone(), llm));
            let state = AppState {
                graph: graph.clone(),
                index: index.clone(),
                store: store.clone(),
                pipeline,
            };
            let bearer = match auth_env {
                Some(env_name) => Some(
                    std::env::var(&env_name)
                        .with_context(|| format!("env var `{env_name}` is unset"))?,
                ),
                None => None,
            };
            let app = ontology_server::build_router_with_auth(state, bearer);
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            tracing::info!(addr = %bind, "server listening");
            axum::serve(listener, app).await?;
        }
    }

    Ok(())
}

/// Resolve the `--anthropic` / `--openai` / `--deepseek` triple plus an
/// optional model override to a concrete `LanguageModel`. clap already
/// guarantees at most one of the three flags is set.
fn build_llm(
    anthropic: bool,
    openai: bool,
    deepseek: bool,
    model: Option<&str>,
) -> Result<Arc<dyn ontology_rag::LanguageModel>> {
    if anthropic {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY required for --anthropic")?;
        let mut m = AnthropicModel::new(key);
        if let Some(name) = model {
            m = m.with_model(name);
        }
        Ok(Arc::new(m))
    } else if openai {
        let key = std::env::var("OPENAI_API_KEY")
            .context("OPENAI_API_KEY required for --openai")?;
        let mut m = OpenAiModel::new(key);
        if let Some(name) = model {
            m = m.with_model(name);
        }
        Ok(Arc::new(m))
    } else if deepseek {
        let key = std::env::var("DEEPSEEK_API_KEY")
            .context("DEEPSEEK_API_KEY required for --deepseek")?;
        let mut m = OpenAiModel::deepseek(key);
        if let Some(name) = model {
            m = m.with_model(name);
        }
        Ok(Arc::new(m))
    } else {
        Ok(Arc::new(EchoModel))
    }
}
