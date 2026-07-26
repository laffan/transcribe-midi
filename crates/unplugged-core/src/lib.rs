//! Platform-agnostic core for Unplugged.
//!
//! Everything here is pure Rust with no Tauri and no platform dependency, so it builds
//! and its tests run on any host — including the Apple targets we cannot link on CI.
//! The Tauri layer in `src-tauri` is a thin wrapper over this crate; logic belongs here.

pub mod error;
pub mod model;
pub mod smf;
pub mod store;

pub use error::{CoreError, Result};
pub use model::{
    color_for_index, InstrumentRef, Note, Project, ProjectListError, ProjectListing,
    ProjectManifest, ProjectSummary, Ticks, TimeSignature, Track, TrackMeta, DEFAULT_PPQ,
    DEFAULT_TEMPO, MAX_TEMPO, MIN_TEMPO, SCHEMA_VERSION,
};
pub use store::ProjectStore;
