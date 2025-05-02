use std::sync::Arc;

use starknet::{
    accounts::{Account, ExecutionEncoding, SingleOwnerAccount},
    macros::{felt, selector},
    signers::{LocalWallet, SigningKey},
};
use starknet_core::types::{contract::SierraClass, Call};
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient};
use starknet_types_core::felt::Felt;

use crate::{MadaraCmd, MadaraCmdBuilder};

pub const DEVNET_CHAIN_ID: Felt = felt!("0x4d41444152415f4445564e4554");

/// The default accounts deployed on the devnet.
/// Tuple (address, private_key)
pub const DEVNET_ACCOUNTS: [(Felt, Felt); 10] = [
    (
        felt!("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d"),
        felt!("0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07"),
    ),
    (
        felt!("0x008a1719e7ca19f3d91e8ef50a48fc456575f645497a1d55f30e3781f786afe4"),
        felt!("0x0514977443078cf1e0c36bc88b89ada9a46061a5cf728f40274caea21d76f174"),
    ),
    (
        felt!("0x0733a8e2bcced14dcc2608462bd96524fb64eef061689b6d976708efc2c8ddfd"),
        felt!("0x00177100ae65c71074126963e695e17adf5b360146f960378b5cdfd9ed69870b"),
    ),
    (
        felt!("0x025073e0772b1e348a5da66ea67fb46f75ecdca1bd24dbbc98567cbf4a0e00b3"),
        felt!("0x07ae55c8093920562c1cbab9edeb4eb52f788b93cac1d5721bda20c96100d743"),
    ),
    (
        felt!("0x0294f066a54e07616fd0d50c935c2b5aa616d33631fec94b34af8bd4f6296f68"),
        felt!("0x02ce1754eb64b7899c64dcdd0cff138864be2514e70e7761c417b728f2bf7457"),
    ),
    (
        felt!("0x005d1d65ea82aa0107286e68537adf0371601789e26b1cd6e455a8e5be5c5665"),
        felt!("0x037a683c3969bf18044c9d2bbe0b1739897c89cf25420342d6dfc36c30fc519d"),
    ),
    (
        felt!("0x01d775883a0a6e5405a345f18d7639dcb54b212c362d5a99087f742fba668396"),
        felt!("0x07b4a2263d9cc475816a03163df7efd58552f1720c8df0bd2a813663895ef022"),
    ),
    (
        felt!("0x04add50f5bcc31a8418b43b1ddc8d703986094baf998f8e9625e13dbcc3df18b"),
        felt!("0x064b37f84e667462b95dc56e3c5e93a703ef16d73de7b9c5bfd92b90f11f90e1"),
    ),
    (
        felt!("0x03dbe3dd8c2f721bc24e87bcb739063a10ee738cef090bc752bc0d5a29f10b72"),
        felt!("0x0213d0d77d5ff9ffbeabdde0af7513e89aafd5e36ae99b8401283f6f57c57696"),
    ),
    (
        felt!("0x07484e8e3af210b2ead47fa08c96f8d18b616169b350a8b75fe0dc4d2e01d493"),
        felt!("0x0410c6eadd73918ea90b6658d24f5f2c828e39773819c1443d8602a3c72344c2"),
    ),
];

pub const STRK_CONTRACT_ADDRESS: Felt = felt!("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
pub const ETH_CONTRACT_ADDRESS: Felt = felt!("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");

pub const MAX_FEE: Felt = felt!("0x186a0"); // 100000

pub struct RpcTestContext {
    pub madara: MadaraCmd,
}

impl RpcTestContext {
    pub async fn new() -> Self {
        let args =
            ["--devnet", "--no-l1-sync", "--gas-price=10", "--chain-config-override=null_timestamp=true,block_time=5s"];

        let mut madara = MadaraCmdBuilder::new().args(args).run();

        madara.wait_for_ready().await;

        madara.wait_for_sync_to(0).await;

        Self { madara }
    }

    pub fn rpc_client(&self) -> &JsonRpcClient<HttpTransport> {
        self.madara.json_rpc()
    }

    async fn transfer(
        &self,
        from_idx: usize,
        to_idx: usize,
        amount: u128,
        token_address: &Felt,
    ) -> Result<Felt, anyhow::Error> {
        assert!(from_idx < DEVNET_ACCOUNTS.len(), "from index out of bounds");
        assert!(to_idx < DEVNET_ACCOUNTS.len(), "to index out of bounds");

        let signer = LocalWallet::from(SigningKey::from_secret_scalar(DEVNET_ACCOUNTS[from_idx].1));
        let client = self.rpc_client();

        let account = SingleOwnerAccount::new(
            client,
            signer,
            DEVNET_ACCOUNTS[from_idx].0,
            DEVNET_CHAIN_ID,
            ExecutionEncoding::New,
        );

        // calldatsa for ERC20 transfer
        // [recipient, amount_lsb, amount_msb]
        let calldata = vec![DEVNET_ACCOUNTS[to_idx].0, Felt::from(amount), Felt::ZERO];

        let call = Call { to: *token_address, selector: selector!("transfer"), calldata };

        Ok(account.execute_v1(vec![call]).max_fee(MAX_FEE).send().await.map(|res| res.transaction_hash)?)
    }

    async fn declare_sierra_test_contract(&self, from_idx: usize) -> Result<Felt, anyhow::Error> {
        assert!(from_idx < DEVNET_ACCOUNTS.len(), "from index out of bounds");

        let signer = LocalWallet::from(SigningKey::from_secret_scalar(DEVNET_ACCOUNTS[from_idx].1));
        let client = self.rpc_client();

        let account = SingleOwnerAccount::new(
            client,
            signer,
            DEVNET_ACCOUNTS[from_idx].0,
            DEVNET_CHAIN_ID,
            ExecutionEncoding::New,
        );

        let sierra_class: SierraClass = serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
        let compiled_class_hash = felt!("0x0138105ded3d2e4ea1939a0bc106fb80fd8774c9eb89c1890d4aeac88e6a1b27");

        Ok(account
            .declare_v2(Arc::new(sierra_class.flatten().expect("failed to flatten sierra class")), compiled_class_hash)
            .send()
            .await
            .map(|res| res.transaction_hash)?)
    }

    async fn deploy_sierra_contract(&self, from_idx: usize, class_hash: Felt) -> Result<Felt, anyhow::Error> {
        assert!(from_idx < DEVNET_ACCOUNTS.len(), "from index out of bounds");

        let signer = LocalWallet::from(SigningKey::from_secret_scalar(DEVNET_ACCOUNTS[from_idx].1));
        let client = self.rpc_client();

        let account = SingleOwnerAccount::new(
            client,
            signer,
            DEVNET_ACCOUNTS[from_idx].0,
            DEVNET_CHAIN_ID,
            ExecutionEncoding::New,
        );

        let constructor_calldata = vec![];
        let salt = Felt::ZERO;
        let mut calldata = vec![class_hash, salt, constructor_calldata.len().into()];
        calldata.extend_from_slice(&constructor_calldata);
        calldata.push(Felt::ONE); // deploy from zero

        let call = Call { to: account.address(), selector: selector!("deploy_contract"), calldata };

        Ok(account.execute_v1(vec![call]).send().await.map(|res| res.transaction_hash)?)
    }

    pub async fn block_1(&self) -> Vec<Felt> {
        let transferts = [
            (0, 1, 10),
            (1, 2, 20),
            (2, 3, 30),
            (3, 4, 40),
            (4, 5, 50),
            (5, 6, 60),
            (6, 7, 70),
            (7, 8, 80),
            (8, 9, 90),
        ];

        let mut txs_hashes: Vec<Felt> = Vec::new();

        for (from, to, amount) in transferts.iter() {
            let tx_hash =
                self.transfer(*from, *to, *amount, &STRK_CONTRACT_ADDRESS).await.expect("failed to send transfer");
            txs_hashes.push(tx_hash);
        }

        let fail_transfert =
            self.transfer(9, 0, u128::MAX, &STRK_CONTRACT_ADDRESS).await.expect("failed to send transfer");
        txs_hashes.push(fail_transfert);

        txs_hashes
    }

    pub async fn block_2(&self) -> Vec<Felt> {
        let transferts = [(0, 1, 10), (1, 2, 20), (2, 3, 30)];

        let mut txs_hashes: Vec<Felt> = Vec::new();

        for (from, to, amount) in transferts.iter() {
            let tx_hash =
                self.transfer(*from, *to, *amount, &ETH_CONTRACT_ADDRESS).await.expect("failed to send transfer");
            txs_hashes.push(tx_hash);
        }

        txs_hashes
    }

    // pub async fn block_3(&self) -> Vec<Felt> {
    //     vec![self.declare_sierra_test_contract(0).await.expect("failed to declare sierra test contract")]
    // }

    // pub async fn block_4(&self) -> Vec<Felt> {
    //     vec![self.deploy_sierra_test_contract(0, ).await.expect("failed to deploy sierra test contract")]
    // }
}
