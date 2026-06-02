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
            commands::get_recording_status,
            commands::get_recording_error,
            commands::get_logs,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let state = window.state::<AppState>();
                let _ = state.recorder.stop();
                Recorder::kill_all();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
