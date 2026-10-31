use sha2::{Digest, Sha256};

use crate::Split;

/// Fixed for good: a question is in dev when the first byte of sha256(id) is divisible by 5 (~20%).
/// Changing this rule would let test questions leak into tuning, so it never changes.
pub fn is_dev(id: &str) -> bool {
    Sha256::digest(id.as_bytes())[0] % 5 == 0
}

pub fn includes(split: Split, id: &str) -> bool {
    match split {
        Split::Dev => is_dev(id),
        Split::Test => !is_dev(id),
        Split::All => true,
    }
}
