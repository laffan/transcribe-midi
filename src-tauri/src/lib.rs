mod ai;
mod audition;
mod commands;
mod editor;
mod error;
mod input;
mod interchange;
mod platform;
mod shared_container;
mod state;
mod transcribe;

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
    in_count_in: bool,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Somewhere the AUv3 extension can read too. The App Group when the build is
            // signed for one, a fixed path under the home directory otherwise — the
            // plugin reaches that through a sandbox exception — and the app's own
            // directory only if neither is available, which leaves the plugin with
            // nothing to play.
            let location = shared_container::resolve(app.path().app_data_dir()?);
            let data_dir = location.dir.clone();
            std::fs::create_dir_all(&data_dir)?;

            if let Some(from) = &location.migrated_from {
                eprintln!(
                    "projects copied to {} from {}; the originals were left in place",
                    data_dir.display(),
                    from.display()
                );
            }
            match location.sharing {
                shared_container::Sharing::AppGroup => {}
                shared_container::Sharing::HomeDirectory => eprintln!(
                    "no App Group — sharing {} with the plugin instead; this works for an \
                     ad-hoc build but not for a sandboxed one",
                    data_dir.display()
                ),
                shared_container::Sharing::Private => eprintln!(
                    "no shared directory available — the AUv3 plugin will not see these \
                     projects"
                ),
            }

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
                let mut last_wrap_count: u32 = 0;

                loop {
                    std::thread::sleep(interval);

                    let Some(state) = handle.try_state::<AppState>() else {
                        continue;
                    };
                    // The audio thread cannot call into the recorder, so loop wraps are
                    // published as a counter and picked up here. Missing one would leave
                    // a held note running past the loop point in the recorded take.
                    let wraps = state.audio.wrap_count();
                    if wraps != last_wrap_count {
                        if let Some((start, end)) = state.audio.loop_region() {
                            input::on_loop_wrap(&state, end, start);
                        }
                        last_wrap_count = wraps;
                    }

                    let event = PlayheadEvent {
                        position_ticks: state.audio.position_ticks(),
                        playing: state.audio.is_playing(),
                        in_count_in: state.audio.in_count_in(),
                    };

                    // Skip identical frames so a stopped transport is silent on the wire.
                    let changed = match &last_emitted {
                        Some(previous) => {
                            previous.position_ticks != event.position_ticks
                                || previous.playing != event.playing
                                || previous.in_count_in != event.in_count_in
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
            commands::build_info,
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
            // Live input and recording (phase 4)
            input::live_note_on,
            input::live_note_off,
            input::panic_all_notes_off,
            input::input_settings,
            input::midi_ports,
            input::midi_connect,
            input::midi_disconnect,
            input::midi_set_channel,
            input::set_armed_track,
            input::set_keyboard_velocity,
            input::set_count_in_bars,
            input::set_metronome,
            input::record_start,
            input::record_stop,
            input::record_cancel,
            // Interchange (phase 5)
            interchange::export_project_smf,
            interchange::export_track_smf,
            interchange::write_export,
            interchange::stage_export,
            interchange::preview_import,
            interchange::import_smf,
            platform::copy_file_to_pasteboard,
            platform::share_file,
            platform::begin_file_drag,
            platform::platform_capabilities,
            // AI (phase 6)
            ai::ai_status,
            ai::ai_set_key,
            ai::ai_clear_key,
            ai::ai_models,
            ai::ai_set_model,
            ai::ai_propose,
            ai::ai_accept,
            ai::ai_reject,
            // Audio to MIDI (phase 7)
            transcribe::capture_start,
            transcribe::capture_poll,
            transcribe::capture_transcribe,
            transcribe::capture_retranscribe,
            transcribe::capture_load_file,
            transcribe::capture_waveform,
            transcribe::capture_progress,
            transcribe::capture_set_notes,
            transcribe::capture_accept,
            transcribe::capture_cancel,
            audition::capture_audition_play,
            audition::capture_audition_stop,
            audition::capture_audition_position,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Unplugged");
}
