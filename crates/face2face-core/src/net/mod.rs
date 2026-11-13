//! Networking: the Sender side of the WebSocket transport, the pairing code
//! that guards it, and this computer's addresses. Finding the partner
//! automatically and cable detection are still to come (Milestone 1).

pub mod addresses;
pub mod pairing;
pub mod transport;
