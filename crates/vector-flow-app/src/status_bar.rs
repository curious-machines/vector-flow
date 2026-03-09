/// Transient status bar for non-critical notifications.
///
/// Displays a single message that persists until cleared by a new undo entry
/// or replaced by another message.
pub struct StatusBar {
    /// Current message, if any.
    message: Option<String>,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            message: None,
        }
    }

    /// Set a transient message. Replaces any existing message.
    pub fn show_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
    }

    /// Clear any current message immediately.
    pub fn clear(&mut self) {
        self.message = None;
    }

    /// Returns the message to display (if any).
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}
