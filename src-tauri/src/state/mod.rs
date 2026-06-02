use crate::ffmpeg::Recorder;
use crate::logger::Logger;

pub struct AppState {
    pub recorder: Recorder,
}

impl AppState {
    pub fn new() -> Self {
        let logger = Logger::new();
        logger.log("=== Chronicle iniciado ===");
        Self {
            recorder: Recorder::new(logger),
        }
    }
}
