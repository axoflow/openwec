use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkData {
    pub subscription: String,
    pub machine: String,
    pub bookmark: String,
}
