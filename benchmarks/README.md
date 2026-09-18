# Labelled benchmark

Two numbers here are not, as far as we could find, published by any comparable tool:

- **Correct abstention** — how often the engine answers "no evidence" for a question the corpus
  does not cover.
- **False abstention** — how often it refuses one the corpus does answer.

Neither is worth anything alone. An engine that never abstains scores perfectly on the second and
zero on the first; one that always abstains scores the reverse. A vector search is the first case
by construction: it returns `k` results whatever you ask it.

## The question set

[`labelled_questions.json`](labelled_questions.json) holds 55 questions over five public
documents:

| | |
|---|---|
| Answerable | 40, eight per document, each with the page range where the answer lives |
| …of those, paraphrased | 38 — the question deliberately avoids the wording of the section title |
| Unanswerable | 15, covered by no document in the corpus |

The page ranges come from each document's own outline, not from a guess. The paraphrasing matters:
a question that reuses its section title is answered by exact keyword match, which measures
nothing. *"Why does my query use a sequential scan when there is an index on that column"* is a
question someone would actually ask; *"what statistics are used by the planner"* is the heading
read back.

The unanswerable questions are deliberately close to the corpus rather than absurd. *"In MySQL 8,
how large should I set `innodb_buffer_pool_size`"* shares almost all its vocabulary with the
PostgreSQL manual and is answered by none of it. Refusing *"what colour is the sky"* would prove
nothing.

## Reproducing it

```bash
# 1. Get the documents. They are public; none is redistributed in this repository.
mkdir -p corpus && cd corpus
curl -LO https://www.postgresql.org/files/documentation/pdf/17/postgresql-17-A4.pdf
curl -LO https://www.open-std.org/jtc1/sc22/wg21/docs/papers/2023/n4950.pdf
curl -LO https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-53r5.pdf
curl -LO https://www.boe.es/buscar/pdf/1995/BOE-A-1995-25444-consolidado.pdf
cd ..

# 2. Index them.
cargo build --release
./target/release/docugraph index ./corpus/

# 3. Score.
cargo run --release --example labelled_eval
```

Questions whose document is not indexed are reported as skipped rather than counted, so a partial
corpus gives a smaller but still honest number. The fifth document in the shipped set,
*Operating Systems: Three Easy Pieces*, has no stable public URL; its eight questions skip unless
you index your own copy.

## Reading the result

`RETRIEVAL` counts a question as found when any of the top 3 hits cites a page inside the expected
range. Three, because that is what an agent reads before deciding where to look: a top-1 figure
flatters a ranking that got lucky, a top-10 figure hides one that did not really rank at all.

`wrongly refused` is the expensive failure mode. The agent is told to rephrase a question that was
already right, and a user who sees that twice stops trusting the refusals — which makes the
correct ones worthless too.

## What this does not measure

Whether the answer on the cited page is *correct*, or whether an agent reading the evidence
reaches the right conclusion. It measures whether the engine put the agent in front of the right
pages and whether it knew when it could not.
