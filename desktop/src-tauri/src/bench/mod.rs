//! The Bench: a read-only owner dashboard as a private #bench channel, written
//! only by the Bench service with its own key. `item` is the trust boundary
//! for drop files; the service loop lands in the next build-order steps.
#![cfg_attr(not(test), allow(dead_code))] // stub until run() is wired

use serde::{Deserialize, Serialize};

mod item;

/// A relay event the Bench posted and still owns (a card or the board).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Posted {
    pub event_id: String,
    pub created_at: u64,
    pub hash: String,
    /// Option seeds landed so far.
    #[serde(default)]
    pub seeded: usize,
}

#[cfg(test)]
mod tests;
