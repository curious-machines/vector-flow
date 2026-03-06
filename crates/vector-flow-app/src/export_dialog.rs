use std::path::PathBuf;

use egui::Ui;

use crate::export::{CameraMode, ExportState, VideoFormat};

// ---------------------------------------------------------------------------
// Image export dialog state
// ---------------------------------------------------------------------------

pub struct ImageExportDialog {
    pub open: bool,
    pub width: u32,
    pub height: u32,
    pub camera: CameraMode,
    pub path: PathBuf,
    pub last_error: Option<String>,
    pub last_success: Option<String>,
}

impl Default for ImageExportDialog {
    fn default() -> Self {
        Self {
            open: false,
            width: 1920,
            height: 1080,
            camera: CameraMode::FitToContent,
            path: PathBuf::from("export.png"),
            last_error: None,
            last_success: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Video export dialog state
// ---------------------------------------------------------------------------

pub struct VideoExportDialog {
    pub open: bool,
    pub width: u32,
    pub height: u32,
    pub camera: CameraMode,
    pub format: VideoFormat,
    pub output_dir: PathBuf,
    pub mp4_path: PathBuf,
    pub start_frame: u64,
    pub end_frame: u64,
    pub last_error: Option<String>,
    pub last_success: Option<String>,
}

impl Default for VideoExportDialog {
    fn default() -> Self {
        Self {
            open: false,
            width: 1920,
            height: 1080,
            camera: CameraMode::FitToContent,
            format: VideoFormat::PngSequence,
            output_dir: PathBuf::from("frames"),
            mp4_path: PathBuf::from("output.mp4"),
            start_frame: 0,
            end_frame: 100,
            last_error: None,
            last_success: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Image export dialog UI
// ---------------------------------------------------------------------------

pub enum ImageDialogAction {
    None,
    Export,
}

pub fn show_image_export_dialog(
    ctx: &egui::Context,
    dialog: &mut ImageExportDialog,
) -> ImageDialogAction {
    let mut action = ImageDialogAction::None;

    if !dialog.open {
        return action;
    }

    let mut close_requested = false;
    egui::Window::new("Export Canvas Image")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::Grid::new("img_export_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Width:");
                    let mut w = dialog.width as i64;
                    if ui.add(egui::DragValue::new(&mut w).range(1..=8192)).changed() {
                        dialog.width = w.max(1) as u32;
                    }
                    ui.end_row();

                    ui.label("Height:");
                    let mut h = dialog.height as i64;
                    if ui.add(egui::DragValue::new(&mut h).range(1..=8192)).changed() {
                        dialog.height = h.max(1) as u32;
                    }
                    ui.end_row();

                    ui.label("Camera:");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut dialog.camera, CameraMode::CurrentView, "Current View");
                        ui.radio_value(&mut dialog.camera, CameraMode::FitToContent, "Fit to Content");
                    });
                    ui.end_row();

                    ui.label("Output:");
                    ui.horizontal(|ui| {
                        let display = dialog.path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| dialog.path.to_string_lossy().into_owned());
                        ui.label(&display);
                        if ui.button("Browse...").clicked() {
                            if let Some(mut path) = rfd::FileDialog::new()
                                .set_title("Export Image")
                                .add_filter("PNG Image", &["png"])
                                .save_file()
                            {
                                if path.extension().is_none() {
                                    path.set_extension("png");
                                }
                                dialog.path = path;
                            }
                        }
                    });
                    ui.end_row();
                });

            ui.add_space(8.0);

            show_status_messages(ui, &dialog.last_error, &dialog.last_success);

            ui.horizontal(|ui| {
                if ui.button("Export").clicked() {
                    dialog.last_error = None;
                    dialog.last_success = None;
                    action = ImageDialogAction::Export;
                }
                if ui.button("Close").clicked() {
                    close_requested = true;
                }
            });
        });

    if close_requested {
        dialog.open = false;
    }
    action
}

// ---------------------------------------------------------------------------
// Video export dialog UI
// ---------------------------------------------------------------------------

pub enum VideoDialogAction {
    None,
    Start,
    Cancel,
}

pub fn show_video_export_dialog(
    ctx: &egui::Context,
    dialog: &mut VideoExportDialog,
    export_state: &ExportState,
    fps: f32,
) -> VideoDialogAction {
    let mut action = VideoDialogAction::None;

    if !dialog.open {
        return action;
    }

    let is_exporting = matches!(export_state, ExportState::ExportingVideo { error: None, .. });

    let mut close_requested = false;
    egui::Window::new("Export Canvas Video")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_enabled_ui(!is_exporting, |ui| {
            egui::Grid::new("vid_export_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Width:");
                    let mut w = dialog.width as i64;
                    if ui.add(egui::DragValue::new(&mut w).range(1..=8192)).changed() {
                        dialog.width = w.max(1) as u32;
                    }
                    ui.end_row();

                    ui.label("Height:");
                    let mut h = dialog.height as i64;
                    if ui.add(egui::DragValue::new(&mut h).range(1..=8192)).changed() {
                        dialog.height = h.max(1) as u32;
                    }
                    ui.end_row();

                    ui.label("Camera:");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut dialog.camera, CameraMode::CurrentView, "Current View");
                        ui.radio_value(&mut dialog.camera, CameraMode::FitToContent, "Fit to Content");
                    });
                    ui.end_row();

                    ui.label("Format:");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut dialog.format, VideoFormat::PngSequence, "PNG Sequence");
                        ui.radio_value(&mut dialog.format, VideoFormat::Mp4, "MP4 (ffmpeg)");
                    });
                    ui.end_row();

                    ui.label("Start Frame:");
                    let mut sf = dialog.start_frame as i64;
                    if ui.add(egui::DragValue::new(&mut sf).range(0..=99999)).changed() {
                        dialog.start_frame = sf.max(0) as u64;
                    }
                    ui.end_row();

                    ui.label("End Frame:");
                    let mut ef = dialog.end_frame as i64;
                    if ui.add(egui::DragValue::new(&mut ef).range(0..=99999)).changed() {
                        dialog.end_frame = ef.max(0) as u64;
                    }
                    ui.end_row();

                    ui.label("FPS:");
                    ui.label(format!("{fps:.1}"));
                    ui.end_row();

                    let total = dialog.end_frame.saturating_sub(dialog.start_frame) + 1;
                    let duration = total as f32 / fps;
                    ui.label("Duration:");
                    ui.label(format!("{total} frames ({duration:.2}s)"));
                    ui.end_row();

                    ui.label("Output:");
                    ui.horizontal(|ui| {
                        let display = if dialog.format == VideoFormat::Mp4 {
                            dialog.mp4_path.to_string_lossy().into_owned()
                        } else {
                            dialog.output_dir.to_string_lossy().into_owned()
                        };
                        ui.label(&display);
                        if ui.button("Browse...").clicked() {
                            if dialog.format == VideoFormat::Mp4 {
                                if let Some(mut path) = rfd::FileDialog::new()
                                    .set_title("Export MP4")
                                    .add_filter("MP4 Video", &["mp4"])
                                    .save_file()
                                {
                                    if path.extension().is_none() {
                                        path.set_extension("mp4");
                                    }
                                    dialog.mp4_path = path;
                                }
                            } else if let Some(path) = rfd::FileDialog::new()
                                .set_title("Select Output Directory")
                                .pick_folder()
                            {
                                dialog.output_dir = path;
                            }
                        }
                    });
                    ui.end_row();
                });
            }); // end add_enabled_ui

            ui.add_space(8.0);

            // Progress bar during export.
            if let ExportState::ExportingVideo {
                current_frame,
                config,
                error,
                ..
            } = export_state
            {
                if error.is_none() {
                    let total = config.end_frame.saturating_sub(config.start_frame) + 1;
                    let done = current_frame.saturating_sub(config.start_frame);
                    let progress = done as f32 / total as f32;
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .text(format!("Frame {done}/{total}")),
                    );
                    ui.add_space(4.0);
                }

                if let Some(err) = error {
                    ui.colored_label(egui::Color32::RED, err);
                    ui.add_space(4.0);
                }
            }

            show_status_messages(ui, &dialog.last_error, &dialog.last_success);

            ui.horizontal(|ui| {
                if is_exporting {
                    if ui.button("Cancel").clicked() {
                        action = VideoDialogAction::Cancel;
                    }
                } else {
                    if ui.button("Export").clicked() {
                        dialog.last_error = None;
                        dialog.last_success = None;
                        action = VideoDialogAction::Start;
                    }
                    if ui.button("Close").clicked() {
                        close_requested = true;
                    }
                }
            });
        });

    if close_requested {
        dialog.open = false;
    }
    action
}

fn show_status_messages(ui: &mut Ui, error: &Option<String>, success: &Option<String>) {
    if let Some(err) = error {
        ui.colored_label(egui::Color32::RED, err);
        ui.add_space(4.0);
    }
    if let Some(msg) = success {
        ui.colored_label(egui::Color32::GREEN, msg);
        ui.add_space(4.0);
    }
}
