use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkData {
    pub subscription: String,
    pub machine: String,
    pub bookmark: String,
}

pub fn machine_id_key(machine_id: &str) -> String {
    format!("MachineID:{}", machine_id)
}
