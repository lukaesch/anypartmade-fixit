//! Any Part Made Studio (fix it with a photo): the demo client Worker `anypartmade-demo`.
//! Meta SAM 3.1 measures from a phone photo, Muse Spark designs the part,
//! Any Part Made (an external MCP service) checks, quotes and guarantees it.

pub mod agent;
pub mod agent_page;
pub mod cad;
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
pub mod simplify;
pub mod threemf;

#[cfg(all(target_arch = "wasm32", feature = "worker"))]
mod entry;
