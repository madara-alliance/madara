use starknet_providers::sequencer::models::Block as StarknetBlock;

use crate::l2::get_highest_block_hash_and_number;

/// Check for a reorg on Starknet and fix the current state if detected.
///
/// On Starknet with the current system relying on a single sequencer it's rare to detect a reorg,
/// but if the L1 reorgs we must handle it the following way:
///
/// 1. The last fetched block parent hash is not equal to the last synced block by Deoxys: a reorg
///    is detected.
/// 2. We remove the last synced substrate digest and the associated classes/state_update we stored
///    until we reach the last common ancestor.
///
/// ### Arguments
///
/// * `block` - The last fetched block from the sequencer (before beeing converted).
///
/// ### Returns
/// This function will return a `Bool` returning `true` if a reorg was detected and `false` if not.
pub async fn reorg(block: StarknetBlock) -> bool {
    let last_synced_block_hash = get_highest_block_hash_and_number().0;
    if block.parent_block_hash != last_synced_block_hash {
        let mut new_lsbh = last_synced_block_hash;
        while block.parent_block_hash != new_lsbh {
            // 1. Remove the last synced block in the digest
            // 2. Remove all the downloaded stuff from the state updates
            new_lsbh = get_highest_block_hash_and_number().0;
        }
        // 3. Revert the state commitment tries to the correct block number
        true
    } else {
        false
    }
}
