//! The window: file list, settings, preview and the rename button.

use crate::engine::{self, Action, Conflict, Episode, Kind, MatchMode, Op, PlanItem, Settings};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const DEFAULT_EPISODE_FORMAT: &str = "{n} - {s00e00} - {t}{subt}";
const DEFAULT_MOVIE_FORMAT: &str = "{n.colon(' - ')} ({y}){subt}";
const SCREENSHOT_FRAMES: u32 = 120;

/// One file in the list, plus the title typed by hand (movies).
struct FileRow {
    path: PathBuf,
    title: String,
}

pub struct App {
    kind: Kind,
    mode: MatchMode,
    action: Action,
    conflict: Conflict,
    format: String,
    series: String,
    episode_text: String,
    files: Vec<FileRow>,
    log: Vec<String>,
    undo: Vec<Op>,
    // --screenshot support
    shot_path: Option<PathBuf>,
    frames: u32,
    shot_asked: bool,
    shot_slot: Arc<Mutex<Option<egui::ColorImage>>>,
}

impl App {
    pub fn new(shot_path: Option<PathBuf>) -> Self {
        Self {
            kind: Kind::Episode,
            mode: MatchMode::Opportunistic,
            action: Action::Move,
            conflict: Conflict::Skip,
            format: DEFAULT_EPISODE_FORMAT.into(),
            series: String::new(),
            episode_text: String::new(),
            files: Vec::new(),
            log: vec!["Drop video files here, or use Add files.".into()],
            undo: Vec::new(),
            shot_path,
            frames: 0,
            shot_asked: false,
            shot_slot: Arc::new(Mutex::new(None)),
        }
    }

    fn add_path(&mut self, p: PathBuf) {
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                let mut found: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|x| x.is_file()).collect();
                found.sort();
                for f in found {
                    self.push_file(f);
                }
            }
        } else if p.is_file() {
            self.push_file(p);
        }
    }

    fn push_file(&mut self, p: PathBuf) {
        if !self.files.iter().any(|f| f.path == p) {
            self.files.push(FileRow { path: p, title: String::new() });
        }
    }

    fn switch_kind(&mut self, kind: Kind) {
        if self.kind != kind {
            self.kind = kind;
            // Keep the format only if the user changed it; otherwise use the new default.
            let defaults = [DEFAULT_EPISODE_FORMAT, DEFAULT_MOVIE_FORMAT];
            if defaults.contains(&self.format.as_str()) {
                self.format = match kind {
                    Kind::Episode => DEFAULT_EPISODE_FORMAT.into(),
                    Kind::Movie => DEFAULT_MOVIE_FORMAT.into(),
                };
            }
        }
    }

    fn plan(&self) -> Vec<PlanItem> {
        let episodes: Vec<Episode> = engine::parse_episode_list(&self.episode_text);
        let files: Vec<(PathBuf, String)> = self.files.iter().map(|f| (f.path.clone(), f.title.clone())).collect();
        let settings = Settings {
            kind: self.kind,
            mode: self.mode,
            format: &self.format,
            series: &self.series,
            episodes: &episodes,
        };
        engine::plan(&files, &settings)
    }

    fn run_rename(&mut self, plan: &[PlanItem]) {
        let (lines, ops) = engine::execute(plan, self.action, self.conflict);
        self.log = lines;
        if self.action != Action::Test {
            for op in &ops {
                if op.action == Action::Move {
                    if let Some(row) = self.files.iter_mut().find(|f| f.path == op.src) {
                        row.path = op.dst.clone();
                    }
                }
            }
            if !ops.is_empty() {
                self.undo = ops;
            }
        }
    }

    fn run_undo(&mut self) {
        let ops = std::mem::take(&mut self.undo);
        self.log = engine::undo(&ops);
        for op in &ops {
            if op.action == Action::Move {
                if let Some(row) = self.files.iter_mut().find(|f| f.path == op.dst) {
                    row.path = op.src.clone();
                }
            }
        }
    }
}

fn save_png(path: &Path, img: &egui::ColorImage) -> Result<(), String> {
    let [w, h] = img.size;
    let buf = image::RgbaImage::from_raw(w as u32, h as u32, img.as_raw().to_vec())
        .ok_or_else(|| "screenshot size mismatch".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    buf.save(path).map_err(|e| e.to_string())
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Files dropped on the window.
        let dropped: Vec<PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            self.add_path(p);
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Media Renamer");
                ui.separator();
                if ui.button("Add files…").clicked() {
                    if let Some(paths) = rfd::FileDialog::new().set_title("Add files").pick_files() {
                        for p in paths {
                            self.add_path(p);
                        }
                    }
                }
                if ui.button("Clear list").clicked() {
                    self.files.clear();
                }
                let can_undo = !self.undo.is_empty();
                if ui.add_enabled(can_undo, egui::Button::new("Undo last rename")).clicked() {
                    self.run_undo();
                }
            });
            ui.add_space(4.0);
        });

        egui::Panel::left("settings").default_size(320.0).resizable(false).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.strong("These files are");
                ui.horizontal(|ui| {
                    if ui.radio(self.kind == Kind::Episode, "TV episodes").clicked() {
                        self.switch_kind(Kind::Episode);
                    }
                    if ui.radio(self.kind == Kind::Movie, "Movies").clicked() {
                        self.switch_kind(Kind::Movie);
                    }
                });
                ui.add_space(6.0);

                ui.strong("Match mode");
                ui.radio_value(&mut self.mode, MatchMode::Opportunistic, "Best possible match");
                ui.radio_value(&mut self.mode, MatchMode::Strict, "Perfect match or none");
                ui.add_space(6.0);

                if self.kind == Kind::Episode {
                    ui.strong("Series name");
                    ui.text_edit_singleline(&mut self.series);
                    ui.add_space(4.0);
                    ui.strong("Episode list");
                    ui.label("One episode per line, like \"S01E02 Title\" or \"1x02 Title\".");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.episode_text)
                            .desired_rows(7)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(6.0);
                }

                ui.strong("New name format");
                ui.add(egui::TextEdit::singleline(&mut self.format).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    if ui.small_button("Reset to default").clicked() {
                        self.format = match self.kind {
                            Kind::Episode => DEFAULT_EPISODE_FORMAT.into(),
                            Kind::Movie => DEFAULT_MOVIE_FORMAT.into(),
                        };
                    }
                });
                ui.small(match self.kind {
                    Kind::Episode => "{n} series, {s} season, {e} episode, {t} title, {s00e00} S01E02, {sxe} 1x02",
                    Kind::Movie => "{n} title, {y} year. Functions: .colon(' - '), .space('.'), .pad(2)",
                });
                ui.add_space(6.0);

                ui.strong("What to do");
                egui::ComboBox::from_id_salt("action")
                    .selected_text(action_label(self.action))
                    .show_ui(ui, |ui| {
                        for a in [Action::Move, Action::Copy, Action::HardLink, Action::Symlink, Action::Test] {
                            ui.selectable_value(&mut self.action, a, action_label(a));
                        }
                    });
                egui::ComboBox::from_id_salt("conflict")
                    .selected_text(conflict_label(self.conflict))
                    .show_ui(ui, |ui| {
                        for c in [Conflict::Skip, Conflict::Replace, Conflict::AutoNumber] {
                            ui.selectable_value(&mut self.conflict, c, conflict_label(c));
                        }
                    });
                ui.small("If the new name already exists:");
            });
        });

        let plan = self.plan();
        let ready = plan.iter().filter(|p| p.dst.is_some()).count();

        egui::Panel::bottom("bottom").default_size(150.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                let label = match self.action {
                    Action::Test => format!("Check {ready} file(s)"),
                    _ => format!("{} {ready} file(s)", action_verb(self.action)),
                };
                if ui.add_enabled(ready > 0, egui::Button::new(label)).clicked() {
                    self.run_rename(&plan);
                }
                ui.label(format!("{} file(s) in the list", self.files.len()));
            });
            ui.separator();
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for line in &self.log {
                    ui.monospace(line);
                }
            });
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            if self.files.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(120.0);
                    ui.label(egui::RichText::new("Drop video files or folders here").size(22.0));
                    ui.label("Subtitle files next to a video are renamed with it.");
                });
                return;
            }
            // Movie titles can be typed by hand; the preview shows the result.
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("preview").striped(true).num_columns(5).spacing([14.0, 6.0]).show(ui, |ui| {
                    ui.strong("File");
                    ui.strong("Found");
                    ui.strong("Title");
                    ui.strong("New name");
                    ui.strong("Status");
                    ui.end_row();
                    for row in self.files.iter_mut() {
                        ui.label(engine::file_name(&row.path));
                        match plan.iter().find(|p| p.src == row.path) {
                            Some(p) => {
                                ui.label(p.detected.as_str());
                                if self.kind == Kind::Movie {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut row.title)
                                            .desired_width(180.0)
                                            .hint_text(p.title.as_str()),
                                    );
                                } else {
                                    ui.label(p.title.as_str());
                                }
                                let new_name = p.dst.as_ref().map(|d| engine::file_name(d)).unwrap_or_default();
                                ui.label(new_name);
                                let color = if p.dst.is_some() {
                                    egui::Color32::from_rgb(40, 140, 60)
                                } else {
                                    egui::Color32::from_rgb(190, 110, 20)
                                };
                                ui.colored_label(color, p.status.as_str());
                            }
                            None => {
                                ui.label("");
                                ui.label("");
                                ui.label("");
                                ui.label("");
                                ui.label("not a video, subtitle or folder we rename");
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        });

        self.screenshot_step(ui.ctx());
    }
}

impl App {
    /// With --screenshot: after about 120 frames, save the window and quit.
    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot_path.clone() else { return };
        self.frames += 1;
        if self.frames >= SCREENSHOT_FRAMES && !self.shot_asked {
            self.shot_asked = true;
            let slot = self.shot_slot.clone();
            ctx.request_screenshot(move |img| {
                if let Ok(mut s) = slot.lock() {
                    *s = Some((*img).clone());
                }
            });
        }
        let got = self.shot_slot.lock().ok().and_then(|mut s| s.take());
        if let Some(img) = got {
            match save_png(&path, &img) {
                Ok(()) => self.log.push(format!("saved {}", path.display())),
                Err(e) => self.log.push(format!("could not save the screenshot: {e}")),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

fn action_label(a: Action) -> &'static str {
    match a {
        Action::Move => "Move (rename)",
        Action::Copy => "Copy",
        Action::HardLink => "Hard link",
        Action::Symlink => "Symbolic link",
        Action::Test => "Test only (change nothing)",
    }
}

fn action_verb(a: Action) -> &'static str {
    match a {
        Action::Move => "Rename",
        Action::Copy => "Copy",
        Action::HardLink => "Link",
        Action::Symlink => "Link",
        Action::Test => "Check",
    }
}

fn conflict_label(c: Conflict) -> &'static str {
    match c {
        Conflict::Skip => "Skip the file",
        Conflict::Replace => "Replace the old file",
        Conflict::AutoNumber => "Add a number (2), (3)...",
    }
}
