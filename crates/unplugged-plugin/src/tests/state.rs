//! What the host persists in its session, and what a stale blob does.

use super::*;

#[test]
fn state_round_trips_through_json() {
    let dir = temp_dir("state");
    let id = seeded(&dir, "Round Trip");

    let mut plugin = Plugin::new(dir.clone());
    plugin.open(&id).unwrap();

    let json = serde_json::to_string(&plugin.state()).unwrap();
    let mut restored = Plugin::new(dir.clone());
    restored.set_state(serde_json::from_str(&json).unwrap());

    assert_eq!(restored.state().project_id.as_deref(), Some(id.as_str()));
    assert_eq!(restored.track_count(), 1);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_blob_from_a_newer_build_means_no_project_rather_than_a_refusal() {
    let dir = temp_dir("future");
    let mut plugin = Plugin::new(dir.clone());
    let state: PluginState = serde_json::from_str(r#"{"unknown_field":42}"#).unwrap_or_default();
    plugin.set_state(state);
    assert_eq!(plugin.state().project_id, None);
    std::fs::remove_dir_all(&dir).ok();
}

