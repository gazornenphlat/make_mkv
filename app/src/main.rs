//! Make MKV: copies the titles you pick from a DVD, Blu-ray disc or folder of video files into MKV
//! files with ffmpeg. Personal tool. Streams are copied, not re-encoded. Protected discs are not decrypted.

mod disc;
mod drives;
mod ifo;
mod logic;

use eframe::egui;
use logic::{DEFAULT_MIN_LENGTH_SECS, DEFAULT_NAME_TEMPLATE, DiscKind, Segment, Title};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

const STORAGE_KEY: &str = "settings";
/// Log of failed rips, written next to the MKV files.
const LOG_FILE: &str = "make_mkv.log";
/// Space to keep free on the output drive after a rip.
const SPARE_BYTES: u64 = 1 << 30;

/// What is kept between runs.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    output: String,
    template: String,
    min_secs: u32,
    /// Folder with ffmpeg and ffprobe. Blank means use the PATH.
    ffmpeg_dir: String,
    eject_when_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output: String::new(),
            template: DEFAULT_NAME_TEMPLATE.to_string(),
            min_secs: DEFAULT_MIN_LENGTH_SECS,
            ffmpeg_dir: String::new(),
            eject_when_done: false,
        }
    }
}

/// Where ffmpeg and ffprobe are.
#[derive(Clone)]
struct Tools {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl Tools {
    fn from_dir(dir: &str) -> Self {
        let dir = dir.trim();
        let tool = |name: &str| {
            if dir.is_empty() {
                PathBuf::from(name)
            } else {
                Path::new(dir).join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
            }
        };
        Self {
            ffmpeg: tool("ffmpeg"),
            ffprobe: tool("ffprobe"),
        }
    }
}

/// Messages from the scan and rip workers to the window.
enum Msg {
    Scanned(Result<Scan, String>),
    CurrentStarted(String),
    Progress(f32),
    FileDone(Result<String, String>),
    Finished { cancelled: bool },
}

struct Scan {
    titles: Vec<Title>,
    disc_name: Option<String>,
}

struct Job {
    title: Title,
    out: PathBuf,
    log: PathBuf,
}

/// Keeps Windows from opening a console window for every tool call.
fn hidden(cmd: Command) -> Command {
    #[cfg(windows)]
    let mut cmd = cmd;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn ffmpeg_available(tools: &Tools) -> bool {
    hidden(Command::new(&tools.ffmpeg))
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Reads a child's standard output, stopping the child if Stop is pressed first.
fn run_capture(mut child: Child, cancel: &AtomicBool) -> Option<String> {
    let mut stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    loop {
        if child.try_wait().ok()?.is_some() {
            break;
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
    reader.join().ok()
}

fn probe_duration(tools: &Tools, path: &Path, cancel: &AtomicBool) -> Option<f64> {
    let child = hidden(Command::new(&tools.ffprobe))
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    logic::parse_duration(&run_capture(child, cancel)?)
}

/// Walks the folder (not following links) and keeps the video files, up to six levels down.
fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            // BACKUP holds disc backups; their copies would be listed twice.
            if !entry.file_name().eq_ignore_ascii_case("BACKUP") {
                collect(&path, depth + 1, out);
            }
        } else if kind.is_file() && logic::is_media(&path) {
            out.push(path);
        }
    }
}

fn scan(
    folder: PathBuf,
    min_secs: u32,
    tools: Tools,
    tx: mpsc::Sender<Msg>,
    cancel: Arc<AtomicBool>,
) {
    // A DVD or Blu-ray layout gives real titles; any other folder is searched for video files.
    let (disc_name, found) = match disc::find_disc_titles(&folder) {
        Some(found) => (found.disc_name, found.titles),
        None => {
            let mut paths = Vec::new();
            collect(&folder, 0, &mut paths);
            paths.sort();
            let titles = paths
                .into_iter()
                .map(|path| disc::FoundTitle {
                    name: path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    sources: vec![Segment::whole(path)],
                    duration_secs: None,
                })
                .collect();
            (None, titles)
        }
    };

    let mut titles = Vec::with_capacity(found.len());
    for (i, f) in found.into_iter().enumerate() {
        // Checked before each title, so Stop scan answers within one probe.
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let duration_secs = f.duration_secs.or_else(|| {
            logic::direct_path(&f.sources).and_then(|p| probe_duration(&tools, p, &cancel))
        });
        let selected = !logic::is_too_short(duration_secs, min_secs);
        titles.push(Title {
            number: i + 1,
            label: f.name,
            sources: f.sources,
            duration_secs,
            selected,
        });
    }

    let result = if cancel.load(Ordering::Relaxed) {
        Err("Scan stopped.".to_string())
    } else if titles.is_empty() {
        Err("No titles were found in that folder.".to_string())
    } else {
        Ok(Scan { titles, disc_name })
    };
    let _ = tx.send(Msg::Scanned(result));
}

/// Copies one piece of a file into `out`.
fn copy_segment(seg: &Segment, out: &mut impl Write) -> io::Result<()> {
    let mut file = File::open(&seg.path)?;
    file.seek(SeekFrom::Start(seg.offset))?;
    io::copy(&mut file.take(seg.len), out)?;
    Ok(())
}

fn append_log(path: &Path, text: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(text.as_bytes());
    }
}

/// Remuxes one title to MKV, copying the video, audio and subtitle streams without re-encoding.
fn convert(
    job: &Job,
    cancel: &AtomicBool,
    tx: &mpsc::Sender<Msg>,
    tools: &Tools,
) -> Result<(), String> {
    let direct = logic::direct_path(&job.title.sources);
    let mut cmd = hidden(Command::new(&tools.ffmpeg));
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-progress",
        "pipe:1",
        "-n",
    ]);
    match direct {
        Some(path) => {
            cmd.args(["-nostdin", "-i"]).arg(path);
        }
        // Several pieces are fed in order through stdin, so they play as one stream.
        None => {
            cmd.args([
                "-probesize",
                "64M",
                "-analyzeduration",
                "100M",
                "-i",
                "pipe:0",
            ])
            .stdin(Stdio::piped());
        }
    }
    cmd.args(["-map", "0:v", "-map", "0:a?", "-map", "0:s?", "-c", "copy"])
        .arg(&job.out)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start ffmpeg ({e})"))?;

    let writer = child.stdin.take().map(|mut stdin| {
        let pieces = job.title.sources.clone();
        thread::spawn(move || {
            for piece in &pieces {
                if copy_segment(piece, &mut stdin).is_err() {
                    break;
                }
            }
        })
    });

    // Read the error text on its own thread so a full pipe can't stall the progress reading.
    let mut stderr = child.stderr.take();
    let errors = thread::spawn(move || {
        let mut text = String::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_string(&mut text);
        }
        text
    });

    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                break;
            }
            if let Some(share) = logic::progress_fraction(&line, job.title.duration_secs) {
                let _ = tx.send(Msg::Progress(share));
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let error_text = errors.join().unwrap_or_default();

    if cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&job.out);
        return Err("stopped by you".to_string());
    }
    // With -n, ffmpeg refuses an existing file. That file belongs to the user, so it is never removed.
    let already_exists = error_text.contains("already exists");
    if status.success() && !already_exists {
        return Ok(());
    }
    if !already_exists {
        // Don't leave a half-written MKV behind.
        let _ = std::fs::remove_file(&job.out);
    }
    append_log(
        &job.log,
        &format!("{}\n{}\n", job.out.display(), error_text.trim()),
    );
    let last = error_text
        .lines()
        .last()
        .unwrap_or("ffmpeg stopped with an error");
    Err(last.trim().to_string())
}

fn rip_all(jobs: Vec<Job>, cancel: Arc<AtomicBool>, tx: mpsc::Sender<Msg>, tools: Tools) {
    for job in &jobs {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let name = job
            .out
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = tx.send(Msg::CurrentStarted(name.clone()));
        let result = convert(job, &cancel, &tx, &tools)
            .map(|()| name.clone())
            .map_err(|e| format!("{name}: {e}"));
        let _ = tx.send(Msg::FileDone(result));
    }
    let _ = tx.send(Msg::Finished {
        cancelled: cancel.load(Ordering::Relaxed),
    });
}

/// Optical drives (and removable drives holding a disc), with what is in each.
fn scan_drives() -> Vec<(PathBuf, DiscKind)> {
    drives::drive_roots()
        .into_iter()
        .filter_map(|(root, optical)| {
            let kind = logic::disc_kind(&root);
            // A removable drive only counts when it holds a DVD or Blu-ray structure.
            (optical || matches!(kind, DiscKind::BluRay | DiscKind::Dvd)).then_some((root, kind))
        })
        .collect()
}

fn fmt_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

fn format_length(secs: f64) -> String {
    let total = secs.round() as u64;
    format!(
        "{}:{:02}:{:02}",
        total / 3600,
        (total / 60) % 60,
        total % 60
    )
}

struct App {
    settings: Settings,
    source: String,
    titles: Vec<Title>,
    /// The disc's own name, used for output names in place of the folder name.
    disc_name: Option<String>,
    scanning: bool,
    ripping: bool,
    cancel: Arc<AtomicBool>,
    scan_cancel: Arc<AtomicBool>,
    rx: Option<mpsc::Receiver<Msg>>,
    jobs_total: usize,
    jobs_done: usize,
    file_share: f32,
    current: String,
    log: Vec<String>,
    tools: Tools,
    /// The ffmpeg folder the last check used.
    checked_dir: String,
    /// None while the ffmpeg check is still running.
    ffmpeg_ok: Option<bool>,
    ffmpeg_rx: Option<mpsc::Receiver<bool>>,
    /// Optical drives found by the last check, with what is in each.
    drives: Vec<(PathBuf, DiscKind)>,
    drives_rx: Option<mpsc::Receiver<Vec<(PathBuf, DiscKind)>>>,
    drives_checked: Instant,
    /// When set, the window saves one picture of itself to this file and closes.
    shot: Option<PathBuf>,
    frames: u32,
    shot_saved: Arc<AtomicBool>,
}

impl App {
    fn new(shot: Option<PathBuf>, settings: Settings) -> Self {
        drives::quiet_drive_errors();
        let tools = Tools::from_dir(&settings.ffmpeg_dir);
        let mut app = Self {
            checked_dir: settings.ffmpeg_dir.clone(),
            settings,
            source: String::new(),
            titles: Vec::new(),
            disc_name: None,
            scanning: false,
            ripping: false,
            cancel: Arc::new(AtomicBool::new(false)),
            scan_cancel: Arc::new(AtomicBool::new(false)),
            rx: None,
            jobs_total: 0,
            jobs_done: 0,
            file_share: 0.0,
            current: String::new(),
            log: Vec::new(),
            tools,
            ffmpeg_ok: None,
            ffmpeg_rx: None,
            drives: Vec::new(),
            drives_rx: None,
            drives_checked: Instant::now(),
            shot,
            frames: 0,
            shot_saved: Arc::new(AtomicBool::new(false)),
        };
        // Both checks run in the background so the window opens at once.
        app.recheck_ffmpeg();
        app.refresh_drives();
        app
    }

    /// Checks ffmpeg in the background with the current folder setting.
    fn recheck_ffmpeg(&mut self) {
        self.tools = Tools::from_dir(&self.settings.ffmpeg_dir);
        self.checked_dir = self.settings.ffmpeg_dir.clone();
        self.ffmpeg_ok = None;
        let (tx, rx) = mpsc::channel();
        self.ffmpeg_rx = Some(rx);
        let tools = self.tools.clone();
        thread::spawn(move || {
            let _ = tx.send(ffmpeg_available(&tools));
        });
    }

    /// Starts a drive check on a worker thread, unless one is already running.
    fn refresh_drives(&mut self) {
        if self.drives_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.drives_rx = Some(rx);
        self.drives_checked = Instant::now();
        thread::spawn(move || {
            let _ = tx.send(scan_drives());
        });
    }

    /// Picks up results from the background checks without waiting for them.
    fn poll_background(&mut self) {
        if let Some(ok) = self.ffmpeg_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.ffmpeg_ok = Some(ok);
            self.ffmpeg_rx = None;
            if !ok {
                self.add_log(
                    "ffmpeg was not found. Install it, or set its folder below, then restart.",
                );
            }
        }
        if let Some(list) = self.drives_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.drives = list;
            self.drives_rx = None;
        }
    }

    fn add_log(&mut self, text: impl Into<String>) {
        self.log.push(text.into());
        if self.log.len() > 500 {
            self.log.remove(0);
        }
    }

    fn start_scan(&mut self) {
        let folder = PathBuf::from(self.source.trim());
        if !folder.is_dir() {
            self.add_log("That source folder does not exist.");
            return;
        }
        // A disc with an AACS folder is protected. This app does not decrypt it.
        if folder.join("AACS").is_dir() {
            self.add_log("This disc uses AACS copy protection, which this app cannot read.");
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.titles.clear();
        self.disc_name = None;
        self.scanning = true;
        self.scan_cancel.store(false, Ordering::Relaxed);
        let cancel = self.scan_cancel.clone();
        let min_secs = self.settings.min_secs;
        let tools = self.tools.clone();
        thread::spawn(move || scan(folder, min_secs, tools, tx, cancel));
        self.add_log("Looking for titles…");
    }

    fn start_rip(&mut self) {
        let output = self.settings.output.trim().to_string();
        if output.is_empty() {
            self.add_log("Output folder is not set.");
            return;
        }
        let out_dir = PathBuf::from(&output);
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            self.add_log(format!("Could not use the output folder: {e}"));
            return;
        }
        let disc_label = self
            .disc_name
            .clone()
            .unwrap_or_else(|| logic::folder_label(Path::new(self.source.trim())));
        let min_secs = self.settings.min_secs;

        let mut jobs = Vec::new();
        let mut skipped = Vec::new();
        for t in self.titles.iter().filter(|t| t.selected) {
            if logic::is_too_short(t.duration_secs, min_secs) {
                skipped.push(format!(
                    "{} is shorter than {} seconds and was skipped.",
                    t.label, min_secs
                ));
                continue;
            }
            let out = logic::output_path(
                &out_dir,
                &self.settings.template,
                t.number,
                &disc_label,
                |p| p.exists(),
            );
            jobs.push(Job {
                title: t.clone(),
                out,
                log: out_dir.join(LOG_FILE),
            });
        }
        for line in skipped {
            self.add_log(line);
        }
        if jobs.is_empty() {
            self.add_log("Nothing to rip. Pick at least one title.");
            return;
        }

        let needed: u64 = jobs.iter().map(|j| j.title.size()).sum();
        if let Some(free) = drives::free_space(&out_dir)
            && free < needed + SPARE_BYTES
        {
            self.add_log(format!(
                "Not enough space on the output drive: the titles need {} and {} is free. Keep 1 GB spare.",
                fmt_gb(needed),
                fmt_gb(free)
            ));
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.cancel.store(false, Ordering::Relaxed);
        self.jobs_total = jobs.len();
        self.jobs_done = 0;
        self.file_share = 0.0;
        self.current.clear();
        self.ripping = true;
        let cancel = self.cancel.clone();
        let tools = self.tools.clone();
        thread::spawn(move || rip_all(jobs, cancel, tx, tools));
        self.add_log(format!("Starting {} file(s).", self.jobs_total));
    }

    fn drain_messages(&mut self) {
        let msgs: Vec<Msg> = match &self.rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for msg in msgs {
            match msg {
                Msg::Scanned(Ok(scan)) => {
                    self.add_log(format!("Found {} title(s).", scan.titles.len()));
                    self.titles = scan.titles;
                    self.disc_name = scan.disc_name;
                    self.scanning = false;
                    self.rx = None;
                }
                Msg::Scanned(Err(e)) => {
                    self.add_log(e);
                    self.scanning = false;
                    self.rx = None;
                }
                Msg::CurrentStarted(name) => {
                    self.current = name;
                    self.file_share = 0.0;
                }
                Msg::Progress(share) => self.file_share = share,
                Msg::FileDone(Ok(name)) => {
                    self.jobs_done += 1;
                    self.add_log(format!("Saved {name}."));
                }
                Msg::FileDone(Err(e)) => {
                    self.jobs_done += 1;
                    self.add_log(format!("Failed: {e}"));
                }
                Msg::Finished { cancelled } => {
                    self.ripping = false;
                    self.rx = None;
                    self.current.clear();
                    self.file_share = 0.0;
                    self.add_log(if cancelled { "Stopped." } else { "All done." });
                    if !cancelled && self.settings.eject_when_done {
                        self.eject_source();
                    }
                }
            }
        }
    }

    /// Ejects the disc when the source is a drive root. Done on a thread, since it can take a moment.
    fn eject_source(&mut self) {
        let source = PathBuf::from(self.source.trim());
        if source.ancestors().last() != Some(source.as_path()) {
            return;
        }
        self.add_log("Ejecting the disc.");
        thread::spawn(move || {
            drives::eject(&source);
        });
    }

    /// Asks the window for one picture and saves it from the callback, once the frame is drawn.
    fn take_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot.clone() else {
            return;
        };
        self.frames += 1;
        // Keep frames coming: an idle window is not redrawn, so the count would stop.
        ctx.request_repaint();
        if self.frames == 120 {
            let saved = self.shot_saved.clone();
            ctx.request_screenshot(move |image| {
                let raw = image.as_raw();
                let [w, h] = image.size;
                match image::save_buffer(&path, raw, w as u32, h as u32, image::ColorType::Rgba8) {
                    Ok(()) => println!("saved {}", path.display()),
                    Err(e) => eprintln!("could not save screenshot: {e}"),
                }
                saved.store(true, Ordering::Relaxed);
            });
        }
        if self.shot_saved.load(Ordering::Relaxed) || self.frames > 400 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_background();
        self.drain_messages();
        self.take_screenshot(&ctx);
        if self.scanning || self.ripping {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        // Looks again every few seconds, so a disc put in (or taken out) shows up on its own.
        if !self.ripping
            && self.drives_rx.is_none()
            && self.drives_checked.elapsed() > Duration::from_secs(3)
        {
            self.refresh_drives();
        }
        ctx.request_repaint_after(Duration::from_secs(1));

        egui::Panel::top("header").show(ui, |ui| {
            ui.add_space(6.0);
            ui.heading("Make MKV");
            ui.label(egui::RichText::new("Copies the titles you pick into MKV files.").weak());
            ui.add_space(6.0);
        });

        egui::Panel::bottom("footer").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let can_rip = !self.ripping
                    && !self.scanning
                    && self.ffmpeg_ok == Some(true)
                    && self.titles.iter().any(|t| t.selected);
                if ui
                    .add_enabled(can_rip, egui::Button::new("Rip selected"))
                    .clicked()
                {
                    self.start_rip();
                }
                if ui
                    .add_enabled(self.scanning, egui::Button::new("Stop scan"))
                    .clicked()
                {
                    self.scan_cancel.store(true, Ordering::Relaxed);
                }
                if ui
                    .add_enabled(self.ripping, egui::Button::new("Stop"))
                    .clicked()
                {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                match self.ffmpeg_ok {
                    None => {
                        ui.label(egui::RichText::new("Checking ffmpeg…").weak());
                    }
                    Some(false) => {
                        ui.colored_label(egui::Color32::LIGHT_RED, "ffmpeg not found");
                    }
                    Some(true) => {}
                }
                if self.ripping {
                    let overall =
                        (self.jobs_done as f32 + self.file_share) / self.jobs_total.max(1) as f32;
                    ui.add(
                        egui::ProgressBar::new(overall.clamp(0.0, 1.0))
                            .show_percentage()
                            .desired_width(260.0),
                    );
                    ui.label(format!(
                        "{} of {} · {}",
                        self.jobs_done + 1,
                        self.jobs_total,
                        self.current
                    ));
                }
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &self.log {
                        ui.label(line);
                    }
                });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(4.0);
            egui::Grid::new("setup")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Source folder");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.source)
                                .desired_width(420.0)
                                .hint_text("Disc or folder with video files"),
                        );
                        if ui.button("Browse").clicked()
                            && let Some(dir) = rfd::FileDialog::new().pick_folder()
                        {
                            self.source = dir.display().to_string();
                        }
                        if ui
                            .add_enabled(
                                !self.scanning && !self.ripping,
                                egui::Button::new("Look for titles"),
                            )
                            .clicked()
                        {
                            self.start_scan();
                        }
                    });
                    ui.end_row();

                    ui.label("Discs and drives");
                    let mut picked = None;
                    ui.horizontal_wrapped(|ui| {
                        if self.drives.is_empty() {
                            ui.label(
                                egui::RichText::new("No CD, DVD or Blu-ray drive found.").weak(),
                            );
                        }
                        for (root, kind) in &self.drives {
                            let text = match kind {
                                DiscKind::Empty => "empty",
                                DiscKind::BluRay => "Blu-ray disc",
                                DiscKind::Dvd => "DVD",
                                DiscKind::Other => "disc (format not recognized)",
                            };
                            ui.label(format!("{}  {text}", root.display()));
                            let usable =
                                *kind != DiscKind::Empty && !self.scanning && !self.ripping;
                            if ui
                                .add_enabled(usable, egui::Button::new("Use this"))
                                .clicked()
                            {
                                picked = Some(root.display().to_string());
                            }
                            ui.add_space(12.0);
                        }
                    });
                    if let Some(path) = picked {
                        self.source = path;
                        self.start_scan();
                    }
                    ui.end_row();

                    ui.label("Output folder");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings.output)
                                .desired_width(420.0)
                                .hint_text("Where the MKV files go"),
                        );
                        if ui.button("Browse").clicked()
                            && let Some(dir) = rfd::FileDialog::new().pick_folder()
                        {
                            self.settings.output = dir.display().to_string();
                        }
                    });
                    ui.end_row();

                    ui.label("Shortest title");
                    ui.horizontal(|ui| {
                        let changed = ui
                            .add(
                                egui::DragValue::new(&mut self.settings.min_secs)
                                    .range(1..=3600)
                                    .suffix(" seconds"),
                            )
                            .changed();
                        ui.label(egui::RichText::new("shorter titles are skipped").weak());
                        if changed {
                            // Re-tick the list so the checkboxes match the new minimum.
                            let min = self.settings.min_secs;
                            for t in self.titles.iter_mut() {
                                t.selected = !logic::is_too_short(t.duration_secs, min);
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("File name");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.template)
                            .desired_width(260.0)
                            .hint_text("%s_%02u.mkv  (%s = disc name, %02u = number)"),
                    );
                    ui.end_row();

                    ui.label("ffmpeg folder");
                    ui.horizontal(|ui| {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.settings.ffmpeg_dir)
                                .desired_width(420.0)
                                .hint_text("Blank uses the PATH"),
                        );
                        if resp.lost_focus() && self.settings.ffmpeg_dir != self.checked_dir {
                            self.recheck_ffmpeg();
                        }
                        if ui.button("Browse").clicked()
                            && let Some(dir) = rfd::FileDialog::new().pick_folder()
                        {
                            self.settings.ffmpeg_dir = dir.display().to_string();
                            self.recheck_ffmpeg();
                        }
                    });
                    ui.end_row();

                    ui.label("After ripping");
                    ui.checkbox(
                        &mut self.settings.eject_when_done,
                        "Eject the disc when done",
                    );
                    ui.end_row();
                });

            ui.add_space(10.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Titles");
                ui.label(egui::RichText::new(format!("{} found", self.titles.len())).weak());
                if ui
                    .add_enabled(!self.titles.is_empty(), egui::Button::new("Select all"))
                    .clicked()
                {
                    self.titles.iter_mut().for_each(|t| t.selected = true);
                }
                if ui
                    .add_enabled(!self.titles.is_empty(), egui::Button::new("Clear"))
                    .clicked()
                {
                    self.titles.iter_mut().for_each(|t| t.selected = false);
                }
            });

            if self.titles.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Pick a source folder and press Look for titles.")
                            .weak(),
                    );
                });
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("titles")
                        .striped(true)
                        .num_columns(5)
                        .spacing([14.0, 6.0])
                        .show(ui, |ui| {
                            for t in self.titles.iter_mut() {
                                let short =
                                    logic::is_too_short(t.duration_secs, self.settings.min_secs);
                                ui.checkbox(&mut t.selected, "");
                                ui.label(format!("{:02}", t.number));
                                ui.label(t.label.clone());
                                let length = match t.duration_secs {
                                    Some(d) => format_length(d),
                                    None => "length unknown".to_string(),
                                };
                                ui.label(length);
                                ui.label(format!("{:.1} MB", t.size() as f64 / 1_048_576.0));
                                if short {
                                    ui.label(egui::RichText::new("too short").weak());
                                }
                                ui.end_row();
                            }
                        });
                });
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, &self.settings);
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let shot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([940.0, 680.0])
            .with_title("Make MKV"),
        ..Default::default()
    };
    eframe::run_native(
        "Make MKV",
        options,
        Box::new(move |cc| {
            let settings: Settings = cc
                .storage
                .and_then(|s| eframe::get_value(s, STORAGE_KEY))
                .unwrap_or_default();
            Ok(Box::new(App::new(shot.clone(), settings)))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ffmpeg_present() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Two pieces of one file, fed through stdin, rip to a 4-second MKV.
    #[test]
    fn pieces_are_fed_through_stdin_as_one_stream() {
        if !ffmpeg_present() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("mkv_pipe_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("in.ts");
        let made = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=4:size=160x120:rate=25",
                "-c:v",
                "mpeg2video",
                "-f",
                "mpegts",
            ])
            .arg(&src)
            .status()
            .unwrap();
        assert!(made.success());

        let len = logic::file_len(&src);
        let half = len / 2;
        let out = dir.join("out.mkv");
        let job = Job {
            title: Title {
                number: 1,
                label: "t".into(),
                sources: vec![
                    Segment {
                        path: src.clone(),
                        offset: 0,
                        len: half,
                    },
                    Segment {
                        path: src.clone(),
                        offset: half,
                        len: len - half,
                    },
                ],
                duration_secs: Some(4.0),
                selected: true,
            },
            out: out.clone(),
            log: dir.join("log.txt"),
        };
        let tools = Tools::from_dir("");
        let (tx, _rx) = mpsc::channel::<Msg>();
        let cancel = AtomicBool::new(false);
        convert(&job, &cancel, &tx, &tools).unwrap();
        let secs = probe_duration(&tools, &out, &cancel).unwrap();
        assert!((secs - 4.0).abs() < 0.5, "got {secs}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
