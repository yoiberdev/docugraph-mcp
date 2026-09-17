//! Spatial layout analysis and multi-column reading order reconstruction.
//!
//! Handles 2D affine transformations (`cm`, `Tm`, `Td`, `TD`, `T*`) to extract
//! positioned text fragments and accurately reconstruct human reading order across
//! multi-column documents (scientific papers, specifications, newsletters)
//! without interleaved columns.

use lopdf::content::Content;
use lopdf::{Encoding, Object};
use std::collections::BTreeMap;
use tracing::{debug, trace};

/// A 2D bounding box representing an area on a page in PDF user-space units (points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl BoundingBox {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn x_min(&self) -> f32 {
        self.x
    }

    pub fn x_max(&self) -> f32 {
        self.x + self.width
    }

    pub fn y_min(&self) -> f32 {
        self.y
    }

    pub fn y_max(&self) -> f32 {
        self.y + self.height
    }

    pub fn union(&self, other: &BoundingBox) -> BoundingBox {
        let x_min = self.x_min().min(other.x_min());
        let x_max = self.x_max().max(other.x_max());
        let y_min = self.y_min().min(other.y_min());
        let y_max = self.y_max().max(other.y_max());
        BoundingBox {
            x: x_min,
            y: y_min,
            width: (x_max - x_min).max(0.0),
            height: (y_max - y_min).max(0.0),
        }
    }
}

/// A positioned text fragment extracted from a PDF content stream.
#[derive(Debug, Clone)]
pub struct TextFragment {
    pub bbox: BoundingBox,
    pub text: String,
}

/// A line of text containing one or more fragments aligned vertically.
#[derive(Debug, Clone)]
pub struct TextLine {
    pub bbox: BoundingBox,
    pub text: String,
}

/// 2D affine matrix for user-space coordinate transformation according to PDF 32000-1 §8.3.3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix2D {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Matrix2D {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn multiply(&self, other: &Matrix2D) -> Matrix2D {
        Matrix2D {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    pub fn transform_point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            x * self.a + y * self.c + self.e,
            x * self.b + y * self.d + self.f,
        )
    }
}

/// Decode raw bytes using encoding if available, falling back to lossy UTF-8 / UTF-16BE.
fn decode_bytes_with_encoding(bytes: &[u8], encoding: Option<&Encoding>) -> String {
    if let Some(enc) = encoding {
        let mut out = String::new();
        if enc.write_to_string(bytes, &mut out).is_ok() && !out.is_empty() {
            return out;
        }
    }

    if bytes.starts_with(&[0xFE, 0xFF]) {
        let mut u16_chars = Vec::with_capacity((bytes.len().saturating_sub(2)) / 2);
        let mut i = 2;
        while i + 1 < bytes.len() {
            u16_chars.push(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
            i += 2;
        }
        String::from_utf16_lossy(&u16_chars).trim().to_string()
    } else {
        String::from_utf8_lossy(bytes).trim().to_string()
    }
}

/// Heuristic font metrics estimator for Latin proportional typography in digital PDFs.
pub fn estimate_fragment_width(text: &str, font_size: f32) -> f32 {
    let mut w = 0.0;
    for c in text.chars() {
        let factor = match c {
            ' ' => 0.25,
            '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '\"' | '|' => 0.22,
            'i' | 'l' | 'j' | 't' | 'I' | 'f' => 0.28,
            'r' | 's' => 0.38,
            'm' | 'w' | 'M' | 'W' => 0.72,
            _ if c.is_ascii_uppercase() => 0.60,
            _ => 0.48,
        };
        w += factor * font_size;
    }
    w.max(2.0)
}

/// Extract positioned text fragments from a decoded PDF content stream.
pub fn extract_positioned_fragments(
    content: &Content,
    encodings: &BTreeMap<Vec<u8>, Encoding>,
) -> Vec<TextFragment> {
    let mut fragments = Vec::new();

    let mut ctm = Matrix2D::IDENTITY;
    let mut text_matrix = Matrix2D::IDENTITY;
    let mut line_matrix = Matrix2D::IDENTITY;
    let mut font_size: f32 = 12.0;
    let mut leading: f32 = 14.4;
    let mut current_font: Option<Vec<u8>> = None;

    let mut state_stack: Vec<(Matrix2D, f32, f32, Option<Vec<u8>>)> = Vec::new();

    for op in &content.operations {
        match op.operator.as_str() {
            "q" => {
                state_stack.push((ctm, font_size, leading, current_font.clone()));
            }
            "Q" => {
                if let Some((saved_ctm, saved_font_size, saved_leading, saved_font)) =
                    state_stack.pop()
                {
                    ctm = saved_ctm;
                    font_size = saved_font_size;
                    leading = saved_leading;
                    current_font = saved_font;
                }
            }
            "cm" => {
                if op.operands.len() >= 6
                    && let (Ok(a), Ok(b), Ok(c), Ok(d), Ok(e), Ok(f)) = (
                        op.operands[0].as_float(),
                        op.operands[1].as_float(),
                        op.operands[2].as_float(),
                        op.operands[3].as_float(),
                        op.operands[4].as_float(),
                        op.operands[5].as_float(),
                    )
                {
                    let m = Matrix2D { a, b, c, d, e, f };
                    ctm = m.multiply(&ctm);
                }
            }
            "BT" => {
                text_matrix = Matrix2D::IDENTITY;
                line_matrix = Matrix2D::IDENTITY;
            }
            "ET" => {
                text_matrix = Matrix2D::IDENTITY;
                line_matrix = Matrix2D::IDENTITY;
            }
            "Tf" => {
                if let Some(font_name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                    current_font = Some(font_name.to_vec());
                }
                if let Some(size) = op.operands.get(1).and_then(|o| o.as_float().ok()) {
                    font_size = size;
                }
            }
            "TL" => {
                if let Some(lead) = op.operands.first().and_then(|o| o.as_float().ok()) {
                    leading = lead;
                }
            }
            "Tm" => {
                if op.operands.len() >= 6
                    && let (Ok(a), Ok(b), Ok(c), Ok(d), Ok(e), Ok(f)) = (
                        op.operands[0].as_float(),
                        op.operands[1].as_float(),
                        op.operands[2].as_float(),
                        op.operands[3].as_float(),
                        op.operands[4].as_float(),
                        op.operands[5].as_float(),
                    )
                {
                    let m = Matrix2D { a, b, c, d, e, f };
                    text_matrix = m;
                    line_matrix = m;
                }
            }
            "Td" => {
                if op.operands.len() >= 2
                    && let (Ok(tx), Ok(ty)) = (op.operands[0].as_float(), op.operands[1].as_float())
                {
                    let trans = Matrix2D {
                        a: 1.0,
                        b: 0.0,
                        c: 0.0,
                        d: 1.0,
                        e: tx,
                        f: ty,
                    };
                    line_matrix = trans.multiply(&line_matrix);
                    text_matrix = line_matrix;
                }
            }
            "TD" => {
                if op.operands.len() >= 2
                    && let (Ok(tx), Ok(ty)) = (op.operands[0].as_float(), op.operands[1].as_float())
                {
                    leading = -ty;
                    let trans = Matrix2D {
                        a: 1.0,
                        b: 0.0,
                        c: 0.0,
                        d: 1.0,
                        e: tx,
                        f: ty,
                    };
                    line_matrix = trans.multiply(&line_matrix);
                    text_matrix = line_matrix;
                }
            }
            "T*" => {
                let trans = Matrix2D {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 0.0,
                    f: -leading,
                };
                line_matrix = trans.multiply(&line_matrix);
                text_matrix = line_matrix;
            }
            "'" => {
                let trans = Matrix2D {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 0.0,
                    f: -leading,
                };
                line_matrix = trans.multiply(&line_matrix);
                text_matrix = line_matrix;

                let enc = current_font.as_ref().and_then(|f| encodings.get(f));
                if let Some(Object::String(bytes, _)) = op.operands.first() {
                    let text = decode_bytes_with_encoding(bytes, enc);
                    if !text.trim().is_empty() {
                        let eff = text_matrix.multiply(&ctm);
                        let (x, y) = eff.transform_point(0.0, 0.0);
                        let w = estimate_fragment_width(&text, font_size);
                        fragments.push(TextFragment {
                            bbox: BoundingBox::new(x, y, w.max(5.0), font_size.max(5.0)),
                            text,
                        });
                    }
                }
            }
            "\"" => {
                let trans = Matrix2D {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 0.0,
                    f: -leading,
                };
                line_matrix = trans.multiply(&line_matrix);
                text_matrix = line_matrix;

                let enc = current_font.as_ref().and_then(|f| encodings.get(f));
                if let Some(Object::String(bytes, _)) = op.operands.get(2) {
                    let text = decode_bytes_with_encoding(bytes, enc);
                    if !text.trim().is_empty() {
                        let eff = text_matrix.multiply(&ctm);
                        let (x, y) = eff.transform_point(0.0, 0.0);
                        let w = estimate_fragment_width(&text, font_size);
                        fragments.push(TextFragment {
                            bbox: BoundingBox::new(x, y, w.max(5.0), font_size.max(5.0)),
                            text,
                        });
                    }
                }
            }
            "Tj" => {
                let enc = current_font.as_ref().and_then(|f| encodings.get(f));
                if let Some(Object::String(bytes, _)) = op.operands.first() {
                    let text = decode_bytes_with_encoding(bytes, enc);
                    if !text.trim().is_empty() {
                        let eff = text_matrix.multiply(&ctm);
                        let (x, y) = eff.transform_point(0.0, 0.0);
                        let w = estimate_fragment_width(&text, font_size);
                        fragments.push(TextFragment {
                            bbox: BoundingBox::new(x, y, w.max(5.0), font_size.max(5.0)),
                            text: text.clone(),
                        });

                        // Advance text matrix horizontally
                        let advance = Matrix2D {
                            a: 1.0,
                            b: 0.0,
                            c: 0.0,
                            d: 1.0,
                            e: w,
                            f: 0.0,
                        };
                        text_matrix = advance.multiply(&text_matrix);
                    }
                }
            }
            "TJ" => {
                let enc = current_font.as_ref().and_then(|f| encodings.get(f));
                if let Some(Object::Array(items)) = op.operands.first() {
                    let mut combined_text = String::new();
                    for item in items {
                        match item {
                            Object::String(bytes, _) => {
                                let part = decode_bytes_with_encoding(bytes, enc);
                                combined_text.push_str(&part);
                            }
                            // A large negative kerning adjustment is a word break.
                            Object::Integer(i) if *i < -50 && !combined_text.ends_with(' ') => {
                                combined_text.push(' ');
                            }
                            Object::Real(r) if *r < -50.0 && !combined_text.ends_with(' ') => {
                                combined_text.push(' ');
                            }
                            _ => {}
                        }
                    }

                    let clean = combined_text.trim();
                    if !clean.is_empty() {
                        let eff = text_matrix.multiply(&ctm);
                        let (x, y) = eff.transform_point(0.0, 0.0);
                        let w = estimate_fragment_width(clean, font_size);
                        fragments.push(TextFragment {
                            bbox: BoundingBox::new(x, y, w.max(5.0), font_size.max(5.0)),
                            text: clean.to_string(),
                        });

                        // Advance text matrix horizontally
                        let advance = Matrix2D {
                            a: 1.0,
                            b: 0.0,
                            c: 0.0,
                            d: 1.0,
                            e: w,
                            f: 0.0,
                        };
                        text_matrix = advance.multiply(&text_matrix);
                    }
                }
            }
            _ => {}
        }
    }

    fragments
}

/// GoF Strategy pattern: Strategy for determining reading order of text fragments on a page.
pub trait ReadingOrderStrategy {
    fn is_multi_column(&self) -> bool;
    fn reconstruct_text(&self, fragments: &[TextFragment]) -> String;
}

/// Strategy for single-column documents: groups lines and orders top-to-bottom ($Y$ descending).
pub struct SingleColumnFlow;

impl ReadingOrderStrategy for SingleColumnFlow {
    fn is_multi_column(&self) -> bool {
        false
    }

    fn reconstruct_text(&self, fragments: &[TextFragment]) -> String {
        let lines = group_fragments_into_lines(fragments);
        lines
            .into_iter()
            .map(|l| l.text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Strategy for multi-column documents: segments into vertical column slices, ordering column-by-column.
pub struct MultiColumnSpatialFlow {
    /// X coordinate split boundaries that separate columns
    pub split_x_positions: Vec<f32>,
}

impl ReadingOrderStrategy for MultiColumnSpatialFlow {
    fn is_multi_column(&self) -> bool {
        true
    }

    fn reconstruct_text(&self, fragments: &[TextFragment]) -> String {
        if fragments.is_empty() {
            return String::new();
        }

        // Identify content bounding box
        let content_x_min = fragments
            .iter()
            .map(|f| f.bbox.x_min())
            .fold(f32::INFINITY, f32::min);
        let content_x_max = fragments
            .iter()
            .map(|f| f.bbox.x_max())
            .fold(f32::NEG_INFINITY, f32::max);
        let content_width = (content_x_max - content_x_min).max(1.0);

        // Separate spanning elements (headers/footers spanning across columns) from column-bound elements
        let mut headers = Vec::new();
        let mut footers = Vec::new();
        let mut column_fragments = Vec::new();

        // Calculate Y range of column candidate fragments
        let mut col_cand_y_max = f32::NEG_INFINITY;
        let mut col_cand_y_min = f32::INFINITY;

        for f in fragments {
            let is_spanning = f.bbox.width > (content_width * 0.55);
            if !is_spanning {
                col_cand_y_max = col_cand_y_max.max(f.bbox.y_max());
                col_cand_y_min = col_cand_y_min.min(f.bbox.y_min());
            }
        }

        for f in fragments {
            let is_spanning = f.bbox.width > (content_width * 0.55);
            if is_spanning {
                // In PDF coordinates, top of page has highest Y
                if f.bbox.y >= col_cand_y_max - 5.0 {
                    headers.push(f.clone());
                } else if f.bbox.y <= col_cand_y_min + 5.0 {
                    footers.push(f.clone());
                } else {
                    column_fragments.push(f.clone());
                }
            } else {
                column_fragments.push(f.clone());
            }
        }

        let mut output = String::new();

        // 1. Spanning Headers (top of page)
        if !headers.is_empty() {
            let header_lines = group_fragments_into_lines(&headers);
            for line in header_lines {
                output.push_str(&line.text);
                output.push('\n');
            }
            if !output.is_empty() && !output.ends_with("\n\n") {
                output.push('\n');
            }
        }

        // 2. Columns: Partition column fragments into N slices by split_x_positions
        let num_columns = self.split_x_positions.len() + 1;
        let mut columns: Vec<Vec<TextFragment>> = vec![Vec::new(); num_columns];

        for f in column_fragments {
            let mut assigned_col = 0;
            for (idx, &split_x) in self.split_x_positions.iter().enumerate() {
                if f.bbox.x >= split_x {
                    assigned_col = idx + 1;
                }
            }
            columns[assigned_col].push(f);
        }

        // Output column by column: Col 0 (top-to-bottom), Col 1 (top-to-bottom), etc.
        for (col_idx, col_frags) in columns.into_iter().enumerate() {
            if col_frags.is_empty() {
                continue;
            }
            let lines = group_fragments_into_lines(&col_frags);
            for line in lines {
                output.push_str(&line.text);
                output.push('\n');
            }
            if col_idx + 1 < num_columns && !output.ends_with("\n\n") {
                output.push('\n');
            }
        }

        // 3. Spanning Footers (bottom of page)
        if !footers.is_empty() {
            if !output.ends_with("\n\n") {
                output.push('\n');
            }
            let footer_lines = group_fragments_into_lines(&footers);
            for line in footer_lines {
                output.push_str(&line.text);
                output.push('\n');
            }
        }

        output.trim().to_string()
    }
}

/// Group text fragments into horizontal lines, sorting lines top-to-bottom (descending $Y$)
/// and fragments within each line left-to-right (ascending $X$).
pub fn group_fragments_into_lines(fragments: &[TextFragment]) -> Vec<TextLine> {
    if fragments.is_empty() {
        return Vec::new();
    }

    // Sort fragments primarily by descending Y (top of page first), secondarily by ascending X
    let mut sorted = fragments.to_vec();
    sorted.sort_by(|a, b| {
        b.bbox
            .y
            .partial_cmp(&a.bbox.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.bbox
                    .x
                    .partial_cmp(&b.bbox.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let mut lines: Vec<Vec<TextFragment>> = Vec::new();

    for fragment in sorted {
        let mut placed = false;
        for line in &mut lines {
            if let Some(first) = line.first() {
                // Vertical tolerance for same line: within 3.5 points
                if (first.bbox.y - fragment.bbox.y).abs() <= 3.5 {
                    line.push(fragment.clone());
                    placed = true;
                    break;
                }
            }
        }
        if !placed {
            lines.push(vec![fragment]);
        }
    }

    // Convert grouped fragment clusters into TextLines
    let mut result = Vec::with_capacity(lines.len());
    for mut line_frags in lines {
        // Sort fragments inside the line from left to right (ascending X)
        line_frags.sort_by(|a, b| {
            a.bbox
                .x
                .partial_cmp(&b.bbox.x)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if let Some(first) = line_frags.first() {
            let mut union_box = first.bbox;
            let mut line_text = String::new();

            for (idx, frag) in line_frags.iter().enumerate() {
                union_box = union_box.union(&frag.bbox);
                if idx > 0 {
                    let prev_max = line_frags[idx - 1].bbox.x_max();
                    let min_gap = (frag.bbox.height * 0.20).max(2.4);
                    // If there is visible horizontal spacing between fragments, insert space
                    if frag.bbox.x - prev_max >= min_gap
                        && !line_text.ends_with(' ')
                        && !frag.text.starts_with(' ')
                        && !line_text.ends_with('-')
                    {
                        line_text.push(' ');
                    }
                }
                line_text.push_str(&frag.text);
            }

            result.push(TextLine {
                bbox: union_box,
                text: line_text,
            });
        }
    }

    // Ensure final lines are ordered strictly top to bottom (Y descending)
    result.sort_by(|a, b| {
        b.bbox
            .y
            .partial_cmp(&a.bbox.y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    result
}

/// Analyze text fragments on a page and detect whether multi-column gutters exist.
/// Returns the optimal `Box<dyn ReadingOrderStrategy>`:
/// - `MultiColumnSpatialFlow` if distinct vertical gutters partition the text.
/// - `SingleColumnFlow` if document has 1 column or uniform horizontal distribution.
pub fn select_reading_order_strategy(fragments: &[TextFragment]) -> Box<dyn ReadingOrderStrategy> {
    if fragments.len() < 4 {
        return Box::new(SingleColumnFlow);
    }

    let min_x = fragments
        .iter()
        .map(|f| f.bbox.x_min())
        .fold(f32::INFINITY, f32::min);
    let max_x = fragments
        .iter()
        .map(|f| f.bbox.x_max())
        .fold(f32::NEG_INFINITY, f32::max);
    let content_width = max_x - min_x;

    // Minimum width required to form 2 columns
    if content_width < 140.0 {
        return Box::new(SingleColumnFlow);
    }

    // Filter out wide spanning elements (titles, horizontal rules, page headers)
    let column_candidates: Vec<&TextFragment> = fragments
        .iter()
        .filter(|f| f.bbox.width <= content_width * 0.55)
        .collect();

    if column_candidates.len() < 4 {
        return Box::new(SingleColumnFlow);
    }

    // Discretize X axis into 2.0pt bins
    let bin_size: f32 = 2.0;
    let num_bins = ((content_width / bin_size).ceil() as usize).max(1);
    let mut occupancy = vec![0usize; num_bins];

    let total_chars: usize = column_candidates
        .iter()
        .map(|f| f.text.chars().count())
        .sum();
    if total_chars < 15 {
        return Box::new(SingleColumnFlow);
    }

    for f in &column_candidates {
        let start_bin = (((f.bbox.x_min() - min_x) / bin_size).floor() as isize).max(0) as usize;
        let end_bin = (((f.bbox.x_max() - min_x) / bin_size).ceil() as usize).min(num_bins);
        for slot in occupancy.iter_mut().take(end_bin).skip(start_bin) {
            *slot += 1;
        }
    }

    // Search for continuous zero-occupancy valleys of width >= 12.0pt (>= 6 bins)
    // located between 20% and 80% of page content width.
    let min_gutter_bins = (12.0 / bin_size).ceil() as usize;
    let interior_start_bin = (num_bins as f32 * 0.18).floor() as usize;
    let interior_end_bin = (num_bins as f32 * 0.82).ceil() as usize;

    let mut current_zero_start: Option<usize> = None;
    let mut detected_splits: Vec<f32> = Vec::new();

    for (bin, &count) in occupancy
        .iter()
        .enumerate()
        .take(interior_end_bin)
        .skip(interior_start_bin)
    {
        if count == 0 {
            if current_zero_start.is_none() {
                current_zero_start = Some(bin);
            }
        } else if let Some(start) = current_zero_start {
            let width_bins = bin - start;
            if width_bins >= min_gutter_bins {
                let gutter_x_start = min_x + (start as f32 * bin_size);
                let gutter_x_end = min_x + (bin as f32 * bin_size);
                let split_x = (gutter_x_start + gutter_x_end) / 2.0;

                // Verify that significant text exists on BOTH sides of the gutter (>= 15% text each)
                let left_chars: usize = column_candidates
                    .iter()
                    .filter(|f| f.bbox.x_max() <= split_x)
                    .map(|f| f.text.chars().count())
                    .sum();
                let right_chars: usize = column_candidates
                    .iter()
                    .filter(|f| f.bbox.x_min() >= split_x)
                    .map(|f| f.text.chars().count())
                    .sum();

                let left_ratio = (left_chars as f32) / (total_chars as f32);
                let right_ratio = (right_chars as f32) / (total_chars as f32);

                if left_ratio >= 0.15 && right_ratio >= 0.15 {
                    debug!(
                        target: "layout",
                        split_x = split_x,
                        left_ratio = left_ratio,
                        right_ratio = right_ratio,
                        "Multi-column gutter detected"
                    );
                    detected_splits.push(split_x);
                }
            }
            current_zero_start = None;
        }
    }

    // Check if gutter continued to interior_end_bin
    if let Some(start) = current_zero_start {
        let width_bins = interior_end_bin - start;
        if width_bins >= min_gutter_bins {
            let gutter_x_start = min_x + (start as f32 * bin_size);
            let gutter_x_end = min_x + (interior_end_bin as f32 * bin_size);
            let split_x = (gutter_x_start + gutter_x_end) / 2.0;

            let left_chars: usize = column_candidates
                .iter()
                .filter(|f| f.bbox.x_max() <= split_x)
                .map(|f| f.text.chars().count())
                .sum();
            let right_chars: usize = column_candidates
                .iter()
                .filter(|f| f.bbox.x_min() >= split_x)
                .map(|f| f.text.chars().count())
                .sum();

            let left_ratio = (left_chars as f32) / (total_chars as f32);
            let right_ratio = (right_chars as f32) / (total_chars as f32);

            if left_ratio >= 0.15 && right_ratio >= 0.15 {
                detected_splits.push(split_x);
            }
        }
    }

    if !detected_splits.is_empty() {
        detected_splits.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        detected_splits.dedup();
        Box::new(MultiColumnSpatialFlow {
            split_x_positions: detected_splits,
        })
    } else {
        trace!(target: "layout", "No multi-column gutter found; using SingleColumnFlow");
        Box::new(SingleColumnFlow)
    }
}

/// Extract and reconstruct page text respecting multi-column spatial reading order.
/// If `only_if_multi_column` is true, returns `None` when the page layout is single-column,
/// allowing the caller to use default linear stream extraction.
pub fn extract_page_text_spatial(
    doc: &lopdf::Document,
    page_id: (u32, u16),
    only_if_multi_column: bool,
) -> Option<String> {
    let Ok(content_data) =
        doc.get_page_content_with_limit(page_id, crate::document::MAX_DECOMPRESSED_BYTES)
    else {
        return None;
    };
    if content_data.is_empty() {
        return None;
    }

    let Ok(content) = Content::decode(&content_data) else {
        return None;
    };

    // Load font encodings for proper glyph mapping
    let encodings: BTreeMap<Vec<u8>, Encoding> = doc
        .get_page_fonts(page_id)
        .map(|fonts| {
            fonts
                .into_iter()
                .filter_map(|(name, font)| font.get_font_encoding(doc).ok().map(|enc| (name, enc)))
                .collect()
        })
        .unwrap_or_default();

    let fragments = extract_positioned_fragments(&content, &encodings);
    if fragments.is_empty() {
        return None;
    }

    let strategy = select_reading_order_strategy(&fragments);
    if only_if_multi_column && !strategy.is_multi_column() {
        return None;
    }

    let reconstructed = strategy.reconstruct_text(&fragments);
    if reconstructed.trim().is_empty() {
        None
    } else {
        Some(reconstructed)
    }
}
