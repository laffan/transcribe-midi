//! Tests for the plugin core and its boundary.
//!
//! The interesting ones drive `Plugin` directly; `abi` covers the pointer handling that
//! only the C entry points do.

mod abi;
mod projects;
mod render;
mod state;

use super::*;
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use unplugged_core::host_sync::HostTransport;
use unplugged_core::{Note, ProjectStore, TimeSignature};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("unplugged-plugin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A project on disk with one note per bar, as the app would have written it.
fn seeded(dir: &Path, name: &str) -> String {
    let store = ProjectStore::new(dir.join("projects"));
    let manifest = store
        .create(name, 120.0, TimeSignature::new(4, 4).unwrap())
        .unwrap();

    let mut project = store.load(&manifest.id).unwrap();
    project.tracks[0].notes = vec![
        Note::new(60, 0, 480, 100, 0).unwrap(),
        Note::new(64, 1920, 480, 100, 0).unwrap(),
    ];
    store.save(&mut project).unwrap();
    manifest.id
}

fn blank() -> Vec<CRenderedEvent> {
    vec![
        CRenderedEvent {
            frame_offset: 0,
            track: 0,
            kind: 0,
            pitch: 0,
            velocity: 0,
            channel: 0,
            _pad: [0; 2],
        };
        64
    ]
}
