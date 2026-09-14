//! Model Context Protocol (MCP) server implementation over stdio.
//!
//! Enforces that standard output (stdout) is dedicated exclusively
//! to JSON-RPC protocol frames, while all diagnostic logs go to stderr.

pub mod error;
pub mod server;
pub mod tools;

pub use error::{ToolError, ToolResult};
pub use server::DocuGraphServer;
pub use tools::*;
