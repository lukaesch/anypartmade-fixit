//! Fix it with a photo: the demo client Worker `anypartmade-demo`.
//! Meta SAM 3.1 measures from a phone photo, Muse Spark designs the part,
//! Any Part Made (an external MCP service) checks, quotes and guarantees it.

pub mod coins;
pub mod knob;
pub mod mcp;
pub mod measure;
pub mod muse;
pub mod page;
pub mod photo;
pub mod render;
pub mod ruler;
pub mod sam;
pub mod see;
pub mod session;
pub mod threemf;

#[cfg(target_arch = "wasm32")]
mod entry;
