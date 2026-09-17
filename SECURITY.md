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
2. **Path Traversal Prevention:** An attachment's file name comes from the PDF, so it is reduced to a bare file name (`safe_output_name`) and the resolved destination is confirmed to be inside the requested directory (`safe_output_path`) before anything is written. Names that are absolute, contain `..`, or are not usable file names are refused and reported, and refusing one attachment does not abort the rest.
3. **No Code Execution:** Extracted text from PDF documents is treated as pure textual and structural data. DocuGraph never executes code or macros contained within PDF streams.
4. **Memory Safety:** The core codebase is written in 100% safe Rust, protected against buffer overflows, use-after-free, and concurrency data races.
5. **Bounded Traversal:** Every recursive walk over PDF structures — outlines, the `/EmbeddedFiles` name tree, AcroForm field trees, and destination indirection — carries a depth cap and a visited set, and the outline's `/Next` sibling chain is iterated rather than recursed. A cyclic or very long chain would otherwise overflow the stack, which aborts the process and cannot be caught, taking the rest of a batch index down with it.
6. **Bounded Decompression:** PDF streams are decompressed through lopdf's limited APIs with a ceiling of 64 MB per stream, so a small file cannot expand into arbitrary memory.

### What DocuGraph does *not* protect against
- Text extracted from a PDF may contain instructions aimed at an LLM. Invisible (render mode 3) and microscopic text is labelled, but that scan is not exhaustive — text hidden by colour, clipping, placement outside the page box, or an optional-content group is not currently detected. Treat document text as untrusted input.
- Attachment contents are returned as-is; they are not scanned.
