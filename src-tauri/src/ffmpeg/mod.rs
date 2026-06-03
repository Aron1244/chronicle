use crate::logger::Logger;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

#[cfg(windows)]
fn create_command<S: AsRef<std::ffi::OsStr>>(exe: S) -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new(exe);
    c.creation_flags(0x08000000); // CREATE_NO_WINDOW
    c
}

#[cfg(not(windows))]
fn create_command<S: AsRef<std::ffi::OsStr>>(exe: S) -> Command {
    Command::new(exe)
}
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const MAX_SLOTS: usize = 3;

#[derive(Debug, Serialize)]
pub struct StreamFormat {
    pub id: String,
    pub ext: String,
    pub resolution: String,
    pub fps: Option<f64>,
    pub bitrate: Option<f64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CompressionConfig {
    pub crf: u32,
    pub preset: String,
    pub threads: u32,
}

#[derive(Debug, Serialize, Clone)]
pub struct SlotStatus {
    pub slot: usize,
    pub recording: bool,
    pub optimizing: bool,
    pub error: Option<String>,
    pub url: Option<String>,
    pub label: Option<String>,
    pub elapsed_secs: u64,
}

#[derive(Debug)]
enum RecorderBackend {
    Ytdlp,
    Streamlink,
    Ffmpeg,
}

struct SlotInfo {
    url: String,
    output_path: String,
    label: String,
    start_time: std::time::Instant,
    compress: Option<CompressionConfig>,
}

struct SlotState {
    child: Option<Child>,
    error: Arc<Mutex<Option<String>>>,
    info: Option<SlotInfo>,
    optimizing: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
}

impl SlotState {
    fn new() -> Self {
        Self {
            child: None,
            error: Arc::new(Mutex::new(None)),
            info: None,
            optimizing: Arc::new(AtomicBool::new(false)),
            stopping: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct Recorder {
    ytdlp_path: PathBuf,
    streamlink_path: PathBuf,
    ffmpeg_path: PathBuf,
    plugin_dir: Option<PathBuf>,
    logger: Logger,
    slots: Arc<Mutex<[SlotState; MAX_SLOTS]>>,
    compress_queue: Arc<Mutex<VecDeque<(usize, CompressionConfig, PathBuf, String)>>>,
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
            ytdlp_path: yt,
            streamlink_path: sl,
            ffmpeg_path: ff,
            plugin_dir,
            logger,
            slots: Arc::new(Mutex::new([
                SlotState::new(),
                SlotState::new(),
                SlotState::new(),
            ])),
            compress_queue: Arc::new(Mutex::new(VecDeque::new())),
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

        let output = create_command(exe)
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
                let url = f.get("url").and_then(|v| v.as_str()).map(|s| s.to_string());

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
                    url,
                }
            })
            .filter(|f| !f.resolution.is_empty() || f.audio_codec.is_some())
            .collect();

        self.logger.log(&format!("Formatos encontrados: {}", formats.len()));
        Ok(formats)
    }

    pub fn start(&self, slot: usize, url: &str, output_path: &str, format_id: Option<&str>, format_url: Option<&str>, compress: Option<CompressionConfig>) -> Result<String, String> {
        if slot >= MAX_SLOTS {
            return Err(format!("Slot {} inválido (máximo {})", slot, MAX_SLOTS - 1));
        }

        let mut slots = self.slots.lock().map_err(|e| e.to_string())?;
        let s = &mut slots[slot];

        if s.child.is_some() {
            let dead = s.child.as_mut()
                .and_then(|p| p.try_wait().ok())
                .flatten()
                .is_some();
            if !dead {
                return Err(format!("Slot {} ya está grabando", slot));
            }
            let _ = s.child.take();
            s.info = None;
        }

        s.stopping.store(false, Ordering::SeqCst);
        if let Ok(mut e) = s.error.lock() {
            *e = None;
        }

        let url = normalize_url(url);
        let label = extract_label(&url);
        self.logger.log(&format!("[Slot {}] Iniciando grabación: {} → {}", slot, url, output_path));

        let parent = std::path::Path::new(output_path)
            .parent()
            .ok_or("Ruta inválida")?;
        if let Err(e) = std::fs::create_dir_all(parent) {
            self.logger.log(&format!("[Slot {}] Error al crear carpeta: {}", slot, e));
            return Err(format!("Error al crear carpeta: {}", e));
        }

        let backend = self.detect_backend(&url);
        self.logger.log(&format!("[Slot {}] Backend detectado: {:?}", slot, backend));

        let result = match backend {
            RecorderBackend::Ytdlp => {
                let mut fmt_err: Option<String> = None;

                if format_id.is_some() {
                    match self.run_ytdlp(&url, output_path, format_id, s) {
                        Ok(msg) => Ok(msg),
                        Err(e) => {
                            self.logger.log(&format!("[Slot {}] yt-dlp con formato falló: {}. Reintentando sin formato...", slot, e));
                            fmt_err = Some(e);
                            self.run_ytdlp(&url, output_path, None, s)
                        }
                    }
                } else {
                    self.run_ytdlp(&url, output_path, None, s)
                }
                .or_else(|e| {
                    let ytdlp_extra = fmt_err.as_ref().map(|f| format!(" (con formato: {})", f)).unwrap_or_default();
                    self.logger.log(&format!("[Slot {}] yt-dlp sin formato también falló: {}", slot, e));
                    self.logger.log("Intentando con Streamlink como último recurso...");
                    self.run_streamlink(&url, output_path, s)
                        .or_else(|sl_err| {
                            if let Some(furl) = format_url {
                                let abs_url = resolve_url(&url, furl);
                                self.logger.log(&format!("[Slot {}] Intentando con FFmpeg directo: {}", slot, abs_url));
                                self.run_ffmpeg(&abs_url, output_path, s)
                                    .map_err(|ff_err| format!(
                                        "yt-dlp{} | Streamlink: {} | FFmpeg: {}", ytdlp_extra, sl_err, ff_err
                                    ))
                            } else {
                                Err(format!("yt-dlp{} | Streamlink: {}", ytdlp_extra, sl_err))
                            }
                        })
                })
            }
            RecorderBackend::Streamlink => {
                self.run_streamlink(&url, output_path, s)
            }
            RecorderBackend::Ffmpeg => {
                self.run_ffmpeg(&url, output_path, s)
            }
        };

        if result.is_ok() {
            s.info = Some(SlotInfo {
                url: url.clone(),
                output_path: output_path.to_string(),
                label,
                start_time: std::time::Instant::now(),
                compress,
            });
        }

        result
    }

    fn needs_audio_merge(&self, url: &str, format_id: &str) -> bool {
        let exe = &self.ytdlp_path;
        let mut args: Vec<String> = vec!["-J".into(), "--no-part".into()];
        if let Some(ref pd) = self.plugin_dir {
            args.push("--plugin-dirs".into());
            args.push(pd.to_string_lossy().to_string());
        }
        args.push(url.into());
        if let Ok(output) = create_command(exe).args(&args).output() {
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
        slot: &mut SlotState,
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
            let dir_str = ffmpeg_dir.to_string_lossy();
            if !dir_str.is_empty() {
                args.push("--ffmpeg-location".into());
                args.push(dir_str.to_string());
            }
        }
        if format_id.is_some() {
            args.push("--merge-output-format".into());
            args.push("mkv".into());
        }
        args.extend_from_slice(&["-o".into(), output_path.into(), url.into()]);

        self.logger.log(&format!(
            "Ejecutando: {} {}",
            exe.display(),
            args.join(" "),
        ));

        let mut process = match create_command(exe)
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
        let error_state = slot.error.clone();
        let stopping_flag = slot.stopping.clone();
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
                                // Don't set error if stop() was called intentionally
                                if !stopping_flag.load(Ordering::SeqCst) {
                                    if let Ok(mut e) = error_state.lock() {
                                        *e = Some(err_msg);
                                    }
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                logger.log("Monitor de yt-dlp finalizado");
            });
        }

        slot.child = Some(process);
        Ok("Grabando con yt-dlp".to_string())
    }

    fn run_streamlink(
        &self,
        url: &str,
        output_path: &str,
        slot: &mut SlotState,
    ) -> Result<String, String> {
        let exe = &self.streamlink_path;
        self.logger.log(&format!(
            "Ejecutando: {} {} best -o {}",
            exe.display(),
            url,
            output_path
        ));
        let mut process = match create_command(exe)
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
                return self.run_ytdlp(url, output_path, None, slot);
            }
            // Also try yt-dlp as fallback for any error
            if self.ytdlp_path.is_file() {
                self.logger
                    .log("Streamlink falló, probando con yt-dlp...");
                return self.run_ytdlp(url, output_path, None, slot);
            }
            return Err(format!("Streamlink falló: {}", msg));
        }

        slot.child = Some(process);
        Ok("Grabando con Streamlink".to_string())
    }

    fn run_ffmpeg(
        &self,
        url: &str,
        output_path: &str,
        slot: &mut SlotState,
    ) -> Result<String, String> {
        let exe = &self.ffmpeg_path;
        self.logger.log(&format!(
            "Ejecutando: {} -i {} -c copy -f matroska -y {}",
            exe.display(),
            url,
            output_path
        ));
        let mut cmd = create_command(exe);
        cmd.args(["-i", url, "-c", "copy", "-f", "matroska", "-y", output_path]);
        let mut process = match cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
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

        slot.child = Some(process);
        Ok("Grabando con FFmpeg".to_string())
    }

    pub fn stop(&self, slot: usize) -> Result<(), String> {
        if slot >= MAX_SLOTS {
            return Err(format!("Slot {} inválido", slot));
        }
        let mut slots = self.slots.lock().map_err(|e| e.to_string())?;
        let s = &mut slots[slot];
        self.logger.log(&format!("[Slot {}] Deteniendo grabación...", slot));
        // Signal monitor thread to ignore stderr errors from the kill
        s.stopping.store(true, Ordering::SeqCst);
        if let Ok(mut e) = s.error.lock() {
            *e = None;
        }
        // Capture info before clearing
        let compress = s.info.as_ref().and_then(|i| i.compress.clone());
        let output_path = s.info.as_ref().map(|i| i.output_path.clone());
        if let Some(mut process) = s.child.take() {
            let pid = process.id();
            let _ = create_command("taskkill")
                .args(["/F", "/T", "/PID", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .and_then(|mut tk| tk.wait());
            let _ = process.kill();
            let _ = process.wait();
            self.logger.log(&format!("[Slot {}] Grabación detenida", slot));
            s.info = None;

            // Post-processing: compress if configured
            if let (Some(cfg), Some(path)) = (compress, output_path) {
                s.optimizing.store(true, Ordering::SeqCst);
                let ffmpeg = self.ffmpeg_path.clone();
                drop(slots);
                self.spawn_compression(slot, cfg, ffmpeg, path);
            }

            Ok(())
        } else {
            Err(format!("Slot {} no está grabando", slot))
        }
    }

    fn spawn_compression(
        &self,
        slot: usize,
        cfg: CompressionConfig,
        ffmpeg_path: PathBuf,
        output_path: String,
    ) {
        if output_path.is_empty() {
            return;
        }
        let mut queue = self.compress_queue.lock().unwrap();
        queue.push_back((slot, cfg, ffmpeg_path, output_path));
        drop(queue);
        self.dequeue_compress();
    }

    fn dequeue_compress(&self) {
        let queue = self.compress_queue.clone();
        let slots = self.slots.clone();
        let logger = self.logger.clone();
        // Check if any slot is already compressing
        let any_running = {
            if let Ok(s) = slots.lock() {
                s.iter().any(|s| s.optimizing.load(Ordering::SeqCst))
            } else {
                return;
            }
        };
        if any_running {
            return;
        }
        let task = {
            let mut q = queue.lock().unwrap();
            q.pop_front()
        };
        let (slot, cfg, ffmpeg_path, output_path) = match task {
            Some(t) => t,
            None => return,
        };
        logger.log(&format!("[Slot {}] Comprimiendo video (CRF {})...", slot, cfg.crf));

        // Set optimizing flag before spawning
        if let Ok(s) = slots.lock() {
            s[slot].optimizing.store(true, Ordering::SeqCst);
        }

        std::thread::spawn(move || {
            let actual = resolve_actual_file(&output_path);
            let tmp_path = format!("{}_tmp.mkv", output_path);
            logger.log(&format!("[Slot {}] Comprimiendo: {} → {} (archivo real: {})",
                slot, output_path, tmp_path, actual));
            logger.log(&format!("[Slot {}] Ejecutando: {} -i \"{}\" -c:v libx264 -preset {} -crf {} -c:a aac -b:a 128k{} -af aresample=async=1:first_pts=0 -y \"{}\"",
                slot, ffmpeg_path.display(), actual, cfg.preset, cfg.crf,
                if cfg.threads > 0 { format!(" -threads {}", cfg.threads) } else { String::new() },
                tmp_path));

            // Save original path for rename logic; use actual file as input
            let input_path = actual;
            let mut cmd = create_command(&ffmpeg_path);
            cmd.arg("-i").arg(&input_path);
            cmd.arg("-c:v").arg("libx264");
            cmd.arg("-preset").arg(&cfg.preset);
            cmd.arg("-crf").arg(cfg.crf.to_string());
            cmd.arg("-c:a").arg("aac");
            cmd.arg("-b:a").arg("128k");
            if cfg.threads > 0 {
                cmd.arg("-threads").arg(cfg.threads.to_string());
            }
            cmd.arg("-af").arg("aresample=async=1:first_pts=0");
            cmd.arg("-y").arg(&tmp_path);
            let output = cmd
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .and_then(|c| c.wait_with_output());

            let mut slots = match slots.lock() {
                Ok(s) => s,
                Err(_) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    return;
                }
            };
            let s = &mut slots[slot];

            match output {
                Ok(out) if out.status.success() => {
                    let _ = std::fs::remove_file(&input_path);
                    let _ = std::fs::rename(&tmp_path, &output_path);
                    logger.log(&format!("[Slot {}] Compresión completada: {}", slot, output_path));
                }
                Ok(out) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let msg = extract_error("", &stderr);
                    logger.log(&format!("[Slot {}] Compresión falló:\n{}", slot, msg));
                    if let Ok(mut e) = s.error.lock() {
                        *e = Some(format!("Compresión falló: {}", msg));
                    }
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    logger.log(&format!("[Slot {}] Error al ejecutar compresión: {}", slot, e));
                    if let Ok(mut err) = s.error.lock() {
                        *err = Some(format!("Error al ejecutar ffmpeg: {}", e));
                    }
                }
            }
            s.optimizing.store(false, Ordering::SeqCst);
            // Next queued compression will be picked up by next get_all_statuses poll
        });
    }


    fn spawn_sync_fixup(&self, slot: usize, ffmpeg_path: PathBuf, output_path: String) {
        if output_path.is_empty() {
            return;
        }
        let logger = self.logger.clone();
        logger.log(&format!("[Slot {}] Corrigiendo sincronía de audio...", slot));

        std::thread::spawn(move || {
            let actual = resolve_actual_file(&output_path);
            let tmp_path = format!("{}_syncfix.mkv", output_path);
            logger.log(&format!("[Slot {}] Ajustando sync: {} → {} (archivo real: {})",
                slot, output_path, tmp_path, actual));

            let result = create_command(&ffmpeg_path)
                .args([
                    "-i", &actual,
                    "-c:v", "copy",
                    "-af", "aresample=async=1:first_pts=0",
                    "-c:a", "aac",
                    "-b:a", "128k",
                    "-y", &tmp_path,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .and_then(|c| c.wait_with_output());

            match result {
                Ok(out) if out.status.success() => {
                    let _ = std::fs::remove_file(&actual);
                    let _ = std::fs::rename(&tmp_path, &output_path);
                    logger.log(&format!("[Slot {}] Sincronía corregida: {}", slot, output_path));
                }
                Ok(out) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let msg = extract_error("", &stderr);
                    logger.log(&format!("[Slot {}] Corrección de sync falló:\n{}", slot, msg));
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    logger.log(&format!("[Slot {}] Error al corregir sync: {}", slot, e));
                }
            }
        });
    }

    pub fn stop_all(&self) -> Vec<String> {
        let mut results = Vec::new();
        for slot in 0..MAX_SLOTS {
            match self.stop(slot) {
                Ok(_) => results.push(format!("Slot {} detenido", slot)),
                Err(e) => results.push(format!("Slot {}: {}", slot, e)),
            }
        }
        results
    }

    pub fn get_all_statuses(&self) -> Vec<SlotStatus> {
        let mut slots = match self.slots.lock() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let mut statuses = Vec::with_capacity(MAX_SLOTS);
        let mut pending_compress: Vec<(usize, CompressionConfig, PathBuf, String)> = Vec::new();
        let mut pending_fixup: Vec<(usize, PathBuf, String)> = Vec::new();
        for (i, s) in slots.iter_mut().enumerate() {
            let running = match s.child.as_mut() {
                Some(_) if s.optimizing.load(Ordering::SeqCst) => true,
                Some(p) => {
                    match p.try_wait() {
                        Ok(None) => true,
                        _ => {
                            let _ = p.kill();
                            let _ = p.wait();
                            s.child = None;
                            // If compression was configured, save it before clearing info
                            let compress_cfg = s.info.as_ref().and_then(|i| i.compress.clone());
                            let out_path = s.info.as_ref().map(|i| i.output_path.clone());
                            if let (Some(cfg), Some(path)) = (compress_cfg, out_path.clone()) {
                                s.optimizing.store(true, Ordering::SeqCst);
                                pending_compress.push((i, cfg, self.ffmpeg_path.clone(), path));
                            } else if let Some(path) = out_path {
                                // No compression — fix audio sync in background
                                pending_fixup.push((i, self.ffmpeg_path.clone(), path));
                                if s.error.lock().ok().and_then(|mut e| e.take()).is_none() {
                                    if let Ok(mut e) = s.error.lock() {
                                        *e = Some("Stream finalizado".to_string());
                                    }
                                }
                            } else if s.error.lock().ok().and_then(|mut e| e.take()).is_none() {
                                // Stream ended without error and no compression — notify user
                                if let Ok(mut e) = s.error.lock() {
                                    *e = Some("Stream finalizado".to_string());
                                }
                            }
                            s.info = None;
                            false
                        }
                    }
                }
                None => false,
            };
            let optimizing = s.optimizing.load(Ordering::SeqCst);
            let error = s.error.lock().ok().and_then(|mut e| e.take());
            let info = s.info.as_ref();
            let elapsed_secs = info.map(|inf| inf.start_time.elapsed().as_secs()).unwrap_or(0);
            let url = info.map(|i| i.url.clone());
            let label = info.map(|i| i.label.clone());
            statuses.push(SlotStatus {
                slot: i,
                recording: running,
                optimizing,
                error,
                url,
                label,
                elapsed_secs,
            });
        }
        drop(slots);
        for (slot, cfg, ffmpeg, path) in pending_compress {
            self.spawn_compression(slot, cfg, ffmpeg, path);
        }
        for (slot, ffmpeg, path) in pending_fixup {
            self.spawn_sync_fixup(slot, ffmpeg, path);
        }
        self.dequeue_compress();
        statuses
    }

    pub fn is_recording(&self, slot: usize) -> bool {
        if slot >= MAX_SLOTS {
            return false;
        }
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let s = &mut slots[slot];
        match s.child.as_mut() {
            Some(p) => p.try_wait().ok().map(|opt| opt.is_none()).unwrap_or(false),
            None => false,
        }
    }

    pub fn get_error(&self, slot: usize) -> Option<String> {
        if slot >= MAX_SLOTS {
            return None;
        }
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots[slot].error.lock().ok().and_then(|mut e| e.take())
    }

    pub fn preview_stream(&self, url: &str) -> Result<String, String> {
        let exe = &self.ffmpeg_path;
        let mut cmd = create_command(exe);
        cmd.args([
            "-analyzeduration", "500000",
            "-probesize", "500000",
            "-i", url,
            "-vframes", "1",
            "-f", "image2pipe",
            "-vcodec", "png",
            "-",
        ]);
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Error al ejecutar ffmpeg: {}", e))?;

        // Read up to 2MB of stdout (a PNG frame is usually much smaller)
        let mut buf = Vec::new();
        let mut stdout = child.stdout.take().ok_or("No se pudo leer stdout")?;
        let timeout = std::time::Duration::from_secs(8);
        let start = std::time::Instant::now();
        loop {
            let mut chunk = [0u8; 8192];
            let readable = stdout.read(&mut chunk).map_err(|e| e.to_string())?;
            if readable == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..readable]);
            if buf.len() > 2_000_000 {
                break;
            }
            if start.elapsed() > timeout {
                let _ = child.kill();
                return Err("Timeout generando preview".to_string());
            }
        }
        let _ = child.wait();
        if buf.is_empty() {
            return Err("No se pudo capturar el frame — el stream podría ser el anuncio".to_string());
        }
        Ok(base64::engine::general_purpose::STANDARD.encode(&buf))
    }

    pub fn get_logs(&self) -> String {
        self.logger.read_all()
    }

    /// Kill any lingering yt-dlp/ffmpeg processes (safety net for app close)
    pub fn kill_all() {
        let _ = create_command("taskkill")
            .args(["/F", "/IM", "yt-dlp.exe"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .and_then(|mut c| c.wait());
        let _ = create_command("taskkill")
            .args(["/F", "/IM", "ffmpeg.exe"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .and_then(|mut c| c.wait());
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

fn resolve_actual_file(output_path: &str) -> String {
    let path = std::path::Path::new(output_path);
    if path.exists() {
        return output_path.to_string();
    }
    // yt-dlp sometimes appends .mp4/.ts to the output filename
    // (e.g. file.mkv → file.mkv.mp4) because the actual container
    // format differs from the -o extension
    for ext in &["mp4", "ts", "flv", "webm", "mkv", "mov"] {
        let candidate = format!("{}.{}", output_path, ext);
        if std::path::Path::new(&candidate).exists() {
            return candidate;
        }
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent = path.parent().unwrap_or(std::path::Path::new(""));
    for ext in &["mp4", "ts", "flv", "webm", "mkv", "mov"] {
        let candidate = parent.join(format!("{}.{}", stem, ext));
        if candidate.exists() {
            return candidate.to_string_lossy().to_string();
        }
    }
    output_path.to_string()
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

fn extract_label(url: &str) -> String {
    // Manual URL parsing without the `url` crate
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    if let Some(slash_pos) = rest.find('/') {
        let path = &rest[slash_pos..];
        let clean = path.trim_end_matches('/');
        if let Some(last) = clean.rsplit('/').next().filter(|s| !s.is_empty()) {
            return last.to_string();
        }
    }
    // Fallback: domain
    let domain = rest.split('/').next().unwrap_or(rest);
    let parts: Vec<&str> = domain.split('.').collect();
    if parts.len() >= 2 {
        return parts[parts.len() - 2].to_string();
    }
    "stream".to_string()
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

    let mut candidates: Vec<PathBuf> = vec![
        exe_dir.join(name),
        exe_dir.join(fallback).join("bin").join(name),
        project_root.join(fallback).join("bin").join(name),
        project_root.join(name),
    ];

    // Also search in Tauri resource directories (production build)
    for resource_dir in [exe_dir.join("resources"), exe_dir.join("_up_")] {
        if resource_dir.exists() {
            scan_resource_dir(&resource_dir, name, &mut candidates, 3);
        }
    }

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
        if path.exists() && is_valid_executable(path) {
            logger.log(&format!("  → Encontrado: {}", path.display()));
            return path.clone();
        }
    }

    logger.log(&format!("  → No encontrado, usando fallback: {}", fallback));
    PathBuf::from(fallback)
}

fn scan_resource_dir(dir: &std::path::Path, name: &str, candidates: &mut Vec<PathBuf>, max_depth: u32) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                if p.file_name().and_then(|n| n.to_str()) == Some(name) {
                    candidates.push(p);
                }
            } else if p.is_dir() && max_depth > 0 {
                candidates.push(p.join(name));
                candidates.push(p.join("bin").join(name));
                // Also check if the exe is inside a subfolder named after the binary (e.g. ffmpeg/ffmpeg.exe)
                if let Some(stem) = std::path::Path::new(name).file_stem().and_then(|s| s.to_str()) {
                    candidates.push(p.join(stem).join(name));
                }
                scan_resource_dir(&p, name, candidates, max_depth - 1);
            }
        }
    }
}

fn is_valid_executable(path: &std::path::Path) -> bool {
    path.metadata()
        .map(|m| m.len() > 100_000)
        .unwrap_or(false)
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

    let resources_dir = exe_dir.join("resources");

    let candidates = vec![
        exe_dir.join("yt-dlp-plugins"),
        project_root.join("yt-dlp-plugins"),
        resources_dir.join("yt-dlp-plugins"),
        exe_dir.join("_up_").join("yt-dlp-plugins"),
    ];

    for path in &candidates {
        logger.log(&format!("  Buscando plugin dir en {}", path.display()));
        if !path.exists() {
            continue;
        }
        let direct = path.join("yt_dlp_plugins").join("extractor");
        if direct.exists() {
            logger.log(&format!("  → Plugin dir encontrado: {}", path.display()));
            return Some(path.clone());
        }
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

fn resolve_url(base: &str, maybe_relative: &str) -> String {
    if maybe_relative.starts_with("http://") || maybe_relative.starts_with("https://") {
        return maybe_relative.to_string();
    }
    if let Some(rest) = base.strip_prefix("https://").or_else(|| base.strip_prefix("http://")) {
        if let Some(slash_idx) = rest.find('/') {
            let base_domain = &rest[..slash_idx];
            if maybe_relative.starts_with('/') {
                return format!("https://{}{}", base_domain, maybe_relative);
            }
            let base_path = &rest[slash_idx..];
            let last_slash = base_path.rfind('/');
            if let Some(ls) = last_slash {
                return format!("https://{}{}/{}", base_domain, &base_path[..ls], maybe_relative);
            }
            return format!("https://{}/{}", base_domain, maybe_relative);
        }
        return format!("https://{}/{}", rest, maybe_relative);
    }
    maybe_relative.to_string()
}

fn normalize_url(url: &str) -> String {
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
