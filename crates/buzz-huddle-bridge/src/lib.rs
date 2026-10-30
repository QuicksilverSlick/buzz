//! The huddle audio bridge between a relay audio room and one agent voice on Talos.
//!
//! [`join`] admits the agent to the room and [`frames`] is the v1 contract with Talos. Room
//! input, output, reconnect and the socket loop come in later slices. This crate loads no keys
//! and starts no tasks.
#![deny(unsafe_code)]

pub mod frames;
pub mod join;
