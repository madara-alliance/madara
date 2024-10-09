//! ExEx of Pragma Dispatcher
//! Adds a new TX at the end of each block, dispatching a message through
//! Hyperlane.
use std::{sync::Arc, time::Duration};

use anyhow::bail;
use futures::StreamExt;
use mp_block::MadaraPendingBlock;
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

lazy_static::lazy_static! {
    // TODO: Keystore path?
    pub static ref ACCOUNT_ADDRESS: Felt = felt!("0x9ea0674c4d7b87b4afcb4c4ddc783b0c07e758778b9a1d133adc97cddfe38f");
    pub static ref PRIVATE_KEY: SigningKey = SigningKey::from_secret_scalar(felt!("0x7a9779748888c95d96bbbce041b5109c6ffc0c4f30561c0170384a5922d9e91"));

    pub static ref PRAGMA_FEEDS_REGISTRY_ADDRESS: Felt = felt!("0x13c3404ff9802442d0bf389afcf2fab9201b47c2268fcaa4bd36ba1978af76");
    pub static ref PRAGMA_DISPATCHER_ADDRESS: Felt = felt!("0x38d9b85bf3623681aaa37b1c591b07237dee8b17a11eaac53ddc07a306fefe2");

    pub static ref MAX_FEE: Felt = felt!("2386F26FC10000"); // 0.01 eth

    // NewFeedId event selector
    pub static ref NEW_FEED_ID_SELECTOR: Felt = felt!("0x012eaeb62184f1ca53999ece2d2273b81f9c64bc057a93dad05e09f970b030f9");
    // RemovedFeedId event selector
    pub static ref REMOVED_FEED_ID_SELECTOR: Felt = felt!("0x02a45c5a3b53e7afa46712156f544cec1b9d4679804036a16ec9521389117be4");

    // Empty feed list. Used instead of [`Vec::is_empty`].
    // The first element is the length of the vec & after are the elements.
    pub static ref EMPTY_FEEDS: Vec<Felt> = vec![Felt::ZERO];
}

/// 🧩 Pragma main ExEx.
/// At the end of each produced block by the node, adds a new dispatch transaction
/// using the Pragma Dispatcher contract.
pub async fn exex_pragma_dispatch(mut ctx: ExExContext) -> anyhow::Result<()> {
    // Feed ids that will be dispatched.
    // The first element is the length of the vec & after are the elements.
    let mut feed_ids: Vec<Felt> = get_feed_ids_from_registry(&ctx.starknet).await.unwrap_or(vec![Felt::ZERO]);
    log::info!("🧩 Pragma's ExEx: Initialized feed IDs from Registry. Total feeds: {}", feed_ids[0]);

    while let Some(notification) = ctx.notifications.next().await {
        let (block, block_number) = match notification {
            ExExNotification::BlockProduced { block, block_number } => (block, block_number),
            ExExNotification::BlockSynced { block_number } => {
                ctx.events.send(ExExEvent::FinishedHeight(block_number))?;
                continue;
            }
        };

        // Will update in-place the feed ids vec
        if let Err(e) = update_feed_ids_if_necessary(&ctx.starknet, &block, block_number.0, &mut feed_ids).await {
            log::error!("🧩 [#{}] Pragma's ExEx: Error while updating feed IDs: {:?}", block_number, e);
            ctx.events.send(ExExEvent::FinishedHeight(block_number))?;
            continue;
        }

        if feed_ids == *EMPTY_FEEDS {
            log::warn!("🧩 [#{}] Pragma's ExEx: No feed IDs available, skipping dispatch", block_number);
            ctx.events.send(ExExEvent::FinishedHeight(block_number))?;
            continue;
        }

        if let Err(e) = process_dispatch_transaction(&ctx, block_number.0, &feed_ids).await {
            log::error!("🧩 [#{}] Pragma's ExEx: Error while processing dispatch transaction: {:?}", block_number, e);
        }

        ctx.events.send(ExExEvent::FinishedHeight(block_number))?;
    }
    Ok(())
}

/// Update the feed ids list if necessary.
/// It means:
///   * if the feed id list is empty,
///   * if we find the event [NewFeedId] or [RemovedFeedId] in the block's events.
async fn update_feed_ids_if_necessary(
    starknet: &Arc<Starknet>,
    block: &MadaraPendingBlock,
    block_number: u64,
    feed_ids: &mut Vec<Felt>,
) -> anyhow::Result<()> {
    // If the list is empty, it may be because the contract wasn't deployed before.
    // Requery.
    if *feed_ids == *EMPTY_FEEDS {
        *feed_ids = get_feed_ids_from_registry(starknet).await?;
        log::info!("🧩 [#{}] Pragma's ExEx: Refreshed all feeds. Total feeds: {}", block_number, feed_ids[0]);
        return Ok(());
    }

    for receipt in &block.inner.receipts {
        if let mp_receipt::TransactionReceipt::Invoke(invoke_receipt) = receipt {
            for event in &invoke_receipt.events {
                if event.from_address != *PRAGMA_FEEDS_REGISTRY_ADDRESS {
                    continue;
                }
                if event.keys.is_empty() || event.data.len() != 2 {
                    continue;
                }
                let selector = event.keys[0];
                let feed_id = event.data[1];
                if selector == *NEW_FEED_ID_SELECTOR {
                    if !feed_ids.contains(&feed_id) {
                        feed_ids.push(feed_id);
                        feed_ids[0] += Felt::ONE;
                        log::info!(
                            "🧩 [#{}] Pragma's ExEx: Added new feed ID \"0x{:x}\". Total feeds: {}",
                            block_number,
                            feed_id,
                            feed_ids[0]
                        );
                    }
                } else if selector == *REMOVED_FEED_ID_SELECTOR {
                    if let Some(pos) = feed_ids.iter().position(|x| *x == feed_id) {
                        feed_ids.remove(pos);
                        feed_ids[0] -= Felt::ONE;
                        log::info!(
                            "🧩 [#{}] Pragma's ExEx: Removed feed ID \"0x{:x}\". Total feeds: {}",
                            block_number,
                            feed_id,
                            feed_ids[0]
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

/// Create a Dispatch tx and sends it.
/// Logs info about the tx status.
async fn process_dispatch_transaction(ctx: &ExExContext, block_number: u64, feed_ids: &[Felt]) -> anyhow::Result<()> {
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

/// Creates & Invoke the Dispatch TX.
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

/// Check the status of a transaction & log info about it.
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
