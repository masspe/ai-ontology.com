use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ontology_graph::{Ontology, OntologyGraph};
use ontology_index::{HybridIndex, RetrievalRequest};
use ontology_io::{ingest_records, CsvSource, JsonlSource, TripleSource};
use ontology_server::{build_router, AppState};
use ontology_rag::{AnthropicModel, EchoModel, RagPipeline};
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
    /// Ingest data from a file. Format inferred from extension (.jsonl,
    /// .triples or .csv). For CSV, also pass `--csv-type <ConceptType>`.
    Ingest {
        /// Optional path to an ontology JSON file applied before records.
        #[arg(long)]
        ontology: Option<PathBuf>,
        /// For CSV inputs: the concept type to assign every row.
        #[arg(long)]
        csv_type: Option<String>,
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
    /// --anthropic to call the live Messages API (requires ANTHROPIC_API_KEY).
    Ask {
        query: String,
        #[arg(long, default_value_t = 8)]
        top_k: usize,
        #[arg(long, default_value_t = 2)]
        depth: u32,
        #[arg(long)]
        anthropic: bool,
        #[arg(long, default_value = "claude-opus-4-7")]
        model: String,
    },
    /// Take a durable snapshot of the current graph (only with --data).
    Snapshot,
    /// Run the HTTP server.
    Serve {
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
        #[arg(long)]
        anthropic: bool,
        #[arg(long, default_value = "claude-opus-4-7")]
        model: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,ontology=debug")))
        .with_target(false)
        .init();

    let cli = Cli::parse();

    let store: Arc<dyn Store> = match &cli.data {
        Some(dir) => Arc::new(FileStore::open(dir).await?),
        None => Arc::new(MemoryStore::new()),
    };

    let graph = OntologyGraph::with_arc(Ontology::new());
    store.load_into(&graph).await.context("loading existing data")?;

    let index = Arc::new(HybridIndex::with_default_embedder(graph.clone()));
    index.reindex_all();

    match cli.cmd {
        Cmd::Ingest { ontology, csv_type, path } => {
            if let Some(p) = ontology {
                let raw = tokio::fs::read_to_string(&p).await?;
                let onto: Ontology = serde_json::from_str(&raw)?;
                graph.extend_ontology(|target| { *target = onto.clone(); Ok(()) })?;
                store.append(&ontology_storage::LogRecord::ontology(onto)).await?;
            }
            let stats = match path.extension().and_then(|s| s.to_str()) {
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
                _ => anyhow::bail!("unsupported extension; use .jsonl, .triples or .csv"),
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
        Cmd::Retrieve { query, top_k, depth } => {
            let mut req = RetrievalRequest { query, top_k, ..Default::default() };
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
            println!("\n# subgraph: {} concepts, {} edges",
                subgraph.concepts.len(), subgraph.relations.len());
            for r in &subgraph.relations {
                let s = subgraph.concepts.iter().find(|c| c.id == r.source);
                let t = subgraph.concepts.iter().find(|c| c.id == r.target);
                if let (Some(s), Some(t)) = (s, t) {
                    println!("  {} -[{}]-> {}", s.name, r.relation_type, t.name);
                }
            }
        }
        Cmd::Ask { query, top_k, depth, anthropic, model } => {
            let llm: Arc<dyn ontology_rag::LanguageModel> = if anthropic {
                let key = std::env::var("ANTHROPIC_API_KEY")
                    .context("ANTHROPIC_API_KEY required for --anthropic")?;
                Arc::new(AnthropicModel::new(key).with_model(model))
            } else {
                Arc::new(EchoModel)
            };
            let pipe = RagPipeline::new(index.clone(), llm);
            let mut req = RetrievalRequest { query, top_k, ..Default::default() };
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
                    u.input_tokens, u.output_tokens,
                    u.cache_creation_input_tokens, u.cache_read_input_tokens,
                );
            }
        }
        Cmd::Snapshot => {
            store.snapshot(&graph).await?;
            println!("snapshot written");
        }
        Cmd::Serve { bind, anthropic, model } => {
            let llm: Arc<dyn ontology_rag::LanguageModel> = if anthropic {
                let key = std::env::var("ANTHROPIC_API_KEY")
                    .context("ANTHROPIC_API_KEY required for --anthropic")?;
                Arc::new(AnthropicModel::new(key).with_model(model))
            } else {
                Arc::new(EchoModel)
            };
            let pipeline = Arc::new(RagPipeline::new(index.clone(), llm));
            let state = AppState {
                graph: graph.clone(),
                index: index.clone(),
                store: store.clone(),
                pipeline,
            };
            let app = build_router(state);
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            tracing::info!(addr = %bind, "server listening");
            axum::serve(listener, app).await?;
        }
    }

    Ok(())
}
