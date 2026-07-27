//! Reading a file somebody else wrote.

use super::*;

#[test]
fn a_format_0_file_imports_as_one_track() {
    let bytes = notes_to_smf_bytes(&[note(60, 0, 480, 100, 0)], DEFAULT_PPQ, "Single").unwrap();
    let imported = parse_smf_for_import(&bytes, &p()).unwrap();

    assert_eq!(imported.tracks.len(), 1);
    assert_eq!(imported.tracks[0].name.as_deref(), Some("Single"));
    assert_eq!(imported.note_count(), 1);
    // No tempo meta in a bare per-track file; the caller keeps the project tempo.
    assert_eq!(imported.tempo_bpm, None);
}

#[test]
fn import_rejects_garbage_rather_than_producing_an_empty_project() {
    assert!(parse_smf_for_import(b"not a midi file", &p()).is_err());
    assert!(parse_smf_for_import(&[], &p()).is_err());
}

#[test]
fn a_track_with_no_name_imports_without_one() {
    let bytes = notes_to_smf_bytes(&[note(60, 0, 480, 100, 0)], DEFAULT_PPQ, "").unwrap();
    let imported = parse_smf_for_import(&bytes, &p()).unwrap();
    assert_eq!(imported.tracks[0].name, None, "an empty name must not become Some(\"\")");
}
