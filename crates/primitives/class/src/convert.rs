use flate2::bufread::GzDecoder;
use starknet_core::types::LegacyContractEntryPoint;
use starknet_core::types::{
    contract::legacy::{
        LegacyContractClass, LegacyEntrypointOffset, LegacyProgram, RawLegacyEntryPoint, RawLegacyEntryPoints,
    },
    CompressedLegacyContractClass,
};
use std::io::Read;

#[derive(Debug, thiserror::Error)]
pub enum ParseCompressedLegacyClassError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Unexpected legacy compiler version string")]
    InvalidCompilerVersion,
    #[error("Integer parse error: {0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

/// Attempts to recover a compressed legacy program.
pub fn parse_compressed_legacy_class(
    class: CompressedLegacyContractClass,
) -> Result<LegacyContractClass, ParseCompressedLegacyClassError> {
    let mut gzip_decoder = GzDecoder::new(class.program.as_slice());
    let mut program_json = String::new();
    gzip_decoder.read_to_string(&mut program_json)?;

    let program = serde_json::from_str::<LegacyProgram>(&program_json)?;

    let is_pre_0_11_0 = match &program.compiler_version {
        Some(compiler_version) => {
            let minor_version =
                compiler_version.split('.').nth(1).ok_or(ParseCompressedLegacyClassError::InvalidCompilerVersion)?;

            let minor_version: u8 = minor_version.parse()?;
            minor_version < 11
        }
        None => true,
    };

    let abi = match class.abi {
        Some(abi) => abi.into_iter().map(|item| item.into()).collect(),
        None => vec![],
    };

    Ok(LegacyContractClass {
        abi: Some(abi),
        entry_points_by_type: RawLegacyEntryPoints {
            constructor: class
                .entry_points_by_type
                .constructor
                .into_iter()
                .map(|item| parse_legacy_entrypoint(&item, is_pre_0_11_0))
                .collect(),
            external: class
                .entry_points_by_type
                .external
                .into_iter()
                .map(|item| parse_legacy_entrypoint(&item, is_pre_0_11_0))
                .collect(),
            l1_handler: class
                .entry_points_by_type
                .l1_handler
                .into_iter()
                .map(|item| parse_legacy_entrypoint(&item, is_pre_0_11_0))
                .collect(),
        },
        program,
    })
}

fn parse_legacy_entrypoint(entrypoint: &LegacyContractEntryPoint, pre_0_11_0: bool) -> RawLegacyEntryPoint {
    RawLegacyEntryPoint {
        // This doesn't really matter as it doesn't affect class hashes. We simply try to guess as
        // close as possible.
        offset: if pre_0_11_0 {
            LegacyEntrypointOffset::U64AsHex(entrypoint.offset)
        } else {
            LegacyEntrypointOffset::U64AsInt(entrypoint.offset)
        },
        selector: entrypoint.selector,
    }
}
