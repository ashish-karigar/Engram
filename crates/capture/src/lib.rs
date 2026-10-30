//! Observe a project: files, git commits, and uncommitted work saved on a hidden ref.

pub mod capture;
pub mod git;
pub mod ingest;
pub mod scrub;
pub mod snapshot;
pub mod watcher;

pub use capture::{Capture, CaptureOptions, start_capture};
