use egui::Ui;

use vector_flow_core::types::EvalContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

pub struct TransportState {
    pub eval_ctx: EvalContext,
    pub playback: PlaybackState,
    /// Accumulated wall-clock time since playback started (seconds).
    /// Used to advance frames at the correct rate regardless of UI refresh rate.
    accumulated_time: f64,
    /// Last instant we ticked, for computing elapsed delta.
    last_tick: Option<std::time::Instant>,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            eval_ctx: EvalContext::default(),
            playback: PlaybackState::Stopped,
            accumulated_time: 0.0,
            last_tick: None,
        }
    }
}

impl TransportState {
    /// Advance frames based on real elapsed time. Returns true if frame changed.
    pub fn tick(&mut self) -> bool {
        if self.playback != PlaybackState::Playing {
            self.last_tick = None;
            return false;
        }

        let now = std::time::Instant::now();
        let dt = if let Some(prev) = self.last_tick {
            let raw = now.duration_since(prev).as_secs_f64();
            // Clamp delta to avoid large jumps (e.g. debugger pause, window drag).
            raw.min(0.1)
        } else {
            // First tick after play — no elapsed time yet.
            0.0
        };
        self.last_tick = Some(now);

        self.accumulated_time += dt;
        let frame_duration = 1.0 / self.eval_ctx.fps as f64;

        if self.accumulated_time >= frame_duration {
            // Advance exactly one frame per tick so every frame is displayed,
            // even if the system can't keep up with real-time.
            self.eval_ctx.frame += 1;
            self.eval_ctx.time_secs = self.eval_ctx.frame as f32 / self.eval_ctx.fps;
            self.accumulated_time -= frame_duration;
            // If we've fallen far behind, reset rather than slowly catching up.
            if self.accumulated_time > frame_duration {
                self.accumulated_time = 0.0;
            }
            true
        } else {
            false
        }
    }

    pub fn pause(&mut self) {
        self.playback = PlaybackState::Paused;
        // Reset fractional accumulator so resuming play starts fresh
        // from the current frame without residual time carrying over.
        self.accumulated_time = 0.0;
        self.last_tick = None;
    }

    pub fn rewind(&mut self) {
        self.eval_ctx.frame = 0;
        self.eval_ctx.time_secs = 0.0;
        self.accumulated_time = 0.0;
        self.last_tick = None;
        self.playback = PlaybackState::Stopped;
    }

    pub fn step_forward(&mut self) {
        self.playback = PlaybackState::Paused;
        self.eval_ctx.frame += 1;
        self.eval_ctx.time_secs = self.eval_ctx.frame as f32 / self.eval_ctx.fps;
        self.accumulated_time = 0.0;
        self.last_tick = None;
    }
}

/// Show transport bar. Returns true if time changed.
/// Sets `fps_editing` to true while the FPS widget has keyboard focus.
pub fn show_transport_bar(ui: &mut Ui, state: &mut TransportState, fps_editing: &mut bool) -> bool {
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
                    state.pause();
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

        // Advance time AFTER processing button clicks so that a pause
        // click this frame prevents the tick from advancing the frame.
        time_changed |= state.tick();

        ui.separator();

        // Frame / time display (shown after tick so the counter is accurate).
        ui.label(format!(
            "Frame: {}  Time: {:.2}s",
            state.eval_ctx.frame, state.eval_ctx.time_secs
        ));

        ui.separator();

        ui.label("FPS:");
        let mut fps = state.eval_ctx.fps;
        let response = ui.add(
            egui::DragValue::new(&mut fps)
                .range(1.0..=120.0)
                .speed(0.5)
        );
        *fps_editing = response.has_focus();
        if response.changed() {
            state.eval_ctx.fps = fps;
            // Recompute time_secs from current frame at new fps.
            state.eval_ctx.time_secs = state.eval_ctx.frame as f32 / fps;
            state.accumulated_time = 0.0;
            time_changed = true;
        }
    });

    time_changed
}
