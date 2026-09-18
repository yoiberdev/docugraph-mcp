//! Measure retrieval accuracy and abstention against a labelled question set.
//!
//! Two numbers this reports are not, as far as we can find, published by any
//! comparable tool: how often it correctly answers "no evidence" for a question
//! the corpus does not cover, and how often it wrongly refuses one it does. A
//! retrieval engine that never abstains scores 100% on the second and 0% on the
//! first, and one that always abstains scores the reverse, so neither is worth
//! anything alone.
//!
//! Run it against a corpus you have indexed:
//!
//! ```text
//! docugraph index ./corpus/
//! cargo run --release --example labelled_eval
//! ```
//!
//! `benchmarks/README.md` lists the documents the shipped question set was
//! written against and where to download them.

use docugraph::retrieval::{HybridRetriever, HybridWeights};
use docugraph::storage::{DiskCache, DocumentStore};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct QuestionSet {
    answerable: Vec<Answerable>,
    unanswerable: Vec<Unanswerable>,
}

#[derive(Debug, Deserialize)]
struct Answerable {
    document_title: String,
    question: String,
    expected_page_start: u32,
    expected_page_end: u32,
    grounded_in: String,
    paraphrased: bool,
}

#[derive(Debug, Deserialize)]
struct Unanswerable {
    question: String,
}

/// How many hits count as finding the answer.
///
/// Three, because that is what an agent reads before deciding where to look. A
/// top-1 figure flatters a ranking that got lucky and a top-10 figure hides one
/// that did not rank at all.
const TOP_K: usize = 3;

fn main() {
    let raw = std::fs::read_to_string("benchmarks/labelled_questions.json")
        .expect("run this from the repository root, where benchmarks/ lives");
    let set: QuestionSet = serde_json::from_str(&raw).expect("the question set must parse");

    let cache = DiskCache::new(DiskCache::default_dir()).ok();
    let store = DocumentStore::new(cache);
    let docs: Vec<_> = store
        .list_documents()
        .into_iter()
        .filter_map(|m| store.get(&m.id))
        .collect();

    if docs.is_empty() {
        eprintln!("No indexed documents. Run `docugraph index <pdf-or-directory>` first.");
        eprintln!("See benchmarks/README.md for the corpus this set was written against.");
        std::process::exit(1);
    }

    let titles: Vec<&str> = docs.iter().map(|d| d.metadata.title.as_str()).collect();
    let pages: u32 = docs.iter().map(|d| d.metadata.total_pages).sum();
    println!("Corpus: {} documents, {pages} pages", docs.len());
    for t in &titles {
        println!("  - {t}");
    }
    println!();

    let retriever = HybridRetriever::build(&docs, None);

    // Questions whose document is not in this corpus cannot be scored against it.
    let (scorable, skipped): (Vec<&Answerable>, Vec<&Answerable>) = set
        .answerable
        .iter()
        .partition(|q| titles.iter().any(|t| *t == q.document_title));

    let mut found = 0usize;
    let mut refused = 0usize;
    let mut para_total = 0usize;
    let mut para_found = 0usize;
    let mut misses: Vec<(&Answerable, String)> = Vec::new();

    for q in &scorable {
        if q.paraphrased {
            para_total += 1;
        }
        match retriever.search(&q.question, TOP_K, &HybridWeights::DEFAULT) {
            Ok(hits) => {
                let hit = hits.iter().any(|h| {
                    h.snippet_page >= q.expected_page_start && h.snippet_page <= q.expected_page_end
                });
                if hit {
                    found += 1;
                    if q.paraphrased {
                        para_found += 1;
                    }
                } else {
                    let landed = hits
                        .first()
                        .map(|h| format!("{} p.{}", h.title, h.snippet_page))
                        .unwrap_or_else(|| "nothing".to_string());
                    misses.push((q, landed));
                }
            }
            // Refusing a question the corpus does answer is the expensive mistake:
            // the agent is told to rephrase something that was already right.
            Err(_) => {
                refused += 1;
                misses.push((q, "refused as no evidence".to_string()));
            }
        }
    }

    let mut abstained = 0usize;
    let mut leaked: Vec<(&str, String)> = Vec::new();
    for q in &set.unanswerable {
        match retriever.search(&q.question, TOP_K, &HybridWeights::DEFAULT) {
            Err(_) => abstained += 1,
            Ok(hits) => {
                let landed = hits
                    .first()
                    .map(|h| format!("{} p.{}", h.title, h.snippet_page))
                    .unwrap_or_else(|| "empty".to_string());
                leaked.push((q.question.as_str(), landed));
            }
        }
    }

    let n = scorable.len().max(1);
    let u = set.unanswerable.len().max(1);
    println!(
        "RETRIEVAL   answer in top-{TOP_K}     {found}/{}  ({:.0}%)",
        scorable.len(),
        100.0 * found as f64 / n as f64
    );
    println!("            of those, paraphrased  {para_found}/{para_total}");
    println!(
        "            wrongly refused        {refused}/{}  ({:.0}%)",
        scorable.len(),
        100.0 * refused as f64 / n as f64
    );
    println!();
    println!(
        "ABSTENTION  correctly refused      {abstained}/{}  ({:.0}%)",
        set.unanswerable.len(),
        100.0 * abstained as f64 / u as f64
    );
    println!(
        "            answered anyway        {}/{}",
        leaked.len(),
        set.unanswerable.len()
    );

    if !skipped.is_empty() {
        let mut by_doc: HashMap<&str, usize> = HashMap::new();
        for q in &skipped {
            *by_doc.entry(q.document_title.as_str()).or_default() += 1;
        }
        println!("\nNot scored, their document is not indexed here:");
        for (doc, count) in by_doc {
            println!("  {count} questions  {doc}");
        }
    }

    if !misses.is_empty() {
        println!("\nMissed:");
        for (q, landed) in &misses {
            println!("  {}", q.question.chars().take(84).collect::<String>());
            println!(
                "     wanted p.{}-{} ({})",
                q.expected_page_start, q.expected_page_end, q.grounded_in
            );
            println!("     got    {landed}");
        }
    }

    if !leaked.is_empty() {
        println!("\nAnswered though uncovered:");
        for (q, landed) in &leaked {
            println!("  {}", q.chars().take(84).collect::<String>());
            println!("     got  {landed}");
        }
    }
}
