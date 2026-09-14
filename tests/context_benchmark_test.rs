use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use docugraph::benchmark::{
    AggressiveBudgetStrategy, BalancedBudgetStrategy, BenchmarkObserver, BenchmarkQueryResult,
    BenchmarkReport, BenchmarkRunner, ExhaustiveBudgetStrategy, create_benchmark_sample_document,
};

#[test]
fn test_benchmark_questions_loading_and_parsing() {
    let questions = BenchmarkRunner::load_questions_from_file("evaluation/questions.json")
        .expect("Loading evaluation questions must succeed");

    assert!(
        questions.len() >= 3,
        "Expected at least 3 benchmark questions, got {}",
        questions.len()
    );

    let q1 = &questions[0];
    assert_eq!(q1.id, "eval-01");
    assert!(q1.query.contains("Strategy pattern"));
    assert!(!q1.expected_concepts.is_empty());
    assert!(q1.evaluation_criteria.requires_provenance);
}

#[test]
fn test_benchmark_runner_balanced_strategy_metrics() {
    let questions = BenchmarkRunner::load_questions_from_file("evaluation/questions.json")
        .expect("Loading evaluation questions");
    let doc = create_benchmark_sample_document();
    let docs = vec![doc];

    let runner = BenchmarkRunner::new(Box::new(BalancedBudgetStrategy));
    let report = runner
        .run_suite(&questions, &docs)
        .expect("Running benchmark suite");

    assert_eq!(report.total_queries, questions.len());
    // In our rich sample document, all 3 questions match key concepts
    assert!(
        report.passed_queries >= 2,
        "Expected at least 2 passed queries, got {}",
        report.passed_queries
    );

    // Verify token reduction is substantial (> 40% vs full 5-page manual)
    assert!(
        report.avg_token_reduction_percent >= 40.0,
        "Token reduction must be >= 40%, got {:.1}%",
        report.avg_token_reduction_percent
    );

    // Verify recall is high (> 60%)
    assert!(
        report.avg_concept_recall >= 0.6,
        "Average concept recall must be >= 60%, got {:.1}%",
        report.avg_concept_recall * 100.0
    );

    // Verify sub-100ms latency
    assert!(
        report.avg_latency_ms < 100.0,
        "Average latency must be < 100ms, got {:.2}ms",
        report.avg_latency_ms
    );
}

#[test]
fn test_benchmark_budget_strategies_tradeoff() {
    let questions = BenchmarkRunner::load_questions_from_file("evaluation/questions.json")
        .expect("Loading evaluation questions");
    let doc = create_benchmark_sample_document();
    let docs = vec![doc];

    // Aggressive: lowest token count (highest reduction)
    let aggressive_runner = BenchmarkRunner::new(Box::new(AggressiveBudgetStrategy));
    let aggressive_report = aggressive_runner
        .run_suite(&questions, &docs)
        .expect("Aggressive suite");

    // Exhaustive: highest tokens (more surrounding context)
    let exhaustive_runner = BenchmarkRunner::new(Box::new(ExhaustiveBudgetStrategy));
    let exhaustive_report = exhaustive_runner
        .run_suite(&questions, &docs)
        .expect("Exhaustive suite");

    // Aggressive should reduce tokens more than exhaustive
    assert!(
        aggressive_report.avg_token_reduction_percent
            >= exhaustive_report.avg_token_reduction_percent,
        "Aggressive strategy ({:.1}%) should have higher or equal reduction than exhaustive ({:.1}%)",
        aggressive_report.avg_token_reduction_percent,
        exhaustive_report.avg_token_reduction_percent
    );
}

struct TestObserver {
    started: AtomicUsize,
    queries_evaluated: AtomicUsize,
    completed: AtomicUsize,
}

impl BenchmarkObserver for TestObserver {
    fn on_start(&self, total_queries: usize, _strategy: &str) {
        self.started.store(total_queries, Ordering::SeqCst);
    }
    fn on_query_evaluated(&self, _result: &BenchmarkQueryResult) {
        self.queries_evaluated.fetch_add(1, Ordering::SeqCst);
    }
    fn on_completed(&self, _report: &BenchmarkReport) {
        self.completed.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn test_benchmark_observer_pattern() {
    let questions = BenchmarkRunner::load_questions_from_file("evaluation/questions.json")
        .expect("Loading evaluation questions");
    let doc = create_benchmark_sample_document();
    let docs = vec![doc];

    let observer = Arc::new(TestObserver {
        started: AtomicUsize::new(0),
        queries_evaluated: AtomicUsize::new(0),
        completed: AtomicUsize::new(0),
    });

    let mut runner = BenchmarkRunner::new(Box::new(BalancedBudgetStrategy));
    runner.add_observer(observer.clone());

    let _report = runner.run_suite(&questions, &docs).expect("Benchmark run");

    assert_eq!(observer.started.load(Ordering::SeqCst), questions.len());
    assert_eq!(
        observer.queries_evaluated.load(Ordering::SeqCst),
        questions.len()
    );
    assert_eq!(observer.completed.load(Ordering::SeqCst), 1);
}

#[test]
fn test_benchmark_markdown_report_formatting() {
    let questions = BenchmarkRunner::load_questions_from_file("evaluation/questions.json")
        .expect("Loading evaluation questions");
    let doc = create_benchmark_sample_document();
    let docs = vec![doc];

    let runner = BenchmarkRunner::new(Box::new(BalancedBudgetStrategy));
    let report = runner.run_suite(&questions, &docs).expect("Benchmark run");

    let markdown = report.to_markdown_summary();
    assert!(markdown.contains("# 📊 DocuGraph Benchmark Report:"));
    assert!(markdown.contains("| **Total Queries** |"));
    assert!(markdown.contains("| **Average Token Reduction** |"));
    assert!(markdown.contains("| **Average Concept Recall** |"));
    assert!(markdown.contains("| `eval-01` |"));
}
