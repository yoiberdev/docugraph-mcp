# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

We take security seriously in DocuGraph MCP. If you discover a potential security vulnerability, please do NOT create a public GitHub issue.

Instead, please send a detailed report to the maintainers via GitHub Security Advisories or by contacting `yoiberdev` privately.

### Security Guarantees of DocuGraph:
1. **Local Execution & Privacy:** DocuGraph runs entirely locally. It does not transmit document contents, embeddings, or queries to external cloud servers unless explicitly configured.
2. **Path Traversal Prevention:** File paths supplied via CLI or MCP tools are normalized and checked to prevent path traversal attacks.
3. **No Code Execution:** Extracted text from PDF documents is treated as pure textual and structural data. DocuGraph never executes code or macros contained within PDF streams.
4. **Memory Safety:** The core codebase is written in 100% safe Rust, protected against buffer overflows, use-after-free, and concurrency data races.
