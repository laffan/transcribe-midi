//! Listing and opening projects the standalone app wrote.

use super::*;

#[test]
fn it_lists_and_opens_projects_the_app_wrote() {
    let dir = temp_dir("list");
    let id = seeded(&dir, "Plugin Test");

    let mut plugin = Plugin::new(dir.clone());
    let json = plugin.projects_json();
    assert!(json.contains("Plugin Test"), "{json}");

    plugin.open(&id).unwrap();
    assert_eq!(plugin.track_count(), 1);
    assert_eq!(plugin.state().project_id.as_deref(), Some(id.as_str()));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn opening_something_that_is_not_there_fails_without_panicking() {
    let dir = temp_dir("missing");
    let mut plugin = Plugin::new(dir.clone());
    assert!(plugin.open("no-such-project").is_err());
    assert_eq!(plugin.projects_json(), "[]");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_session_referencing_a_deleted_project_still_loads() {
    // The host is restoring a session. Refusing would lose everything else about it,
    // so a missing project must degrade to "nothing open" plus an explanation.
    let dir = temp_dir("stale");
    let mut plugin = Plugin::new(dir.clone());

    plugin.set_state(PluginState { project_id: Some("gone".into()) });
    assert_eq!(plugin.state().project_id, None);
    assert!(plugin.take_last_error().is_some_and(|e| e.contains("gone")));

    std::fs::remove_dir_all(&dir).ok();
}

