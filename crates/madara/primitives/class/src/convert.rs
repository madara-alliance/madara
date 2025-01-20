use flate2::bufread::GzDecoder;
use starknet_core::types::LegacyContractEntryPoint;
use starknet_core::types::{
    contract::legacy::{
        LegacyContractClass, LegacyEntrypointOffset, LegacyProgram, RawLegacyEntryPoint, RawLegacyEntryPoints,
    },
    CompressedLegacyContractClass,
};
use std::io::{self, Read};

#[derive(Debug, thiserror::Error)]
pub enum ParseCompressedLegacyClassError {
    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Unexpected legacy compiler version string")]
    InvalidCompilerVersion,
    #[error("Integer parse error: {0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[allow(non_upper_case_globals)]
const MiB: u64 = 1024 * 1024;
const CLASS_SIZE_LIMIT: u64 = 4 * MiB;

/// Attempts to recover a compressed legacy program.
pub fn parse_compressed_legacy_class(
    class: CompressedLegacyContractClass,
) -> Result<LegacyContractClass, ParseCompressedLegacyClassError> {
    // decompress and parse as a single [`Read`] pipeline to avoid having an intermediary buffer here.
    let program: LegacyProgram =
        serde_json::from_reader(ReadSizeLimiter::new(GzDecoder::new(class.program.as_slice()), CLASS_SIZE_LIMIT))?;

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
        abi,
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

#[derive(thiserror::Error, Debug)]
#[error("Read input is too large")]
struct InputTooLarge;

/// [`std::io::Read`] combinator that works very much like [`std::io::Take`], but returns an error
/// if the underlying buffer is bigger than the limit instead of just returning EOF.
pub struct ReadSizeLimiter<R> {
    inner: R,
    limit: u64,
}
impl<R: Read> ReadSizeLimiter<R> {
    pub fn new(inner: R, limit: u64) -> Self {
        Self { inner, limit }
    }
}
impl<R: Read> Read for ReadSizeLimiter<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.limit == 0 {
            // check if the inner read still has data for us
            if self.inner.read(&mut [0])? > 0 {
                return Err(io::Error::new(io::ErrorKind::Other, InputTooLarge));
            }
        }

        let max = u64::min(buf.len() as u64, self.limit) as usize;
        let n = self.inner.read(&mut buf[..max])?;
        // can only panic if the inner Read impl returns a bogus number
        assert!(n as u64 <= self.limit, "number of read bytes exceeds limit");
        self.limit -= n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_size_limiter() {
        assert!(ReadSizeLimiter::new(&[0u8; 3][..], 5).read_to_end(&mut vec![]).is_ok());
        assert!(ReadSizeLimiter::new(&[0u8; 5][..], 5).read_to_end(&mut vec![]).is_ok());
        assert!(ReadSizeLimiter::new(&[0u8; 6][..], 5).read_to_end(&mut vec![]).is_err());
        assert!(ReadSizeLimiter::new(&[0u8; 64][..], 5).read_to_end(&mut vec![]).is_err());
    }
}
