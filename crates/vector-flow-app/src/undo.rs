use egui_snarl::Snarl;
use vector_flow_core::graph::Graph;

use crate::id_map::IdMap;
use crate::project::{rebuild_id_map, ProjectSettings};
use crate::ui_node::UiNode;

const MAX_UNDO: usize = 100;

#[derive(Clone)]
struct Snapshot {
    graph: Graph,
    snarl: Snarl<UiNode>,
    settings: ProjectSettings,
    label: String,
}

/// Info captured at begin_frame for computing undo labels by comparison.
#[derive(Clone)]
struct FrameInfo {
    graph_gen: u64,
    node_pos_hash: u64,
    fps_bits: u32,
    canvas_width: u32,
    canvas_height: u32,
    bg_color: Option<[u32; 4]>,
}

impl FrameInfo {
    fn diff_label(&self, other: &FrameInfo) -> String {
        if self.graph_gen != other.graph_gen {
            return "Edit Graph".to_string();
        }
        if self.node_pos_hash != other.node_pos_hash {
            return "Move Nodes".to_string();
        }
        if self.fps_bits != other.fps_bits {
            return "Change FPS".to_string();
        }
        if self.canvas_width != other.canvas_width || self.canvas_height != other.canvas_height {
            return "Resize Canvas".to_string();
        }
        if self.bg_color != other.bg_color {
            return "Change Background".to_string();
        }
        "Edit".to_string()
    }
}

/// Snapshot-based undo/redo with automatic coalescing of continuous edits.
///
/// Coalescing works by tracking a fingerprint (graph generation + node position hash).
/// Only the first frame of a change sequence pushes a snapshot; subsequent frames
/// in the same sequence (e.g. slider drag, node drag) are coalesced into one entry.
/// Coalescing continues as long as the pointer is held down, ensuring that an entire
/// drag gesture produces exactly one undo entry.
pub struct UndoHistory {
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    /// Fingerprint from the end of the previous frame.
    prev_fingerprint: u64,
    /// Whether the previous frame was already part of a change sequence.
    was_changing: bool,
    /// Snapshot taken at the start of the current frame (before mutations).
    /// Only captured when not already in a change sequence.
    pre_frame: Option<Snapshot>,
    /// State info captured alongside pre_frame, for computing diff labels.
    pre_frame_info: Option<FrameInfo>,
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            prev_fingerprint: 0,
            was_changing: false,
            pre_frame: None,
            pre_frame_info: None,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Call at the start of each frame, before any mutations.
    /// Captures a snapshot if we're not in the middle of a continuous edit.
    pub fn begin_frame(&mut self, graph: &Graph, snarl: &Snarl<UiNode>, settings: &ProjectSettings, node_pos_hash: u64) {
        if !self.was_changing {
            self.pre_frame_info = Some(FrameInfo {
                graph_gen: graph.generation(),
                node_pos_hash,
                fps_bits: settings.fps.to_bits(),
                canvas_width: settings.canvas_width,
                canvas_height: settings.canvas_height,
                bg_color: settings.background_color.map(|c| {
                    [c[0].to_bits(), c[1].to_bits(), c[2].to_bits(), c[3].to_bits()]
                }),
            });
            self.pre_frame = Some(Snapshot {
                graph: graph.clone(),
                snarl: snarl.clone(),
                settings: settings.clone(),
                label: String::new(),
            });
        }
    }

    /// Call at the end of each frame, after all mutations.
    /// Detects changes and auto-pushes undo entries.
    /// `current_graph` and `current_settings` are used to compute a descriptive label.
    /// `pointer_down`: true if the primary mouse button is held — prevents premature
    /// coalescing reset during slow drags where the value may not change every frame.
    /// Returns `true` if a new undo entry was pushed this frame.
    pub fn end_frame(&mut self, fingerprint: u64, current_graph: &Graph, current_settings: &ProjectSettings, node_pos_hash: u64, pointer_down: bool) -> bool {
        let current_info = FrameInfo {
            graph_gen: current_graph.generation(),
            node_pos_hash,
            fps_bits: current_settings.fps.to_bits(),
            canvas_width: current_settings.canvas_width,
            canvas_height: current_settings.canvas_height,
            bg_color: current_settings.background_color.map(|c| {
                [c[0].to_bits(), c[1].to_bits(), c[2].to_bits(), c[3].to_bits()]
            }),
        };
        let changed = fingerprint != self.prev_fingerprint;
        let mut pushed = false;

        if changed {
            if !self.was_changing {
                // First frame of a change sequence — push the pre-frame snapshot.
                if let Some(mut snapshot) = self.pre_frame.take() {
                    // Compute label by comparing pre-frame info with current state.
                    let label = if let Some(ref pre_info) = self.pre_frame_info {
                        pre_info.diff_label(&current_info)
                    } else {
                        "Edit".to_string()
                    };
                    snapshot.label = label;
                    self.push_snapshot(snapshot);
                    self.redo_stack.clear();
                    pushed = true;
                }
            }
            self.was_changing = true;
        } else if !pointer_down {
            // Only reset when the pointer is released — during slow drags the
            // value may not change every frame, but we still want to coalesce.
            self.was_changing = false;
        }

        self.prev_fingerprint = fingerprint;
        pushed
    }

    /// Undo: restore the previous state. Returns the restored (graph, snarl, id_map) if successful.
    pub fn undo(&mut self, graph: &Graph, snarl: &Snarl<UiNode>, settings: &ProjectSettings) -> Option<(Graph, Snarl<UiNode>, IdMap, ProjectSettings)> {
        let snapshot = self.undo_stack.pop()?;

        // Push current state to redo with the label of the entry we're undoing.
        self.redo_stack.push(Snapshot {
            graph: graph.clone(),
            snarl: snarl.clone(),
            settings: settings.clone(),
            label: snapshot.label.clone(),
        });

        let id_map = rebuild_id_map(&snapshot.snarl);
        self.was_changing = false;
        self.pre_frame = None;
        self.pre_frame_info = None;
        Some((snapshot.graph, snapshot.snarl, id_map, snapshot.settings))
    }

    /// Redo: restore the next state. Returns the restored (graph, snarl, id_map, settings) if successful.
    pub fn redo(&mut self, graph: &Graph, snarl: &Snarl<UiNode>, settings: &ProjectSettings) -> Option<(Graph, Snarl<UiNode>, IdMap, ProjectSettings)> {
        let snapshot = self.redo_stack.pop()?;

        // Push current state to undo with the label of the entry we're redoing.
        self.push_snapshot(Snapshot {
            graph: graph.clone(),
            snarl: snarl.clone(),
            settings: settings.clone(),
            label: snapshot.label.clone(),
        });

        let id_map = rebuild_id_map(&snapshot.snarl);
        self.was_changing = false;
        self.pre_frame = None;
        self.pre_frame_info = None;
        Some((snapshot.graph, snapshot.snarl, id_map, snapshot.settings))
    }

    /// Label of the top undo entry (what will be undone).
    pub fn undo_label(&self) -> Option<&str> {
        self.undo_stack.last().map(|s| s.label.as_str())
    }

    /// Label of the top redo entry (what will be redone).
    pub fn redo_label(&self) -> Option<&str> {
        self.redo_stack.last().map(|s| s.label.as_str())
    }

    /// Update the fingerprint without triggering change detection.
    /// Call after restoring state from undo/redo.
    pub fn sync_fingerprint(&mut self, fingerprint: u64) {
        self.prev_fingerprint = fingerprint;
        self.was_changing = false;
        self.pre_frame = None;
        self.pre_frame_info = None;
    }

    /// Clear all history (e.g. on project load or new).
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.was_changing = false;
        self.pre_frame = None;
        self.pre_frame_info = None;
        self.prev_fingerprint = 0;
    }

    /// Returns the FPS from the pre-frame snapshot (captured before UI mutations).
    pub fn pre_frame_fps(&self) -> Option<f32> {
        self.pre_frame.as_ref().map(|s| s.settings.fps)
    }

    fn push_snapshot(&mut self, snapshot: Snapshot) {
        if self.undo_stack.len() >= MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> (Graph, Snarl<UiNode>, ProjectSettings) {
        (Graph::new(), Snarl::new(), ProjectSettings::default())
    }

    fn fingerprint(gen: u64, pos: u64) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        gen.hash(&mut h);
        pos.hash(&mut h);
        h.finish()
    }

    #[test]
    fn continuous_drag_coalesces_into_one_entry() {
        // Dragging a node across multiple frames should produce one undo entry.
        let (graph, snarl, settings) = make_state();
        let mut undo = UndoHistory::new();
        undo.sync_fingerprint(fingerprint(0, 0));

        // Frame 1: drag starts.
        undo.begin_frame(&graph, &snarl, &settings, 0);
        undo.end_frame(fingerprint(0, 1), &graph, &settings, 1, true);
        assert_eq!(undo.undo_stack.len(), 1);

        // Frame 2: drag continues.
        undo.begin_frame(&graph, &snarl, &settings, 1);
        undo.end_frame(fingerprint(0, 2), &graph, &settings, 2, true);
        assert_eq!(undo.undo_stack.len(), 1, "continuous drag should coalesce");

        // Frame 3: drag continues.
        undo.begin_frame(&graph, &snarl, &settings, 2);
        undo.end_frame(fingerprint(0, 3), &graph, &settings, 3, true);
        assert_eq!(undo.undo_stack.len(), 1, "still coalescing");
    }

    #[test]
    fn drag_with_unchanged_frames_still_coalesces() {
        // During a slow drag, some frames may have no value change.
        // The entire gesture should still coalesce into one undo entry.
        let (graph, snarl, settings) = make_state();
        let mut undo = UndoHistory::new();
        undo.sync_fingerprint(fingerprint(0, 0));

        // Frame 1: drag starts, value changes.
        undo.begin_frame(&graph, &snarl, &settings, 0);
        undo.end_frame(fingerprint(0, 1), &graph, &settings, 1, true);
        assert_eq!(undo.undo_stack.len(), 1);

        // Frame 2: pointer still down, no value change this frame.
        undo.begin_frame(&graph, &snarl, &settings, 1);
        undo.end_frame(fingerprint(0, 1), &graph, &settings, 1, true);
        assert_eq!(undo.undo_stack.len(), 1, "no change = no new snapshot");

        // Frame 3: value changes again, pointer still down.
        undo.begin_frame(&graph, &snarl, &settings, 1);
        undo.end_frame(fingerprint(0, 2), &graph, &settings, 2, true);
        assert_eq!(undo.undo_stack.len(), 1, "should still coalesce with original drag");

        // Frame 4: pointer released, no change.
        undo.begin_frame(&graph, &snarl, &settings, 2);
        undo.end_frame(fingerprint(0, 2), &graph, &settings, 2, false);
        assert_eq!(undo.undo_stack.len(), 1, "release with no change = no new snapshot");
    }

    #[test]
    fn separate_actions_after_pointer_release_produce_two_entries() {
        // Two distinct gestures (pointer down/up, then down/up again) should
        // produce two separate undo entries.
        let (graph, snarl, settings) = make_state();
        let mut undo = UndoHistory::new();
        undo.sync_fingerprint(fingerprint(0, 0));

        // Gesture 1: drag value.
        undo.begin_frame(&graph, &snarl, &settings, 0);
        undo.end_frame(fingerprint(1, 0), &graph, &settings, 0, true);
        assert_eq!(undo.undo_stack.len(), 1);

        // Release pointer.
        undo.begin_frame(&graph, &snarl, &settings, 0);
        undo.end_frame(fingerprint(1, 0), &graph, &settings, 0, false);

        // Gesture 2: drag node.
        undo.begin_frame(&graph, &snarl, &settings, 0);
        undo.end_frame(fingerprint(1, 1), &graph, &settings, 1, true);
        assert_eq!(undo.undo_stack.len(), 2, "second gesture = second undo entry");
    }
}
