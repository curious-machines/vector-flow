mod app;
mod canvas_panel;
mod export;
mod export_dialog;
mod id_map;
mod overlay;
mod project;
mod properties_panel;
mod recent_files;
mod status_bar;
mod style_promote;
mod transport_panel;
mod ui_node;
mod undo;
mod viewer;

use app::VectorFlowApp;

fn main() -> eframe::Result {
    env_logger::init();

    // Accept an optional file path as the first positional argument.
    let file_arg = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .map(|p| {
            // Canonicalize so relative paths work regardless of cwd changes.
            std::fs::canonicalize(&p).unwrap_or(p)
        });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("Vector Flow"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Vector Flow",
        options,
        Box::new(move |cc| Ok(Box::new(VectorFlowApp::new_with_file(cc, file_arg)))),
    )
}
