//! Table structure extraction and GitHub Flavored Markdown (GFM) table reconstruction.
//!
//! Implements the GoF Builder pattern (`MarkdownTableBuilder`) and GoF Visitor pattern
//! (`TableStructureVisitor`) to identify tabular text regions in document pages and
//! transform them into clean, structured GFM Markdown tables (`|---|---|`).

use tracing::debug;

/// Alignment for a table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableAlignment {
    #[default]
    Left,
    Center,
    Right,
}

impl TableAlignment {
    pub fn separator_markdown(&self) -> &'static str {
        match self {
            TableAlignment::Left => "|---",
            TableAlignment::Center => "|:---:",
            TableAlignment::Right => "|---:",
        }
    }
}

/// GoF Builder pattern: Builds a GitHub Flavored Markdown (GFM) table string.
#[derive(Debug, Clone, Default)]
pub struct MarkdownTableBuilder {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    alignments: Vec<TableAlignment>,
}

impl MarkdownTableBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set header row for the table.
    pub fn set_headers(&mut self, headers: Vec<String>) -> &mut Self {
        self.headers = headers;
        self
    }

    /// Add a data row to the table.
    pub fn add_row(&mut self, row: Vec<String>) -> &mut Self {
        self.rows.push(row);
        self
    }

    /// Set optional column alignments.
    pub fn set_alignments(&mut self, alignments: Vec<TableAlignment>) -> &mut Self {
        self.alignments = alignments;
        self
    }

    /// Escape cell content so pipes and newlines do not break GFM table formatting.
    fn clean_cell(content: &str) -> String {
        let trimmed = content.trim();
        let without_newlines = trimmed.replace(['\r', '\n'], " ");
        without_newlines.replace('|', "\\|")
    }

    /// Build the formatted GitHub Flavored Markdown (GFM) table string.
    pub fn build(self) -> String {
        let max_cols = self
            .headers
            .len()
            .max(self.rows.iter().map(|r| r.len()).max().unwrap_or(0));

        if max_cols == 0 {
            return String::new();
        }

        let mut output = String::new();

        // 1. Header Row
        output.push('|');
        for col_idx in 0..max_cols {
            let val = self.headers.get(col_idx).map(|s| s.as_str()).unwrap_or("");
            output.push(' ');
            output.push_str(&Self::clean_cell(val));
            output.push_str(" |");
        }
        output.push('\n');

        // 2. Separator Row
        output.push('|');
        for col_idx in 0..max_cols {
            let align = self
                .alignments
                .get(col_idx)
                .copied()
                .unwrap_or(TableAlignment::Left);
            match align {
                TableAlignment::Left => output.push_str("---|"),
                TableAlignment::Center => output.push_str(":---:|"),
                TableAlignment::Right => output.push_str("---:|"),
            }
        }
        output.push('\n');

        // 3. Data Rows
        for row in &self.rows {
            output.push('|');
            for col_idx in 0..max_cols {
                let val = row.get(col_idx).map(|s| s.as_str()).unwrap_or("");
                output.push(' ');
                output.push_str(&Self::clean_cell(val));
                output.push_str(" |");
            }
            output.push('\n');
        }

        output.trim_end().to_string()
    }
}

/// Split a line of text into table cells based on tabs, pipes, or 2+ consecutive spaces.
pub fn split_line_into_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // 1. Pipe-delimited table line (| Col 1 | Col 2 | Col 3 |)
    if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 3 {
        return trimmed
            .trim_matches('|')
            .split('|')
            .map(|s| s.trim().to_string())
            .collect();
    }

    // 2. Tab-delimited table line (Col1 \t Col2 \t Col3)
    if trimmed.contains('\t') {
        let cells: Vec<String> = trimmed
            .split('\t')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if cells.len() >= 2 {
            return cells;
        }
    }

    // 3. Spacing-delimited line (2 or more spaces between columns)
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut space_count = 0;

    for ch in trimmed.chars() {
        if ch == ' ' {
            space_count += 1;
        } else {
            if space_count >= 2 {
                let cell = current.trim();
                if !cell.is_empty() {
                    cells.push(cell.to_string());
                }
                current.clear();
            } else if space_count == 1 {
                current.push(' ');
            }
            space_count = 0;
            current.push(ch);
        }
    }
    let final_cell = current.trim();
    if !final_cell.is_empty() {
        cells.push(final_cell.to_string());
    }

    cells
}

/// Check if a line is a markdown separator line (e.g. `|---|---|` or `+---+---+`)
fn is_table_separator_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed
        .chars()
        .all(|c| c == '-' || c == '|' || c == '+' || c == ':' || c == '=' || c.is_whitespace())
        && trimmed.contains('-')
}

/// Check if a sequence of cell arrays represents a cohesive tabular block.
fn is_valid_table_block(rows: &[Vec<String>]) -> bool {
    // A table must contain at least 2 rows
    if rows.len() < 2 {
        return false;
    }

    // Each row must have between 2 and 8 columns
    let counts: Vec<usize> = rows.iter().map(|r| r.len()).collect();
    if counts.iter().any(|&c| !(2..=8).contains(&c)) {
        return false;
    }

    // Find the most frequent column count (modal column count)
    let mut freq = std::collections::HashMap::new();
    for &c in &counts {
        *freq.entry(c).or_insert(0usize) += 1;
    }
    let (modal_count, modal_freq) = freq
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .unwrap_or((0, 0));

    if modal_count < 2 {
        return false;
    }

    // At least 65% of rows must match the modal column count
    let consistency_ratio = (modal_freq as f32) / (rows.len() as f32);
    if consistency_ratio < 0.65 {
        return false;
    }

    // Disallow blocks where all cells are single words in a plain list (e.g., bullet lists)
    let total_chars: usize = rows
        .iter()
        .flat_map(|r| r.iter())
        .map(|c| c.chars().count())
        .sum();
    if total_chars < 8 {
        return false;
    }

    true
}

/// GoF Visitor pattern: Visits document lines, converts tabular zones to GFM, and preserves regular text.
pub struct TableStructureVisitor;

impl TableStructureVisitor {
    /// Reconstruct tables within text, replacing raw tabular runs with GFM tables.
    pub fn visit_and_reconstruct(text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return String::new();
        }

        let mut output = Vec::new();
        let mut idx = 0;

        while idx < lines.len() {
            let line = lines[idx];

            // If line is empty or a separator, emit and advance
            if line.trim().is_empty() {
                output.push(line.to_string());
                idx += 1;
                continue;
            }

            // Check if this line can start a table
            let first_cells = split_line_into_cells(line);
            if first_cells.len() >= 2 && first_cells.len() <= 8 && !is_table_separator_line(line) {
                // Look ahead to find all contiguous tabular rows
                let mut table_rows: Vec<Vec<String>> = vec![first_cells];
                let mut advance_count = 1;

                while idx + advance_count < lines.len() {
                    let next_line = lines[idx + advance_count];
                    let next_trimmed = next_line.trim();

                    // If it's a separator line (like |---|---| or +---+---+), skip it in row accumulation
                    if is_table_separator_line(next_line) {
                        advance_count += 1;
                        continue;
                    }

                    if next_trimmed.is_empty() {
                        // An empty line marks the end of the table
                        break;
                    }

                    let next_cells = split_line_into_cells(next_line);
                    if next_cells.len() >= 2 && next_cells.len() <= 8 {
                        table_rows.push(next_cells);
                        advance_count += 1;
                    } else {
                        // Non-tabular line encountered
                        break;
                    }
                }

                // If candidate block meets table criteria, build GFM table
                if is_valid_table_block(&table_rows) {
                    debug!(
                        target: "table",
                        rows = table_rows.len(),
                        cols = table_rows[0].len(),
                        "Reconstructed table to Markdown GFM"
                    );

                    let mut builder = MarkdownTableBuilder::new();
                    let mut iter = table_rows.into_iter();
                    if let Some(header_row) = iter.next() {
                        builder.set_headers(header_row);
                    }
                    for row in iter {
                        builder.add_row(row);
                    }

                    let gfm_table = builder.build();
                    output.push(gfm_table);
                    idx += advance_count;
                    continue;
                }
            }

            // Regular line pass-through
            output.push(line.to_string());
            idx += 1;
        }

        output.join("\n")
    }
}

/// Convenience function to process a page's text and reconstruct any embedded tables to GFM.
pub fn reconstruct_tables_in_text(text: &str) -> String {
    TableStructureVisitor::visit_and_reconstruct(text)
}
