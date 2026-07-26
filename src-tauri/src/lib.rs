mod commands;
mod editor;
mod error;
mod state;

use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager};
use unplugged_audio::AudioEngine;

use state::AppState;

/// Playhead updates per second.
///
/// The frontend never drives timing — it only observes this. 30 Hz is smooth enough for
/// a moving playhead and cheap enough not to flood the IPC channel; the audio thread
/// publishes position continuously regardless of whether anyone is listening.
const PLAYHEAD_HZ: u64 = 30;

#[derive(Clone, Serialize)]
struct PlayheadEvent {
    position_ticks: u32,
    playing: bool,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            // A failure here must not stop the app from launching: the project picker
            // and the editor are still useful without sound, and reporting it in the
            // console beats refusing to start.
            let audio = match AudioEngine::new() {
                Ok(engine) => engine,
                Err(error) => {
                    eprintln!("audio engine unavailable: {error}");
                    AudioEngine::with_backend(Box::new(unplugged_audio::NullBackend::new()))
                }
            };

            app.manage(AppState::new(data_dir, audio));

            // Publish the playhead. Runs for the life of the app and only emits while
            // something is moving, so an idle app costs nothing.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let interval = Duration::from_millis(1000 / PLAYHEAD_HZ);
                let mut last_emitted: Option<PlayheadEvent> = None;

                loop {
                    std::thread::sleep(interval);

                    let Some(state) = handle.try_state::<AppState>() else {
                        continue;
                    };
                    let event = PlayheadEvent {
                        position_ticks: state.audio.position_ticks(),
                        playing: state.audio.is_playing(),
                    };

                    // Skip identical frames so a stopped transport is silent on the wire.
                    let changed = match &last_emitted {
                        Some(previous) => {
                            previous.position_ticks != event.position_ticks
                                || previous.playing != event.playing
                        }
                        None => true,
                    };
                    if !changed {
                        continue;
                    }

                    let _ = handle.emit("playhead", event.clone());
                    last_emitted = Some(event);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Projects
            commands::list_projects,
            commands::create_project,
            commands::load_project,
            commands::save_project,
            commands::rename_project,
            commands::delete_project,
            commands::add_track,
            commands::delete_track,
            commands::projects_root,
            // Editor
            editor::open_project,
            editor::close_project,
            editor::save_open_project,
            editor::editor_state,
            editor::apply_edit,
            editor::undo,
            editor::redo,
            // Transport
            editor::transport_play,
            editor::transport_stop,
            editor::transport_seek,
            editor::transport_get,
            editor::set_tempo,
            editor::set_loop_region,
            // Live input
            editor::live_note_on,
            editor::live_note_off,
            editor::panic_all_notes_off,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Unplugged");
}
