//! End-to-end Agent Usefulness & Design Patterns Benchmark Test.
//!
//! Validates that an AI coding agent can inspect a real code anti-pattern,
//! query DocuGraph MCP for architectural guidance, and receive compact,
//! evidence-backed context with exact page provenance without saturating the LLM context window.

use docugraph::benchmark::{
    BalancedBudgetStrategy, BenchmarkRunner, create_benchmark_sample_document,
};
use docugraph::document::model::Document;
use docugraph::retrieval::{ContextBudget, ContextBuilder, HybridRetriever, estimate_tokens};

/// The bilingual synthetic manual these tests run against.
///
/// This used to try a copyrighted PDF at a hardcoded absolute path first and fall
/// back to the sample document in silence when it was missing. That is the same
/// antipattern these fixes remove from the server: on any machine but one, the
/// suite quietly measured something other than what it claimed to.
fn get_test_documents() -> Vec<Document> {
    vec![create_benchmark_sample_document()]
}

#[test]
fn test_agent_code_analysis_recommends_strategy_with_compact_evidence() {
    let docs = get_test_documents();
    let retriever = HybridRetriever::build(&docs, None, None);

    // Agent query generated from inspecting a code snippet with carrier if/else branches
    let query = "Tengo una clase OrderProcessor con múltiples condicionales switch para calcular el envío según el transportista. ¿Qué patrón permite definir una familia de algoritmos y hacerlos intercambiables?";
    let hits = retriever
        .search(query, 5)
        .expect("the bilingual sample manual covers this query");

    assert!(!hits.is_empty(), "Must find relevant pattern candidates");

    // Top match should reference Strategy
    let top = &hits[0];
    let top_text_lower = format!("{} {}", top.title, top.snippet).to_lowercase();
    assert!(
        top_text_lower.contains("strategy") || top_text_lower.contains("algoritmo"),
        "Top hit must relate to Strategy or algorithms, got: {}",
        top.snippet
    );

    // Context Builder: Compact conceptual context
    let budget = ContextBudget {
        max_tokens: 600,
        max_chunks: 3,
        compact: true,
    };
    let context = ContextBuilder::build_conceptual_context(query, &hits, &docs, budget);
    let token_count = estimate_tokens(&context);

    // Verify Agent Usefulness criteria
    assert!(
        token_count <= 800,
        "Context tokens must respect budget, got {}",
        token_count
    );
    assert!(
        context.contains("Strategy") || context.contains("strategy"),
        "Context must identify Strategy pattern"
    );
    assert!(
        context.contains("[Doc:")
            || context.contains("[Source:")
            || context.contains("pp.")
            || context.contains("Página"),
        "Context must maintain strict provenance citations"
    );
}

#[test]
fn test_agent_state_vs_strategy_comparison_evidence() {
    let docs = get_test_documents();
    let retriever = HybridRetriever::build(&docs, None, None);

    let query = "Comparar la diferencia entre el patrón Strategy y el patrón State en cuanto a intención y cambio de comportamiento";
    let hits = retriever
        .search(query, 5)
        .expect("the bilingual sample manual covers this query");
    assert!(!hits.is_empty());

    let budget = ContextBudget {
        max_tokens: 800,
        max_chunks: 4,
        compact: true,
    };
    let bundle = ContextBuilder::build_evidence(query, &hits, budget);
    let md = bundle.to_markdown();

    assert!(
        md.contains("Evidencia") || md.contains("Evidence") || md.contains("[Doc:"),
        "Evidence bundle must have formatted citations, got: {}",
        md
    );
    assert!(
        estimate_tokens(&md) <= 1000,
        "Token size must not explode LLM context"
    );
}

#[test]
fn test_agent_observer_event_notification_retrieval() {
    let docs = get_test_documents();
    let retriever = HybridRetriever::build(&docs, None, None);

    let query = "notificar a múltiples objetos suscriptores sobre eventos sin acoplar las clases";
    let hits = retriever
        .search(query, 5)
        .expect("the bilingual sample manual covers this query");
    assert!(!hits.is_empty());

    let budget = ContextBudget {
        max_tokens: 600,
        max_chunks: 3,
        compact: true,
    };
    let context = ContextBuilder::build_conceptual_context(query, &hits, &docs, budget);
    assert!(!context.is_empty());
    assert!(estimate_tokens(&context) <= 800);
}

#[test]
fn test_agent_usefulness_end_to_end_suite() {
    let questions = BenchmarkRunner::load_questions_from_file("evaluation/questions.json")
        .expect("Loading benchmark questions");
    let docs = get_test_documents();

    let runner = BenchmarkRunner::new(Box::new(BalancedBudgetStrategy));
    let report = runner
        .run_suite(&questions, &docs)
        .expect("Running benchmark suite");

    assert_eq!(report.total_queries, questions.len());
    assert!(
        report.passed_queries >= 2,
        "Expected at least 2 queries passed, got {}",
        report.passed_queries
    );
    assert!(
        report.avg_token_reduction_percent >= 40.0,
        "Average token reduction must be >= 40%, got {:.1}%",
        report.avg_token_reduction_percent
    );
    assert!(
        report.avg_latency_ms < 50.0,
        "Average retrieval latency must be < 50ms, got {:.2}ms",
        report.avg_latency_ms
    );
}
