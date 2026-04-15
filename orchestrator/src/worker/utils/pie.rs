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
