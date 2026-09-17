//! `offsetof` for the LSB s2c `PacketData` bodies this crate decodes, scraped
//! from vendor/server/src/map/packets/s2c/*.h by build.rs.
//!
//! The decoders keep their own named offsets — those carry the citations and
//! the field prose — but every one of them is const-asserted against the module
//! here, so a vendor bump that moves a field fails the build with the field's
//! name instead of failing a live session.

include!(concat!(env!("OUT_DIR"), "/s2c_layout.rs"));
