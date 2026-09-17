# Changelog

All notable changes to **DocuGraph MCP** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Nothing has been tagged yet (`git tag -l` is empty), so everything below is still
unreleased. This entry previously claimed a shipped `0.1.0` and listed two tools,
`pattern_get` and `pattern_compare`, that were removed in `7d936a8` and exist
nowhere in `src/`.

### Added
- **MCP Server Core (`rmcp`):** Native stdio transport with strict separation of stdout (JSON-RPC protocol only) and stderr (telemetry with `tracing`). `Cargo.toml` requests `3.3`; the lock currently resolves `3.4.0`.
- **PDF Ingestion Engine (`lopdf`):** Digital PDF parser extracting pages, metadata, and native outline bookmarks (`/Outlines`) with full UTF-16BE decoding and named destination resolution.
- **Document Graph & Provenance:** Hierarchical section tree (H1, H2, H3) and precise evidence citation tracking (`[Doc: ... p. ... § ...]`).
- **Persistence & Caching:** Disk cache (`.docugraph_cache/`, overridable with `DOCUGRAPH_CACHE_DIR`) keyed by SHA-256 content hashes. It is also the handoff between the `index` CLI process and the `serve` process.
- **Okapi BM25 Search Engine:** In-memory inverted index ($k_1=1.2, b=0.75$) with bilingual Spanish/English tokenization and stop-word filtering.
- **Semantic Embeddings & Cosine Similarity:** Modular `EmbeddingProvider` abstraction with an offline deterministic subword n-gram vectorizer (`DeterministicSubwordEmbedding`). It is a character n-gram sketch, useful as a tie-breaker rather than as a meaning model.
- **Hybrid Retrieval:** Lexical IDF-coverage admission followed by score fusion over BM25, cosine similarity and structural heading boosts.
- **Context Budgeting Engine:** Token estimation heuristics (~3.8 chars/token), snippet compaction, and `ContextBudget` boundaries to prevent context explosion.
- **PDF Security:** Decryption of protected documents and detection of invisible (render mode 3) or microscopic text, surfaced to the agent as `[Untrusted Hidden Text: ...]` rather than silently stripped.
- **Scanned Document Detection:** Per-page classification (digital text / scanned image / empty) with an actionable OCR advisory instead of silent empty output.
- **Spatial Reading Order:** Content-stream matrix interpreter with gutter-histogram column detection for multi-column layouts.
- **Table Reconstruction:** Tabular zones rebuilt into GitHub-Flavored Markdown.
- **Multimodal Page Rendering:** Dependency-free rasterizer with a hand-written PNG encoder (RFC 2083 + ISO 3309 CRC-32). It draws text blocks, not glyphs, and interprets no path operators.
- **Links & Outlines:** Hyperlink graph (`/URI`, `/GoTo`, named destinations through both the catalog `/Dests` and the `/Names` name tree).
- **Forms & Tagged PDF:** AcroForm traversal with dotted fully-qualified names and `/FT`/`/Ff` inheritance, plus `StructTreeRoot` detection.
- **Embedded Attachments:** Extraction from `/Names/EmbeddedFiles`, `/Root/AF` and `/FileAttachment` annotations, with stream-id deduplication.
- **Benchmark Engine:** `BudgetStrategy` variants (aggressive, balanced, exhaustive), an observer-based runner and a bilingual synthetic corpus.
- **MCP Tools Suite (15 tools):** `document_ping`, `document_list`, `document_info`, `document_outline`, `document_search`, `document_search_hybrid`, `document_get_section`, `document_get_context`, `document_get_evidence`, `document_read_pages`, `document_render_page`, `document_get_links`, `document_get_forms`, `document_get_attachments`, `document_read_attachment`.
- **CLI Commands:** `docugraph serve`, `index`, `list`, `info`, `search`, `render`, `bench`, `links`, `forms`, `attachments`.
- **Automated Testing & CI:** 85 integration tests passing, `clippy -D warnings` and `cargo fmt --check` gates, and a GitHub Actions CI workflow.

### Fixed
- **Unresolvable `document_id` no longer widens the search to the whole corpus.** `document_search`, `document_search_hybrid`, `document_get_context` and `document_get_evidence` fell back to every indexed document when an explicit id failed to resolve, so a typo returned confidently-cited passages from a different document. They now return an error naming the available ids. A blank id still means "no filter".
- **`DocumentStore::get` resolves a content hash from memory**, so the documented "id or content hash" contract no longer depends on a disk cache being configured.
- **The retrieval engine can now answer "no evidence".** Every section unit used to clear the inclusion threshold on its structural bonus alone, and pages cleared it on embedding noise, so an off-topic question returned section openings with real page numbers and real citations. Admission is now decided on lexical IDF coverage and the fused score is used only for ranking.
- **The structural score compares terms instead of substrings.** `title.contains(word)` matched "con" inside "Conceptos", so any Spanish particle inflated the structural score of any title.
- **The benchmark fixture covers the questions the committed dataset actually asks.** `evaluation/questions.json` is bilingual, but Observer, Decorator and the Open/Closed Principle were only asked in Spanish and the fixture was English-only. The test helper also no longer falls back in silence to that fixture when a hardcoded absolute PDF path is missing.
- **`rmcp::model::ServerInfo` replaced with `ServerConfig`.** The deprecated alias failed the `clippy -D warnings` CI gate.
