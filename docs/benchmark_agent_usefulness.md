# 📊 DocuGraph Benchmark Report: Balanced (Default: ~1000 tokens)

| Metric | Value |
|---|---|
| **Total Queries** | 5 (Passed: 5/5) |
| **Average Token Reduction** | **99.27%** |
| **Average Concept Recall** | **100.0%** |
| **Average Retrieval Latency** | **9.38 ms** |

### Detailed Query Results

| ID | Query | Full Tokens | DocuGraph Tokens | Reduction | Recall | Latency | Provenance |
|---|---|---|---|---|---|---|---|
| `eval-gof-strategy` | Tengo una clase OrderProcessor co... | 106202 | 1046 | **99.0%** | 100% | 14.86ms | ✅ |
| `eval-gof-state-vs-strategy` | Comparar la diferencia entre el p... | 106202 | 442 | **99.6%** | 100% | 10.21ms | ✅ |
| `eval-gof-observer` | ¿Cómo notificar a múltiples ob... | 106202 | 1014 | **99.0%** | 100% | 8.29ms | ✅ |
| `eval-gof-decorator` | ¿Cómo añadir responsabilidades... | 106202 | 986 | **99.1%** | 100% | 6.80ms | ✅ |
| `eval-solid-ocp` | Principio de abierto cerrado OCP ... | 106202 | 393 | **99.6%** | 100% | 6.74ms | ✅ |
