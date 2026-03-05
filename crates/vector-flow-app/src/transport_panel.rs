use egui::Ui;

use vector_flow_core::types::TimeContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

pub struct TransportState {
    pub time_ctx: TimeContext,
    pub playback: PlaybackState,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            time_ctx: TimeContext::default(),
            playback: PlaybackState::Stopped,
        }
    }
}

impl TransportState {
    /// Advance one frame if playing. Returns true if time changed.
    pub fn tick(&mut self) -> bool {
        if self.playback != PlaybackState::Playing {
            return false;
        }
        self.time_ctx.frame += 1;
        self.time_ctx.time_secs = self.time_ctx.frame as f32 / self.time_ctx.fps;
        true
    }

    pub fn rewind(&mut self) {
        self.time_ctx.frame = 0;
        self.time_ctx.time_secs = 0.0;
        self.playback = PlaybackState::Stopped;
    }

    pub fn step_forward(&mut self) {
        self.playback = PlaybackState::Paused;
        self.time_ctx.frame += 1;
        self.time_ctx.time_secs = self.time_ctx.frame as f32 / self.time_ctx.fps;
    }
}

/// Show transport bar. Returns true if time changed.
pub fn show_transport_bar(ui: &mut Ui, state: &mut TransportState) -> bool {
    let mut time_changed = false;

    ui.horizontal(|ui| {
        // Rewind
        if ui.button("|<").on_hover_text("Rewind").clicked() {
            state.rewind();
            time_changed = true;
        }

        // Play / Pause
        match state.playback {
            PlaybackState::Playing => {
                if ui.button("||").on_hover_text("Pause").clicked() {
                    state.playback = PlaybackState::Paused;
                }
            }
            _ => {
                if ui.button(">").on_hover_text("Play").clicked() {
                    state.playback = PlaybackState::Playing;
                }
            }
        }

        // Step
        if ui.button(">|").on_hover_text("Step").clicked() {
            state.step_forward();
            time_changed = true;
        }

        ui.separator();

        // Frame / time display.
        ui.label(format!(
            "Frame: {}  Time: {:.2}s  FPS: {}",
            state.time_ctx.frame, state.time_ctx.time_secs, state.time_ctx.fps as u32
        ));
    });

    time_changed
}
