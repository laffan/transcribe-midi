//! Conversion between the in-memory note list and Standard MIDI Files.
//!
//! Per-track storage is SMF **format 0** (one track chunk) carrying only note events and
//! a track-name meta event. Tempo and time signature deliberately live in `project.json`
//! and are not duplicated there — see DECISIONS.md. Format 1, with a conductor track, is
//! what interchange (export and drag-out) uses.
//!
//! Reading and writing are separate files because they fail differently and are tested
//! differently: the writer's job is to emit bytes another program will accept, and the
//! reader's is to survive bytes another program emitted.
//!
//! | Module | Holds |
//! |---|---|
//! | [`write`] | note list → bytes: format 0 storage, format 1 export, standalone track |
//! | [`read`] | bytes → note list, for the single-track storage case |
//! | [`import`] | bytes → many tracks, for a file the user dragged in |
//! | [`tempo`] | BPM ↔ SMF's microseconds-per-quarter, in both directions |

mod import;
mod read;
mod tempo;
mod write;

#[cfg(test)]
mod tests;

pub use import::{parse_smf_for_import, rescale_ppq, ImportedFile, ImportedTrack};
pub use read::smf_bytes_to_notes;
pub use tempo::{bpm_to_micros_per_quarter, micros_per_quarter_to_bpm};
pub use write::{notes_to_smf_bytes, project_to_smf_type1, track_to_standalone_smf};
