//! Printed page labels from the catalog `/PageLabels` number tree (ISO 32000-1, 12.4.2).
//!
//! PDF page numbers are physical positions (1..=N). Books usually print something else on the
//! page: roman numerals in the front matter, then arabic numbers that restart at 1. When the
//! producer records that scheme in `/PageLabels`, these helpers compute the label of every page.
//! Documents without `/PageLabels` get no labels: the offset is never guessed.

use std::collections::HashSet;

use super::links::{
    MAX_TREE_DEPTH, catalog_dictionary, decode_pdf_text, dereference_array, dereference_dictionary,
};

/// Longest label prefix kept, in characters, so a hostile prefix cannot be multiplied by every page.
const MAX_PREFIX_CHARS: usize = 64;

/// Beyond this many repeated letters (A..Z, AA..ZZ, ...) an alphabetic label falls back to decimal.
const MAX_LETTER_REPEAT: u64 = 16;

/// Numbering style of a page label range (`/S`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageLabelStyle {
    /// `/D`: 1, 2, 3
    Decimal,
    /// `/R`: I, II, III
    UpperRoman,
    /// `/r`: i, ii, iii
    LowerRoman,
    /// `/A`: A..Z, AA..ZZ
    UpperLetters,
    /// `/a`: a..z, aa..zz
    LowerLetters,
}

/// One `/PageLabels` range: it applies from `start_index` (0-based) until the next range starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageLabelRange {
    /// 0-based index of the first page in the range
    pub start_index: u32,
    /// Numbering style; `None` means the label is only the prefix
    pub style: Option<PageLabelStyle>,
    /// Label prefix (`/P`)
    pub prefix: String,
    /// Number of the first page in the range (`/St`, default 1)
    pub first_number: u64,
}

impl PageLabelRange {
    /// Label of the page at 0-based `page_index` (expected to be `>= start_index`).
    pub fn label_for(&self, page_index: u32) -> String {
        let mut label = self.prefix.clone();
        if let Some(style) = self.style {
            let offset = u64::from(page_index.saturating_sub(self.start_index));
            label.push_str(&format_page_number(
                self.first_number.saturating_add(offset),
                style,
            ));
        }
        label
    }
}

/// Read the `/PageLabels` ranges of a document, sorted by start index.
///
/// Returns `None` when the catalog has no `/PageLabels` entry.
pub fn extract_page_label_ranges(doc: &lopdf::Document) -> Option<Vec<PageLabelRange>> {
    let root = catalog_dictionary(doc)?
        .get(b"PageLabels")
        .ok()
        .and_then(|obj| dereference_dictionary(doc, obj))?;

    let mut ranges = Vec::new();
    let mut stack = vec![(root, 0usize)];
    let mut visited = HashSet::new();

    while let Some((node, depth)) = stack.pop() {
        if let Some(nums) = node
            .get(b"Nums")
            .ok()
            .and_then(|obj| dereference_array(doc, obj))
        {
            let (pairs, _) = nums.as_chunks::<2>();
            for [start, value] in pairs {
                let Some(start_index) = start.as_i64().ok().and_then(|n| u32::try_from(n).ok())
                else {
                    continue;
                };
                if let Some(dict) = dereference_dictionary(doc, value) {
                    ranges.push(parse_range(doc, start_index, dict));
                }
            }
        }

        if depth >= MAX_TREE_DEPTH {
            continue;
        }
        if let Some(kids) = node
            .get(b"Kids")
            .ok()
            .and_then(|obj| dereference_array(doc, obj))
        {
            for kid in kids {
                if let Ok(kid_id) = kid.as_reference()
                    && !visited.insert(kid_id)
                {
                    continue;
                }
                if let Some(kid_node) = dereference_dictionary(doc, kid) {
                    stack.push((kid_node, depth + 1));
                }
            }
        }
    }

    ranges.sort_by_key(|range| range.start_index);
    ranges.dedup_by_key(|range| range.start_index);
    Some(ranges)
}

/// Compute the printed label of every page; index 0 is PDF page 1.
///
/// Returns `None` when the document has no `/PageLabels`. Pages before the first range, and
/// ranges that only define an empty prefix, get an empty label.
pub fn extract_page_labels(doc: &lopdf::Document, total_pages: u32) -> Option<Vec<String>> {
    let ranges = extract_page_label_ranges(doc)?;
    Some(labels_for_ranges(&ranges, total_pages))
}

/// Expand sorted label ranges into one label per page.
pub fn labels_for_ranges(ranges: &[PageLabelRange], total_pages: u32) -> Vec<String> {
    let mut labels = Vec::with_capacity(total_pages as usize);
    let mut current: Option<&PageLabelRange> = None;
    let mut next = 0;
    for index in 0..total_pages {
        while let Some(range) = ranges.get(next).filter(|r| r.start_index <= index) {
            current = Some(range);
            next += 1;
        }
        labels.push(current.map(|r| r.label_for(index)).unwrap_or_default());
    }
    labels
}

/// Suffix that shows the printed label next to a physical page number: ` (impresa 17)`.
///
/// Empty when there is no label or when the label is the same number as the PDF page.
pub fn printed_label_suffix(page_number: u32, label: Option<&str>) -> String {
    match label {
        Some(label) if !label.is_empty() && label != page_number.to_string() => {
            format!(" (impresa {label})")
        }
        _ => String::new(),
    }
}

/// Format a page number in the given label style.
///
/// Roman numerals cover 1..=3999 and letters up to [`MAX_LETTER_REPEAT`] repetitions; numbers
/// outside those ranges are written in decimal.
pub fn format_page_number(number: u64, style: PageLabelStyle) -> String {
    let formatted = match style {
        PageLabelStyle::Decimal => None,
        PageLabelStyle::UpperRoman => to_roman(number),
        PageLabelStyle::LowerRoman => to_roman(number).map(|s| s.to_lowercase()),
        PageLabelStyle::UpperLetters => to_letters(number, b'A'),
        PageLabelStyle::LowerLetters => to_letters(number, b'a'),
    };
    formatted.unwrap_or_else(|| number.to_string())
}

fn parse_range(
    doc: &lopdf::Document,
    start_index: u32,
    dict: &lopdf::Dictionary,
) -> PageLabelRange {
    let style = match dict.get(b"S").and_then(|s| s.as_name()) {
        Ok(b"D") => Some(PageLabelStyle::Decimal),
        Ok(b"R") => Some(PageLabelStyle::UpperRoman),
        Ok(b"r") => Some(PageLabelStyle::LowerRoman),
        Ok(b"A") => Some(PageLabelStyle::UpperLetters),
        Ok(b"a") => Some(PageLabelStyle::LowerLetters),
        _ => None,
    };
    let prefix = dict
        .get(b"P")
        .ok()
        .and_then(|obj| doc.dereference(obj).ok())
        .and_then(|(_, obj)| obj.as_str().ok())
        .map(|bytes| {
            decode_pdf_text(bytes)
                .chars()
                .take(MAX_PREFIX_CHARS)
                .collect()
        })
        .unwrap_or_default();
    let first_number = dict
        .get(b"St")
        .ok()
        .and_then(|obj| obj.as_i64().ok())
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| *n >= 1)
        .unwrap_or(1);

    PageLabelRange {
        start_index,
        style,
        prefix,
        first_number,
    }
}

fn to_roman(mut number: u64) -> Option<String> {
    if !(1..=3999).contains(&number) {
        return None;
    }
    const NUMERALS: [(u64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, numeral) in NUMERALS {
        while number >= value {
            out.push_str(numeral);
            number -= value;
        }
    }
    Some(out)
}

fn to_letters(number: u64, base: u8) -> Option<String> {
    let zero_based = number.checked_sub(1)?;
    let repeat = zero_based / 26 + 1;
    if repeat > MAX_LETTER_REPEAT {
        return None;
    }
    let letter = char::from(base + (zero_based % 26) as u8);
    Some(std::iter::repeat_n(letter, repeat as usize).collect())
}
