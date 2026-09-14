# 📊 DocuGraph Benchmark Report: Balanced (Default: ~1000 tokens)

| Metric | Value |
|---|---|
| **Total Queries** | 3 (Passed: 2/3) |
| **Average Token Reduction** | **59.96%** |
| **Average Concept Recall** | **91.7%** |
| **Average Retrieval Latency** | **0.74 ms** |

### Detailed Query Results

| ID | Query | Full Tokens | DocuGraph Tokens | Reduction | Recall | Latency | Provenance |
|---|---|---|---|---|---|---|---|
| `eval-01` | What problem does the Strategy pa... | 1511 | 1059 | **29.9%** | 100% | 0.79ms | ✅ |
| `eval-02` | Compare Strategy and State patter... | 1511 | 408 | **73.0%** | 100% | 0.85ms | ✅ |
| `eval-03` | How to resolve merge conflicts in... | 1511 | 348 | **77.0%** | 75% | 0.59ms | ✅ |
