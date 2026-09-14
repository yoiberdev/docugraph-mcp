//! Multimodal visual page rendering engine for Vision LLMs.
//!
//! Provides pure-Rust page rasterization, RFC-2083 PNG compression,
//! GoF Adapter pattern (`PageRenderer`) and GoF Proxy pattern (`CachedPageRendererProxy`).

pub mod buffer;
pub mod png;
pub mod proxy;
pub mod renderer;

pub use buffer::RgbaImageBuffer;
pub use png::{encode_rgba_to_png, encode_rgba_to_png as encode_png};
pub use proxy::{CachedPageRendererProxy, RenderedPage};
pub use renderer::{NativePageRenderer, PageRenderer};
