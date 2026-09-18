<h1 align="center">DocuGraph MCP</h1>

<p align="center">
  <strong>Ask a 3,000-page PDF a question.<br>Get the paragraph, the page number, and nothing else.</strong>
</p>

<p align="center">
  <em>English &middot; <a href="README.es.md">Español</a></em>
</p>

<p align="center">
  <a href="https://github.com/yoiberdev/docugraph-mcp/actions/workflows/ci.yml"><img src="https://github.com/yoiberdev/docugraph-mcp/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/yoiberdev/docugraph-mcp/releases/latest"><img src="https://img.shields.io/github/v/release/yoiberdev/docugraph-mcp?color=brightgreen" alt="Release"></a>
  <img src="https://img.shields.io/badge/tests-126-brightgreen" alt="126 tests">
  <img src="https://img.shields.io/badge/network-none-blue" alt="No network">
  <img src="https://img.shields.io/badge/API%20keys-none-blue" alt="No API keys">
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="MIT"></a>
</p>

---

## The problem

Your agent can already read a PDF. That works fine until the PDF is a manual.

The PostgreSQL 17 documentation is **3,100 pages, about 1.9 million tokens**. The C++ working
draft is another 1.5 million. No context window holds either one, and if it did, you would pay
for all of it to answer one question about `autovacuum`.

The usual fix is a RAG pipeline: a vector database, an embedding model, an API key, a chunking
strategy, and a service that now has your document. That is a lot of moving parts to look
something up in a file already on your disk.

## What DocuGraph does

One binary. It reads the PDF's own structure, indexes it, and answers questions out of it.

```
Question:  "how does the planner decide between a sequential scan and an index scan"

Answer:    380 tokens of evidence, each snippet carrying [PostgreSQL 17.11, p. 683]
Cost:      6x less than opening the section it came from
           0 API calls, 0 bytes over the network
```

|  |  |
|---|---|
| **~400 tokens** | average evidence returned per question, with page citations |
| **6x less** | than opening the section the answer is in |
| **25 ms** | per query across 6,576 indexed pages |
| **9.5 s** | to index the 3,100-page PostgreSQL manual |
| **0** | API keys, network calls, model weights, native dependencies |

<sub>Measured on five public documents: the PostgreSQL 17 manual, the C++ working draft N4950,
NIST SP 800-53r5, the consolidated Spanish criminal code, and <em>Operating Systems: Three Easy
Pieces</em>. 6,576 pages, 4.96 million tokens. Every number is reproducible, see
<a href="#measured">Measured</a>.</sub>

## It knows when it does not know

This is the part most retrieval systems do not have.

A vector search always returns `k` results. Ask it something the document does not cover and it
returns the least bad passages, ranked, formatted, and indistinguishable from real answers. Your
agent then reasons over them.

DocuGraph measures how much of your question's *information* a passage actually carries, using
IDF against the corpus itself, and refuses when nothing clears the bar:

```
> "what is the recommended dose of ibuprofen"

No evidence in the indexed corpus.
Terms absent from every document: ibuprofen, dose, recommended
```

There is no threshold to tune and no constant to recalibrate per document. The bar is the mean
information of your own query terms, so asking about something the corpus lacks raises it rather
than lowering it.

## How it compares

The honest version: this is a deployment-and-provenance tool, not a smarter embedding model.

|  | DocuGraph | Cloud RAG MCPs | Vector DB MCPs | Native PDF reading |
|---|:---:|:---:|:---:|:---:|
| Your document leaves your machine | **never** | uploaded | depends | sent per call |
| Needs an API key | **no** | yes | usually | n/a |
| Works offline | **yes** | no | depends | no |
| LLM calls per query | **0** | 1+ | 0-1 | n/a |
| Says "no evidence" | **yes** | no | no | no |
| Exact page citation | **yes** | varies | rarely | no |
| Install | **one binary** | npm + account | server + model | built in |
| Handles a 3,000-page PDF | **yes** | yes | yes | no |
| Beats a real embedding model on paraphrase | **no** | yes | yes | - |

That last row is not a typo. See [What it does not do](#what-it-does-not-do).

## Quick start

**1. Get the binary.** Download for your platform from the
[latest release](https://github.com/yoiberdev/docugraph-mcp/releases/latest): Windows, Linux and
macOS, x86_64 and arm64. Each asset ships a `.sha256` beside it. No runtime, nothing to install.

```bash
docugraph --version
```

Or build it with `cargo install --git https://github.com/yoiberdev/docugraph-mcp` (Rust 1.88+).

**2. Index your documents.** A separate step on purpose: a 3,000-page manual takes seconds, and
doing that inside a tool call would blow an MCP client's timeout.

```bash
docugraph index ./manuals/postgresql-17.pdf
docugraph index ./manuals/
```

**3. Point your agent at it.**

```json
{
  "mcpServers": {
    "docugraph": {
      "command": "/path/to/docugraph",
      "args": ["serve"]
    }
  }
}
```

Works with any MCP client over stdio: Claude Code, Claude Desktop, Antigravity, Codex, Trae, Kiro.

## The tools

Nine, and the count is deliberate. Every tool a server declares is schema the agent pays for in
**every session, before asking anything**. A server whose argument is that you should spend fewer
tokens cannot show up with thirty tools. These nine cost 1,907 tokens of schema; the fifteen they
replaced cost 2,509.

| Tool | What it does |
|---|---|
| `document_list` | The indexed documents, with page counts and hashes. **Call this first.** |
| `document_info` | Metadata and a preview of the section tree. |
| `document_outline` | The navigation tree with exact page ranges. |
| `document_query` | **The main one.** Answers a question. `mode`: `evidence` (default, cited snippets), `context` (with parent headings), `hits` (the ranked list). |
| `document_get_section` | The full text of one section, within a token budget. |
| `document_read_pages` | Raw pages, when you already know where to look. |
| `document_render_page` | A page as PNG, for vision models. |
| `document_extract` | `kind`: `links`, `forms` (AcroForm fields) or `attachments` (embedded files). |
| `document_read_attachment` | The contents of an embedded file. |

## How it works

**Sections, not blind chunks.** The retrieval unit is a real section of the document, taken from
its `/Outlines` tree or inferred typographically when it has none. A fixed 500-token window cuts
across headings and loses the relationship between a clause and the chapter it belongs to. On the
C++ working draft this yields 3,075 sections with a median of one page each.

**Three signals, one ranking.** Okapi BM25 (k1=1.2, b=0.75) for lexical match, cosine similarity
for approximate matching, and a structural bonus when the query hits a heading.

**Admission is separate from ranking.** A fused relevance score is normalised per query, so its
best hit always looks good whatever you asked. IDF is absolute, so it answers a different
question: *does this passage carry enough of what was asked to count as evidence at all?* That
separation is what makes "no evidence" decidable.

**Citations name the page the text is on**, not the page the section starts on. For a chapter
spanning 40 pages those are rarely the same, and a citation you cannot check is not a citation.

**Nothing repeats.** Documents restate text, and restatements score alike, so a naive ranking
returns the same paragraph three times under three headings. Snippets are taken while walking the
ranking rather than after cutting it, so what comes back is distinct.

## Measured

Everything above, reproducible on your machine. No number here comes from a synthetic fixture.

| Measurement | Result | Corpus |
|---|---|---|
| Evidence per question | 395 tokens avg | 6 questions, 5 documents |
| vs. opening the section it came from | 6.1x less | same |
| Query latency | 25.4 ms | 6,576 pages, 8,780 sections |
| Index the PostgreSQL manual | 9.5 s | 3,100 pages |
| Index NIST SP 800-53r5 | 2.4 s | 492 pages |
| Repeated snippets returned | 0% | was 13% before dedup |
| Retrieval, document's own vocabulary | 4/4 | labelled questions |
| Retrieval, paraphrased questions | 5/6 | labelled questions |
| Tool schema cost | 1,907 tokens | real `tools/list` over stdio |

Corpus:
[PostgreSQL 17](https://www.postgresql.org/files/documentation/pdf/17/postgresql-17-A4.pdf) &middot;
[C++ N4950](https://www.open-std.org/jtc1/sc22/wg21/docs/papers/2023/n4950.pdf) &middot;
[NIST SP 800-53r5](https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-53r5.pdf) &middot;
[Código Penal](https://www.boe.es/buscar/pdf/1995/BOE-A-1995-25444-consolidado.pdf)

## What it does not do

Reading this section is the fastest way to know whether DocuGraph fits your problem.

**No OCR.** Scanned pages are detected and reported as scanned; they are not read. If your PDFs
are photographs of paper, run [OCRmyPDF](https://github.com/ocrmypdf/OCRmyPDF) or
[MinerU](https://github.com/opendatalab/MinerU) first, then index the result here.

**No synonyms.** The semantic channel is a deterministic character n-gram sketch, not an
embedding model. It catches typos, plurals and accents; it does not know that "make objects
interchangeable" means *Strategy*. Measured: **4/4** when the question uses the document's own
vocabulary, **5/6** when it is paraphrased, and the miss landed in an adjacent chapter. A real
embedding model would do better on paraphrase, and would cost the single binary, the offline
guarantee and the absence of model weights. That is the trade this project has chosen.

**Abstention is not a correctness check.** It catches *"the corpus does not cover this."* It does
not catch *"the corpus covers this, elsewhere."* A confident wrong answer drawn from an adjacent
section is still possible.

**Indexing is a separate step.** By design, but it does mean a two-step setup rather than just
pointing an agent at a folder.

**Hostile PDFs are handled, not solved.** Path traversal, unbounded recursion, decompression
bombs and invisible-text injection are tested against, and a page carrying hidden text is
labelled as such. That is not the same as a security guarantee.

## Built for documents that actually exist

Every fix in this repository came from running a real, public document through it and watching
what broke:

- The **C++ working draft** stores every outline title as an indirect reference. All 3,075 of its
  sections indexed as "Untitled Section" until that reference was followed.
- The **Spanish criminal code** writes its titles in PDFDocEncoding. 946 of 953 headings came
  back with `U+FFFD` where their accents belonged, so no accented query could match them.
- **NIST SP 800-53r5** reports 545 pieces of hidden text on a single page. Annotating them
  amplified the page until the process died on a 12.3 GB allocation, twelve minutes in. It now
  ingests in 2.4 seconds.

Each one has a regression test naming the document it came from.

## Contributing

Issues and pull requests welcome. The bar for a change to the retrieval engine is a measurement,
not an argument; see [docs/git-workflow.md](docs/git-workflow.md).

```bash
cargo test --all          # 126 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

## License

MIT &copy; [yoiberdev](https://github.com/yoiberdev)

<p align="center">
  <a href="https://ko-fi.com/yoiberdev"><img src="https://img.shields.io/badge/Ko--fi-Support-F16061?logo=ko-fi&logoColor=white" alt="Ko-fi"></a>
  <a href="https://buymeacoffee.com/yoiber"><img src="https://img.shields.io/badge/Buy%20Me%20a%20Coffee-FFDD00?logo=buy-me-a-coffee&logoColor=black" alt="Buy Me a Coffee"></a>
</p>
