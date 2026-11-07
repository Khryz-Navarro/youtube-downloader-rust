// The app's backend: installs and updates yt-dlp,
// runs downloads (pause/cancel with a Windows process-tree kill), deletes leftovers, and applies the
// Settings window's theme, page size and file names. The page talks to it through ui/api.js.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ytdlp;

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, Theme, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

// Set once yt-dlp is installed and update-checked; every yt-dlp call waits on it.
static READY: OnceLock<Result<(), String>> = OnceLock::new();

// Settings window values. They come from the page, so only these are accepted.
const SETTINGS: [(&str, &[&str]); 4] = [
    ("theme", &["system", "light", "dark"]),
    ("scale", &["1", "1.25", "1.5", "1.75"]),
    ("filename", &["classic", "basic", "pretty", "nerdy"]), // ytdlp::styled_name
    ("speed", &ytdlp::SPEED_LIMITS),
];

struct Job {
    pid: u32,
    stop: Arc<Mutex<Option<&'static str>>>, // why it was stopped: what the item shows afterwards
}

#[derive(Default, Clone)]
struct Files {
    streams: BTreeSet<PathBuf>, // every file yt-dlp reported writing
    file: Option<PathBuf>,      // the finished video
}

struct AppState {
    status: Mutex<Value>,
    jobs: Mutex<HashMap<String, Job>>,        // download id -> running yt-dlp
    downloads: Mutex<HashMap<String, Files>>, // download id -> its files, for Delete (kept across Retry/Resume)
    settings: Mutex<Value>,
}

fn command(bin: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(bin);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

fn data_dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().expect("no app data folder")
}

fn bin_path(app: &AppHandle) -> PathBuf {
    data_dir(app).join("bin").join(ytdlp::BINARY_NAME)
}

// Tauri puts bundle.externalBin next to the app's exe, both in the installed app and in `cargo run` builds.
fn ffmpeg_path() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.with_file_name(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    path.exists().then_some(path)
}

fn set_status(app: &AppHandle, status: Value) {
    *app.state::<AppState>().status.lock().unwrap() = status.clone();
    let _ = app.emit("status", status);
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

// Runs on every launch: fetch yt-dlp the first time, otherwise let it self-update.
fn prepare_ytdlp(app: &AppHandle) -> Result<(), String> {
    let bin = bin_path(app);
    let (mut checked, mut updated) = (true, false);
    if !bin.exists() {
        set_status(app, json!({ "state": "busy", "text": "Downloading yt-dlp…" }));
        std::fs::create_dir_all(bin.parent().unwrap()).map_err(|e| e.to_string())?;
        let part = bin.with_extension("part");
        // curl ships with Windows 10 and later, so the app needs no HTTP client of its own.
        let out = command("curl").args(["-fsSL", "-o"]).arg(&part).arg(ytdlp::BINARY_URL).output();
        let out = out.map_err(|e| format!("couldn't run curl ({e})"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&part, &bin).map_err(|e| e.to_string())?;
    } else {
        set_status(app, json!({ "state": "busy", "text": "Checking for yt-dlp updates…" }));
        match command(&bin).arg("-U").output() {
            Ok(out) if out.status.success() => {
                let text = format!("{}{}", stdout(&out), String::from_utf8_lossy(&out.stderr));
                updated = text.to_lowercase().contains("updated yt-dlp to");
            }
            _ => checked = false, // offline or GitHub unreachable: keep using the binary we have
        }
    }
    let version = command(&bin).arg("--version").output().ok().filter(|o| o.status.success());
    let version = stdout(&version.ok_or("yt-dlp didn't start")?);
    let ffmpeg = ffmpeg_path()
        .and_then(|f| command(f).arg("-version").output().ok())
        .and_then(|out| ytdlp::ffmpeg_version(&stdout(&out)));
    let checked_at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let state = if checked { "ok" } else { "warn" };
    set_status(app, json!({ "state": state, "version": version, "updated": updated, "ffmpeg": ffmpeg, "checkedAt": checked_at }));
    Ok(())
}

fn wait_ready() -> Result<(), String> {
    READY.wait().clone()
}

// reason is what the item shows afterwards: "canceled", or "paused" (the .part file is kept, so Resume continues it).
fn kill(job: &Job, reason: &'static str) {
    *job.stop.lock().unwrap() = Some(reason);
    let pid = job.pid.to_string();
    // yt-dlp.exe is a PyInstaller bundle that starts its own child (and ffmpeg), so kill the whole tree.
    let _ = if cfg!(windows) {
        command("taskkill").args(["/pid", &pid, "/T", "/F"]).spawn()
    } else {
        command("kill").arg(&pid).spawn()
    };
}

#[tauri::command]
fn status(state: State<AppState>) -> Value {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
fn about(app: AppHandle) -> Value {
    json!({ "version": app.package_info().version.to_string(), "tauri": tauri::VERSION })
}

#[tauri::command]
fn default_folder(app: AppHandle) -> Option<PathBuf> {
    app.path().download_dir().ok()
}

#[tauri::command]
fn paste() -> String {
    arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default()
}

#[tauri::command]
fn show_file(file: PathBuf) {
    let _ = tauri_plugin_opener::reveal_item_in_dir(file);
}

#[tauri::command]
async fn pick_folder(window: WebviewWindow, current: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = window.dialog().file().set_parent(&window);
    if let Some(dir) = current {
        dialog = dialog.set_directory(dir);
    }
    dialog.blocking_pick_folder()?.into_path().ok()
}

#[tauri::command]
async fn info(app: AppHandle, url: String) -> Value {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let args = ytdlp::info_args(&ytdlp::assert_url(&url)?);
        wait_ready()?;
        let out = command(bin_path(&app)).args(args).output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(ytdlp::error_text(&stderr).unwrap_or_else(|| format!("yt-dlp exited with code {:?}", out.status.code())));
        }
        serde_json::from_slice::<Value>(&out.stdout).map_err(|e| e.to_string())
    })
    .await;
    result.map_err(|e| e.to_string()).and_then(|r| r).unwrap_or_else(|error| json!({ "error": error }))
}

#[tauri::command]
fn download(app: AppHandle, id: String, url: String, quality: String, folder: String) {
    std::thread::spawn(move || run_download(&app, &id, &url, &quality, &folder));
}

fn run_download(app: &AppHandle, id: &str, url: &str, quality: &str, folder: &str) {
    let send = |mut ev: Value| {
        ev["id"] = id.into();
        let _ = app.emit("progress", ev);
    };
    let state = app.state::<AppState>();
    let ffmpeg = ffmpeg_path();
    // the settings when the download starts (or resumes): changing them doesn't touch one that's running
    let settings = state.settings.lock().unwrap().clone();
    let style = settings["filename"].as_str().unwrap_or("classic");
    let limit = settings["speed"].as_str().filter(|s| *s != "unlimited");
    let spawned = ytdlp::assert_url(url)
        .and_then(|url| ytdlp::download_args(&url, quality, folder, ffmpeg.as_deref().and_then(Path::to_str), limit))
        .and_then(|args| {
            wait_ready()?;
            let cmd = command(bin_path(app)).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
            cmd.map_err(|e| e.to_string())
        });
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return send(json!({ "state": "error", "error": error })),
    };
    let stop = Arc::new(Mutex::new(None));
    state.jobs.lock().unwrap().insert(id.into(), Job { pid: child.id(), stop: stop.clone() });

    // stdout and stderr are read on their own threads; their lines are handled here, in arrival order.
    let (tx, rx) = mpsc::channel();
    let streams: [Box<dyn Read + Send>; 2] = [Box::new(child.stdout.take().unwrap()), Box::new(child.stderr.take().unwrap())];
    for stream in streams {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stream).split(b'\n').map_while(Result::ok) {
                let _ = tx.send(String::from_utf8_lossy(&line).trim_end_matches('\r').to_string());
            }
        });
    }
    drop(tx);
    let (mut file, mut error, mut name) = (None::<PathBuf>, None, Value::Null);
    for line in rx {
        if line.starts_with("ERROR:") {
            error = ytdlp::error_text(&line);
        }
        let Some(ev) = ytdlp::parse_line(&line) else { continue };
        if let Some(parts) = ev.get("name") {
            name = parts.clone(); // for the finished file's name
            continue;
        }
        let mut downloads = state.downloads.lock().unwrap();
        let record = downloads.entry(id.into()).or_default();
        if let Some(name) = ev["filename"].as_str() {
            record.streams.insert(name.into());
        }
        if let Some(path) = ev["file"].as_str() {
            file = Some(path.into());
            // audio: the download is only the source; it counts as finished (Recycle Bin on Delete) once converted
            if ytdlp::audio_target(quality).is_none() {
                record.file = file.clone();
            }
        } else {
            drop(downloads);
            send(ev);
        }
    }
    let exit = child.wait();
    let ok = exit.as_ref().is_ok_and(|s| s.success());
    // Audio downloads: yt-dlp fetched the stream as-is; convert it while still registered as this job
    let converted = match (ytdlp::audio_target(quality), &file) {
        (Some(target), Some(source)) if ok && stop.lock().unwrap().is_none() => {
            let styled = ytdlp::styled_name(style, &name, target.0);
            Some(convert_audio(app, id, &stop, source, target, styled, ffmpeg.as_deref()))
        }
        _ => None,
    };
    state.jobs.lock().unwrap().remove(id);
    if let Some(reason) = *stop.lock().unwrap() {
        return send(json!({ "state": reason }));
    }
    if !ok {
        let fallback = || exit.map_or_else(|e| e.to_string(), |s| format!("yt-dlp exited with code {:?}", s.code()));
        return send(json!({ "state": "error", "error": error.unwrap_or_else(fallback) }));
    }
    match converted {
        Some(Err(error)) => return send(json!({ "state": "error", "error": error })),
        Some(Ok(path)) => file = Some(path),
        // a video: yt-dlp's working name is unique per video and quality; the finished file gets the
        // Settings → File names name, or keeps the working one if that can't be done
        None => {
            file = file.map(|f| {
                let styled = f.extension().and_then(|ext| ytdlp::styled_name(style, &name, &ext.to_string_lossy()));
                styled.and_then(|n| place(&f, &n).ok()).unwrap_or(f)
            })
        }
    }
    if let Some(record) = state.downloads.lock().unwrap().get_mut(id) {
        record.file = file.clone(); // what Delete sends to the Recycle Bin
    }
    let size = file.as_ref().and_then(|f| std::fs::metadata(f).ok()).map(|m| m.len());
    send(json!({ "state": "done", "file": file, "size": size }));
}

// Moves a finished file to `name` in its folder, never over another file: a taken name gets " (2)", " (3)"….
fn place(file: &Path, name: &str) -> std::io::Result<PathBuf> {
    static LOCK: Mutex<()> = Mutex::new(()); // two downloads finishing at once mustn't pick the same free name
    let _lock = LOCK.lock().unwrap();
    let free = (1..).map(|n| file.with_file_name(ytdlp::numbered_name(name, n))).find(|p| p == file || !p.exists());
    let target = free.expect("an endless range always finds a free name");
    std::fs::rename(file, &target)?;
    Ok(target)
}

// Converts a downloaded audio stream with the bundled ffmpeg into "<stem>.temp.<ext>", moves it to its finished
// name (`styled`, the Settings → File names one, else "Title [id].<ext>") and deletes the download. The job's pid
// becomes ffmpeg's, so Cancel stops the conversion too.
fn convert_audio(
    app: &AppHandle,
    id: &str,
    stop: &Arc<Mutex<Option<&'static str>>>,
    source: &Path,
    (ext, name): (&str, &str),
    styled: Option<String>,
    ffmpeg: Option<&Path>,
) -> Result<PathBuf, String> {
    let fail = |why: String| format!("Couldn't convert to {name}: {why}");
    let file_name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (temp, done) = ytdlp::audio_names(&file_name, ext);
    let (temp, done) = (source.with_file_name(temp), styled.unwrap_or(done));
    if source.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
        // already in that format (YouTube's AAC): only give it the finished name
        return place(source, &done).map_err(|e| fail(e.to_string()));
    }
    let ffmpeg = ffmpeg.ok_or_else(|| fail("ffmpeg is missing".into()))?;
    let _ = app.emit("progress", json!({ "id": id, "state": "processing", "stream": "audio" }));
    let args = ytdlp::convert_args(&source.to_string_lossy(), &temp.to_string_lossy(), ext);
    let mut child = command(ffmpeg).args(args).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| fail(e.to_string()))?;
    app.state::<AppState>().jobs.lock().unwrap().insert(id.into(), Job { pid: child.id(), stop: stop.clone() });
    if stop.lock().unwrap().is_some() {
        let _ = child.kill(); // canceled while ffmpeg was starting
    }
    let out = child.wait_with_output().map_err(|e| fail(e.to_string()))?;
    if !out.status.success() || stop.lock().unwrap().is_some() {
        let _ = std::fs::remove_file(&temp);
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(fail(stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg failed").to_string()));
    }
    let done = place(&temp, &done).map_err(|e| fail(e.to_string()))?;
    let _ = std::fs::remove_file(source);
    Ok(done)
}

#[tauri::command]
fn cancel(state: State<AppState>, id: String) {
    if let Some(job) = state.jobs.lock().unwrap().get(&id) {
        kill(job, "canceled");
    }
}

#[tauri::command]
fn pause(state: State<AppState>, id: String) {
    if let Some(job) = state.jobs.lock().unwrap().get(&id) {
        kill(job, "paused");
    }
}

// Delete what a download left on disk: its partial/temporary files (permanently) and, for a finished
// download, the video itself (to the Recycle Bin). Only paths yt-dlp reported for this id are touched,
// never a path sent by the page.
#[tauri::command]
async fn delete_files(app: AppHandle, id: String) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let running = state.jobs.lock().unwrap().contains_key(&id);
    let record = state.downloads.lock().unwrap().get(&id).cloned();
    let Some(record) = record.filter(|_| !running) else { return Ok(json!({ "removed": 0 })) }; // unknown, or still running
    let mut removed = 0;
    let mut by_folder: BTreeMap<&Path, Vec<String>> = BTreeMap::new();
    for stream in &record.streams {
        if let (Some(folder), Some(name)) = (stream.parent(), stream.file_name()) {
            by_folder.entry(folder).or_default().push(name.to_string_lossy().into());
        }
    }
    for (folder, streams) in by_folder {
        let names: Vec<String> = std::fs::read_dir(folder)
            .map(|dir| dir.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect())
            .unwrap_or_default();
        for name in ytdlp::leftover_names(&streams, &names) {
            let target = folder.join(name);
            if record.file.as_ref() == Some(&target) {
                continue; // the finished video goes to the Recycle Bin below
            }
            match std::fs::remove_file(&target) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
                _ => removed += 1,
            }
        }
    }
    if let Some(file) = record.file.clone().filter(|f| f.exists()) {
        // On a fresh thread: `trash` sets up COM per thread and panics if a pooled thread has it in another mode.
        let trashed = std::thread::spawn(move || trash::delete(file).map_err(|e| e.to_string())).join();
        trashed.map_err(|_| "Couldn't move the video to the Recycle Bin".to_string())??;
        removed += 1;
    }
    state.downloads.lock().unwrap().remove(&id);
    Ok(json!({ "removed": removed }))
}

/* ---------- settings (Settings window): theme, interface size and file names, saved in settings.json ---------- */

fn settings_path(app: &AppHandle) -> PathBuf {
    data_dir(app).join("settings.json")
}

fn load_settings(app: &AppHandle) -> Value {
    let saved: Value = std::fs::read(settings_path(app)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    // first run or unreadable file: defaults
    let mut settings = json!({ "theme": "system", "scale": "1", "filename": "classic", "speed": "unlimited" });
    for (key, allowed) in SETTINGS {
        if let Some(value) = saved[key].as_str().filter(|v| allowed.contains(v)) {
            settings[key] = value.into();
        }
    }
    settings
}

// Theme drives the page's prefers-color-scheme; "Text and page size" zooms the whole page.
fn apply_settings(window: &WebviewWindow, settings: &Value) {
    let theme = match settings["theme"].as_str() {
        Some("light") => Some(Theme::Light),
        Some("dark") => Some(Theme::Dark),
        _ => None, // Desktop: follow Windows
    };
    let _ = window.set_theme(theme);
    let _ = window.set_background_color(Some(ground(theme.or(window.theme().ok()))));
    let _ = window.set_zoom(settings["scale"].as_str().and_then(|s| s.parse().ok()).unwrap_or(1.0));
}

// The page's background color for a theme, so the window never flashes the other theme while it paints.
fn ground(theme: Option<Theme>) -> Color {
    if theme == Some(Theme::Light) { Color(0xF2, 0xF1, 0xEE, 255) } else { Color(0x13, 0x11, 0x0F, 255) }
}

#[tauri::command]
fn settings(state: State<AppState>) -> Value {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_setting(app: AppHandle, window: WebviewWindow, key: String, value: String) {
    if !SETTINGS.iter().any(|(k, allowed)| *k == key && allowed.contains(&value.as_str())) {
        return; // values come from the page
    }
    let state = app.state::<AppState>();
    let mut settings = state.settings.lock().unwrap();
    settings[&key] = value.into();
    let _ = std::fs::create_dir_all(data_dir(&app));
    let _ = std::fs::write(settings_path(&app), settings.to_string()); // losing a preference isn't worth an error
    apply_settings(&window, &settings);
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            status: Mutex::new(json!({ "state": "busy", "text": "Starting…" })),
            jobs: Default::default(),
            downloads: Default::default(),
            settings: Default::default(),
        })
        .invoke_handler(tauri::generate_handler![
            status, about, settings, set_setting, default_folder, paste, pick_folder, show_file, info, download, cancel,
            pause, delete_files
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let result = prepare_ytdlp(&handle);
                if let Err(e) = &result {
                    let text = format!("Couldn't set up yt-dlp: {e}. Check your connection and restart.");
                    set_status(&handle, json!({ "state": "error", "text": text }));
                }
                let _ = READY.set(result);
            });
            let settings = load_settings(app.handle());
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("YT Downloader")
                .inner_size(1040.0, 760.0)
                .min_inner_size(900.0, 600.0)
                .decorations(false) // the page draws the whole title bar, window buttons included
                // Tauri's drop handler would swallow a link dragged into the link box; api.js refuses other drops
                .disable_drag_drop_handler()
                .visible(false)
                .build()?;
            apply_settings(&window, &settings); // before it's shown, so there's no flash of the other theme
            *app.state::<AppState>().settings.lock().unwrap() = settings;
            window.show()?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // On "Desktop", Windows switching light/dark changes the window's theme; keep its background in step
            if let tauri::WindowEvent::ThemeChanged(theme) = event {
                let _ = window.set_background_color(Some(ground(Some(*theme))));
            }
        })
        .build(tauri::generate_context!())
        .expect("couldn't start YT Downloader");
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            // don't leave downloads running headless
            for job in app.state::<AppState>().jobs.lock().unwrap().values() {
                kill(job, "canceled");
            }
        }
    });
}
