//! Real-node acceptance: signed account invokes -> durable Madara blocks -> RPC replay -> SNOS PIE.
//! The same historical transactions must trace on a replicating full node and remain replayable
//! after a chain revert. Resource reports keep receipt billing separate from proof reservations.
mod node;

use anyhow::{ensure, Context, Result};
use blockifier::execution::syscalls::committed_data::{self, CommittedDataBatchProof, CommittedDataSet};
use blockifier_test_utils::{
    cairo_versions::{CairoVersion, RunnableCairo1},
    contracts::FeatureContract,
};
use cairo_vm::types::layout_name::LayoutName;
use generate_pie::{generate_pie, ChainConfig, OsHintsConfiguration, PieGenerationInput};
use node::{rpc, Node};
use serde_json::{json, Value};
use starknet::{
    accounts::{Account, ExecutionEncoding, SingleOwnerAccount},
    contract::{ContractFactory, UdcSelector},
    core::{
        types::{contract::SierraClass, BlockId, BlockTag, Call},
        utils::get_selector_from_name,
    },
    providers::{jsonrpc::HttpTransport, JsonRpcClient},
    signers::{LocalWallet, SigningKey},
};
use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};
use std::{sync::Arc, time::Duration};

// Public, deterministic Madara devnet fixtures; never use funded external accounts here.
const ACCOUNT: &str = "0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d";
const SECRET: &str = "0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07";
const READ_ADDRESS: &str = "0x6c5f4559c7041984537bc078c71443fdc58b2e1bab302b341ec12b3d2cec44";
const SUCCESS_GAS_LIMIT: u64 = 1_000_000_000;
const OUT_OF_GAS_LIMIT: u64 = 1_000_000;

/// Persist raw receipts, traces and OS counters so a passing test is not mistaken for a fee
/// calibration. Complete PIE validation establishes execution consistency, not SHARP settlement.
fn save(name: &str, value: &Value) -> Result<()> {
    println!("COMMITTED_DATA_E2E {name}: {value}");
    if let Ok(directory) = std::env::var("COMMITTED_DATA_E2E_OUTPUT") {
        std::fs::create_dir_all(&directory)?;
        std::fs::write(
            std::path::Path::new(&directory).join(format!("{name}.json")),
            serde_json::to_vec_pretty(value)?,
        )?;
    }
    Ok(())
}

/// Storage diffs are maps serialized as arrays. Their iteration order can change across
/// processes/restarts; keep calls, events, return values and resource counters exact.
fn comparable_trace(mut trace: Value) -> Value {
    if let Some(changes) = trace["state_diff"]["storage_diffs"].as_array_mut() {
        for change in changes.iter_mut() {
            if let Some(entries) = change["storage_entries"].as_array_mut() {
                entries.sort_by_key(|entry| entry["key"].as_str().unwrap().to_owned());
            }
        }
        changes.sort_by_key(|change| change["address"].as_str().unwrap().to_owned());
    }
    trace
}

async fn import(node: &Node, tree: &CommittedDataSet) -> Result<()> {
    rpc(&node.private, "madara_importCommittedDataSet", json!([tree.root(), tree.entries(), []])).await?;
    Ok(())
}

/// Successful calls pay the deterministic witness-size schedule; a low gas limit exercises
/// the authentication-before-OOG path. Each case gets its own block and full SNOS execution.
#[tokio::test(flavor = "multi_thread")]
async fn committed_data_real_madara_snos_fullnode_and_revert() -> Result<()> {
    let mut source = Node::start(None).await?;
    save("madara_baseline", &source.footprint()?)?;
    let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(source.rpc.parse::<url::Url>()?)));
    let chain: Felt = serde_json::from_value(rpc(&source.rpc, "starknet_chainId", json!([])).await?)?;
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(Felt::from_hex(SECRET)?));
    let mut account =
        SingleOwnerAccount::new(provider, signer, Felt::from_hex(ACCOUNT)?, chain, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
    let entries = std::env::var("COMMITTED_DATA_E2E_ENTRIES")
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(100_000);
    ensure!((1024..=committed_data::MAX_COMMITTED_DATA_VALUES).contains(&entries), "invalid acceptance dataset size");
    let values: Vec<_> =
        (0..entries).map(|index| (Poseidon::hash(&Felt::from(index), &Felt::ONE), Felt::from(index + 10))).collect();
    let tree = CommittedDataSet::new(values.clone())?;
    import(&source, &tree).await?;
    // Zero and each power of two form the maximal 252-branch path for key zero.
    let mut deep_values = vec![(Felt::ZERO, Felt::ONE)];
    for bit in 0..252 {
        let mut bytes = [0u8; 32];
        bytes[31 - bit / 8] = 1 << (bit % 8);
        deep_values.push((Felt::from_bytes_be(&bytes), Felt::from(bit + 2)));
    }
    let deep = CommittedDataSet::new(deep_values)?;
    import(&source, &deep).await?;
    save("madara_after_import", &source.footprint()?)?;
    ensure!(deep.witness(Felt::ZERO).unwrap().siblings.len() == 252, "max-depth fixture drift");
    // Use the same caught-error contract as Blockifier's VM/native parity regression. The
    // fixture is compiled from the pinned Sequencer source, rather than copied stale bytecode.
    let fixture = FeatureContract::TestContract(CairoVersion::Cairo1(RunnableCairo1::Casm));
    let class: SierraClass = serde_json::from_str(&fixture.get_raw_sierra().context("Sierra fixture missing")?)?;
    let class = Arc::new(class.flatten()?);
    let compiled_hash =
        fixture.get_compiled_class_hash(&starknet_api::contract_class::compiled_class_hash::HashVersion::V2).0;
    let declared = account.declare_v3(class.clone(), compiled_hash).send().await?;
    ensure!(
        source.receipt(declared.transaction_hash).await?["execution_status"] == "SUCCEEDED",
        "fixture declaration reverted"
    );
    let factory = ContractFactory::new_with_udc(class.class_hash(), &account, UdcSelector::Legacy);
    let deployment = factory.deploy_v3(vec![Felt::ZERO, Felt::ZERO], Felt::from(1261), false);
    let catcher = deployment.deployed_address();
    let deployed = deployment.send().await?;
    ensure!(
        source.receipt(deployed.transaction_hash).await?["execution_status"] == "SUCCEEDED",
        "fixture deployment reverted"
    );
    let mut observations = Vec::new();
    let mut accepted = Vec::new();
    for (name, dataset, keys, batch, gas_limit) in [
        ("single", &tree, vec![values[0].0], false, SUCCESS_GAS_LIMIT),
        ("caught_out_of_gas", &deep, vec![Felt::ZERO], true, 4_000_000),
        ("batch_100", &tree, values.iter().take(100).map(|entry| entry.0).collect(), true, SUCCESS_GAS_LIMIT),
        ("batch_150", &tree, values.iter().take(150).map(|entry| entry.0).collect(), true, SUCCESS_GAS_LIMIT),
        ("batch_1024", &tree, values.iter().take(1024).map(|entry| entry.0).collect(), true, SUCCESS_GAS_LIMIT),
        ("duplicate_1024", &tree, vec![values[0].0; 1024], true, SUCCESS_GAS_LIMIT),
        ("depth_252", &deep, vec![Felt::ZERO], false, SUCCESS_GAS_LIMIT),
        ("depth_252_out_of_gas", &deep, vec![Felt::ZERO], false, OUT_OF_GAS_LIMIT),
    ] {
        let (mut selector, mut calldata, scheduled_gas) = if batch {
            let proof = CommittedDataBatchProof::from_dataset(dataset, &keys)?;
            let mut calldata = vec![dataset.root(), Felt::from(keys.len())];
            calldata.extend(&keys);
            ("get_values", calldata, committed_data::batch_gas(keys.len(), proof.unique_leaves(), proof.branches))
        } else {
            (
                "get_value",
                vec![dataset.root(), keys[0]],
                committed_data::read_gas(dataset.witness(keys[0]).unwrap().siblings.len()),
            )
        };
        let address = if name == "caught_out_of_gas" {
            selector = "test_committed_data_catch";
            // root, Span length, key, expect_error, three independent caught calls.
            calldata = vec![dataset.root(), Felt::ONE, Felt::ZERO, Felt::ONE, Felt::from(3)];
            catcher
        } else {
            Felt::from_hex(READ_ADDRESS)?
        };
        let submitted = account
            .execute_v3(vec![Call { to: address, selector: get_selector_from_name(selector)?, calldata }])
            .l1_gas(100_000)
            .l1_gas_price(1_000_000_000_000_000)
            .l1_data_gas(100_000)
            .l1_data_gas_price(1_000_000_000_000_000)
            .l2_gas(gas_limit)
            .l2_gas_price(1_000_000_000)
            .send()
            .await?;
        let receipt = source.receipt(submitted.transaction_hash).await?;
        let expected = if gas_limit == OUT_OF_GAS_LIMIT { "REVERTED" } else { "SUCCEEDED" };
        ensure!(receipt["execution_status"] == expected, "{name}: unexpected receipt {receipt}");
        let block = receipt["block_number"].as_u64().context("confirmed block missing")?;
        let trace = rpc(&source.rpc, "starknet_traceTransaction", json!([submitted.transaction_hash])).await?;
        let weights: Value = reqwest::get(format!("{}get_block_bouncer_weights?blockNumber={block}", source.feeder))
            .await?
            .error_for_status()?
            .json()
            .await?;
        // Preserve node-side evidence even if proving fails. A receipt alone does not
        // establish that the accepted transaction can be replayed by SNOS.
        save(&format!("{name}_node"), &json!({"receipt":receipt,"trace":trace,"bouncer_weights":weights}))?;
        let result = generate_pie(PieGenerationInput {
            rpc_url: source.rpc.clone(),
            committed_data_rpc_url: Some(source.feeder.clone()),
            blocks: vec![block],
            chain_config: ChainConfig::default_with_chain("MADARA_DEVNET"),
            os_hints_config: OsHintsConfiguration { use_committed_data: true, use_kzg_da: false, ..Default::default() },
            output_path: None,
            layout: LayoutName::all_cairo,
            versioned_constants: None,
            public_keys: None,
        })
        .await
        .with_context(|| format!("{name}: SNOS replay of Madara block {block}"))?;
        result.output.cairo_pie.run_validity_checks()?;
        let observation = json!({"case":name,"entries":dataset.entries().len(),"keys":keys.len(),
            "scheduled_special_call_gas_per_attempt":scheduled_gas,
            "special_call_attempts":if name == "caught_out_of_gas" { 3 } else { 1 },"receipt":receipt,"trace":trace,"bouncer_weights":weights,
            "os_transactions":result.output.txs_trace,"os_resources":result.output.cairo_pie.execution_resources});
        save(name, &observation)?;
        observations.push(observation);
        accepted.push((submitted.transaction_hash, block));
    }
    save("madara_after_execution", &source.footprint()?)?;
    let replica = Node::start(Some(&source)).await?;
    let target = accepted.last().unwrap().1;
    for _ in 0..180 {
        if rpc(&replica.rpc, "starknet_blockNumber", json!([]))
            .await
            .ok()
            .and_then(|tip| tip.as_u64())
            .is_some_and(|tip| tip >= target)
        {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    for (hash, _) in &accepted {
        let original = rpc(&source.rpc, "starknet_traceTransaction", json!([hash])).await?;
        let replicated = rpc(&replica.rpc, "starknet_traceTransaction", json!([hash])).await?;
        ensure!(comparable_trace(original) == comparable_trace(replicated), "replicated historical trace differs");
    }
    save("fullnode_after_trace", &replica.footprint()?)?;
    let legacy_reader = Node::start_with_data(Some(&source), false).await?;
    for _ in 0..180 {
        if rpc(&legacy_reader.rpc, "starknet_blockNumber", json!([]))
            .await
            .ok()
            .and_then(|tip| tip.as_u64())
            .is_some_and(|tip| tip >= target)
        {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let legacy_tip = rpc(&legacy_reader.rpc, "starknet_blockNumber", json!([])).await?;
    ensure!(legacy_tip.as_u64().is_some_and(|tip| tip >= target), "feature-disabled block sync stalled");
    let legacy_receipt = rpc(&legacy_reader.rpc, "starknet_getTransactionReceipt", json!([accepted[0].0])).await?;
    ensure!(legacy_receipt == observations[0]["receipt"], "feature-disabled reader receipt differs");
    let legacy_catalog: Value = reqwest::get(format!("{}get_committed_data_roots", legacy_reader.feeder))
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(legacy_catalog == json!([]), "feature-disabled reader unexpectedly replicated datasets");
    let retained = &observations[0]["receipt"];
    source.restart_after_revert(retained["block_hash"].clone()).await?;
    ensure!(
        rpc(&source.rpc, "starknet_blockNumber", json!([])).await? == retained["block_number"],
        "revert head mismatch"
    );
    ensure!(
        comparable_trace(rpc(&source.rpc, "starknet_traceTransaction", json!([accepted[0].0])).await?)
            == comparable_trace(observations[0]["trace"].clone()),
        "retained trace changed after revert"
    );
    ensure!(
        rpc(&source.rpc, "starknet_getTransactionReceipt", json!([accepted.last().unwrap().0])).await.is_err(),
        "reverted transaction remains canonical"
    );
    // Dataset availability is immutable and independent of canonical block ownership.
    let witness: Value =
        reqwest::get(format!("{}get_committed_data_witness?root={:#x}&key=0x0", source.feeder, deep.root()))
            .await?
            .error_for_status()?
            .json()
            .await?;
    ensure!(!witness.is_null(), "revert deleted immutable dataset");
    save(
        "summary",
        &json!({"scenarios":observations.len(),"fullnode_traces_equal":true,"feature_disabled_reader_syncs":true,"revert_and_restart":true,
        "dataset_survives_revert":true,"snos_pies_valid":true,"proof_or_settlement_submitted":false}),
    )?;
    Ok(())
}
