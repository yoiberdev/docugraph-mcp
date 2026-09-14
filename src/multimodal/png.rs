//! Pure Rust PNG image encoder conforming to RFC 2083.
//!
//! Encodes raw RGBA image buffers into compressed PNG byte streams without
//! external C/C++ dependencies.

use anyhow::{Result, bail};

/// Calculate standard CRC-32 checksum for PNG chunks according to ISO 3309 / RFC 2083.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Write a single PNG chunk (Length + Type + Data + CRC-32) to the output vector.
fn write_chunk(output: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    let len = data.len() as u32;
    output.extend_from_slice(&len.to_be_bytes());

    let mut crc_payload = Vec::with_capacity(4 + data.len());
    crc_payload.extend_from_slice(chunk_type);
    crc_payload.extend_from_slice(data);

    output.extend_from_slice(chunk_type);
    output.extend_from_slice(data);

    let crc = crc32(&crc_payload);
    output.extend_from_slice(&crc.to_be_bytes());
}

/// Encode an RGBA buffer (4 bytes per pixel: R, G, B, A) into standard PNG format.
pub fn encode_rgba_to_png(width: u32, height: u32, rgba_data: &[u8]) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        bail!("Image dimensions must be greater than zero: {width}x{height}");
    }

    let expected_len = (width as usize) * (height as usize) * 4;
    if rgba_data.len() != expected_len {
        bail!(
            "Invalid RGBA buffer length: expected {expected_len} bytes for {width}x{height}, got {}",
            rgba_data.len()
        );
    }

    let mut output = Vec::with_capacity(8 + 25 + expected_len / 2 + 12);

    // 1. PNG Signature (RFC 2083 §3.1)
    output.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    // 2. IHDR Chunk (Image Header)
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // Bit depth: 8 bits per channel
    ihdr.push(6); // Color type: 6 = RGBA (Truecolor with alpha)
    ihdr.push(0); // Compression method: 0 (deflate/inflate)
    ihdr.push(0); // Filter method: 0 (standard adaptive filter)
    ihdr.push(0); // Interlace method: 0 (no interlace)
    write_chunk(&mut output, b"IHDR", &ihdr);

    // 3. IDAT Chunk (Image Data)
    // Prepare scanlines: each row is preceded by a 1-byte filter type (0 = None)
    let row_len = width as usize * 4;
    let mut raw_scanlines = Vec::with_capacity((height as usize) * (1 + row_len));

    for y in 0..(height as usize) {
        raw_scanlines.push(0); // Filter type 0 (None)
        let row_start = y * row_len;
        raw_scanlines.extend_from_slice(&rgba_data[row_start..row_start + row_len]);
    }

    // Compress raw scanlines with zlib using miniz_oxide
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw_scanlines, 6);
    write_chunk(&mut output, b"IDAT", &compressed);

    // 4. IEND Chunk (Image Trailer)
    write_chunk(&mut output, b"IEND", &[]);

    Ok(output)
}

/// Alias for `encode_rgba_to_png`.
pub use encode_rgba_to_png as encode_png;
