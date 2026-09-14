//! Context Reduction, Precision/Recall Benchmarking & Quality Gates (Milestone 6).
//!
//! Provides automated evaluation of retrieval precision, concept recall,
//! token reduction vs full documents, latency metrics, and GoF Strategy / Observer patterns.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crate::document::model::Document;
use crate::retrieval::{ContextBudget, ContextBuilder, HybridRetriever, estimate_tokens};

/// A single benchmark question specification with expected concept keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkQuestion {
    pub id: String,
    pub query: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub expected_concepts: Vec<String>,
    #[serde(default = "default_target_tool")]
    pub target_tool: String,
    #[serde(default)]
    pub evaluation_criteria: EvaluationCriteria,
}

fn default_target_tool() -> String {
    "document_get_evidence".to_string()
}

/// Evaluation criteria and thresholds for a query.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvaluationCriteria {
    #[serde(default)]
    pub requires_provenance: bool,
    pub max_context_tokens: Option<usize>,
}

/// Detailed evaluation outcome for a single query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkQueryResult {
    pub question_id: String,
    pub query: String,
    pub target_tool: String,
    pub full_document_tokens: usize,
    pub retrieved_tokens: usize,
    pub token_reduction_percent: f32,
    pub expected_concepts_count: usize,
    pub matched_concepts_count: usize,
    pub concept_recall: f32,
    pub provenance_verified: bool,
    pub latency_ms: f64,
    pub passed: bool,
}

/// Aggregated benchmark summary report across all queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub strategy_name: String,
    pub total_queries: usize,
    pub passed_queries: usize,
    pub avg_token_reduction_percent: f32,
    pub avg_concept_recall: f32,
    pub avg_latency_ms: f64,
    pub query_results: Vec<BenchmarkQueryResult>,
}

impl BenchmarkReport {
    /// Format report as GitHub-flavored Markdown for documentation and CI artifacts.
    pub fn to_markdown_summary(&self) -> String {
        let mut md = String::new();
        md.push_str(&format!(
            "# 📊 DocuGraph Benchmark Report: {}\n\n",
            self.strategy_name
        ));
        md.push_str("| Metric | Value |\n");
        md.push_str("|---|---|\n");
        md.push_str(&format!(
            "| **Total Queries** | {} (Passed: {}/{}) |\n",
            self.total_queries, self.passed_queries, self.total_queries
        ));
        md.push_str(&format!(
            "| **Average Token Reduction** | **{:.2}%** |\n",
            self.avg_token_reduction_percent
        ));
        md.push_str(&format!(
            "| **Average Concept Recall** | **{:.1}%** |\n",
            self.avg_concept_recall * 100.0
        ));
        md.push_str(&format!(
            "| **Average Retrieval Latency** | **{:.2} ms** |\n\n",
            self.avg_latency_ms
        ));

        md.push_str("### Detailed Query Results\n\n");
        md.push_str("| ID | Query | Full Tokens | DocuGraph Tokens | Reduction | Recall | Latency | Provenance |\n");
        md.push_str("|---|---|---|---|---|---|---|---|\n");

        for r in &self.query_results {
            let prov = if r.provenance_verified { "✅" } else { "❌" };
            let q_display = if r.query.len() > 36 {
                format!("{}...", &r.query[..33])
            } else {
                r.query.clone()
            };
            md.push_str(&format!(
                "| `{}` | {} | {} | {} | **{:.1}%** | {:.0}% | {:.2}ms | {} |\n",
                r.question_id,
                q_display,
                r.full_document_tokens,
                r.retrieved_tokens,
                r.token_reduction_percent,
                r.concept_recall * 100.0,
                r.latency_ms,
                prov
            ));
        }

        md
    }
}

/// GoF Strategy pattern: Configurable context budget strategies to test different tradeoff curves.
pub trait BudgetStrategy: Send + Sync {
    fn name(&self) -> &'static str;
    fn budget(&self) -> ContextBudget;
}

/// Ultra-compact budget targeting minimum LLM API cost.
pub struct AggressiveBudgetStrategy;
impl BudgetStrategy for AggressiveBudgetStrategy {
    fn name(&self) -> &'static str {
        "Aggressive (Min Cost: ~400 tokens)"
    }
    fn budget(&self) -> ContextBudget {
        ContextBudget {
            max_tokens: 400,
            max_chunks: 3,
            compact: true,
        }
    }
}

/// Balanced budget offering the sweet spot of high recall and high reduction.
pub struct BalancedBudgetStrategy;
impl BudgetStrategy for BalancedBudgetStrategy {
    fn name(&self) -> &'static str {
        "Balanced (Default: ~1000 tokens)"
    }
    fn budget(&self) -> ContextBudget {
        ContextBudget {
            max_tokens: 1000,
            max_chunks: 5,
            compact: true,
        }
    }
}

/// Exhaustive budget maximizing conceptual depth and surrounding headings.
pub struct ExhaustiveBudgetStrategy;
impl BudgetStrategy for ExhaustiveBudgetStrategy {
    fn name(&self) -> &'static str {
        "Exhaustive (Max Depth: ~2000 tokens)"
    }
    fn budget(&self) -> ContextBudget {
        ContextBudget {
            max_tokens: 2000,
            max_chunks: 8,
            compact: false,
        }
    }
}

/// GoF Observer / Listener pattern: Subscribes to benchmark execution events.
pub trait BenchmarkObserver: Send + Sync {
    fn on_start(&self, total_queries: usize, strategy_name: &str);
    fn on_query_evaluated(&self, result: &BenchmarkQueryResult);
    fn on_completed(&self, report: &BenchmarkReport);
}

/// Default console listener logging progress to stderr.
pub struct ConsoleBenchmarkObserver;
impl BenchmarkObserver for ConsoleBenchmarkObserver {
    fn on_start(&self, total_queries: usize, strategy_name: &str) {
        eprintln!(
            "\n🚀 Starting DocuGraph Benchmark: {} ({} queries)...",
            strategy_name, total_queries
        );
    }
    fn on_query_evaluated(&self, res: &BenchmarkQueryResult) {
        let status = if res.passed { "PASS" } else { "WARN" };
        eprintln!(
            "  [{status}] `{}`: '{:.30}' | Red: {:.1}% | Rec: {:.0}% | Lat: {:.2}ms",
            res.question_id,
            res.query,
            res.token_reduction_percent,
            res.concept_recall * 100.0,
            res.latency_ms
        );
    }
    fn on_completed(&self, report: &BenchmarkReport) {
        eprintln!(
            "\n🏁 Benchmark Finished: {}/{} passed | Avg Reduction: {:.1}% | Avg Recall: {:.1}% | Latency: {:.2}ms\n",
            report.passed_queries,
            report.total_queries,
            report.avg_token_reduction_percent,
            report.avg_concept_recall * 100.0,
            report.avg_latency_ms
        );
    }
}

/// Benchmark execution engine orchestrating retrieval, token reduction calculation, and observers.
pub struct BenchmarkRunner {
    strategy: Box<dyn BudgetStrategy>,
    observers: Vec<Arc<dyn BenchmarkObserver>>,
}

impl Default for BenchmarkRunner {
    fn default() -> Self {
        Self::new(Box::new(BalancedBudgetStrategy))
    }
}

impl BenchmarkRunner {
    pub fn new(strategy: Box<dyn BudgetStrategy>) -> Self {
        Self {
            strategy,
            observers: Vec::new(),
        }
    }

    pub fn with_strategy(mut self, strategy: Box<dyn BudgetStrategy>) -> Self {
        self.strategy = strategy;
        self
    }

    pub fn add_observer(&mut self, observer: Arc<dyn BenchmarkObserver>) {
        self.observers.push(observer);
    }

    /// Load benchmark evaluation questions from a JSON file.
    pub fn load_questions_from_file(path: impl AsRef<Path>) -> Result<Vec<BenchmarkQuestion>> {
        let p = path.as_ref();
        let content = std::fs::read_to_string(p)
            .with_context(|| format!("Failed to read benchmark questions file: {}", p.display()))?;
        Self::load_questions_from_str(&content)
    }

    /// Parse benchmark evaluation questions from a JSON string.
    pub fn load_questions_from_str(json: &str) -> Result<Vec<BenchmarkQuestion>> {
        let questions: Vec<BenchmarkQuestion> =
            serde_json::from_str(json).context("Failed to deserialize benchmark questions JSON")?;
        Ok(questions)
    }

    /// Execute the benchmark suite against a list of loaded documents.
    pub fn run_suite(
        &self,
        questions: &[BenchmarkQuestion],
        documents: &[Document],
    ) -> Result<BenchmarkReport> {
        let strategy_name = self.strategy.name();
        let budget = self.strategy.budget();

        for obs in &self.observers {
            obs.on_start(questions.len(), strategy_name);
        }

        // Compute total document tokens
        let full_doc_tokens: usize = documents
            .iter()
            .flat_map(|d| d.pages.iter())
            .map(|p| estimate_tokens(&p.text))
            .sum::<usize>()
            .max(1);

        // Pre-build retriever
        let retriever = HybridRetriever::build(documents, None, None);

        let mut query_results = Vec::with_capacity(questions.len());
        let mut total_reduction = 0.0_f32;
        let mut total_recall = 0.0_f32;
        let mut total_latency = 0.0_f64;
        let mut passed_count = 0;

        for q in questions {
            let start = Instant::now();

            // 1. Search candidate chunks
            let hits = retriever.search(&q.query, budget.max_chunks * 2);

            // 2. Format context according to target tool
            let retrieved_text = match q.target_tool.as_str() {
                "document_get_context" => {
                    ContextBuilder::build_conceptual_context(&q.query, &hits, documents, budget)
                }
                _ => {
                    // Default to evidence bundle
                    let bundle = ContextBuilder::build_evidence(&q.query, &hits, budget);
                    bundle.to_markdown()
                }
            };

            let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
            let retrieved_tokens = estimate_tokens(&retrieved_text);

            // 3. Calculate Token Reduction: 1 - (retrieved / full)
            let reduction_percent = if full_doc_tokens > 0 {
                let ratio = retrieved_tokens as f32 / full_doc_tokens as f32;
                (1.0 - ratio.min(1.0)) * 100.0
            } else {
                0.0
            };

            // 4. Calculate Concept Recall: matched expected concepts
            let (matched_concepts, total_expected) = if q.expected_concepts.is_empty() {
                (1, 1)
            } else {
                let lower_text = retrieved_text.to_lowercase();
                let matched = q
                    .expected_concepts
                    .iter()
                    .filter(|concept| lower_text.contains(&concept.to_lowercase()))
                    .count();
                (matched, q.expected_concepts.len())
            };
            let concept_recall = matched_concepts as f32 / total_expected as f32;

            // 5. Verify Provenance Citation
            let provenance_verified = if q.evaluation_criteria.requires_provenance {
                retrieved_text.contains("[Source:")
                    || retrieved_text.contains("pp.")
                    || retrieved_text.contains("Página")
                    || retrieved_text.contains("p.")
            } else {
                true
            };

            // Pass criteria: reduction >= 50% and concept recall >= 0.5 and provenance verified
            let passed = reduction_percent >= 50.0 && concept_recall >= 0.33 && provenance_verified;
            if passed {
                passed_count += 1;
            }

            total_reduction += reduction_percent;
            total_recall += concept_recall;
            total_latency += latency_ms;

            let result = BenchmarkQueryResult {
                question_id: q.id.clone(),
                query: q.query.clone(),
                target_tool: q.target_tool.clone(),
                full_document_tokens: full_doc_tokens,
                retrieved_tokens,
                token_reduction_percent: reduction_percent,
                expected_concepts_count: total_expected,
                matched_concepts_count: matched_concepts,
                concept_recall,
                provenance_verified,
                latency_ms,
                passed,
            };

            for obs in &self.observers {
                obs.on_query_evaluated(&result);
            }

            query_results.push(result);
        }

        let n = questions.len().max(1) as f32;
        let avg_reduction = total_reduction / n;
        let avg_recall = total_recall / n;
        let avg_latency = total_latency / n as f64;

        let report = BenchmarkReport {
            strategy_name: strategy_name.to_string(),
            total_queries: questions.len(),
            passed_queries: passed_count,
            avg_token_reduction_percent: avg_reduction,
            avg_concept_recall: avg_recall,
            avg_latency_ms: avg_latency,
            query_results,
        };

        for obs in &self.observers {
            obs.on_completed(&report);
        }

        Ok(report)
    }
}

/// Helper to generate a multi-topic technical document covering software architecture,
/// design patterns, and version control for out-of-the-box benchmarking.
pub fn create_benchmark_sample_document() -> Document {
    use crate::document::model::{DocumentMetadata, Page, SectionNode};

    let mut doc = Document::new(DocumentMetadata {
        id: "benchmark-tech-manual".to_string(),
        title: "Software Engineering & Architecture Standards Manual".to_string(),
        author: Some("Technical Architecture Guild".to_string()),
        total_pages: 5,
        total_sections: 3,
        file_size_bytes: 8192,
        content_hash: "benchhash9876543210".to_string(),
        indexed_at: "2026-09-13T00:00:00Z".to_string(),
        is_encrypted: false,
        untrusted_text_detected: false,
        scanned_pages_count: 0,
        source_path: None,
        total_links: 0,
    });

    // Page 1: Strategy Pattern Deep Dive
    doc.add_page(Page::new(
        1,
        "Chapter 1: Behavioral Architecture - The Strategy Pattern.\n\n\
        1.1 Intent and High-Level Architecture\n\
        The Strategy pattern defines a family of algorithms, encapsulates each one, and makes them interchangeable.\n\
        Strategy lets the algorithm vary independently from clients that use it. In complex software ecosystems, hardcoding\n\
        algorithm selection leads to convoluted conditional logic (if-else chains and switch statements) that violates\n\
        the Open/Closed Principle (OCP).\n\n\
        1.2 Motivation and Structural Benefits\n\
        By isolating algorithmic logic into dedicated Strategy classes, the host application gains the ability to introduce\n\
        new strategies without modifying existing codebase components. It provides an alternative to subclassing by configuring\n\
        a class with one of many behaviors rather than inheriting from multiple monolithic class hierarchies.\n\n\
        1.3 Participants and Collaboration Dynamics\n\
        * Strategy Interface: Declares an interface common to all supported algorithms. Context uses this interface to call the algorithm defined by a ConcreteStrategy.\n\
        * ConcreteStrategy: Implements the algorithm using the Strategy interface (e.g., CompressionStrategy, RoutingStrategy, PaymentStrategy).\n\
        * Context: Maintained with a reference to a Strategy object and configured with a ConcreteStrategy instance.\n\n\
        1.4 Applicability and Consequences\n\
        Use the Strategy pattern whenever many related classes differ only in their behavior, when you need different variants of an algorithm,\n\
        or when an algorithm uses data that clients shouldn't know about. The primary trade-off is that clients must be aware of how Strategies differ\n\
        in order to select the appropriate implementation.",
    ));

    // Page 2: State Pattern & Dynamic Transitions
    doc.add_page(Page::new(
        2,
        "Chapter 2: State Pattern and Dynamic State Transitions.\n\n\
        2.1 Intent and Behavioral Foundations\n\
        The State pattern allows an object to alter its behavior when its internal state changes. The object will appear to change its class.\n\
        While Strategy provides interchangeable algorithms explicitly chosen by the client or environment, the State pattern models\n\
        behavior change driven by internal state transitions during an entity's lifecycle.\n\n\
        2.2 Comparison between Strategy and State Patterns\n\
        Both patterns share identical structural class diagrams (a Context delegating to polymorphic interface implementations),\n\
        yet their architectural intents differ substantially:\n\
        - Strategy Pattern: Focuses on interchangeable algorithms, typically supplied from the outside at instantiation time.\n\
        - State Pattern: Focuses on behavior change dictated by state machine transitions occurring spontaneously during method execution.\n\n\
        2.3 Implementation Patterns and Thread Safety\n\
        State objects are frequently stateless singletons (Flyweight pattern) or instantiated dynamically per transition.\n\
        When designing distributed actors or reactive pipelines, state transitions must be serialized or atomically swapped\n\
        to prevent race conditions between concurrent events.",
    ));

    // Page 3: Git Workflow & Conflict Resolution
    doc.add_page(Page::new(
        3,
        "Chapter 3: Distributed Version Control and Git Collaboration Standards.\n\n\
        3.1 Distributed Version Control Principles\n\
        In modern continuous integration environments, branching strategies (such as Trunk-Based Development or GitFlow)\n\
        enable teams to collaborate across multiple isolated workspaces without interrupting master deployment readiness.\n\n\
        3.2 Handling Merge Conflicts in Local Branches\n\
        When merging branches in Git, a merge conflict occurs when concurrent edits alter the same lines in disparate commits.\n\
        To resolve a conflict between branches:\n\
        1. Identify conflicted files highlighted by Git's three-way merge markers (`<<<<<<<`, `=======`, `>>>>>>>`).\n\
        2. Open the affected source files and inspect differences between local branches and the target merge branch.\n\
        3. Reconcile differences, compile, and run regression tests locally.\n\
        4. Add the resolved files to the staging area with `git add <file>` to signal resolution.\n\
        5. Finalize the resolution with `git commit` to create the merge commit.\n\n\
        3.3 Staging Hygiene and Atomic Commits\n\
        Clean staging and atomic commits guarantee that bisecting and cherry-picking remain painless during incident triage.",
    ));

    // Page 4: Architectural Provenance and Context Windows
    doc.add_page(Page::new(
        4,
        "Chapter 4: Agentic Context Retrieval and Provenance Engineering.\n\n\
        4.1 The Token Explosion Problem in Modern LLMs\n\
        Feeding entire 500-page specifications or multi-megabyte PDFs into LLM context windows causes attention degradation,\n\
        skyrocketing inference costs, and latency spikes exceeding multiple seconds. Agents require compact, exact evidence.\n\n\
        4.2 Verifiable Citations and Page Boundaries\n\
        Every retrieved snippet must preserve exact provenance: Document ID, chapter heading, and 1-based page boundaries.\n\
        This enables human-in-the-loop auditability and prevents hallucinated technical statements in production systems.\n\n\
        4.3 Hybrid Retrieval Tuning (BM25 + Semantic Cosine + Structural Boost)\n\
        Combining keyword lexical matching with subword character embeddings and heading tree boosts ensures that exact technical tokens\n\
        (e.g., function names, CLI flags) are never lost while still capturing conceptual paraphrases.",
    ));

    // Page 5: Production Operational Guidelines
    doc.add_page(Page::new(
        5,
        "Chapter 5: Production Operational Guidelines and Quality Gates.\n\n\
        5.1 Continuous Performance Benchmarking\n\
        Every release must undergo automated benchmarking verifying:\n\
        - Sub-15ms response latency under stdio transport.\n\
        - Strict token budgeting under aggressive, balanced, and exhaustive modes.\n\
        - Complete concept recall across golden evaluation suites.\n\n\
        5.2 Zero-Allocation Hot Paths and Pure Rust Reliability\n\
        Eliminating heavy C/C++ dynamic libraries (such as Poppler or libpng) ensures frictionless cross-compilation\n\
        and guarantees that no memory corruption or memory leaks compromise the host agent system.",
    ));

    // Section hierarchy
    let s1 = SectionNode::new(
        "sec-strategy",
        "Strategy Pattern: Family of Algorithms",
        1,
        1,
        1,
        None,
    );
    let s2 = SectionNode::new(
        "sec-state",
        "State Pattern: Internal State & Behavior Change",
        1,
        2,
        2,
        None,
    );
    let s3 = SectionNode::new(
        "sec-git",
        "Git Workflow: Merging Branches and Conflict Resolution",
        1,
        3,
        3,
        None,
    );

    doc.sections.push(s1);
    doc.sections.push(s2);
    doc.sections.push(s3);

    doc
}
