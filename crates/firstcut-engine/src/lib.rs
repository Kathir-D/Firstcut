//! The session: the scan → batch → rate → finish loop and its database, undo log and listeners.
//!
//! Ported verbatim from v1 `firstcut_core::session` (as module `session`) at the M1 port; only
//! imports were re-pointed to the ladder crates. Owner: core-store (docs/contracts/session-api.md).

pub mod keymap;
pub mod model;
pub mod pipeline;
pub mod registry;
pub mod session;
pub mod settings;
pub mod watch;
