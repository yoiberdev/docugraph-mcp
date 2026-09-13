//! Model Context Protocol (MCP) server implementation over stdio.
//!
//! Enforces that standard output (stdout) is dedicated exclusively
//! to JSON-RPC protocol frames, while all diagnostic logs go to stderr.

pub mod server;
pub mod tools;

pub use server::DocuGraphServer;
