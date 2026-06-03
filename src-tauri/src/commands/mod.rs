use crate::ffmpeg::{CompressionConfig, SlotStatus, StreamFormat, MAX_SLOTS};
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn start_recording(
    state: State<AppState>,
    slot: usize,
    stream_url: String,
    output_path: String,
    format_id: Option<String>,
    format_url: Option<String>,
    compress: Option<CompressionConfig>,
) -> Result<String, String> {
    if slot >= MAX_SLOTS {
        return Err(format!("Slot inválido (0-{})", MAX_SLOTS - 1));
    }
    state
        .recorder
        .start(slot, &stream_url, &output_path, format_id.as_deref(), format_url.as_deref(), compress)
}

#[tauri::command]
pub fn fetch_formats(
    state: State<AppState>,
    stream_url: String,
) -> Result<Vec<StreamFormat>, String> {
    state.recorder.fetch_formats(&stream_url)
}

#[tauri::command]
pub fn stop_recording(state: State<AppState>, slot: usize) -> Result<String, String> {
    if slot >= MAX_SLOTS {
        return Err(format!("Slot inválido (0-{})", MAX_SLOTS - 1));
    }
    state
        .recorder
        .stop(slot)
        .map(|_| format!("Slot {} detenido", slot))
}

#[tauri::command]
pub fn stop_all_recordings(state: State<AppState>) -> Result<Vec<String>, String> {
    Ok(state.recorder.stop_all())
}

#[tauri::command]
pub fn get_all_statuses(state: State<AppState>) -> Result<Vec<SlotStatus>, String> {
    Ok(state.recorder.get_all_statuses())
}

#[tauri::command]
pub fn get_recording_status(state: State<AppState>, slot: usize) -> Result<bool, String> {
    Ok(state.recorder.is_recording(slot))
}

#[tauri::command]
pub fn get_recording_error(state: State<AppState>, slot: usize) -> Result<Option<String>, String> {
    Ok(state.recorder.get_error(slot))
}

#[tauri::command]
pub fn get_logs(state: State<AppState>) -> Result<String, String> {
    Ok(state.recorder.get_logs())
}

#[tauri::command]
pub fn preview_stream(url: String, state: State<AppState>) -> Result<String, String> {
    state.recorder.preview_stream(&url)
}
