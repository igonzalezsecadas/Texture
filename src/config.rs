//! Deployment configuration comes from environment variables. User-level
//! preferences (active hero profile, session idle gap, currency confirmation)
//! live in the settings table and are read from storage, not from here.

use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    /// Directory containing PokerStars hand-history files. Mounted read-only in
    /// the container.
    pub hand_history_dir: PathBuf,
    /// Directory holding the SQLite database. Mounted read-write from the host.
    pub data_dir: PathBuf,
    /// Directory snapshots are written to. Defaults to `<data_dir>/snapshots`.
    pub snapshot_dir: PathBuf,
    pub bind_addr: SocketAddr,
    /// Cron-ish interval in seconds between automatic snapshots. 0 disables.
    pub snapshot_interval_secs: u64,
    /// How many snapshots to keep.
    pub snapshot_retention: usize,
    /// Optional override for the hero screen name, bypassing folder detection.
    pub hero_name: Option<String>,
    /// Filesystem poll fallback interval for the watcher, in seconds.
    pub scan_interval_secs: u64,
    /// Set when logging should be emitted as JSON.
    pub log_json: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Config> {
        let data_dir = env_path("TEXTURE_DATA_DIR", "./data");
        let hand_history_dir = env_path("TEXTURE_HAND_HISTORY_DIR", "./hands");
        let snapshot_dir = std::env::var_os("TEXTURE_SNAPSHOT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("snapshots"));
        let bind_addr = std::env::var("TEXTURE_BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid TEXTURE_BIND_ADDR: {e}"))?;
        Ok(Config {
            hand_history_dir,
            data_dir,
            snapshot_dir,
            bind_addr,
            snapshot_interval_secs: env_u64("TEXTURE_SNAPSHOT_INTERVAL_SECS", 6 * 60 * 60),
            snapshot_retention: env_u64("TEXTURE_SNAPSHOT_RETENTION", 14) as usize,
            hero_name: std::env::var("TEXTURE_HERO_NAME").ok().filter(|s| !s.is_empty()),
            scan_interval_secs: env_u64("TEXTURE_SCAN_INTERVAL_SECS", 30),
            log_json: std::env::var("TEXTURE_LOG_JSON").map(|v| v == "1" || v == "true").unwrap_or(false),
        })
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("texture.sqlite")
    }
}

fn env_path(key: &str, default: &str) -> PathBuf {
    std::env::var_os(key).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(default))
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
