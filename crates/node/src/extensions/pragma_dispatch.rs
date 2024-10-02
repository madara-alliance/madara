//! ExEx of Pragma Dispatcher
//! Adds a new TX at the end of each block, dispatching a message through
//! Hyperlane.
use std::{sync::Arc, time::Duration};

use anyhow::bail;
use futures::StreamExt;
use mp_rpc::Starknet;
use starknet_api::felt;
use starknet_core::types::{
    BlockId, BlockTag, BroadcastedInvokeTransaction, BroadcastedInvokeTransactionV1, BroadcastedTransaction,
    ExecutionResult, Felt, FunctionCall, InvokeTransactionReceipt, InvokeTransactionResult, TransactionReceipt,
    TransactionReceiptWithBlockInfo, TransactionStatus,
};
use starknet_signers::SigningKey;

use mc_devnet::{Call, Multicall, Selector};
use mc_mempool::transaction_hash;
use mc_rpc::versions::v0_7_1::{StarknetReadRpcApiV0_7_1Server, StarknetWriteRpcApiV0_7_1Server};
use mp_convert::ToFelt;
use mp_exex::{ExExContext, ExExEvent, ExExNotification};
use mp_transactions::broadcasted_to_blockifier;
use tokio::time::sleep;

const PENDING_BLOCK: BlockId = BlockId::Tag(BlockTag::Pending);
// Update the feed ids from the Feed Registry every 500 blocks. (500s)
const UPDATE_FEEDS_INTERVAL: u64 = 500;

lazy_static::lazy_static! {
    // TODO: Keystore path?
    pub static ref ACCOUNT_ADDRESS: Felt = felt!("0x4a2b383d808b7285cc98b2309f974f5111633c84fd82c9375c118485d2d57ba");
    pub static ref PRIVATE_KEY: SigningKey = SigningKey::from_secret_scalar(felt!("0x7a9779748888c95d96bbbce041b5109c6ffc0c4f30561c0170384a5922d9e91"));

    // TODO: Replace by the correct addresses
    pub static ref PRAGMA_FEEDS_REGISTRY_ADDRESS: Felt = felt!("0x6c05a18cb507fdbb2049047538f2824116e118e5699ae163c7473da38df2bb");
    pub static ref PRAGMA_DISPATCHER_ADDRESS: Felt = felt!("0x25a70290a333bc22397a6dac4c44d3af50c3adfe7f397d504422fd72cb9858a");

    pub static ref MAX_FEE: Felt = felt!("2386F26FC10000"); // 0.01 eth
}

/// 🧩 Pragma main ExEx.
/// At the end of each produced block by the node, adds a new dispatch transaction
/// using the Pragma Dispatcher contract.
pub async fn exex_pragma_dispatch(mut ctx: ExExContext) -> anyhow::Result<()> {
    let mut feed_ids: Vec<Felt> = Vec::new();
    let mut last_fetch_block = 0;

    while let Some(notification) = ctx.notifications.next().await {
        let block_number = match notification {
            ExExNotification::BlockProduced { block: _, block_number } => block_number,
            ExExNotification::BlockSynced { block_number } => {
                ctx.events.send(ExExEvent::FinishedHeight(block_number))?;
                continue;
            }
        };

        if should_update_feed_ids(block_number.0, last_fetch_block, &feed_ids) {
            match update_feed_ids(&ctx.starknet, block_number.0, &mut feed_ids).await {
                Ok(()) => last_fetch_block = block_number.0,
                Err(_) => {
                    ctx.events.send(ExExEvent::FinishedHeight(block_number))?;
                    continue;
                }
            }
        }

        if feed_ids.is_empty() {
            log::warn!("🧩 [#{}] Pragma's ExEx: No feed IDs available, skipping dispatch", block_number);
            ctx.events.send(ExExEvent::FinishedHeight(block_number))?;
            continue;
        }

        if let Err(e) = process_dispatch_transaction(&ctx, &feed_ids, block_number.0).await {
            log::error!("🧩 [#{}] Pragma's ExEx: Error processing dispatch transaction: {:?}", block_number, e);
        }

        ctx.events.send(ExExEvent::FinishedHeight(block_number))?;
    }
    Ok(())
}

fn should_update_feed_ids(current_block: u64, last_fetch_block: u64, feed_ids: &[Felt]) -> bool {
    current_block.saturating_sub(last_fetch_block) >= UPDATE_FEEDS_INTERVAL || feed_ids.is_empty()
}

async fn update_feed_ids(starknet: &Arc<Starknet>, block_number: u64, feed_ids: &mut Vec<Felt>) -> anyhow::Result<()> {
    match get_feed_ids_from_registry(starknet).await {
        Ok(new_feed_ids) => {
            *feed_ids = new_feed_ids;
            log::info!("🧩 [#{}] Pragma's ExEx: 📜 Updated feed IDs: {:?}", block_number, feed_ids);
            Ok(())
        }
        Err(e) => {
            log::warn!("🧩 [#{}] Pragma's ExEx: Failed to fetch feed IDs: {:?}", block_number, e);
            Err(e)
        }
    }
}

async fn process_dispatch_transaction(ctx: &ExExContext, feed_ids: &[Felt], block_number: u64) -> anyhow::Result<()> {
    let invoke_result = create_and_add_dispatch_tx(&ctx.starknet, feed_ids, block_number).await?;
    let status = get_transaction_status(&ctx.starknet, &invoke_result.transaction_hash).await?;

    match status {
        TransactionStatus::AcceptedOnL2(_) | TransactionStatus::AcceptedOnL1(_) => {
            handle_accepted_transaction(ctx, &invoke_result.transaction_hash, block_number).await?;
        }
        TransactionStatus::Rejected => {
            log::error!("🧩 [#{}] Pragma's ExEx: Transaction rejected. Status: {:?}", block_number, status);
        }
        TransactionStatus::Received => {
            log::warn!("🧩 [#{}] Pragma's ExEx: Unexpected 'Received' status after polling", block_number);
        }
    }

    Ok(())
}

async fn create_and_add_dispatch_tx(
    starknet: &Arc<Starknet>,
    feed_ids: &[Felt],
    block_number: u64,
) -> anyhow::Result<InvokeTransactionResult> {
    let dispatch_tx = create_dispatch_tx(starknet, feed_ids)?;
    log::info!("🧩 [#{}] Pragma's ExEx: Adding dispatch transaction...", block_number);
    let invoke_result = starknet.add_invoke_transaction(dispatch_tx).await?;
    Ok(invoke_result)
}

async fn handle_accepted_transaction(
    ctx: &ExExContext,
    transaction_hash: &Felt,
    block_number: u64,
) -> anyhow::Result<()> {
    log::info!("🧩 [#{}] Pragma's ExEx: Transaction accepted.", block_number);

    let receipt = ctx.starknet.get_transaction_receipt(*transaction_hash).await?;

    if let TransactionReceiptWithBlockInfo {
        receipt:
            TransactionReceipt::Invoke(InvokeTransactionReceipt {
                execution_result: ExecutionResult::Reverted { reason },
                ..
            }),
        ..
    } = receipt
    {
        log::error!("🧩 [#{}] Pragma's ExEx: Transaction execution reverted. Reason: {}", block_number, reason);
    }

    Ok(())
}

/// Creates a new Dispatch transaction.
/// The transaction will be signed using the `ACCOUNT_ADDRESS` and `PRIVATE_KEY` constants.
fn create_dispatch_tx(starknet: &Arc<Starknet>, feed_ids: &[Felt]) -> anyhow::Result<BroadcastedInvokeTransaction> {
    let mut tx = BroadcastedInvokeTransaction::V1(BroadcastedInvokeTransactionV1 {
        sender_address: *ACCOUNT_ADDRESS,
        calldata: Multicall::default()
            .with(Call {
                to: *PRAGMA_DISPATCHER_ADDRESS,
                selector: Selector::from("dispatch"),
                calldata: feed_ids.to_vec(),
            })
            .flatten()
            .collect(),
        max_fee: *MAX_FEE,
        signature: vec![], // This will get filled below
        nonce: starknet.get_nonce(PENDING_BLOCK, *ACCOUNT_ADDRESS)?,
        is_query: false,
    });
    tx = sign_tx(starknet, tx)?;
    Ok(tx)
}

/// Sign a transaction using the constants.
fn sign_tx(
    starknet: &Arc<Starknet>,
    mut tx: BroadcastedInvokeTransaction,
) -> anyhow::Result<BroadcastedInvokeTransaction> {
    let (blockifier_tx, _) = broadcasted_to_blockifier(
        BroadcastedTransaction::Invoke(tx.clone()),
        starknet.chain_config.chain_id.to_felt(),
        starknet.chain_config.latest_protocol_version,
    )?;

    let signature = PRIVATE_KEY.sign(&transaction_hash(&blockifier_tx))?;
    let tx_signature = match &mut tx {
        BroadcastedInvokeTransaction::V1(tx) => &mut tx.signature,
        BroadcastedInvokeTransaction::V3(tx) => &mut tx.signature,
    };
    *tx_signature = vec![signature.r, signature.s];
    Ok(tx)
}

/// Wait for the TX to be accepted on chain & return status
async fn get_transaction_status(
    starknet: &Arc<Starknet>,
    transaction_hash: &Felt,
) -> anyhow::Result<TransactionStatus> {
    const POLLING_INTERVAL: Duration = Duration::from_millis(100);
    const ERROR_RETRY_INTERVAL: Duration = Duration::from_millis(10);
    const MAX_RETRIES: u32 = 50; // 5 seconds total retry time

    let mut retries = 0;

    loop {
        match starknet.get_transaction_status(*transaction_hash) {
            Ok(TransactionStatus::Received) => {
                log::debug!("🧩 Transaction status: Received. Polling again in {:?}...", POLLING_INTERVAL);
                sleep(POLLING_INTERVAL).await;
            }
            Ok(status) => return Ok(status),
            Err(e) if retries < MAX_RETRIES => {
                log::debug!("🧩 Failed to get transaction status (retry {}/{}): {:?}", retries + 1, MAX_RETRIES, e);
                retries += 1;
                sleep(ERROR_RETRY_INTERVAL).await;
            }
            Err(e) => bail!("Failed to get transaction status after {} retries: {:?}", MAX_RETRIES, e),
        }
    }
}

/// Retrieves the available feed ids from the Pragma Feeds Registry.
async fn get_feed_ids_from_registry(starknet: &Arc<Starknet>) -> anyhow::Result<Vec<Felt>> {
    let call = FunctionCall {
        contract_address: *PRAGMA_FEEDS_REGISTRY_ADDRESS,
        entry_point_selector: Selector::from("get_all_feeds").into(),
        calldata: vec![],
    };
    let feed_ids = starknet.call(call, PENDING_BLOCK)?;
    Ok(feed_ids)
}
