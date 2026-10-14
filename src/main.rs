//! CLI entrypoint. Three operational modes plus a manual snapshot:
//!
//! - `serve`   — web UI + continuous folder watcher + importer
//! - `import`  — bulk import of a directory
//! - `reparse` — rebuild parsed data from retained raw text
//! - `snapshot`— take a database snapshot now

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use texture::config::Config;
use texture::storage::Db;
use texture::{importer, snapshot, web};

#[derive(Parser, Debug)]
#[command(name = "texture", version = texture::VERSION, about = "Self-hosted PokerStars NLHE cash-game tracker")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the web UI, the continuous hand-history watcher, and the importer.
    Serve {
        /// Hand-history directory to watch (overrides TEXTURE_HAND_HISTORY_DIR).
        #[arg(long, env = "TEXTURE_HAND_HISTORY_DIR")]
        hands: Option<PathBuf>,
    },
    /// Bulk-import a hand-history directory.
    Import {
        /// Directory to import. Defaults to the configured hand-history dir.
        path: Option<PathBuf>,
        /// Re-read files already recorded in file_state.
        #[arg(long)]
        force: bool,
    },
    /// Rebuild parsed rows from retained raw hand text.
    Reparse {
        /// Reparse even hands already at the current parser version.
        #[arg(long)]
        force: bool,
    },
    /// Take a database snapshot now and prune old ones.
    Snapshot,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut cfg = Config::from_env()?;
    if let Command::Serve { hands: Some(h) } = &cli.command {
        cfg.hand_history_dir = h.clone();
    }
    init_tracing(cfg.log_json);

    match cli.command {
        Command::Serve { .. } => run_serve(cfg),
        Command::Import { path, force } => {
            let dir = path.unwrap_or_else(|| cfg.hand_history_dir.clone());
            let db = Db::open(&cfg.db_path())?;
            run_import(&db, &dir, force)
        }
        Command::Reparse { force } => {
            let db = Db::open(&cfg.db_path())?;
            let mut last = 0u64;
            let mut progress = |p: &importer::ImportProgress| {
                if p.hands_imported / 1000 > last / 1000 {
                    last = p.hands_imported;
                    eprintln!(
                        "reparse: {} hands, {} failures",
                        p.hands_imported, p.failures
                    );
                }
            };
            let outcome = importer::reparse(&db, force, &mut progress)?;
            println!(
                "reparse complete: {} hands, {} failures in {} ms",
                outcome.hands_imported, outcome.failures, outcome.duration_ms
            );
            Ok(())
        }
        Command::Snapshot => {
            let db = Db::open(&cfg.db_path())?;
            let path = snapshot::take_snapshot(&db, &cfg.snapshot_dir)?;
            snapshot::prune(&cfg.snapshot_dir, cfg.snapshot_retention)?;
            println!("snapshot written to {}", path.display());
            Ok(())
        }
    }
}

fn run_import(db: &Db, dir: &std::path::Path, force: bool) -> anyhow::Result<()> {
    let opts = importer::ImportOptions { force, quiet: false };
    let start = std::time::Instant::now();
    let mut last_report = std::time::Instant::now();
    let mut progress = |p: &importer::ImportProgress| {
        if last_report.elapsed().as_secs() >= 2 {
            last_report = std::time::Instant::now();
            eprintln!(
                "import: {}/{} files, {} hands, {} duplicates, {} failures",
                p.files_done, p.files_total, p.hands_imported, p.duplicates, p.failures
            );
        }
    };
    let outcome = importer::import_directory(db, dir, &opts, &mut progress)?;
    println!(
        "import complete: {} files, {} hands, {} duplicates, {} quarantined, {} failures in {} ms",
        outcome.files_seen,
        outcome.hands_imported,
        outcome.duplicates,
        outcome.quarantined,
        outcome.failures,
        start.elapsed().as_millis()
    );
    Ok(())
}

fn run_serve(cfg: Config) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let bind = cfg.bind_addr;
        let db = Db::open(&cfg.db_path())?;
        let (tx, rx) = tokio::sync::watch::channel(false);

        let watcher = tokio::spawn(importer::run_watcher(db.clone(), cfg.clone(), rx.clone()));
        let snapshots = tokio::spawn(snapshot::snapshot_loop(
            db.clone(),
            cfg.snapshot_dir.clone(),
            cfg.snapshot_interval_secs,
            cfg.snapshot_retention,
            rx.clone(),
        ));

        let http = tokio::spawn(web::serve(db.clone(), cfg.clone(), rx.clone()));
        tracing::info!(%bind, "texture serving");

        let ctrl_c = tokio::signal::ctrl_c();
        tokio::select! {
            _ = ctrl_c => tracing::info!("shutdown signal received"),
            r = watcher => { if let Err(e) = r { tracing::error!(error=%e, "watcher task failed") } }
            r = http => { if let Err(e) = r { tracing::error!(error=%e, "http task failed") } }
        }
        let _ = tx.send(true);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), snapshots).await;
        Ok::<_, anyhow::Error>(())
    })
}

fn init_tracing(json: bool) {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("texture=info,tower_http=info,warn"));
    if json {
        tracing_subscriber::fmt().json().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).compact().init();
    }
}

// Keep `Arc` in scope for future state wiring without an unused-import warning.
#[allow(dead_code)]
fn _unused(_: Arc<Config>) {}
