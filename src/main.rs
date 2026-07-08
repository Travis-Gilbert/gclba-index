use anyhow::Result;
use clap::{Parser, Subcommand};
use gclba_index::http;
use gclba_index::{
    attach_datawave_summary, build_index, ingest_datawave_records, parse_adapter_bytes,
    AdapterKind, ExportStore, IndexPayload,
};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "gclba-index")]
#[command(about = "Private GCLBA property intelligence index")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Ingest {
        #[arg(long)]
        adapter: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        skip_datawave: bool,
    },
    Build {
        #[arg(long = "source", required = true)]
        sources: Vec<String>,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        skip_datawave: bool,
    },
    Serve {
        #[arg(long)]
        index: Option<PathBuf>,
        #[arg(long, default_value = "exports")]
        exports_dir: PathBuf,
        #[arg(long, default_value = "0.0.0.0:8080")]
        bind: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Ingest {
            adapter,
            file,
            output,
            skip_datawave,
        } => {
            let adapter: AdapterKind = adapter.parse().map_err(anyhow::Error::msg)?;
            let bytes = std::fs::read(&file)?;
            let records = parse_adapter_bytes(adapter, &bytes)?;
            let payload = if skip_datawave {
                build_index(records)
            } else {
                let datawave = ingest_datawave_records(&records)?;
                let mut payload = build_index(records);
                attach_datawave_summary(&mut payload, &datawave);
                payload
            };
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&output, serde_json::to_vec_pretty(&payload)?)?;
            println!(
                "wrote {} parcels, {} homes, {} active programs to {}{}",
                payload.coverage.parcels_indexed,
                payload.coverage.home_count,
                payload.coverage.active_program_count,
                output.display(),
                datawave_summary_suffix(&payload),
            );
        }
        Command::Build {
            sources,
            output,
            skip_datawave,
        } => {
            let mut records = Vec::new();
            for source in sources {
                let (adapter, path) = source.split_once(':').ok_or_else(|| {
                    anyhow::anyhow!("source must be formatted as adapter:path, got {source:?}")
                })?;
                let adapter: AdapterKind = adapter.parse().map_err(anyhow::Error::msg)?;
                let bytes = std::fs::read(path)?;
                records.extend(parse_adapter_bytes(adapter, &bytes)?);
            }
            let payload = if skip_datawave {
                build_index(records)
            } else {
                let datawave = ingest_datawave_records(&records)?;
                let mut payload = build_index(records);
                attach_datawave_summary(&mut payload, &datawave);
                payload
            };
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&output, serde_json::to_vec_pretty(&payload)?)?;
            println!(
                "wrote {} parcels, {} homes, {} active programs, {} conflicts, {} candidates to {}{}",
                payload.coverage.parcels_indexed,
                payload.coverage.home_count,
                payload.coverage.active_program_count,
                payload.coverage.open_conflict_count,
                payload.coverage.candidate_count,
                output.display(),
                datawave_summary_suffix(&payload),
            );
        }
        Command::Serve {
            index,
            exports_dir,
            bind,
        } => {
            let store = ExportStore::new(exports_dir);
            let payload = match index {
                Some(path) => {
                    let bytes = std::fs::read(path)?;
                    serde_json::from_slice::<IndexPayload>(&bytes)?
                }
                None => store.rebuild_index()?,
            };
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            println!(
                "serving gclba-index on {bind} with exports from {}",
                store.root().display()
            );
            axum::serve(listener, http::router_with_store(payload, store)).await?;
        }
    }
    Ok(())
}

fn datawave_summary_suffix(payload: &IndexPayload) -> String {
    if payload.coverage.datawave_ingested == 0 && payload.coverage.datawave_skipped == 0 {
        " (datawave skipped)".to_string()
    } else {
        format!(
            " (datawave ingested {}, skipped {})",
            payload.coverage.datawave_ingested, payload.coverage.datawave_skipped
        )
    }
}
