mod app;
mod canvas_panel;
mod export;
mod export_dialog;
mod id_map;
mod project;
mod properties_panel;
mod transport_panel;
mod ui_node;
mod undo;
mod viewer;

use app::VectorFlowApp;

fn main() -> eframe::Result {
    env_logger::init();

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
        Box::new(|cc| Ok(Box::new(VectorFlowApp::new(cc)))),
    )
}
