/// Transient status bar for non-critical notifications.
///
/// Displays a single message that clears on the next user interaction
/// (mouse click, key press, or after a timeout).
pub struct StatusBar {
    /// Current message, if any.
    message: Option<String>,
    /// Whether the message has been shown for at least one frame
    /// (we clear on the *next* interaction after display).
    shown: bool,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            message: None,
            shown: false,
        }
    }

    /// Set a transient message. Replaces any existing message.
    pub fn show_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
        self.shown = false;
    }

    /// Call once per frame. Clears the message if user interacted since it was shown.
    /// Returns the message to display (if any).
    pub fn update(&mut self, ctx: &egui::Context) -> Option<&str> {
        self.message.as_ref()?;

        // After at least one frame of display, clear on any interaction.
        if self.shown {
            let any_interaction = ctx.input(|i| {
                i.pointer.any_click()
                    || i.pointer.any_pressed()
                    || i.keys_down.iter().next().is_some()
            });
            if any_interaction {
                self.message = None;
                return None;
            }
        }

        self.shown = true;
        self.message.as_deref()
    }
}
