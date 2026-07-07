#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod core;
mod editor;
mod game;
mod gfx;
mod llm;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 860.0])
            .with_title("NewOldMaker — HD-2D RPG Engine"),
        ..Default::default()
    };
    eframe::run_native(
        "NewOldMaker",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
