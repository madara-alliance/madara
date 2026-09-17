//! Wire format and deterministic validation shared by signers and settlement clients.
//! A certificate attests to past blob validation, not retention or execution validity.

use alloy_primitives::{b256, keccak256, Address, Bytes, Signature, B256, U256};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const STARK_PRIME: B256 = b256!("0800000000000011000000000000000000000000000000000000000000000001");
pub const MAX_OUTPUT_WORDS: usize = 65536;
pub const MAX_COMMITTEE_MEMBERS: usize = 32;
pub const BLOB_BYTES: usize = 131072;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid committee policy: {0}")]
    Policy(&'static str),
    #[error("invalid aggregator output: {0}")]
    Output(&'static str),
    #[error("invalid certificate: {0}")]
    Certificate(&'static str),
}

/// Trusted configuration, loaded independently of an attestation request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub chain_id: u64,
    pub core_address: Address,
    pub os_program_hash: B256,
    pub aggregator_program_hash: B256,
    pub config_hash: B256,
    pub committee_epoch: u64,
    pub members: Vec<Address>,
    pub threshold: usize,
    pub max_blobs: usize,
}

impl Policy {
    pub fn validate(&self) -> Result<(), Error> {
        if self.chain_id == 0 || self.core_address.is_zero() || self.committee_epoch == 0 {
            return Err(Error::Policy("chain, core and epoch must be nonzero"));
        }
        if self.threshold == 0 || self.threshold > self.members.len() || self.members.len() > MAX_COMMITTEE_MEMBERS {
            return Err(Error::Policy("invalid threshold or committee size"));
        }
        let mut members = self.members.clone();
        members.sort();
        if members.iter().any(|member| member.is_zero()) || members.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::Policy("zero or duplicate member"));
        }
        if self.os_program_hash.is_zero()
            || self.aggregator_program_hash.is_zero()
            || self.os_program_hash >= STARK_PRIME
            || self.aggregator_program_hash >= STARK_PRIME
            || self.config_hash >= STARK_PRIME
            || self.max_blobs == 0
            || self.max_blobs > 64
        {
            return Err(Error::Policy("invalid program/config hash or blob bound"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestationRequest {
    pub chain_id: u64,
    pub core_address: Address,
    pub committee_epoch: u64,
    pub program_output: Vec<B256>,
    /// Raw EIP-4844 blob bytes in output order, encoded as 0x-prefixed hex in JSON.
    pub blobs: Vec<Bytes>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attestation {
    pub digest: B256,
    pub committee_epoch: u64,
    pub signer: Address,
    pub signature: Bytes,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Certificate {
    pub digest: B256,
    pub committee_epoch: u64,
    /// Exactly threshold signatures, sorted by recovered address.
    pub signatures: Vec<Bytes>,
}

/// Small polling response; blob bytes are fetched only for work a signer has not completed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkOffer {
    pub job_id: String,
    pub batch: u64,
    pub digest: B256,
    pub committee_epoch: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkPage {
    pub jobs: Vec<WorkOffer>,
    pub next_cursor: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkData {
    pub job_id: String,
    pub request: AttestationRequest,
}

pub struct ParsedOutput {
    pub z: [u8; 32],
    pub commitments: Vec<[u8; 48]>,
    pub evaluations: Vec<[u8; 32]>,
}

fn count(word: B256) -> Result<usize, Error> {
    usize::try_from(U256::from_be_bytes(word.0)).map_err(|_| Error::Output("length overflow"))
}

pub fn validate_output(output: &[B256], policy: &Policy) -> Result<ParsedOutput, Error> {
    policy.validate()?;
    if output.len() < 18 || output.len() > MAX_OUTPUT_WORDS {
        return Err(Error::Output("invalid word count"));
    }
    if output.iter().any(|word| *word >= STARK_PRIME) {
        return Err(Error::Output("word outside Stark field"));
    }
    if output[6] != policy.os_program_hash
        || output[7] != policy.config_hash
        || output[8] != word(1)
        || output[9] != B256::ZERO
    {
        return Err(Error::Output("wrong OS, config, KZG flag or full-output flag"));
    }
    let n = count(output[11])?;
    if n == 0 || n > policy.max_blobs {
        return Err(Error::Output("blob count outside policy"));
    }
    let end = 12 + 4 * n;
    if output.len() < end + 2 {
        return Err(Error::Output("truncated KZG segment"));
    }
    let mut commitments = Vec::with_capacity(n);
    let mut evaluations = Vec::with_capacity(n);
    for i in 0..n {
        let low = output[12 + 2 * i];
        let high = output[13 + 2 * i];
        if low[..8] != [0; 8] || high[..8] != [0; 8] {
            return Err(Error::Output("commitment limb exceeds 192 bits"));
        }
        let mut commitment = [0; 48];
        commitment[..24].copy_from_slice(&high[8..]);
        commitment[24..].copy_from_slice(&low[8..]);
        commitments.push(commitment);
        let low = output[12 + 2 * n + 2 * i];
        let high = output[13 + 2 * n + 2 * i];
        if low[..16] != [0; 16] || high[..16] != [0; 16] {
            return Err(Error::Output("evaluation limb exceeds 128 bits"));
        }
        let mut evaluation = [0; 32];
        evaluation[..16].copy_from_slice(&high[16..]);
        evaluation[16..].copy_from_slice(&low[16..]);
        evaluations.push(evaluation);
    }
    // Validate both message segments, including each variable-length message.
    let mut offset = end;
    for (prefix, payload_offset) in [(3usize, 2usize), (5, 4)] {
        let length = count(*output.get(offset).ok_or(Error::Output("missing message segment"))?)?;
        offset += 1;
        let end = offset
            .checked_add(length)
            .filter(|end| *end <= output.len())
            .ok_or(Error::Output("truncated message segment"))?;
        while offset < end {
            if end - offset < prefix {
                return Err(Error::Output("truncated message header"));
            }
            let payload = count(output[offset + payload_offset])?;
            offset = offset
                .checked_add(prefix)
                .and_then(|x| x.checked_add(payload))
                .filter(|x| *x <= end)
                .ok_or(Error::Output("truncated message payload"))?;
        }
    }
    if offset != output.len() {
        return Err(Error::Output("trailing output words"));
    }
    Ok(ParsedOutput { z: output[10].0, commitments, evaluations })
}

pub fn validate_request(request: &AttestationRequest, policy: &Policy) -> Result<ParsedOutput, Error> {
    if request.chain_id != policy.chain_id
        || request.core_address != policy.core_address
        || request.committee_epoch != policy.committee_epoch
    {
        return Err(Error::Policy("request domain or epoch mismatch"));
    }
    let parsed = validate_output(&request.program_output, policy)?;
    if request.blobs.len() != parsed.commitments.len() || request.blobs.iter().any(|b| b.len() != BLOB_BYTES) {
        return Err(Error::Output("blob count or byte length mismatch"));
    }
    Ok(parsed)
}

pub fn word(value: u64) -> B256 {
    B256::from(U256::from(value).to_be_bytes::<32>())
}

fn hash_words(words: &[B256]) -> B256 {
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.0).collect();
    keccak256(bytes)
}

/// Exactly matches the core's hashMainPublicInput, without array offset/length words.
pub fn state_transition_fact(output: &[B256]) -> B256 {
    hash_words(output)
}

pub fn sharp_fact(output: &[B256], aggregator_program_hash: B256) -> B256 {
    hash_words(&[aggregator_program_hash, state_transition_fact(output)])
}

/// Call validate_output / validate_request before signing this digest.
pub fn signing_digest(output: &[B256], policy: &Policy) -> B256 {
    let mut core_word = [0; 32];
    core_word[12..].copy_from_slice(policy.core_address.as_slice());
    let domain = hash_words(&[
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
        keccak256("KzgCommitmentAttestor"),
        keccak256("1"),
        word(policy.chain_id),
        B256::from(core_word),
    ]);
    let statement = hash_words(&[
        keccak256("BlobCheck(bytes32 sharpFact,uint256 committeeEpoch)"),
        sharp_fact(output, policy.aggregator_program_hash),
        word(policy.committee_epoch),
    ]);
    let mut preimage = Vec::with_capacity(66);
    preimage.extend_from_slice(&[0x19, 0x01]);
    preimage.extend_from_slice(domain.as_slice());
    preimage.extend_from_slice(statement.as_slice());
    keccak256(preimage)
}

pub fn recover_signer(digest: B256, signature: &[u8]) -> Result<Address, Error> {
    if signature.len() != 65 || !matches!(signature[64], 27 | 28) {
        return Err(Error::Certificate("signature must be 65 bytes with v=27/28"));
    }
    let signature = Signature::from_raw(signature).map_err(|_| Error::Certificate("invalid signature"))?;
    if signature.normalize_s().is_some() {
        return Err(Error::Certificate("high-s signature"));
    }
    signature.recover_address_from_prehash(&digest).map_err(|_| Error::Certificate("signature recovery failed"))
}

pub fn verify_certificate(output: &[B256], policy: &Policy, certificate: &Certificate) -> Result<(), Error> {
    validate_output(output, policy)?;
    let digest = signing_digest(output, policy);
    if certificate.committee_epoch != policy.committee_epoch
        || certificate.digest != digest
        || certificate.signatures.len() != policy.threshold
    {
        return Err(Error::Certificate("digest, epoch or threshold mismatch"));
    }
    let mut previous = Address::ZERO;
    for signature in &certificate.signatures {
        let signer = recover_signer(digest, signature)?;
        if signer <= previous || !policy.members.contains(&signer) {
            return Err(Error::Certificate("duplicate, unsorted or unauthorized signer"));
        }
        previous = signer;
    }
    Ok(())
}
