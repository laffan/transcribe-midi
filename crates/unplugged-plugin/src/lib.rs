//! What the AUv3 extension links against.
//!
//! **Scope of this first pass.** The plugin plays a project the standalone app authored,
//! following the host's transport, and emits its notes as MIDI. It does not yet host the
//! editor UI — that needs the whole Tauri command surface re-homed behind a transport
//! that is not Tauri, and doing it in the same step as getting an extension to load at all
//! would mean two unverified things failing together with no way to tell which.
//!
//! So: the projects directory is shared with the app. You author in the app; the plugin
//! plays what you authored, into Logic's instrument, in sync with Logic's transport.
//!
//! Two rules the whole file is shaped by:
//!
//! * **The render path allocates nothing and locks nothing.** It is called on the audio
//!   thread by the host. Everything it touches is preallocated by `prepare`.
//! * **No panic crosses the boundary.** A panic unwinding into Swift is undefined
//!   behaviour, and in a plugin it takes the host down with it — which means it takes
//!   the user's unsaved session down too. Every entry point catches.

//!
//! | Module | Holds |
//! |---|---|
//! | [`event`] | the `#[repr(C)]` event struct the Swift header mirrors |
//! | [`plugin`] | the state, the project list, and the render path — ordinary Rust |
//! | [`abi`] | the `extern "C"` entry points, each one a panic-catching wrapper |
//!
//! The split is what makes the second rule checkable: raw pointers appear in exactly one
//! file, so "does every entry point catch?" is a question you can answer by reading it.

mod abi;
mod event;
mod plugin;

#[cfg(test)]
mod tests;

pub use event::CRenderedEvent;
pub use plugin::{Plugin, PluginState};

/// Events one block may produce. Beyond this the block is truncated rather than
/// allocating on the audio thread.
const MAX_EVENTS: usize = 512;
