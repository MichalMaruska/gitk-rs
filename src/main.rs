// gitk-rs — a Rust/egui port of gitk (the Tcl/Tk git repository browser)
// Three-pane layout:
//   Top      : commit graph (DAG) with author / date columns
//   Bottom-L : commit message + unified diff / blame
//   Bottom-R : list of changed files

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod git;
mod graph;
mod ui;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let repo_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".".to_string());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("gitk-rs — {}", repo_path))
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "gitk-rs",
        options,
        Box::new(move |cc| Box::new(ui::GitkApp::new(cc, &repo_path))),
    )
}
