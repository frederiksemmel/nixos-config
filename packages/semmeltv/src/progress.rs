use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub season: u32,
    pub episode: u32,
    pub finished: bool,
}

pub type Progress = HashMap<String, Position>;
