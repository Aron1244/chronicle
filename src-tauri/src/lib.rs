mod commands;
mod ffmpeg;
mod logger;
mod state;

use ffmpeg::Recorder;
use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::start_recording,
            commands::fetch_formats,
            commands::stop_recording,
            commands::stop_all_recordings,
            commands::get_all_statuses,
            commands::get_recording_status,
            commands::get_recording_error,
            commands::get_logs,
            commands::preview_stream,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let state = window.state::<AppState>();
                state.recorder.stop_all();
                Recorder::kill_all();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
