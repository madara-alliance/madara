//! Helpers for serializing CairoPIEs into bytes without materializing the
//! complete uncompressed `memory.bin` alongside the PIE.

use bytes::Bytes;
use cairo_vm::types::relocatable::{MaybeRelocatable, Relocatable};
use cairo_vm::vm::runners::cairo_pie::{CairoPie, CairoPieMemory, SegmentInfo};
use color_eyre::Result;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use tempfile::NamedTempFile;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::types::constant::BYTE_CHUNK_SIZE;

const ADDR_BYTE_LEN: usize = 8;
const FIELD_BYTE_LEN: usize = 32;
const CELL_BYTE_LEN: usize = ADDR_BYTE_LEN + FIELD_BYTE_LEN;
const ADDR_BASE: u64 = 1 << 63;
const OFFSET_BASE: u64 = 1 << 47;

/// Convert a [`CairoPie`] into zip bytes via a temp file.
///
/// The PIE is consumed and dropped before the bytes are streamed back,
/// so only one in-memory representation of the PIE exists at a time.
pub async fn cairo_pie_to_zip_bytes(cairo_pie: CairoPie) -> Result<Bytes> {
    tokio::task::spawn_blocking(move || cairo_pie_to_zip_bytes_blocking(cairo_pie)).await?
}

/// Blocking CairoPIE serialization for callers that already run on Tokio's
/// blocking pool.
pub fn cairo_pie_to_zip_bytes_blocking(cairo_pie: CairoPie) -> Result<Bytes> {
    let zip_file = NamedTempFile::new()?;
    write_zip_file_streaming(&cairo_pie, zip_file.path())?;
    drop(cairo_pie); // Release PIE memory before we buffer the zip bytes.
    let bytes = std::fs::read(zip_file.path())?;
    zip_file.close()?;
    Ok(Bytes::from(bytes))
}

fn write_zip_file_streaming(cairo_pie: &CairoPie, path: &Path) -> Result<()> {
    let mut metadata = cairo_pie.metadata.clone();
    let segment_offsets = merged_extra_segments(cairo_pie).map(|(segment, offsets)| {
        metadata.extra_segments = vec![segment];
        offsets
    });

    let mut zip_writer = ZipWriter::new(File::create(path)?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).large_file(true);

    zip_writer.start_file("version.json", options)?;
    serde_json::to_writer(&mut zip_writer, &cairo_pie.version)?;
    zip_writer.start_file("metadata.json", options)?;
    serde_json::to_writer(&mut zip_writer, &metadata)?;
    zip_writer.start_file("memory.bin", options)?;
    write_memory(&cairo_pie.memory, segment_offsets.as_ref(), &mut zip_writer)?;
    zip_writer.start_file("additional_data.json", options)?;
    serde_json::to_writer(&mut zip_writer, &cairo_pie.additional_data)?;
    zip_writer.start_file("execution_resources.json", options)?;
    serde_json::to_writer(&mut zip_writer, &cairo_pie.execution_resources)?;
    zip_writer.finish()?;
    Ok(())
}

fn write_memory<W: Write>(
    memory: &CairoPieMemory,
    segment_offsets: Option<&HashMap<usize, Relocatable>>,
    writer: W,
) -> std::io::Result<()> {
    let mut writer = BufWriter::with_capacity(BYTE_CHUNK_SIZE, writer);

    for ((segment, offset), value) in &memory.0 {
        let (segment, offset) = relocate_value(*segment, *offset, segment_offsets);
        let address = ADDR_BASE + segment as u64 * OFFSET_BASE + offset as u64;
        let mut cell = [0_u8; CELL_BYTE_LEN];
        cell[..ADDR_BYTE_LEN].copy_from_slice(&address.to_le_bytes());

        match value {
            MaybeRelocatable::RelocatableValue(value) => {
                let (segment, offset) = relocate_value(value.segment_index as usize, value.offset, segment_offsets);
                let encoded = segment as u64 * OFFSET_BASE + offset as u64;
                cell[ADDR_BYTE_LEN..ADDR_BYTE_LEN * 2].copy_from_slice(&encoded.to_le_bytes());
                cell[CELL_BYTE_LEN - 1] = 0x80;
            }
            MaybeRelocatable::Int(value) => {
                cell[ADDR_BYTE_LEN..].copy_from_slice(&value.to_bytes_le());
            }
        }

        writer.write_all(&cell)?;
    }

    writer.flush()
}

fn relocate_value(
    segment: usize,
    offset: usize,
    segment_offsets: Option<&HashMap<usize, Relocatable>>,
) -> (usize, usize) {
    segment_offsets
        .and_then(|offsets| offsets.get(&segment))
        .map(|relocatable| (relocatable.segment_index as usize, relocatable.offset + offset))
        .unwrap_or((segment, offset))
}

fn merged_extra_segments(cairo_pie: &CairoPie) -> Option<(SegmentInfo, HashMap<usize, Relocatable>)> {
    let first = cairo_pie.metadata.extra_segments.first()?;
    let new_index = first.index;
    let mut accumulated_size = 0;
    let offsets = cairo_pie
        .metadata
        .extra_segments
        .iter()
        .map(|segment| {
            let entry = (segment.index as usize, Relocatable { segment_index: new_index, offset: accumulated_size });
            accumulated_size += segment.size;
            entry
        })
        .collect();

    Some((SegmentInfo { index: new_index, size: accumulated_size }, offsets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};
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

    #[test]
    fn streamed_memory_matches_cairo_vm_encoding() {
        let pie = load_test_pie();
        let segment_offsets = merged_extra_segments(&pie).map(|(_, offsets)| offsets);
        let expected = pie.memory.to_bytes(segment_offsets);
        let bytes = cairo_pie_to_zip_bytes_blocking(pie).expect("streaming serialization failed");
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("invalid zip");
        let mut actual = Vec::new();
        archive.by_name("memory.bin").expect("missing memory.bin").read_to_end(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }
}
