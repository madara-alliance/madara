use bincode::Options;
use futures::TryStreamExt;
use mc_e2e_tests::MadaraCmdBuilder;
use mc_external_db::mongodb::MempoolTransactionDocument;
use mp_class::FlattenedSierraClass;
use mp_convert::Felt;
use mp_transactions::validated::ValidatedTransaction;
use starknet::accounts::{Account, AccountFactory, ExecutionEncoding, OpenZeppelinAccountFactory, SingleOwnerAccount};
use starknet::signers::LocalWallet;
use starknet_core::types::contract::SierraClass;
use starknet_core::types::{BlockId, BlockTag, Call as StarknetCall, MaybePreConfirmedStateUpdate};
use starknet_core::utils::starknet_keccak;
use starknet_providers::Provider;
use starknet_signers::SigningKey;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use testcontainers::{core::IntoContainerPort, runners::AsyncRunner, ContainerAsync, GenericImage};
use tokio::sync::OnceCell;

const DEVNET_STRK_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
const DEVNET_ACCOUNT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d");
const DEVNET_ACCOUNT_SECRET: Felt =
    Felt::from_hex_unchecked("0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07");

const L2_GAS_PRICE: u128 = 200_000;
const L2_GAS: u64 = 1_000_000_000;
const L1_GAS_PRICE: u128 = 128;
const L1_GAS: u64 = 30_000;
const L1_DATA_GAS_PRICE: u128 = 128;
const L1_DATA_GAS: u64 = 30_000;

macro_rules! with_devnet_fees {
    ($builder:expr) => {
        $builder
            .l2_gas_price(L2_GAS_PRICE)
            .l2_gas(L2_GAS)
            .l1_gas_price(L1_GAS_PRICE)
            .l1_gas(L1_GAS)
            .l1_data_gas_price(L1_DATA_GAS_PRICE)
            .l1_data_gas(L1_DATA_GAS)
    };
}

/// Poll a condition until success or timeout.
async fn wait_for_cond<F, R>(mut cond: impl FnMut() -> F, sleep_duration: Duration, max_attempts: u32) -> R
where
    F: std::future::Future<Output = Result<R, anyhow::Error>>,
{
    let start = std::time::Instant::now();
    let mut attempt = 0;
    loop {
        let err = match cond().await {
            Ok(r) => break r,
            Err(err) => err,
        };

        attempt += 1;
        if attempt >= max_attempts {
            panic!("Condition not satisfied after {:?}: {err:#}", start.elapsed());
        }

        tokio::time::sleep(sleep_duration).await;
    }
}

/// Call admin RPC to revert the chain to a given block hash.
async fn admin_revert_to(admin_url: &str, block_hash: Felt) {
    let client = reqwest::Client::new();
    let response = client
        .post(admin_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "madara_revertTo",
            "params": [format!("0x{:x}", block_hash)],
            "id": 1,
        }))
        .send()
        .await
        .unwrap();
    let value = response.json::<serde_json::Value>().await.unwrap();
    if value.get("error").is_some() {
        panic!("admin revert failed: {value}");
    }
}

/// Wait until a transaction is included in a block.
async fn wait_for_tx_in_block<P: Provider + Sync>(provider: &P, tx_hash: Felt, timeout_attempts: u32) {
    wait_for_cond(
        || async {
            let receipt = provider.get_transaction_receipt(tx_hash).await?;
            if receipt.block.is_block() {
                Ok(())
            } else {
                Err(anyhow::anyhow!("tx not in block yet"))
            }
        },
        Duration::from_millis(500),
        timeout_attempts,
    )
    .await
}

/// Build a devnet STRK transfer call.
fn make_transfer_call(to: Felt, amount: u64) -> StarknetCall {
    StarknetCall {
        to: DEVNET_STRK_CONTRACT_ADDRESS,
        selector: starknet_keccak(b"transfer"),
        calldata: vec![to, Felt::from(amount), Felt::ZERO],
    }
}

/// Wait for MongoDB to accept connections.
async fn wait_for_mongo_ready(uri: &str) {
    let mut attempts = 0;
    loop {
        if let Ok(client) = mongodb::Client::with_uri_str(uri).await {
            if client.database("admin").run_command(mongodb::bson::doc! { "ping": 1 }).await.is_ok() {
                break;
            }
        }
        attempts += 1;
        if attempts > 20 {
            panic!("MongoDB did not become ready in time");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

struct MongoFixture {
    _node: ContainerAsync<GenericImage>,
    uri: String,
}

static MONGO_FIXTURE: OnceCell<MongoFixture> = OnceCell::const_new();

/// Resolve the devnet chain config path used in tests.
fn test_devnet_path() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/test_devnet.yaml").to_string_lossy().into_owned()
}

/// Extract the new root from a confirmed update.
fn extract_new_root(update: MaybePreConfirmedStateUpdate) -> Felt {
    match update {
        MaybePreConfirmedStateUpdate::Update(update) => update.new_root,
        MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
            panic!("unexpected pre-confirmed state update when confirmed update was expected")
        }
    }
}

/// Extract the block hash from a confirmed update.
fn extract_block_hash(update: MaybePreConfirmedStateUpdate) -> Felt {
    match update {
        MaybePreConfirmedStateUpdate::Update(update) => update.block_hash,
        MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
            panic!("unexpected pre-confirmed state update when confirmed update was expected")
        }
    }
}

/// Create a singleton MongoDB test container.
async fn mongo_fixture() -> &'static MongoFixture {
    MONGO_FIXTURE
        .get_or_init(|| async {
            let image = GenericImage::new("mongo", "7.0").with_exposed_port(27017.tcp());
            let node = image.start().await.unwrap();
            let port = node.get_host_port_ipv4(27017).await.unwrap();
            let uri = format!("mongodb://127.0.0.1:{port}");
            wait_for_mongo_ready(&uri).await;
            MongoFixture { _node: node, uri }
        })
        .await
}

/// E2E: devnet + Mongo WAL replay should reproduce the same state root.
///
/// Flow:
/// 1) Start devnet with external DB enabled and fast block time.
/// 2) Submit multiple tx types (invoke, declare, deploy_account) to generate WAL entries.
/// 3) Wait for inclusion and record the state root at the last tx block.
/// 4) Revert to genesis, restart node (clears in-memory state).
/// 5) Replay WAL in arrival order and wait for inclusion.
/// 6) Assert replayed root == original root.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_devnet_replay_from_mongo_matches_root() {
    // Spin up MongoDB for the external DB WAL.
    let fixture = mongo_fixture().await;
    let uri = fixture.uri.clone();

    let db_suffix = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis();
    let db_name = format!("madara_devnet_replay_{db_suffix}");

    // Start a devnet node with external DB enabled and a fast block time to reach 11+ blocks quickly.
    let test_devnet_path = test_devnet_path();
    let args = [
        "--devnet",
        "--no-l1-sync",
        "--no-mempool-saving",
        "--chain-config-path",
        test_devnet_path.as_str(),
        "--chain-config-override",
        "block_time=2s",
        "--rpc-admin",
        "--rpc-unsafe",
        "--gateway",
        "--gateway-trusted-add-transaction-endpoint",
    ];

    let cmd_builder = MadaraCmdBuilder::new()
        .env([
            ("MADARA_EXTERNAL_DB_ENABLED", "true"),
            ("MADARA_EXTERNAL_DB_MONGODB_URI", uri.as_str()),
            ("MADARA_EXTERNAL_DB_DATABASE", db_name.as_str()),
            ("MADARA_EXTERNAL_DB_COLLECTION", "mempool_transactions"),
            ("MADARA_EXTERNAL_DB_BATCH_SIZE", "50"),
            ("MADARA_EXTERNAL_DB_FLUSH_INTERVAL_MS", "100"),
            ("MADARA_EXTERNAL_DB_STRICT_OUTBOX", "true"),
        ])
        .args(args)
        .enable_gateway()
        .label("external-db-devnet");

    let mut madara = cmd_builder.clone().run();

    madara.wait_for_ready().await;
    madara.wait_for_sync_to(0).await;

    let rpc = madara.json_rpc();
    let chain_id = rpc.chain_id().await.unwrap();

    // Build a devnet account for declare + invoke transactions.
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(DEVNET_ACCOUNT_SECRET));
    let mut account =
        SingleOwnerAccount::new(rpc.clone(), signer, DEVNET_ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::Latest));

    // Compute a fresh deploy-account address and fund it first.
    let oz_class_hash = rpc.get_class_hash_at(BlockId::Tag(BlockTag::Latest), DEVNET_ACCOUNT_ADDRESS).await.unwrap();
    let new_account_key = SigningKey::from_random();
    let mut factory = OpenZeppelinAccountFactory::new(
        oz_class_hash,
        chain_id,
        LocalWallet::from_signing_key(new_account_key.clone()),
        &rpc,
    )
    .await
    .unwrap();
    factory.set_block_id(BlockId::Tag(BlockTag::Latest));

    let deploy = factory.deploy_v3(Felt::from_hex_unchecked("0x123"));
    let deploy_address = deploy.address();

    // Use a strict nonce sequence for predictable WAL ordering.
    let mut nonce = rpc.get_nonce(BlockId::Tag(BlockTag::Latest), DEVNET_ACCOUNT_ADDRESS).await.unwrap();

    // 1) Transfer to fund the new account.
    let funding_tx =
        with_devnet_fees!(account.execute_v3(vec![make_transfer_call(deploy_address, 1_000_000_000_000_000)]))
            .nonce(nonce)
            .send()
            .await
            .unwrap();
    wait_for_tx_in_block(&rpc, funding_tx.transaction_hash, 60).await;
    nonce += Felt::ONE;

    // 2) Declare a Sierra class to cover DECLARE flow.
    let sierra_class: SierraClass = serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
    let flattened_class = sierra_class.clone().flatten().unwrap();
    let compiled_hashes = FlattenedSierraClass::from(flattened_class.clone()).compile_to_casm_with_hashes().unwrap();
    let compiled_class_hash = compiled_hashes.blake_hash;

    let declare_tx =
        with_devnet_fees!(account.declare_v3(flattened_class.clone().into(), compiled_class_hash).nonce(nonce))
            .send()
            .await
            .unwrap();
    wait_for_tx_in_block(&rpc, declare_tx.transaction_hash, 60).await;
    nonce += Felt::ONE;

    // 3) Deploy the new account to cover DEPLOY_ACCOUNT flow.
    let deploy_tx = with_devnet_fees!(deploy.nonce(Felt::ZERO)).send().await.unwrap();
    wait_for_tx_in_block(&rpc, deploy_tx.transaction_hash, 60).await;

    // 4) Another invoke to ensure multiple blocks are produced.
    let invoke_tx = with_devnet_fees!(account.execute_v3(vec![make_transfer_call(DEVNET_ACCOUNT_ADDRESS, 40)]))
        .nonce(nonce)
        .send()
        .await
        .unwrap();
    wait_for_tx_in_block(&rpc, invoke_tx.transaction_hash, 60).await;

    // Record the block and root after the last transaction we care about.
    let invoke_receipt = rpc.get_transaction_receipt(invoke_tx.transaction_hash).await.unwrap();
    let tx_block = invoke_receipt.block.block_number();

    // Ensure we have at least 11 blocks to exercise block production timing (no admin close).
    wait_for_cond(
        || async {
            let block = rpc.block_hash_and_number().await?;
            if block.block_number >= 11 {
                Ok(())
            } else {
                Err(anyhow::anyhow!("chain not at 11 blocks yet"))
            }
        },
        Duration::from_millis(500),
        60,
    )
    .await;

    let target_block = tx_block;
    let target_root = extract_new_root(rpc.get_state_update(BlockId::Number(target_block)).await.unwrap());

    // Verify WAL entries landed in Mongo for each tx hash.
    let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
    let collection = client.database(&db_name).collection::<MempoolTransactionDocument>("mempool_transactions");

    let tx_ids = vec![
        format!("0x{:x}", funding_tx.transaction_hash),
        format!("0x{:x}", declare_tx.transaction_hash),
        format!("0x{:x}", deploy_tx.transaction_hash),
        format!("0x{:x}", invoke_tx.transaction_hash),
    ];
    wait_for_cond(
        || async {
            let count = collection.count_documents(mongodb::bson::doc! { "tx_hash": { "$in": &tx_ids } }).await?;
            if count >= tx_ids.len() as u64 {
                Ok(())
            } else {
                Err(anyhow::anyhow!("mongo not updated yet"))
            }
        },
        Duration::from_millis(200),
        60,
    )
    .await;

    // Revert to genesis and restart to avoid in-memory stale state.
    let admin_url = format!("{}rpc/v0.1.0/", madara.rpc_admin_url());
    let genesis_hash = extract_block_hash(rpc.get_state_update(BlockId::Number(0)).await.unwrap());
    admin_revert_to(&admin_url, genesis_hash).await;

    wait_for_cond(
        || async {
            let block = rpc.block_hash_and_number().await?;
            if block.block_number == 0 {
                Ok(())
            } else {
                Err(anyhow::anyhow!("chain not reverted yet"))
            }
        },
        Duration::from_millis(500),
        30,
    )
    .await;

    // Restart after revert to avoid stale in-memory state in Madara (known issue after reorg).
    madara.stop();
    let mut madara = cmd_builder.clone().run();
    madara.wait_for_ready().await;
    madara.wait_for_sync_to(0).await;

    let rpc = madara.json_rpc();

    // Replay WAL entries in arrival order, skipping duplicates.
    let mut cursor = collection
        .find(mongodb::bson::doc! {})
        .sort(mongodb::bson::doc! { "block_number": 1, "arrived_at": 1, "_id": 1 })
        .await
        .unwrap();
    let mut replayed = 0usize;
    let mut seen_hashes = HashSet::new();
    while let Some(doc) = cursor.try_next().await.unwrap() {
        if !seen_hashes.insert(doc.tx_hash.clone()) {
            continue;
        }
        let raw = doc.raw_transaction.bytes;
        let validated: ValidatedTransaction = bincode::deserialize(&raw).unwrap();
        let bytes = bincode::options().serialize(&validated).unwrap();
        let response = madara
            .gateway_root_post("madara/trusted_add_validated_transaction")
            .await
            .body(bytes)
            .header("Content-Type", "application/octet-stream")
            .send()
            .await
            .unwrap();
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            if body.contains("DuplicateTransaction")
                || body.contains("TransactionAlreadyExists")
                || body.contains("already in the mempool")
            {
                continue;
            }
            panic!("replay request failed: {}", body);
        }
        replayed += 1;
        // Ensure the replayed transaction is included before moving on.
        wait_for_tx_in_block(&rpc, validated.hash, 120).await;
    }
    assert!(replayed >= tx_ids.len(), "expected at least {} replayed transactions, got {replayed}", tx_ids.len());

    // Wait for block production to reach the recorded block before comparing roots.
    wait_for_cond(
        || async {
            let block = rpc.block_hash_and_number().await?;
            if block.block_number >= target_block {
                Ok(())
            } else {
                Err(anyhow::anyhow!("chain not caught up yet"))
            }
        },
        Duration::from_millis(500),
        80,
    )
    .await;

    let replay_root = extract_new_root(rpc.get_state_update(BlockId::Number(target_block)).await.unwrap());
    assert_eq!(replay_root, target_root);
}
