use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use egui_wgpu::wgpu;
use glam::Vec2;

use vector_flow_render::offscreen::{ExportCamera, OffscreenRenderer};
use vector_flow_render::PreparedScene;

// ---------------------------------------------------------------------------
// Configuration types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    CurrentView,
    FitToContent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VideoFormat {
    PngSequence,
    Mp4,
}

pub struct ImageExportConfig {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub camera: CameraMode,
}

pub struct VideoExportConfig {
    pub output_dir: PathBuf,
    pub mp4_path: Option<PathBuf>,
    pub format: VideoFormat,
    pub width: u32,
    pub height: u32,
    pub camera: CameraMode,
    pub start_frame: u64,
    pub end_frame: u64,
    pub fps: f32,
}

// ---------------------------------------------------------------------------
// Export state (lives on VectorFlowApp)
// ---------------------------------------------------------------------------

#[derive(Default)]
pub enum ExportState {
    #[default]
    Idle,
    ExportingVideo {
        config: VideoExportConfig,
        current_frame: u64,
        renderer: Box<OffscreenRenderer>,
        ffmpeg_child: Option<Child>,
        error: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Single-image export
// ---------------------------------------------------------------------------

pub fn export_canvas_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &PreparedScene,
    config: &ImageExportConfig,
    canvas_center: Vec2,
    canvas_zoom: f32,
) -> Result<(), String> {
    let camera_mode = match config.camera {
        CameraMode::CurrentView => ExportCamera::Explicit {
            center: canvas_center,
            zoom: canvas_zoom,
        },
        CameraMode::FitToContent => ExportCamera::FitToContent,
    };

    let mut renderer = OffscreenRenderer::new(device, config.width, config.height);
    let pixels = renderer.render_scene(device, queue, scene, &camera_mode);

    save_png(&config.path, config.width, config.height, &pixels)
}

// ---------------------------------------------------------------------------
// Video export — frame-at-a-time
// ---------------------------------------------------------------------------

pub fn start_video_export(
    device: &wgpu::Device,
    config: VideoExportConfig,
) -> ExportState {
    let renderer = Box::new(OffscreenRenderer::new(device, config.width, config.height));

    let ffmpeg_child = if config.format == VideoFormat::Mp4 {
        let mp4_path = config
            .mp4_path
            .as_ref()
            .cloned()
            .unwrap_or_else(|| config.output_dir.join("output.mp4"));

        match Command::new("ffmpeg")
            .args([
                "-y",
                "-f", "rawvideo",
                "-pix_fmt", "rgba",
                "-s", &format!("{}x{}", config.width, config.height),
                "-r", &format!("{}", config.fps),
                "-i", "pipe:0",
                "-c:v", "libx264",
                "-pix_fmt", "yuv420p",
            ])
            .arg(mp4_path.to_str().unwrap_or("output.mp4"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => Some(child),
            Err(e) => {
                return ExportState::ExportingVideo {
                    current_frame: config.start_frame,
                    renderer,
                    ffmpeg_child: None,
                    error: Some(format!("Failed to start ffmpeg: {e}")),
                    config,
                };
            }
        }
    } else {
        // PNG sequence — create output directory if needed.
        if let Err(e) = std::fs::create_dir_all(&config.output_dir) {
            return ExportState::ExportingVideo {
                current_frame: config.start_frame,
                renderer,
                ffmpeg_child: None,
                error: Some(format!("Failed to create output directory: {e}")),
                config,
            };
        }
        None
    };

    ExportState::ExportingVideo {
        current_frame: config.start_frame,
        renderer,
        ffmpeg_child,
        error: None,
        config,
    }
}

/// Render and export one frame. Returns `true` when the export is complete.
pub fn export_video_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &PreparedScene,
    state: &mut ExportState,
    canvas_center: Vec2,
    canvas_zoom: f32,
) -> bool {
    let ExportState::ExportingVideo {
        config,
        current_frame,
        renderer,
        ffmpeg_child,
        error,
    } = state
    else {
        return true;
    };

    if error.is_some() {
        return true;
    }

    if *current_frame > config.end_frame {
        return true;
    }

    let camera_mode = match config.camera {
        CameraMode::CurrentView => ExportCamera::Explicit {
            center: canvas_center,
            zoom: canvas_zoom,
        },
        CameraMode::FitToContent => ExportCamera::FitToContent,
    };

    let pixels = renderer.render_scene(device, queue, scene, &camera_mode);

    match config.format {
        VideoFormat::PngSequence => {
            let filename = format!("frame_{:06}.png", *current_frame);
            let path = config.output_dir.join(filename);
            if let Err(e) = save_png(&path, config.width, config.height, &pixels) {
                *error = Some(e);
                return true;
            }
        }
        VideoFormat::Mp4 => {
            if let Some(child) = ffmpeg_child.as_mut() {
                if let Some(stdin) = child.stdin.as_mut() {
                    if let Err(e) = stdin.write_all(&pixels) {
                        *error = Some(format!("Failed to write to ffmpeg: {e}"));
                        return true;
                    }
                }
            }
        }
    }

    *current_frame += 1;
    *current_frame > config.end_frame
}

/// Finish video export — close ffmpeg stdin and wait for it to exit.
pub fn finish_video_export(state: &mut ExportState) -> Option<String> {
    let old = std::mem::take(state);
    if let ExportState::ExportingVideo {
        mut ffmpeg_child,
        error,
        ..
    } = old
    {
        if let Some(ref err) = error {
            return Some(err.clone());
        }
        if let Some(ref mut child) = ffmpeg_child {
            // Drop stdin to signal EOF.
            drop(child.stdin.take());
            match child.wait() {
                Ok(status) if !status.success() => {
                    return Some(format!("ffmpeg exited with status {status}"));
                }
                Err(e) => {
                    return Some(format!("Failed to wait for ffmpeg: {e}"));
                }
                _ => {}
            }
        }
        None
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Graph screenshot (egui framebuffer capture)
// ---------------------------------------------------------------------------

/// Convert an egui `ColorImage` (RGBA, u8) cropped to `crop_rect` into a PNG.
pub fn save_graph_screenshot(
    image: &egui::ColorImage,
    crop_rect: egui::Rect,
    path: &std::path::Path,
) -> Result<(), String> {
    let img_w = image.width() as f32;
    let img_h = image.height() as f32;

    // Clamp crop rect to image bounds.
    let x0 = (crop_rect.min.x.max(0.0) as u32).min(image.width() as u32);
    let y0 = (crop_rect.min.y.max(0.0) as u32).min(image.height() as u32);
    let x1 = (crop_rect.max.x.min(img_w) as u32).min(image.width() as u32);
    let y1 = (crop_rect.max.y.min(img_h) as u32).min(image.height() as u32);

    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);
    if w == 0 || h == 0 {
        return Err("Crop rect has zero area".into());
    }

    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in y0..y1 {
        for col in x0..x1 {
            let color = image[(col as usize, row as usize)];
            pixels.push(color.r());
            pixels.push(color.g());
            pixels.push(color.b());
            pixels.push(color.a());
        }
    }

    save_png(path, w, h, &pixels)
}

// ---------------------------------------------------------------------------
// PNG helper
// ---------------------------------------------------------------------------

fn save_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| "Invalid image dimensions".to_string())?;
    img.save(path)
        .map_err(|e| format!("Failed to save PNG: {e}"))
}
