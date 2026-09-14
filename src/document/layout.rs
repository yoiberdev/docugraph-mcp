//! Spatial layout analysis and multi-column reading order reconstruction.
//!
//! Handles 2D affine transformations (`cm`, `Tm`, `Td`, `TD`, `T*`) to extract
//! positioned text fragments and accurately reconstruct human reading order across
//! multi-column documents (scientific papers, specifications, newsletters)
//! without interleaved columns.

use lopdf::content::Content;
use lopdf::{Dictionary, Encoding, Object};
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

/// Advance assumed for glyphs whose font has no width table (thousandths of an em).
const FALLBACK_GLYPH_WIDTH: f32 = 520.0;

/// A TJ adjustment more negative than this (thousandths of an em) is read as a word gap.
const TJ_WORD_GAP_THOUSANDTHS: f32 = 100.0;

/// Horizontal gap between two fragments on the same baseline, as a fraction of the
/// font size, from which a word space is inserted between them.
const WORD_GAP_EM: f32 = 0.15;

/// Width entries of a CID font `/W` array.
#[derive(Debug, Clone)]
enum CidWidths {
    /// `c [w1 w2 ...]`: consecutive CIDs starting at `c`
    List(u32, Vec<f32>),
    /// `c_first c_last w`: every CID in the range shares one width
    Range(u32, u32, f32),
}

/// Glyph advance widths of a font resource, used to place text fragments precisely.
#[derive(Debug, Clone)]
struct FontMetrics {
    /// Bytes per character code: 2 for Type0 (CID) fonts, 1 for simple fonts
    code_len: usize,
    first_char: u32,
    widths: Vec<f32>,
    cid_widths: Vec<CidWidths>,
    default_width: Option<f32>,
    /// Glyph space to text space factor (1/1000, or the FontMatrix of Type3 fonts)
    scale: f32,
}

impl FontMetrics {
    fn from_font(doc: &lopdf::Document, font: &Dictionary) -> Self {
        let number = |obj: &Object| {
            doc.dereference(obj)
                .ok()
                .and_then(|(_, o)| o.as_float().ok())
        };
        let mut metrics = FontMetrics {
            code_len: 1,
            first_char: 0,
            widths: Vec::new(),
            cid_widths: Vec::new(),
            default_width: None,
            scale: 0.001,
        };
        let subtype = font
            .get(b"Subtype")
            .and_then(|o| o.as_name())
            .unwrap_or_default();

        if subtype == b"Type0" {
            metrics.code_len = 2;
            let cid_font = font
                .get_deref(b"DescendantFonts", doc)
                .and_then(|o| o.as_array())
                .ok()
                .and_then(|fonts| fonts.first())
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_dict().ok());
            if let Some(cid_font) = cid_font {
                metrics.default_width =
                    Some(cid_font.get(b"DW").ok().and_then(number).unwrap_or(1000.0));
                if let Ok(w) = cid_font.get_deref(b"W", doc).and_then(|o| o.as_array()) {
                    metrics.cid_widths = parse_cid_widths(w, number);
                }
            }
            return metrics;
        }

        if subtype == b"Type3"
            && let Some(a) = font
                .get_deref(b"FontMatrix", doc)
                .and_then(|o| o.as_array())
                .ok()
                .and_then(|m| m.first())
                .and_then(number)
        {
            metrics.scale = a.abs();
        }
        metrics.first_char = font
            .get(b"FirstChar")
            .ok()
            .and_then(number)
            .map_or(0, |v| v.max(0.0) as u32);
        if let Ok(widths) = font.get_deref(b"Widths", doc).and_then(|o| o.as_array()) {
            metrics.widths = widths.iter().map(|w| number(w).unwrap_or(0.0)).collect();
        }
        metrics.default_width = font
            .get_deref(b"FontDescriptor", doc)
            .and_then(|o| o.as_dict())
            .ok()
            .and_then(|d| d.get(b"MissingWidth").ok())
            .and_then(number)
            .filter(|w| *w > 0.0);
        metrics
    }

    /// Advance of a character code in text space units per unit of font size, if known.
    fn glyph_width(&self, code: u32) -> Option<f32> {
        let raw = if self.code_len == 2 {
            self.cid_widths
                .iter()
                .find_map(|entry| match entry {
                    CidWidths::List(first, list) => code
                        .checked_sub(*first)
                        .and_then(|i| list.get(i as usize))
                        .copied(),
                    CidWidths::Range(first, last, w) => {
                        (*first..=*last).contains(&code).then_some(*w)
                    }
                })
                .or(self.default_width)
        } else {
            code.checked_sub(self.first_char)
                .and_then(|i| self.widths.get(i as usize))
                .copied()
                .or(self.default_width)
        };
        raw.map(|w| w * self.scale)
    }
}

/// Parse a CID font `/W` array (PDF 32000-1 §9.7.4.3).
fn parse_cid_widths(w: &[Object], number: impl Fn(&Object) -> Option<f32>) -> Vec<CidWidths> {
    let mut entries = Vec::new();
    let mut i = 0;
    while i + 1 < w.len() {
        let Some(first) = number(&w[i]) else {
            break;
        };
        let first = first.max(0.0) as u32;
        if let Object::Array(list) = &w[i + 1] {
            entries.push(CidWidths::List(
                first,
                list.iter().map(|o| number(o).unwrap_or(0.0)).collect(),
            ));
            i += 2;
        } else {
            let (Some(last), Some(width)) = (number(&w[i + 1]), w.get(i + 2).and_then(&number))
            else {
                break;
            };
            entries.push(CidWidths::Range(first, last.max(0.0) as u32, width));
            i += 3;
        }
    }
    entries
}

/// Text state parameters that affect glyph placement (PDF 32000-1 §9.3).
#[derive(Debug, Clone)]
struct TextState {
    font: Option<Vec<u8>>,
    font_size: f32,
    leading: f32,
    char_spacing: f32,
    word_spacing: f32,
    horizontal_scale: f32,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            font: None,
            font_size: 12.0,
            leading: 14.4,
            char_spacing: 0.0,
            word_spacing: 0.0,
            horizontal_scale: 1.0,
        }
    }
}

/// Horizontal displacement, in text space, produced by showing `bytes` with the current font.
fn string_advance(bytes: &[u8], metrics: Option<&FontMetrics>, state: &TextState) -> f32 {
    let code_len = metrics.map_or(1, |m| m.code_len);
    bytes
        .chunks(code_len)
        .map(|chunk| {
            let code = chunk.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b));
            let glyph = metrics
                .and_then(|m| m.glyph_width(code))
                .unwrap_or(FALLBACK_GLYPH_WIDTH / 1000.0);
            let word = if code_len == 1 && code == 32 {
                state.word_spacing
            } else {
                0.0
            };
            (glyph * state.font_size + state.char_spacing + word) * state.horizontal_scale
        })
        .sum()
}

fn translation(tx: f32, ty: f32) -> Matrix2D {
    Matrix2D {
        e: tx,
        f: ty,
        ..Matrix2D::IDENTITY
    }
}

/// Record the fragment shown at the current text position and advance the text matrix.
fn show_fragment(
    fragments: &mut Vec<TextFragment>,
    text: &str,
    advance: f32,
    text_matrix: &mut Matrix2D,
    ctm: &Matrix2D,
    state: &TextState,
) {
    if !text.trim().is_empty() {
        let eff = text_matrix.multiply(ctm);
        let (x0, y0) = eff.transform_point(0.0, 0.0);
        let (x1, _) = eff.transform_point(advance, 0.0);
        let size = state.font_size * (eff.c * eff.c + eff.d * eff.d).sqrt();
        fragments.push(TextFragment {
            bbox: BoundingBox::new(x0.min(x1), y0, (x1 - x0).abs(), size.max(1.0)),
            text: text.to_string(),
        });
    }
    *text_matrix = translation(advance, 0.0).multiply(text_matrix);
}

/// Extract positioned text fragments from a decoded PDF content stream.
///
/// Fonts are not resolved here, so glyph widths are estimated; use
/// [`extract_page_text`] to place fragments with the page's real font metrics.
pub fn extract_positioned_fragments(
    content: &Content,
    encodings: &BTreeMap<Vec<u8>, Encoding>,
) -> Vec<TextFragment> {
    extract_fragments_with_metrics(content, encodings, &BTreeMap::new())
}

fn extract_fragments_with_metrics(
    content: &Content,
    encodings: &BTreeMap<Vec<u8>, Encoding>,
    metrics: &BTreeMap<Vec<u8>, FontMetrics>,
) -> Vec<TextFragment> {
    let mut fragments = Vec::new();

    let mut ctm = Matrix2D::IDENTITY;
    let mut text_matrix = Matrix2D::IDENTITY;
    let mut line_matrix = Matrix2D::IDENTITY;
    let mut state = TextState::default();

    let mut state_stack: Vec<(Matrix2D, TextState)> = Vec::new();
    let float_at = |op: &lopdf::content::Operation, idx: usize| {
        op.operands.get(idx).and_then(|o| o.as_float().ok())
    };

    for op in &content.operations {
        match op.operator.as_str() {
            "q" => {
                state_stack.push((ctm, state.clone()));
            }
            "Q" => {
                if let Some((saved_ctm, saved_state)) = state_stack.pop() {
                    ctm = saved_ctm;
                    state = saved_state;
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
            "BT" | "ET" => {
                text_matrix = Matrix2D::IDENTITY;
                line_matrix = Matrix2D::IDENTITY;
            }
            "Tf" => {
                if let Some(font_name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                    state.font = Some(font_name.to_vec());
                }
                if let Some(size) = float_at(op, 1) {
                    state.font_size = size;
                }
            }
            "TL" => {
                if let Some(l) = float_at(op, 0) {
                    state.leading = l;
                }
            }
            "Tc" => {
                if let Some(v) = float_at(op, 0) {
                    state.char_spacing = v;
                }
            }
            "Tw" => {
                if let Some(v) = float_at(op, 0) {
                    state.word_spacing = v;
                }
            }
            "Tz" => {
                if let Some(v) = float_at(op, 0) {
                    state.horizontal_scale = v / 100.0;
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
            "Td" | "TD" => {
                if let (Some(tx), Some(ty)) = (float_at(op, 0), float_at(op, 1)) {
                    if op.operator == "TD" {
                        state.leading = -ty;
                    }
                    line_matrix = translation(tx, ty).multiply(&line_matrix);
                    text_matrix = line_matrix;
                }
            }
            "T*" | "'" | "\"" => {
                if op.operator == "\"" {
                    if let Some(aw) = float_at(op, 0) {
                        state.word_spacing = aw;
                    }
                    if let Some(ac) = float_at(op, 1) {
                        state.char_spacing = ac;
                    }
                }
                line_matrix = translation(0.0, -state.leading).multiply(&line_matrix);
                text_matrix = line_matrix;

                let string_operand = match op.operator.as_str() {
                    "'" => op.operands.first(),
                    "\"" => op.operands.get(2),
                    _ => None,
                };
                if let Some(Object::String(bytes, _)) = string_operand {
                    let font = state.font.as_ref();
                    let text =
                        decode_bytes_with_encoding(bytes, font.and_then(|f| encodings.get(f)));
                    let advance = string_advance(bytes, font.and_then(|f| metrics.get(f)), &state);
                    show_fragment(
                        &mut fragments,
                        &text,
                        advance,
                        &mut text_matrix,
                        &ctm,
                        &state,
                    );
                }
            }
            "Tj" => {
                if let Some(Object::String(bytes, _)) = op.operands.first() {
                    let font = state.font.as_ref();
                    let text =
                        decode_bytes_with_encoding(bytes, font.and_then(|f| encodings.get(f)));
                    let advance = string_advance(bytes, font.and_then(|f| metrics.get(f)), &state);
                    show_fragment(
                        &mut fragments,
                        &text,
                        advance,
                        &mut text_matrix,
                        &ctm,
                        &state,
                    );
                }
            }
            "TJ" => {
                if let Some(Object::Array(items)) = op.operands.first() {
                    let font = state.font.as_ref();
                    let enc = font.and_then(|f| encodings.get(f));
                    let font_metrics = font.and_then(|f| metrics.get(f));
                    let mut combined_text = String::new();
                    let mut advance = 0.0;
                    for item in items {
                        match item {
                            Object::String(bytes, _) => {
                                combined_text.push_str(&decode_bytes_with_encoding(bytes, enc));
                                advance += string_advance(bytes, font_metrics, &state);
                            }
                            Object::Integer(_) | Object::Real(_) => {
                                // Adjustments are in thousandths of an em; negative values move right
                                let adjustment = item.as_float().unwrap_or(0.0);
                                advance -=
                                    adjustment / 1000.0 * state.font_size * state.horizontal_scale;
                                if adjustment < -TJ_WORD_GAP_THOUSANDTHS
                                    && !combined_text.is_empty()
                                    && !combined_text.ends_with(char::is_whitespace)
                                {
                                    combined_text.push(' ');
                                }
                            }
                            _ => {}
                        }
                    }
                    show_fragment(
                        &mut fragments,
                        combined_text.trim(),
                        advance,
                        &mut text_matrix,
                        &ctm,
                        &state,
                    );
                }
            }
            _ => {}
        }
    }

    fragments
}

/// Whether two fragments sit on the same baseline (within half the font size).
fn on_same_baseline(prev: &TextFragment, next: &TextFragment) -> bool {
    let size = prev.bbox.height.max(next.bbox.height);
    (next.bbox.y - prev.bbox.y).abs() <= size * 0.5
}

/// Whether the horizontal distance between two fragments on one line is a word space.
/// A jump backwards larger than the font size also separates words.
fn is_word_gap(prev: &TextFragment, next: &TextFragment) -> bool {
    let size = prev.bbox.height.max(next.bbox.height);
    let gap = next.bbox.x - prev.bbox.x_max();
    gap > size * WORD_GAP_EM || gap < -size
}

/// Join fragments in content stream order, starting a new line when the baseline
/// changes and inserting a space when glyph positions leave a word gap.
pub fn join_fragments_in_stream_order(fragments: &[TextFragment]) -> String {
    let mut output = String::new();
    for (idx, frag) in fragments.iter().enumerate() {
        if idx > 0 {
            let prev = &fragments[idx - 1];
            if !on_same_baseline(prev, frag) {
                output.truncate(output.trim_end_matches([' ', '\t']).len());
                if !output.ends_with('\n') {
                    output.push('\n');
                }
            } else if is_word_gap(prev, frag)
                && !output.ends_with(char::is_whitespace)
                && !frag.text.starts_with(char::is_whitespace)
            {
                output.push(' ');
            }
        }
        output.push_str(&frag.text);
    }
    output
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
            let is_spanning = f.bbox.width > (content_width * 0.65);
            if !is_spanning {
                col_cand_y_max = col_cand_y_max.max(f.bbox.y_max());
                col_cand_y_min = col_cand_y_min.min(f.bbox.y_min());
            }
        }

        for f in fragments {
            let is_spanning = f.bbox.width > (content_width * 0.65);
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
                // If glyph positions leave a word gap between fragments, insert a space
                if idx > 0
                    && is_word_gap(&line_frags[idx - 1], frag)
                    && !line_text.ends_with(char::is_whitespace)
                    && !frag.text.starts_with(char::is_whitespace)
                {
                    line_text.push(' ');
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
        .filter(|f| f.bbox.width <= content_width * 0.65)
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

/// Decode a page content stream into positioned fragments using the page's font
/// encodings and glyph widths.
fn page_fragments(doc: &lopdf::Document, page_id: (u32, u16)) -> Option<Vec<TextFragment>> {
    let content_data = doc.get_page_content(page_id);
    if content_data.is_empty() {
        return None;
    }

    let Ok(content) = Content::decode(&content_data) else {
        return None;
    };

    // Load font encodings for proper glyph mapping, and widths for glyph placement
    let fonts = doc.get_page_fonts(page_id).unwrap_or_default();
    let mut encodings: BTreeMap<Vec<u8>, Encoding> = BTreeMap::new();
    let mut metrics: BTreeMap<Vec<u8>, FontMetrics> = BTreeMap::new();
    for (name, font) in fonts {
        if let Ok(enc) = font.get_font_encoding(doc) {
            encodings.insert(name.clone(), enc);
        }
        metrics.insert(name, FontMetrics::from_font(doc, font));
    }

    let fragments = extract_fragments_with_metrics(&content, &encodings, &metrics);
    (!fragments.is_empty()).then_some(fragments)
}

fn non_empty(text: String) -> Option<String> {
    (!text.trim().is_empty()).then_some(text)
}

/// Extract and reconstruct page text respecting multi-column spatial reading order.
/// If `only_if_multi_column` is true, returns `None` when the page layout is single-column,
/// allowing the caller to use default linear stream extraction.
pub fn extract_page_text_spatial(
    doc: &lopdf::Document,
    page_id: (u32, u16),
    only_if_multi_column: bool,
) -> Option<String> {
    let fragments = page_fragments(doc, page_id)?;

    let strategy = select_reading_order_strategy(&fragments);
    if only_if_multi_column && !strategy.is_multi_column() {
        return None;
    }

    non_empty(strategy.reconstruct_text(&fragments))
}

/// Extract page text from positioned glyphs: multi-column reading order when gutters
/// are detected, otherwise content stream order. Word spaces come from glyph
/// positions (TJ adjustments and text moves), so fonts without a space glyph and
/// words split across operators still read correctly.
pub fn extract_page_text(doc: &lopdf::Document, page_id: (u32, u16)) -> Option<String> {
    let fragments = page_fragments(doc, page_id)?;

    let strategy = select_reading_order_strategy(&fragments);
    let text = if strategy.is_multi_column() {
        strategy.reconstruct_text(&fragments)
    } else {
        join_fragments_in_stream_order(&fragments)
    };
    non_empty(text)
}
