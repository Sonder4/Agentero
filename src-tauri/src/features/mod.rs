//! Domain features (feature-first layout, aligned with frontend `src/lib`).
//!
//! Tauri-free service bodies live in `agentero_core::features::*` and are
//! bridged here; callers use the semantic `crate::features::<domain>::<module>`
//! paths. This crate keeps the desktop shells: `#[tauri::command]` surfaces,
//! JobCenter runners, watcher/agent/integration wiring. The headless CLI
//! consumes `agentero_core` directly; BYOA (`agent`) is desktop-only.

#[cfg(feature = "desktop")]
pub mod agent;
#[cfg(feature = "desktop")]
#[path = "agent/install/mod.rs"]
pub mod cli_install;
#[cfg(feature = "desktop")]
pub mod compile;
#[cfg(feature = "desktop")]
pub mod host_hooks;
#[cfg(feature = "desktop")]
pub mod jobs;

pub mod lifecycle;

#[path = "../app/open_request/mod.rs"]
pub mod open_request;

pub mod markdown;
pub mod paper;
pub mod pdf;
pub mod system;
pub mod translate;
pub mod vault;
#[cfg(feature = "desktop")]
pub mod web;
#[cfg(feature = "desktop")]
pub mod web_ai;
