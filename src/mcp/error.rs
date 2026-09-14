//! Tool-level errors for the DocuGraph MCP server.

use std::fmt;

use rmcp::ErrorData;
use rmcp::handler::server::tool::IntoCallToolResult;
use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock};

/// Error returned by a DocuGraph tool when the request is well formed but cannot be served:
/// an unknown document, section, page or attachment, or an empty page range.
///
/// It is sent as a `CallToolResult` with `isError: true` and the message as text, so agents
/// and hooks can tell a failed lookup apart from a real answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    message: String,
}

impl ToolError {
    /// Create a tool error with a message meant for the calling agent.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Message sent to the client as the text content of the error result.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ToolError {}

impl IntoCallToolResult for ToolError {
    fn into_call_tool_result(self) -> Result<CallToolResponse, ErrorData> {
        CallToolResult::error(vec![ContentBlock::text(self.message)]).into_call_tool_result()
    }
}

/// Result of a DocuGraph MCP tool: the response text, or a tool-level error.
pub type ToolResult = Result<String, ToolError>;
