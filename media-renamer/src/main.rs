mod app;
mod engine;

use eframe::egui;
use std::path::PathBuf;

/// `--screenshot <file.png>`: start normally, save the window after about 120 frames, then quit.
fn screenshot_arg() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--screenshot")?;
    args.get(i + 1).map(PathBuf::from)
}

fn main() -> eframe::Result {
    let shot = screenshot_arg();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Media Renamer")
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Media Renamer",
        options,
        Box::new(move |_cc| {
            let app: Box<dyn eframe::App> = Box::new(app::App::new(shot.clone()));
            Ok(app)
        }),
    )
}
