//! Database snapshots. `VACUUM INTO` produces a consistent, compact copy while
//! the application keeps running. Retention prunes the oldest snapshots.

use crate::storage::Db;
use anyhow::Context;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Take a snapshot into `dir` and record it. The filename is
/// `texture-YYYYmmdd-HHMMSS.sqlite`.
pub fn take_snapshot(db: &Db, dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating snapshot dir {dir:?}"))?;
    let name = format!("texture-{}.sqlite", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let path = dir.join(name);
    db.with_writer(|c| {
        c.execute("VACUUM INTO ?1", [path.to_string_lossy().to_string()])
    })
    .context("VACUUM INTO failed")?;
    let size = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
    db.with_writer(|c| {
        c.execute(
            "INSERT INTO snapshots(path, created_at, size_bytes) VALUES (?1, ?2, ?3)",
            rusqlite::params![path.to_string_lossy(), chrono::Utc::now().timestamp(), size],
        )
    })
    .ok();
    Ok(path)
}

/// Keep the newest `retention` snapshots in `dir`, deleting the rest.
pub fn prune(dir: &Path, retention: usize) -> anyhow::Result<usize> {
    let mut snaps: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("texture-") && n.ends_with(".sqlite"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    snaps.sort();
    let mut removed = 0;
    while snaps.len() > retention {
        let victim = snaps.remove(0);
        if std::fs::remove_file(&victim).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Periodically snapshot until shutdown is signalled.
pub async fn snapshot_loop(
    db: Db,
    dir: PathBuf,
    interval_secs: u64,
    retention: usize,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    if interval_secs == 0 {
        return;
    }
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // The first tick fires immediately; snapshots on a fresh process are cheap
    // and give the operator a known-good copy early.
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let db = db.clone();
                let dir = dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let p = take_snapshot(&db, &dir)?;
                    prune(&dir, retention)?;
                    Ok::<_, anyhow::Error>(p)
                }).await;
                match result {
                    Ok(Ok(p)) => tracing::info!(snapshot = %p.display(), "snapshot taken"),
                    Ok(Err(e)) => tracing::error!(error = %e, "snapshot failed"),
                    Err(e) => tracing::error!(error = %e, "snapshot task panicked"),
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("snapshot loop stopping");
                    return;
                }
            }
        }
    }
}
