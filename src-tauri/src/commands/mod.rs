use crate::ffmpeg::StreamFormat;
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn start_recording(
    state: State<AppState>,
    stream_url: String,
    output_path: String,
    format_id: Option<String>,
) -> Result<String, String> {
    state
        .recorder
        .start(&stream_url, &output_path, format_id.as_deref())
}

#[tauri::command]
pub fn fetch_formats(
    state: State<AppState>,
    stream_url: String,
) -> Result<Vec<StreamFormat>, String> {
    state.recorder.fetch_formats(&stream_url)
}

#[tauri::command]
pub fn stop_recording(state: State<AppState>) -> Result<String, String> {
    state
        .recorder
        .stop()
        .map(|_| "Grabación detenida".to_string())
}

#[tauri::command]
pub fn get_recording_status(state: State<AppState>) -> Result<bool, String> {
    Ok(state.recorder.is_recording())
}

#[tauri::command]
pub fn get_recording_error(state: State<AppState>) -> Result<Option<String>, String> {
    Ok(state.recorder.get_error())
}

#[tauri::command]
pub fn get_logs(state: State<AppState>) -> Result<String, String> {
    Ok(state.recorder.get_log())
}
