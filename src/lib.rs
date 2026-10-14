//! texture — a self-hosted PokerStars NLHE cash-game tracker.
//!
//! The crate is organized around three seams:
//!
//! 1. **Parser boundary** — raw hand text in, [`domain::ParsedHand`] out.
//! 2. **Storage boundary** — [`domain::ParsedHand`] in, queried aggregate rows out.
//! 3. **Reporting boundary** — a [`domain::FilterSpec`] plus stored hands in,
//!    aggregate statistics out.
//!
//! Fewer seams is better; everything is exercised through these.

pub mod config;
pub mod domain;
pub mod error;
pub mod ev;
pub mod importer;
pub mod money;
pub mod parser;
pub mod snapshot;
pub mod stats;
pub mod storage;
pub mod web;

/// The version string reported by the healthcheck and stored in `meta`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
