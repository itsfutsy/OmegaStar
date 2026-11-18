//! The phone link: pairing, the connection to the relay, and phone sessions (section 12 of
//! `docs/mobile-companion.md`).

pub mod api;
#[cfg(test)]
mod fixture;
pub mod push;
pub mod store;
pub mod wire;
