use std::collections::HashMap;

use egui_snarl::NodeId as SnarlNodeId;
use vector_flow_core::types::NodeId as CoreNodeId;

/// Bidirectional mapping between core `NodeId(u64)` and snarl `NodeId(usize)`.
pub struct IdMap {
    core_to_snarl: HashMap<CoreNodeId, SnarlNodeId>,
    snarl_to_core: HashMap<SnarlNodeId, CoreNodeId>,
}

#[allow(dead_code)]
impl IdMap {
    pub fn new() -> Self {
        Self {
            core_to_snarl: HashMap::new(),
            snarl_to_core: HashMap::new(),
        }
    }

    pub fn insert(&mut self, core_id: CoreNodeId, snarl_id: SnarlNodeId) {
        self.core_to_snarl.insert(core_id, snarl_id);
        self.snarl_to_core.insert(snarl_id, core_id);
    }

    pub fn remove_by_core(&mut self, core_id: CoreNodeId) -> Option<SnarlNodeId> {
        if let Some(snarl_id) = self.core_to_snarl.remove(&core_id) {
            self.snarl_to_core.remove(&snarl_id);
            Some(snarl_id)
        } else {
            None
        }
    }

    pub fn remove_by_snarl(&mut self, snarl_id: SnarlNodeId) -> Option<CoreNodeId> {
        if let Some(core_id) = self.snarl_to_core.remove(&snarl_id) {
            self.core_to_snarl.remove(&core_id);
            Some(core_id)
        } else {
            None
        }
    }

    pub fn core_to_snarl(&self, core_id: CoreNodeId) -> Option<SnarlNodeId> {
        self.core_to_snarl.get(&core_id).copied()
    }

    pub fn snarl_to_core(&self, snarl_id: SnarlNodeId) -> Option<CoreNodeId> {
        self.snarl_to_core.get(&snarl_id).copied()
    }
}
