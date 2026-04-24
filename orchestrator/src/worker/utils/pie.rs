//! Helpers for serializing CairoPIEs into bytes.
//!
//! These functions are shared by the SNOS and aggregator job handlers to avoid
//! duplicating the temp-file-based zip encoding logic. The key property is that
//! the CairoPIE is **consumed** by `cairo_pie_to_zip_bytes` and dropped before
//! the zip bytes are read back from disk, so peak memory is roughly one copy
//! of the PIE rather than two.

use bytes::Bytes;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use color_eyre::Result;
use tempfile::NamedTempFile;
use tokio::io::AsyncReadExt;

use crate::types::constant::BYTE_CHUNK_SIZE;

/// Convert a [`CairoPie`] into zip bytes via a temp file.
///
/// The PIE is consumed and dropped before the bytes are streamed back,
/// so only one in-memory representation of the PIE exists at a time.
pub async fn cairo_pie_to_zip_bytes(cairo_pie: CairoPie) -> Result<Bytes> {
    let mut zip_file = NamedTempFile::new()?;
    cairo_pie.write_zip_file(zip_file.path(), true)?;
    drop(cairo_pie); // Release PIE memory before we buffer the zip bytes.
    let bytes = tempfile_to_bytes_streaming(&mut zip_file).await?;
    zip_file.close()?;
    Ok(bytes)
}

/// Stream a [`NamedTempFile`] into [`Bytes`] in chunks.
///
/// Useful when the file may be large (aggregator CairoPIEs can be tens of MB)
/// and we want to avoid reading it in one go via [`std::fs::read`].
pub async fn tempfile_to_bytes_streaming(tmp_file: &mut NamedTempFile) -> Result<Bytes> {
    let file_size = tmp_file.as_file().metadata()?.len() as usize;
    let mut buffer = Vec::with_capacity(file_size);
    let mut chunk = vec![0; BYTE_CHUNK_SIZE];
    let mut file = tokio::fs::File::from_std(tmp_file.as_file().try_clone()?);

    // Propagate I/O errors instead of treating them like EOF — a transient failure
    // mid-read on a large PIE would otherwise silently truncate the bytes we hand
    // back, surfacing later as a confusing parse error.
    loop {
        let n = file.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
    }

    Ok(Bytes::from(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Load the fibonacci.zip test artifact as a CairoPie.
    fn load_test_pie() -> CairoPie {
        let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src", "tests", "artifacts", "fibonacci.zip"].iter().collect();
        CairoPie::read_zip_file(&path).expect("Failed to read fibonacci.zip test artifact")
    }

    #[tokio::test]
    async fn round_trip_pie_to_bytes_and_back() {
        let pie = load_test_pie();
        let bytes = cairo_pie_to_zip_bytes(pie).await.expect("cairo_pie_to_zip_bytes failed");

        // Bytes should be non-empty and start with the ZIP magic number (PK = 0x50 0x4B).
        assert!(!bytes.is_empty(), "zip bytes should be non-empty");
        assert_eq!(&bytes[..2], b"PK", "zip bytes should start with PK magic header");

        // Should be parseable back into a valid CairoPie.
        let round_tripped = CairoPie::from_bytes(&bytes).expect("Failed to parse CairoPie from round-tripped bytes");
        round_tripped.run_validity_checks().expect("Round-tripped CairoPie failed validity checks");
    }

    #[tokio::test]
    async fn tempfile_streaming_reads_all_bytes() {
        use std::io::{Seek, Write};

        // Write known content to a temp file, then stream it back.
        let content = vec![0xABu8; 32 * 1024]; // 32 KB — exercises chunked reading
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(&content).unwrap();
        tmp.seek(std::io::SeekFrom::Start(0)).unwrap(); // rewind before streaming

        let bytes = tempfile_to_bytes_streaming(&mut tmp).await.expect("streaming failed");
        assert_eq!(bytes.len(), content.len());
        assert_eq!(&bytes[..], &content[..]);
    }
}
