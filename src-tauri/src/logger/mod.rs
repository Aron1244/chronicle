use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone)]
pub struct Logger {
    file: std::sync::Arc<Mutex<PathBuf>>,
}

impl Logger {
    pub fn new() -> Self {
        let path = dirs()
            .next()
            .unwrap_or_else(|| PathBuf::from("chronicle.log"));
        Self {
            file: std::sync::Arc::new(Mutex::new(path)),
        }
    }

    pub fn log(&self, msg: &str) {
        let path = self.file.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "[{}] {}", chrono_now(), msg);
        }
    }

    pub fn read_all(&self) -> String {
        let path = self.file.lock().unwrap_or_else(|e| e.into_inner()).clone();
        fs::read_to_string(&path).unwrap_or_default()
    }

    pub fn path(&self) -> PathBuf {
        self.file.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn dirs() -> impl Iterator<Item = PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            v.push(parent.join("chronicle.log"));
        }
    }
    v.push(PathBuf::from("chronicle.log"));
    v.into_iter()
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}
