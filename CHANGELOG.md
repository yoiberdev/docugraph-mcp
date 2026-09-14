# Changelog

All notable changes to **DocuGraph MCP** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-09-13

### Added
- **MCP Server Core (`rmcp` 3.3.0):** Native stdio transport with strict separation of stdout (JSON-RPC protocol only) and stderr (telemetry with `tracing`).
- **PDF Ingestion Engine (`lopdf`):** Robust digital PDF parser extracting pages, metadata, and native outline bookmarks (`/Outlines`) with full UTF-16BE decoding and named destination resolution.
- **Document Graph & Provenance:** Hierarchical section tree (H1, H2, H3) and precise evidence citation tracking (`[Doc: ... p. ... § ...]`).
- **Persistence & Caching:** Persistent disk cache (`.docugraph_cache/`) keyed by SHA-256 content hashes, preventing redundant reprocessing.
- **Okapi BM25 Search Engine:** High-performance in-memory inverted index ($k_1=1.2, b=0.75$) with bilingual Spanish/English tokenization and stop-word filtering.
- **Semantic Embeddings & Cosine Similarity:** Modular `EmbeddingProvider` abstraction with deterministic offline subword n-gram vectorization (`DeterministicSubwordEmbedding`).
- **Hybrid Retrieval:** Multi-factor score fusion combining BM25 keyword matching, semantic cosine similarity, and structural heading boosts.
- **Context Budgeting Engine:** Token estimation heuristics (~3.8 chars/token), snippet compaction, and `ContextBudget` boundaries to prevent context explosion.
- **Domain Knowledge Adapter (Design Patterns):** Dynamic extraction of Pattern, Intent, Motivation, Participants, Consequences, and comparative analysis without hardcoded databases.
- **MCP Tools Suite (11 tools):** `document_ping`, `document_list`, `document_info`, `document_outline`, `document_search`, `document_search_hybrid`, `document_get_section`, `document_get_evidence`, `document_read_pages`, `pattern_get`, `pattern_compare`.
- **CLI Commands:** `docugraph serve`, `index`, `list`, `info`, and `search`.
- **Automated Testing & CI:** 20 integration and unit tests passing, strict clippy gates, and GitHub Actions CI workflow.
