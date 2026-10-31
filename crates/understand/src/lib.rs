//! Turn raw events into verified, linked memories: code index, task grouping, extraction, grounding.

pub mod calls;
pub mod codeindex;
pub mod digest;
pub mod extract;
pub mod link;
pub mod llm;
pub mod notes;
pub mod quality;
pub mod pipeline;
pub mod reconcile;
pub mod system;
pub mod tasks;
pub mod verify;
