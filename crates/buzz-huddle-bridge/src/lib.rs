//! The huddle audio bridge between a relay audio room and one agent voice on Talos.
//!
//! [`join`] admits the agent to the room, [`input`] turns room audio into what the voice may
//! hear, [`output`] plays the voice into the room, [`link`] runs the socket and rejoins, and
//! [`frames`] is the v1 contract with Talos. This crate loads no keys and starts no tasks.
#![deny(unsafe_code)]

pub mod frames;
pub mod input;
pub mod join;
pub mod link;
pub mod output;
