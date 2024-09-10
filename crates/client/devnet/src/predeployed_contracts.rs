use anyhow::Context;
use blockifier::abi::{abi_utils::get_fee_token_var_address, sierra_types::next_storage_key};
use core::fmt;
use mc_db::MadaraBackend;
use mp_block::{BlockId, BlockTag};
use starknet_core::types::Felt;
use starknet_signers::SigningKey;

use crate::{
    ContractFeeTokensBalance, ERC20_ETH_CONTRACT_ADDRESS, ERC20_STRK_CONTRACT_ADDRESS, ETH_WEI_DECIMALS,
    STRK_FRI_DECIMALS,
};

pub struct DevnetPredeployedContract {
    pub address: Felt,
    pub secret: SigningKey,
    pub pubkey: Felt,
    pub balance: ContractFeeTokensBalance,
}

pub struct DevnetKeys(pub Vec<DevnetPredeployedContract>);

impl fmt::Display for DevnetKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        writeln!(f, "==== DEVNET PREDEPLOYED CONTRACTS ====")?;
        writeln!(f)?;
        for (i, contract) in self.0.iter().enumerate() {
            writeln!(f, "(#{}) Address: {:#x}", i + 1, contract.address,)?;
            writeln!(f, "  Private key: {:#x}", contract.secret.secret_scalar())?;
            match contract.balance.as_u128_fri_wei() {
                Ok((fri, wei)) => {
                    let (strk, eth) = (fri / STRK_FRI_DECIMALS, wei / ETH_WEI_DECIMALS);
                    writeln!(f, "  Balance: {strk} STRK, {eth} ETH")?;
                    writeln!(f)?;
                }
                Err(err) => writeln!(f, "Error getting balance: {err:#}\n")?,
            }
        }
        Ok(())
    }
}

/// Returns an `u128`. This is for tests only as an ERC20 contract may have a higher balance than an u128.
pub fn get_bal_contract(
    backend: &MadaraBackend,
    contract_address: Felt,
    fee_token_address: Felt,
) -> anyhow::Result<Felt> {
    let low_key = get_fee_token_var_address(
        contract_address
            .try_into()
            .with_context(|| format!("Converting felt {:#x} to contract address", contract_address))?,
    );
    let high_key = next_storage_key(&low_key).unwrap();
    let low = backend
        .get_contract_storage_at(&BlockId::Tag(BlockTag::Pending), &fee_token_address, &low_key)
        .unwrap()
        .unwrap_or(Felt::ZERO);
    let high = backend
        .get_contract_storage_at(&BlockId::Tag(BlockTag::Pending), &fee_token_address, &high_key)
        .unwrap()
        .unwrap_or(Felt::ZERO);
    log::debug!("get_fee_token_balance contract_address={contract_address:#x} fee_token_address={fee_token_address:#x} low_key={low_key:?}, got {low:#x} {high:#x}");

    assert_eq!(high, Felt::ZERO); // for now we never use high let's keep it out of the api
                                  // (blockifier does not even support it fully I believe, as the total supply of STRK/ETH would not reach the high bits.)

    Ok(low)
}

/// (STRK in FRI, ETH in WEI)
pub fn get_fee_tokens_balance(
    backend: &MadaraBackend,
    contract_address: Felt,
) -> anyhow::Result<ContractFeeTokensBalance> {
    Ok(ContractFeeTokensBalance {
        fri: get_bal_contract(backend, contract_address, ERC20_STRK_CONTRACT_ADDRESS)?,
        wei: get_bal_contract(backend, contract_address, ERC20_ETH_CONTRACT_ADDRESS)?,
    })
}

impl DevnetKeys {
    pub fn from_db(backend: &MadaraBackend) -> anyhow::Result<Self> {
        let keys = backend
            .get_devnet_predeployed_keys()
            .context("Getting the devnet predeployed keys from db")?
            .context("The current database was not initialized in devnet mode")?;

        let keys = keys
            .0
            .into_iter()
            .map(|k| {
                Ok(DevnetPredeployedContract {
                    address: k.address,
                    secret: SigningKey::from_secret_scalar(k.secret),
                    pubkey: k.pubkey,
                    balance: get_fee_tokens_balance(backend, k.address)?,
                })
            })
            .collect::<anyhow::Result<_>>()?;

        Ok(Self(keys))
    }

    pub fn save_to_db(&self, backend: &MadaraBackend) -> anyhow::Result<()> {
        let keys = mc_db::devnet_db::DevnetPredeployedKeys(
            self.0
                .iter()
                .map(|k| mc_db::devnet_db::DevnetPredeployedContractKey {
                    address: k.address,
                    secret: k.secret.secret_scalar(),
                    pubkey: k.pubkey,
                })
                .collect(),
        );
        backend.set_devnet_predeployed_keys(keys).context("Saving devnet predeployed contracts keys to database")?;

        Ok(())
    }
}
