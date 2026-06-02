use crate::logger::Logger;
use serde::Serialize;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize)]
pub struct StreamFormat {
    pub id: String,
    pub ext: String,
    pub resolution: String,
    pub fps: Option<f64>,
    pub bitrate: Option<f64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
}

#[derive(Debug)]
enum RecorderBackend {
    Ytdlp,
    Streamlink,
    Ffmpeg,
}

pub struct Recorder {
    child: Mutex<Option<Child>>,
    error: Arc<Mutex<Option<String>>>,
    ytdlp_path: PathBuf,
    streamlink_path: PathBuf,
    ffmpeg_path: PathBuf,
    plugin_dir: Option<PathBuf>,
    logger: Logger,
}

impl Recorder {
    pub fn new(logger: Logger) -> Self {
        let yt = find_binary("yt-dlp.exe", "yt-dlp", &logger);
        let sl = find_binary("streamlink.exe", "streamlink", &logger);
        let ff = find_binary("ffmpeg.exe", "ffmpeg", &logger);
        let plugin_dir = find_plugin_dir(&logger);
        logger.log(&format!("yt-dlp path: {}", yt.display()));
        logger.log(&format!("Streamlink path: {}", sl.display()));
        logger.log(&format!("FFmpeg path: {}", ff.display()));
        if let Some(ref pd) = plugin_dir {
            logger.log(&format!("Plugin dir: {}", pd.display()));
        }
        Self {
            child: Mutex::new(None),
            error: Arc::new(Mutex::new(None)),
            ytdlp_path: yt,
            streamlink_path: sl,
            ffmpeg_path: ff,
            plugin_dir,
            logger,
        }
    }

    fn detect_backend(&self, url: &str) -> RecorderBackend {
        let is_direct = url.ends_with(".m3u8")
            || url.ends_with(".mp4")
            || url.ends_with(".ts")
            || url.starts_with("rtmp://")
            || url.starts_with("rtsp://");
        if is_direct {
            RecorderBackend::Ffmpeg
        } else if self.ytdlp_path.is_file() || self.ytdlp_path.to_string_lossy() == "yt-dlp" {
            RecorderBackend::Ytdlp
        } else {
            RecorderBackend::Streamlink
        }
    }

    pub fn fetch_formats(&self, url: &str) -> Result<Vec<StreamFormat>, String> {
        let url = normalize_url(url);
        self.logger.log(&format!("Obteniendo formatos: {}", url));

        let exe = &self.ytdlp_path;
        let mut args: Vec<String> = vec!["-J".into(), "--no-part".into()];
        if let Some(ref pd) = self.plugin_dir {
            args.push("--plugin-dirs".into());
            args.push(pd.to_string_lossy().to_string());
        }
        args.push(url.into());

        self.logger.log(&format!(
            "Ejecutando: {} {}",
            exe.display(),
            args.join(" "),
        ));

        let output = Command::new(exe)
            .args(&args)
            .output()
            .map_err(|e| format!("Error al ejecutar yt-dlp: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            self.logger.log(&format!("yt-dlp error: {}", stderr));
            return Err(format!("yt-dlp falló: {}", extract_error("", &stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let data: serde_json::Value =
            serde_json::from_str(&stdout).map_err(|e| format!("Error al parsear JSON: {}", e))?;

        let formats_val = data.get("formats").ok_or("No se encontraron formatos")?;
        let formats: Vec<StreamFormat> = formats_val
            .as_array()
            .ok_or("Formatos no es un array")?
            .iter()
            .map(|f| {
                let id = f.get("format_id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                let ext = f.get("ext").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                let resolution = f
                    .get("resolution")
                    .and_then(|v| v.as_str())
                    .unwrap_or("audio")
                    .to_string();
                let fps = f.get("fps").and_then(|v| v.as_f64());
                let bitrate = f.get("tbr").and_then(|v| v.as_f64());
                let video_codec = f.get("vcodec").and_then(|v| v.as_str()).map(|s| s.to_string());
                let audio_codec = f.get("acodec").and_then(|v| v.as_str()).map(|s| s.to_string());

                // Skip storyboard/images formats
                let is_video = resolution != "audio" && video_codec.as_deref() != Some("none");

                StreamFormat {
                    id,
                    ext,
                    resolution: if is_video { resolution } else { String::new() },
                    fps,
                    bitrate,
                    video_codec,
                    audio_codec,
                }
            })
            .filter(|f| !f.resolution.is_empty() || f.audio_codec.is_some())
            .collect();

        self.logger.log(&format!("Formatos encontrados: {}", formats.len()));
        Ok(formats)
    }

    pub fn start(&self, url: &str, output_path: &str, format_id: Option<&str>) -> Result<String, String> {
        let mut child = self.child.lock().map_err(|e| e.to_string())?;
        if child.is_some() {
            return Err("Ya hay una grabación en curso".to_string());
        }

        if let Ok(mut e) = self.error.lock() {
            *e = None;
        }

        let url = normalize_url(url);
        self.logger
            .log(&format!("Iniciando grabación: {} → {}", url, output_path));

        let parent = std::path::Path::new(output_path)
            .parent()
            .ok_or("Ruta inválida")?;
        if let Err(e) = std::fs::create_dir_all(parent) {
            self.logger.log(&format!("Error al crear carpeta: {}", e));
            return Err(format!("Error al crear carpeta: {}", e));
        }

        let backend = self.detect_backend(&url);
        self.logger
            .log(&format!("Backend detectado: {:?}", backend));

        match backend {
            RecorderBackend::Ytdlp => self.run_ytdlp(&url, output_path, format_id, &mut child),
            RecorderBackend::Streamlink => self.run_streamlink(&url, output_path, &mut child),
            RecorderBackend::Ffmpeg => self.run_ffmpeg(&url, output_path, &mut child),
        }
    }

    fn needs_audio_merge(&self, url: &str, format_id: &str) -> bool {
        let exe = &self.ytdlp_path;
        let mut args: Vec<String> = vec!["-J".into(), "--no-part".into()];
        if let Some(ref pd) = self.plugin_dir {
            args.push("--plugin-dirs".into());
            args.push(pd.to_string_lossy().to_string());
        }
        args.push(url.into());
        if let Ok(output) = Command::new(exe).args(&args).output() {
            if let Ok(data) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                let selected = data.get("format_id").and_then(|v| v.as_str());
                if selected == Some(format_id) {
                    return data.get("acodec").and_then(|v| v.as_str()) == Some("none");
                }
                let formats = data.get("formats").and_then(|v| v.as_array());
                if let Some(fmts) = formats {
                    for f in fmts {
                        let fid = f.get("format_id").and_then(|v| v.as_str()).unwrap_or("");
                        if fid == format_id {
                            return f.get("acodec").and_then(|v| v.as_str()) == Some("none");
                        }
                    }
                    let has_audio_only = fmts.iter().any(|f| {
                        f.get("vcodec").and_then(|v| v.as_str()) == Some("none")
                            && f.get("acodec").and_then(|v| v.as_str()).map_or(false, |c| c != "none")
                    });
                    if !has_audio_only {
                        return false;
                    }
                }
            }
        }
        false
    }

    fn run_ytdlp(
        &self,
        url: &str,
        output_path: &str,
        format_id: Option<&str>,
        child: &mut Option<Child>,
    ) -> Result<String, String> {
        let exe = &self.ytdlp_path;
        let mut args: Vec<String> = vec!["--newline".into(), "--no-part".into()];
        if let Some(fid) = format_id {
            let merge = self.needs_audio_merge(url, fid);
            if merge {
                let merged = format!("{}+bestaudio", fid);
                self.logger.log(&format!("Formato sin audio, mergeando: {}", merged));
                args.push("-f".into());
                args.push(merged);
            } else {
                args.push("-f".into());
                args.push(fid.into());
            }
        }
        if let Some(ref pd) = self.plugin_dir {
            args.push("--plugin-dirs".into());
            args.push(pd.to_string_lossy().to_string());
        }
        if let Some(ffmpeg_dir) = self.ffmpeg_path.parent() {
            args.push("--ffmpeg-location".into());
            args.push(ffmpeg_dir.to_string_lossy().to_string());
        }
        args.extend_from_slice(&["-o".into(), output_path.into(), url.into()]);

        self.logger.log(&format!(
            "Ejecutando: {} {}",
            exe.display(),
            args.join(" "),
        ));

        let mut process = match Command::new(exe)
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(p) => p,
            Err(e) => {
                let msg = format!(
                    "No se encontró yt-dlp en {}. Descargalo de https://github.com/yt-dlp/yt-dlp",
                    exe.display()
                );
                self.logger.log(&format!("{} - {}", msg, e));
                return Err(msg);
            }
        };

        // Wait briefly to detect startup errors (live streams keep running)
        std::thread::sleep(std::time::Duration::from_secs(3));
        match process.try_wait() {
            Ok(Some(_)) => {
                // Process exited early — read full error output
                let mut stderr_buf = String::new();
                let _ = process.stderr.take().map(|mut s| s.read_to_string(&mut stderr_buf));
                self.logger.log(&format!("yt-dlp stderr: {}", stderr_buf));
                let msg = extract_error("", &stderr_buf);
                if msg.contains("Unsupported URL") {
                    return Err("URL no soportada por yt-dlp. Probá con Streamlink o usá una URL directa .m3u8".to_string());
                }
                return Err(format!("yt-dlp falló: {}", msg));
            }
            Ok(None) => {
                self.logger.log("yt-dlp sigue ejecutándose OK");
            }
            Err(e) => {
                self.logger.log(&format!("yt-dlp error: {}", e));
            }
        }

        // Stderr monitoring: spawn thread to detect errors during recording
        let err_child = process.stderr.take();
        let logger = self.logger.clone();
        let error_state = self.error.clone();
        let child_for_monitor = process.try_wait().ok().flatten();
        if err_child.is_some() && child_for_monitor.is_none() {
            std::thread::spawn(move || {
                let reader = BufReader::new(err_child.unwrap());
                for line in reader.lines() {
                    match line {
                        Ok(l) => {
                            logger.log(&format!("[yt-dlp] {}", l));
                            if l.contains("ERROR") || l.contains("ffmpeg exited with code") || l.contains("Error opening input") {
                                let err_msg = l.trim().to_string();
                                logger.log(&format!("⚠ Error detectado: {}", err_msg));
                                if let Ok(mut e) = error_state.lock() {
                                    *e = Some(err_msg);
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                logger.log("Monitor de yt-dlp finalizado");
            });
        }

        *child = Some(process);
        Ok("Grabando con yt-dlp".to_string())
    }

    fn run_streamlink(
        &self,
        url: &str,
        output_path: &str,
        child: &mut Option<Child>,
    ) -> Result<String, String> {
        let exe = &self.streamlink_path;
        self.logger.log(&format!(
            "Ejecutando: {} {} best -o {}",
            exe.display(),
            url,
            output_path
        ));
        let mut process = match Command::new(exe)
            .args([url, "best", "-o", output_path])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(p) => p,
            Err(e) => {
                let msg = format!(
                    "No se encontró Streamlink en {}. Descargalo de https://streamlink.github.io",
                    exe.display()
                );
                self.logger.log(&format!("{} - {}", msg, e));
                return Err(msg);
            }
        };

        let mut stdout_buf = String::new();
        let _ = process.stdout.take().map(|mut s| s.read_to_string(&mut stdout_buf));
        let mut stderr_buf = String::new();
        let _ = process.stderr.take().map(|mut s| s.read_to_string(&mut stderr_buf));
        self.logger.log(&format!("Streamlink stdout: {}", stdout_buf));
        self.logger.log(&format!("Streamlink stderr: {}", stderr_buf));

        if !status_success(&mut process) {
            let msg = extract_error(&stdout_buf, &stderr_buf);
            if msg.contains("No plugin can handle") && self.ytdlp_path.is_file() {
                self.logger
                    .log("Streamlink no soporta la URL, probando con yt-dlp...");
                return self.run_ytdlp(url, output_path, None, child);
            }
            // Also try yt-dlp as fallback for any error
            if self.ytdlp_path.is_file() {
                self.logger
                    .log("Streamlink falló, probando con yt-dlp...");
                return self.run_ytdlp(url, output_path, None, child);
            }
            return Err(format!("Streamlink falló: {}", msg));
        }

        *child = Some(process);
        Ok("Grabando con Streamlink".to_string())
    }

    fn run_ffmpeg(
        &self,
        url: &str,
        output_path: &str,
        child: &mut Option<Child>,
    ) -> Result<String, String> {
        let exe = &self.ffmpeg_path;
        self.logger.log(&format!(
            "Ejecutando: {} -i {} -c copy -f mp4 -y {}",
            exe.display(),
            url,
            output_path
        ));
        let mut process = match Command::new(exe)
            .args(["-i", url, "-c", "copy", "-f", "mp4", "-y", output_path])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(p) => p,
            Err(e) => {
                let msg = format!(
                    "No se encontró FFmpeg en {}. Descargalo y ponelo junto al .exe o en ffmpeg/bin/",
                    exe.display()
                );
                self.logger.log(&format!("{} - {}", msg, e));
                return Err(msg);
            }
        };

        let mut stdout_buf = String::new();
        let _ = process.stdout.take().map(|mut s| s.read_to_string(&mut stdout_buf));
        let mut stderr_buf = String::new();
        let _ = process.stderr.take().map(|mut s| s.read_to_string(&mut stderr_buf));
        self.logger.log(&format!("FFmpeg stdout: {}", stdout_buf));
        self.logger.log(&format!("FFmpeg stderr: {}", stderr_buf));

        if !status_success(&mut process) {
            let msg = extract_error(&stdout_buf, &stderr_buf);
            return Err(format!("FFmpeg falló: {}", msg));
        }

        *child = Some(process);
        Ok("Grabando con FFmpeg".to_string())
    }

    pub fn stop(&self) -> Result<(), String> {
        self.logger.log("Deteniendo grabación...");
        let mut child = self.child.lock().map_err(|e| e.to_string())?;
        if let Ok(mut e) = self.error.lock() {
            *e = None;
        }
        match child.take() {
            Some(mut process) => {
                let pid = process.id();
                let _ = process.kill();
                let _ = process.wait();
                // Kill entire process tree (ffmpeg child of yt-dlp)
                let _ = Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map(|mut c| c.wait());
                self.logger.log("Grabación detenida");
                Ok(())
            }
            None => Err("No hay ninguna grabación activa".to_string()),
        }
    }

    pub fn is_recording(&self) -> bool {
        let running = self
            .child
            .lock()
            .map(|mut c| {
                c.as_mut()
                    .map_or(false, |p| p.try_wait().map(|s| s.is_none()).unwrap_or(false))
            })
            .unwrap_or(false);
        // If process died, clear error state on next poll
        if !running {
            if let Ok(mut e) = self.error.lock() {
                if e.is_none() {
                    *e = Some("El proceso de grabación terminó inesperadamente".to_string());
                }
            }
        }
        running
    }

    pub fn get_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|mut e| e.take())
    }

    pub fn get_log(&self) -> String {
        self.logger.read_all()
    }
}

fn status_success(process: &mut Child) -> bool {
    std::thread::sleep(std::time::Duration::from_secs(3));
    match process.try_wait() {
        Ok(Some(status)) => status.success(),
        Ok(None) => true, // still running
        Err(_) => false,
    }
}

fn extract_error(stdout: &str, stderr: &str) -> String {
    stdout
        .lines()
        .chain(stderr.lines())
        .filter(|l| !l.is_empty())
        .last()
        .unwrap_or("Error desconocido")
        .to_string()
}

fn find_binary(name: &str, fallback: &str, logger: &Logger) -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();

    let project_root = exe_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_default();

    let mut candidates = vec![
        exe_dir.join(name),
        exe_dir.join(fallback).join("bin").join(name),
        project_root.join(fallback).join("bin").join(name),
        project_root.join(name),
    ];

    if let Ok(entries) = std::fs::read_dir(&project_root) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if dir.is_dir() {
                candidates.push(dir.join("bin").join(name));
                candidates.push(dir.join(name));
                candidates.push(dir.join(fallback).join(name));
                candidates.push(dir.join(fallback).join("bin").join(name));
            }
        }
    }

    for path in &candidates {
        logger.log(&format!("  Buscando {} en {}", name, path.display()));
        if path.exists() {
            logger.log(&format!("  → Encontrado: {}", path.display()));
            return path.clone();
        }
    }

    logger.log(&format!("  → No encontrado, usando fallback: {}", fallback));
    PathBuf::from(fallback)
}

fn find_plugin_dir(logger: &Logger) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();

    let project_root = exe_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_default();

    let candidates = vec![
        exe_dir.join("yt-dlp-plugins"),
        project_root.join("yt-dlp-plugins"),
    ];

    for path in &candidates {
        logger.log(&format!("  Buscando plugin dir en {}", path.display()));
        if !path.exists() {
            continue;
        }
        // yt-dlp plugin dirs can be structured as:
        //   <dir>/yt_dlp_plugins/extractor/*.py  (direct)
        //   <dir>/<subpkg>/yt_dlp_plugins/extractor/*.py  (subpackage)
        let direct = path.join("yt_dlp_plugins").join("extractor");
        if direct.exists() {
            logger.log(&format!("  → Plugin dir encontrado: {}", path.display()));
            return Some(path.clone());
        }
        // scan subdirectories for yt_dlp_plugins package
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let sub = entry.path().join("yt_dlp_plugins").join("extractor");
                    if sub.exists() {
                        logger.log(&format!("  → Plugin dir encontrado: {}", path.display()));
                        return Some(path.clone());
                    }
                }
            }
        }
    }

    logger.log("  → Plugin dir no encontrado");
    None
}

fn normalize_url(url: &str) -> String {
    // Remove language subdomain from URLs like es.example.com -> example.com
    if let Some(rest) = url.strip_prefix("https://") {
        if let Some(dot_idx) = rest.find('.') {
            let domain_part = &rest[..dot_idx];
            if domain_part.len() == 2 && domain_part.chars().all(|c| c.is_ascii_alphabetic()) {
                let remaining = &rest[dot_idx + 1..];
                return format!("https://{}", remaining);
            }
        }
    }
    url.to_string()
}
