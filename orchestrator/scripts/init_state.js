/// The purpose of this script is to initialize the state of the madara orchestrator
/// This is what it will do
/// 1. Do a dummy transaction on Madara (SNOS currently can't run on empty blocks)
/// 2. Fetch the latest block B from Madara and ensure it can be run in SNOS
///    - Gas fees and Data gas prices are not 0 (eth and strk)
///    - The block has at least one transaction
/// 3. Add data in the mongo DB fit for block B-1
/// 4. Call updateStateOverride to set the core contract state to B-1

const starknet = require("starknet");
const ethers = require("ethers");
const { MongoClient } = require("mongodb");
const { v4 } = require("uuid");

// using default anvil key which has funds
const MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const eth_provider = new ethers.JsonRpcProvider("http://localhost:8545");
const wallet = new ethers.Wallet(
  MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY,
  eth_provider
);

const starknet_provider = new starknet.RpcProvider({
  nodeUrl: "http://localhost:9944",
});
// TODO: fetch these from bootstrapper output
const ETHEREUM_APP_CHAIN_ADDRESS =
  "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";
const OZ_ACCOUNT_CLASS_HASH =
  "0x01484c93b9d6cf61614d698ed069b3c6992c32549194fc3465258c2194734189";

// TODO: Fetch from env
const MONGO_URL = "mongodb://localhost:27017/orchestrator";

async function getAppChainBalance(address) {
  const abi = [
    {
      name: "balanceOf",
      type: "function",
      inputs: [
        {
          name: "account",
          type: "felt",
        },
      ],
      outputs: [
        {
          name: "balance",
          type: "Uint256",
        },
      ],
      stateMutability: "view",
    },
  ];
  const ethContract = new starknet.Contract(
    abi,
    ETHEREUM_APP_CHAIN_ADDRESS,
    starknet_provider
  );

  // Interaction with the contract with call
  const balance = await ethContract.balanceOf(address);
  return balance.balance;
}

async function bridgeToChain(bridge_address, starnet_expected_account_address) {
  // call deposit function with 10 as argument and also send 10 eth to the contract
  const contract = new ethers.Contract(
    bridge_address,
    ["function deposit(uint256, uint256)"],
    wallet
  );

  const initial_app_chain_balance = await getAppChainBalance(
    starnet_expected_account_address
  );
  const tx = await contract.deposit(
    ethers.parseEther("1"),
    starnet_expected_account_address,
    { value: ethers.parseEther("1.01") }
  );

  tx.wait();
  // wait for the transaction to be successful
  console.log("✅ Successfully sent 1 ETH on L1 bridge");

  let counter = 10;
  while (counter--) {
    const final_app_chain_balance = await getAppChainBalance(
      starnet_expected_account_address
    );
    if (final_app_chain_balance > initial_app_chain_balance) {
      console.log(
        "💰 App chain balance:",
        (final_app_chain_balance / 10n ** 18n).toString(),
        "ETH"
      );
      return;
    }
    console.log("🔄 Waiting for funds to arrive on app chain...");
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  console.log("❌ Failed to get funds on app chain");
  process.exit(1);
}

function calculatePrefactualAccountAddress() {
  // new Open Zeppelin account v0.8.1
  // Generate public and private key pair.
  const privateKey = starknet.stark.randomAddress();
  console.log("🔑 Starknet private key:", privateKey);
  const starkKeyPub = starknet.ec.starkCurve.getStarkKey(privateKey);
  console.log("🔑 Starknet public key:", starkKeyPub);

  // Calculate future address of the account
  const OZaccountConstructorCallData = starknet.CallData.compile({
    publicKey: starkKeyPub,
  });
  const OZcontractAddress = starknet.hash.calculateContractAddressFromHash(
    starkKeyPub,
    OZ_ACCOUNT_CLASS_HASH,
    OZaccountConstructorCallData,
    0
  );
  return {
    address: OZcontractAddress,
    private_key: privateKey,
    public_key: starkKeyPub,
  };
}

async function validateBlockPassesSnosChecks(block_number) {
  console.log("⏳ Checking if block", block_number, "can be run in SNOS...");
  const block = await starknet_provider.getBlock(block_number);

  // block number must be >= 10
  if (block_number < 10) {
    console.log("❌ Block number must be >= 10");
    process.exit(1);
  }
  console.log("✅ Block number is >= 10");

  // block must not be empty
  if (block.transactions.length === 0) {
    console.log("❌ Block has no transactions");
    process.exit(1);
  }
  console.log("✅ Block has transactions");

  // gas price shouldn't be 0
  if (
    block.l1_gas_price.price_in_fri == 0 ||
    block.l1_gas_price.price_in_wei == 0
  ) {
    console.log("❌ L1 gas price is 0", block.l1_gas_price);
    process.exit(1);
  }
  console.log("✅ L1 gas price is non zero");

  // data as price shouldn't be 0
  if (
    block.l1_data_gas_price.price_in_fri == 0 ||
    block.l1_data_gas_price.price_in_wei == 0
  ) {
    console.log("❌ L1 data gas price is 0", block.l1_data_gas_price);
    process.exit(1);
  }
  console.log("✅ L1 data gas price is non zero");
}

async function deployStarknetAccount(
  starknet_private_key,
  starnet_expected_account_address,
  starknet_account_public_key
) {
  console.log("⏳ Deploying Starknet account...");
  const account = new starknet.Account(
    starknet_provider,
    starnet_expected_account_address,
    starknet_private_key,
    "1"
  );
  const { transaction_hash, contract_address } = await account.deployAccount({
    classHash: OZ_ACCOUNT_CLASS_HASH,
    constructorCalldata: [starknet_account_public_key],
    addressSalt: starknet_account_public_key,
  });

  let receipt = await waitForTransactionSuccess(transaction_hash);
  // if txn is pending, block_number won't be available
  while (!receipt.block_number) {
    receipt = await starknet_provider.getTransactionReceipt(transaction_hash);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  return receipt.block_number;
}

async function waitForTransactionSuccess(hash) {
  let receipt = await starknet_provider.waitForTransaction(hash);
  if (!receipt.isSuccess()) {
    console.log("❌ Transaction failed - ", hash);
    process.exit(1);
  }
  return receipt;
}

// Due to restrictions in SNOS at the moment (as it's designed for Sepolia right now),
// we need to skip the starting few blocks from running on SNOS.
// This function overrides the state on the core contract to the block after which we
// can run SNOS
async function overrideStateOnCoreContract(
  block_number,
  core_contract_address
) {
  let state_update = await starknet_provider.getStateUpdate(block_number);
  let abi = [
    {
      type: "function",
      name: "updateStateOverride",
      inputs: [
        {
          name: "globalRoot",
          type: "uint256",
          internalType: "uint256",
        },
        {
          name: "blockNumber",
          type: "int256",
          internalType: "int256",
        },
        {
          name: "blockHash",
          type: "uint256",
          internalType: "uint256",
        },
      ],
      outputs: [],
      stateMutability: "nonpayable",
    },
  ];

  const contract = new ethers.Contract(core_contract_address, abi, wallet);
  const tx = await contract.updateStateOverride(
    state_update.new_root,
    block_number,
    state_update.block_hash
  );
  const receipt = await tx.wait();
  if (!receipt.status) {
    console.log("❌ Failed to override state on core contract");
    process.exit(1);
  }
  console.log("✅ Successfully overridden state on core contract");
}

async function setupMongoDb(block_number) {
  const client = new MongoClient(MONGO_URL);
  await client.connect();
  let db = client.db("orchestrator");
  const collection = db.collection("jobs");

  // delete everything in the collection
  await collection.deleteMany({});

  // insert all jobs
  let insert_promises = [
    "SnosRun",
    "ProofCreation",
    "DataSubmission",
    "StateTransition",
  ].map(async (job_type) => {
    console.log("Inserting job:", job_type);
    let metadata = {};
    if (job_type === "StateTransition") {
      metadata = {
        blocks_number_to_settle: String(block_number),
      };
    }
    await collection.insertOne({
      job_type,
      internal_id: String(block_number),
      external_id: "",
      status: "Completed",
      created_at: new Date(),
      updated_at: new Date(),
      id: v4(),
      metadata,
      version: 0,
    });
  });
  await Promise.all(insert_promises);
  await client.close();
  console.log("✅ Successfully inserted all jobs in MongoDB");
}

async function transfer(
  starknet_account_private_key,
  starnet_expected_account_address
) {
  const account = new starknet.Account(
    starknet_provider,
    starnet_expected_account_address,
    starknet_account_private_key,
    "1"
  );
  const abi = [
    {
      members: [
        {
          name: "low",
          offset: 0,
          type: "felt",
        },
        {
          name: "high",
          offset: 1,
          type: "felt",
        },
      ],
      name: "Uint256",
      size: 2,
      type: "struct",
    },
    {
      inputs: [
        {
          name: "recipient",
          type: "felt",
        },
        {
          name: "amount",
          type: "Uint256",
        },
      ],
      name: "transfer",
      outputs: [
        {
          name: "success",
          type: "felt",
        },
      ],
      type: "function",
    },
  ];
  const contract = new starknet.Contract(
    abi,
    ETHEREUM_APP_CHAIN_ADDRESS,
    starknet_provider
  );
  let calldata = contract.populate("transfer", {
    recipient: "0x1234",
    amount: {
      low: 1,
      high: 0,
    },
  });

  let txn_hash = await account.execute(calldata);
  let receipt = await starknet_provider.waitForTransaction(
    txn_hash.transaction_hash,
    {
      retryInterval: 100,
    }
  );
  if (!receipt.isSuccess()) {
    console.log("❌ Failed to do a transfer on Starknet account");
    process.exit(1);
  }

  // if txn is pending, block_number won't be available
  while (!receipt.block_number) {
    receipt = await starknet_provider.getTransactionReceipt(
      txn_hash.transaction_hash
    );
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  console.log("✅ Successfully did a transfer on Starknet account");
  return receipt.block_number;
}

// This function was created but studying Sepolia transactions
// and relaying them. I am not sure what's exactly happening here
// because the contracts we're working with arne't open source and are
// fetched from Sepolia directly
// TODO: we should move this to Madara bootstrapper
async function upgradeETHToken(
  l2_eth_token_address,
  starknet_account_private_key,
  starnet_expected_account_address
) {
  const account = new starknet.Account(
    starknet_provider,
    starnet_expected_account_address,
    starknet_account_private_key,
    "1"
  );

  // declare and deploy the new ERC20 contract
  // https://sepolia.starkscan.co/tx/0x04b5fa2a2e738a8b7a6c7b15194fbcf4409411743ebbe48cc5b83e5fe0edffdf
  console.log(
    "ℹ️ Sending transaction to declare and deploy new ERC20 contract for ETH..."
  );
  let new_erc20_declare_deploy = await account.declareAndDeploy({
    contract: require("./artifacts/starknet/new_eth_token.sierra.json"),
    casm: require("./artifacts/starknet/new_eth_token.casm.json"),
    constructorCalldata: [
      "eee",
      "eeee",
      "6",
      "0",
      "0",
      "0x137e2eb39d5b20f7257425dbea0a97ab6a53941e7ccdc9168ba3b0f8b39d1ce",
      "0x137e2eb39d5b20f7257425dbea0a97ab6a53941e7ccdc9168ba3b0f8b39d1ce",
      "0x137e2eb39d5b20f7257425dbea0a97ab6a53941e7ccdc9168ba3b0f8b39d1ce",
      "0",
    ],
  });
  console.log("✅ Transaction successful.");

  // declare and deploy the EIC (external initializer contract)
  // this is a method used to upgrade contracts in Starknet's proxy version'
  // however, I couldn't find the code of this contract
  // https://sepolia.starkscan.co/tx/0x03e50d969b41bc98e4da481ec7a48151bb0738137473f8f32f52fa317b9a9fe4
  console.log("ℹ️ Sending transaction to declare and deploy EIC contract...");
  let eic_declare_deploy = await account.declareAndDeploy({
    contract: require("./artifacts/starknet/eic_eth_token.sierra.json"),
    casm: require("./artifacts/starknet/eic_eth_token.casm.json"),
    constructorCalldata: [],
  });
  console.log("✅ Transaction successful.");

  // add_implementation to bridge contarct before we upgrade
  // https://sepolia.starkscan.co/tx/0x064ab87819a2f8ebf91176eeb901f842c23ef6c97c107fe31b14defa352ba045
  console.log(
    "ℹ️ Sending transaction to add implementation to bridge contract..."
  );
  let eth_bridge = new starknet.Contract(
    require("./artifacts/starknet/bridge_proxy_legacy.json").abi,
    l2_eth_token_address,
    account
  );
  let add_implementation_calldata = eth_bridge.populate("add_implementation", [
    new_erc20_declare_deploy.deploy.address,
    eic_declare_deploy.deploy.address,
    [], // init vector
    0, // final
  ]);
  let add_implementation_txn_hash = await eth_bridge.add_implementation(
    add_implementation_calldata.calldata
  );
  await waitForTransactionSuccess(add_implementation_txn_hash.transaction_hash);
  console.log("✅ Transaction successful.");

  // upgrade ETH token contract
  // https://sepolia.starkscan.co/tx/0x03115f88d0d2e97be5e752ff12c4e2f537ca0dcec92ad49b77d3d329efcd1c9f
  console.log("ℹ️ Sending transaction to upgrade ETH token contract...");
  let upgrade_txn_hash = await eth_bridge.upgrade_to(
    // the calldata is the same
    add_implementation_calldata.calldata
  );
  await waitForTransactionSuccess(upgrade_txn_hash.transaction_hash);
  console.log("✅ Transaction successful.");

  // now add a new implementation to the bridge contract for the erc20 class hash
  // https://sepolia.starkscan.co/tx/0x051cc24816ec349c601bbd4e9afc8e0a8c7a93061aba372045bbf7e5d35aff7a
  console.log(
    "ℹ️ Sending transaction to add new implementation to bridge contract..."
  );
  let add_new_implementation_txn_hash = await account.execute([
    {
      contractAddress: l2_eth_token_address,
      entrypoint: "add_new_implementation",
      calldata: [
        // class hash of new_eth_token
        new_erc20_declare_deploy.declare.class_hash,
        "0x1",
        "0x0",
      ],
    },
  ]);
  await waitForTransactionSuccess(
    add_new_implementation_txn_hash.transaction_hash
  );
  console.log("✅ Transaction successful.");

  // finally replace the class hash on the ETH contract
  console.log(
    "ℹ️ Sending transaction to replace class hash on the ETH contract..."
  );
  let replace_to_txn_hash = await account.execute([
    {
      contractAddress: l2_eth_token_address,
      entrypoint: "replace_to",
      calldata: [
        // class hash of new_eth_token
        new_erc20_declare_deploy.declare.class_hash,
        "0x1",
        "0x0",
      ],
    },
  ]);
  await waitForTransactionSuccess(replace_to_txn_hash.transaction_hash);
  console.log("✅ Transaction successful.");
}

// This function was created but studying Sepolia transactions
// and relaying them. I am not sure what's exactly happening here
// because the contracts we're working with arne't open source and are
// fetched from Sepolia directly
// TODO: we should move this to Madara bootstrapper
async function upgradeETHBridge(
  l2_eth_bridge_address,
  starknet_account_private_key,
  starnet_expected_account_address
) {
  const account = new starknet.Account(
    starknet_provider,
    starnet_expected_account_address,
    starknet_account_private_key,
    "1"
  );

  // declare and deploy the new ETH bridge contract
  // https://sepolia.starkscan.co/tx/0x05c266b9069c04f68752f5eb9652d7c0cd130c6d152d2267a8480273ec991de6
  console.log(
    "ℹ️ Sending transaction to declare and deploy new ETH bridge contract for ETH..."
  );
  let new_bridge_declare_deploy = await account.declareAndDeploy({
    contract: require("./artifacts/starknet/new_eth_bridge.sierra.json"),
    casm: require("./artifacts/starknet/new_eth_bridge.casm.json"),
    constructorCalldata: ["0"],
  });
  console.log("✅ Transaction successful.");

  // declare and deploy the EIC (external initializer contract)
  // this is a method used to upgrade contracts in Starknet's proxy version'
  // however, I couldn't find the code of this contract
  // https://sepolia.starkscan.co/tx/0x02fde4be42ecb05b545f53adf9d4c1aed8392e6a3743e9f5b6b8333fc580e684
  console.log("ℹ️ Sending transaction to declare and deploy EIC contract...");
  let eic_declare_deploy = await account.declareAndDeploy({
    contract: require("./artifacts/starknet/eic_eth_bridge.sierra.json"),
    casm: require("./artifacts/starknet/eic_eth_bridge.casm.json"),
    constructorCalldata: [],
  });
  console.log("✅ Transaction successful.");

  // add_implementation to bridge contarct before we upgrade
  // https://sepolia.starkscan.co/call/0x0721b02e1f4daa98ed8928966d66f345cb897f382274b22c89d86c00e755106d_1_1
  console.log(
    "ℹ️ Sending transaction to add implementation to bridge contract..."
  );
  let eth_bridge = new starknet.Contract(
    require("./artifacts/starknet/bridge_proxy_legacy.json").abi,
    l2_eth_bridge_address,
    account
  );
  let add_implementation_calldata = eth_bridge.populate("add_implementation", [
    new_bridge_declare_deploy.deploy.address,
    eic_declare_deploy.deploy.address,
    [
      "ETH",
      "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
    ], // init vector
    0, // final
  ]);
  let add_implementation_txn_hash = await eth_bridge.add_implementation(
    add_implementation_calldata.calldata
  );
  await waitForTransactionSuccess(add_implementation_txn_hash.transaction_hash);
  console.log("✅ Transaction successful.");

  // upgrade ETH token contract
  // https://sepolia.starkscan.co/tx/0x02660d0b82cd88e28a420adf8b5a5139b1f6084af708d10a75269b757ff6367c
  console.log("ℹ️ Sending transaction to upgrade ETH bridge contract...");
  let upgrade_txn_hash = await eth_bridge.upgrade_to(
    // the calldata is the same
    add_implementation_calldata.calldata
  );
  await waitForTransactionSuccess(upgrade_txn_hash.transaction_hash);
  console.log("✅ Transaction successful.");

  // now add a new implementation to the bridge contract for the bridge class hash
  // https://sepolia.starkscan.co/tx/0x051cc24816ec349c601bbd4e9afc8e0a8c7a93061aba372045bbf7e5d35aff7a
  console.log(
    "ℹ️ Sending transaction to add new implementation to bridge contract..."
  );
  let add_new_implementation_txn_hash = await account.execute([
    {
      contractAddress: l2_eth_bridge_address,
      entrypoint: "add_new_implementation",
      calldata: [
        // class hash of new_eth_bridge
        new_bridge_declare_deploy.declare.class_hash,
        "0x1",
        "0x0",
      ],
    },
  ]);
  await waitForTransactionSuccess(
    add_new_implementation_txn_hash.transaction_hash
  );
  console.log("✅ Transaction successful.");

  // finally replace the class hash on the ETH contract
  console.log(
    "ℹ️ Sending transaction to replace class hash on the ETH contract..."
  );
  let replace_to_txn_hash = await account.execute([
    {
      contractAddress: l2_eth_bridge_address,
      entrypoint: "replace_to",
      calldata: [new_bridge_declare_deploy.declare.class_hash, "0x1", "0x0"],
    },
  ]);
  await waitForTransactionSuccess(replace_to_txn_hash.transaction_hash);
  console.log("✅ Transaction successful.");
}

async function upgradeL1EthBridge(l1_bridge_address) {
  // deploy the new bridge which the proxy will use
  const newEthBridge = require("./artifacts/eth/new_eth_bridge.json");
  const contract = new ethers.ContractFactory(
    newEthBridge.abi,
    newEthBridge.bytecode,
    wallet
  );
  const ethBridgeReceipt = await contract.deploy();
  await ethBridgeReceipt.waitForDeployment();
  const ethBridgeAddress = await ethBridgeReceipt.getAddress();
  console.log("✅ New L1 ETH bridge deployed at address:", ethBridgeAddress);

  // deploy the EIC
  const newEic = require("./artifacts/eth/eic_eth_bridge.json");
  const eicContract = new ethers.ContractFactory(
    newEic.abi,
    newEic.bytecode,
    wallet
  );
  const eicReceipt = await eicContract.deploy();
  await eicReceipt.waitForDeployment();
  const eicAddress = await eicReceipt.getAddress();
  console.log("✅ New EIC deployed at address:", eicAddress);

  // add new implementation to the bridge
  const bridge = new ethers.Contract(
    l1_bridge_address,
    [
      {
        type: "function",
        name: "addImplementation",
        inputs: [
          {
            name: "newImplementation",
            type: "address",
            internalType: "address",
          },
          {
            name: "data",
            type: "bytes",
            internalType: "bytes",
          },
          {
            name: "finalize",
            type: "bool",
            internalType: "bool",
          },
        ],
        outputs: [],
        stateMutability: "nonpayable",
      },
      {
        type: "function",
        name: "upgradeTo",
        inputs: [
          {
            name: "newImplementation",
            type: "address",
            internalType: "address",
          },
          {
            name: "data",
            type: "bytes",
            internalType: "bytes",
          },
          {
            name: "finalize",
            type: "bool",
            internalType: "bool",
          },
        ],
        outputs: [],
        stateMutability: "payable",
      },
    ],
    wallet
  );

  // add new implementation to the bridge
  let addImplementationTxn = await bridge.addImplementation(
    ethBridgeAddress,
    chainHexesToBytes([eicAddress, "0x0", "0x0"]),
    false
  );
  await addImplementationTxn.wait();
  console.log("✅ New implementation added to the bridge");

  // upgrade the bridge to the new implementation
  let upgradeToTxn = await bridge.upgradeTo(
    ethBridgeAddress,
    chainHexesToBytes([eicAddress, "0x0", "0x0"]),
    false
  );
  await upgradeToTxn.wait();
  console.log("✅ Bridge upgraded to the new implementation");
}

/**
 * Chain arguments to one big endian Uint8Array.
 * Support address (or other hex strings that fit in 256 bits).
 * @param {string[]} hexes - Array of hex strings
 * @returns {Uint8Array} - Concatenated big endian bytes
 */
// Taken from https://github.com/starknet-io/starkgate-contracts/blob/5a10fd263d29cd032b7229691d043520edae0737/src/solidity/conftest.py#L167
function chainHexesToBytes(hexes) {
  // Create an array to store all bytes
  let result = new Uint8Array(hexes.length * 32);

  hexes.forEach((hex, index) => {
    // Remove '0x' prefix if present
    hex = hex.replace("0x", "");

    // Convert hex string to BigInt
    const num = BigInt("0x" + hex);

    // Convert to bytes and pad to 32 bytes
    const bytes = new Uint8Array(32);
    for (let i = 0; i < 32; i++) {
      // Extract each byte from right to left (big endian)
      bytes[31 - i] = Number((num >> BigInt(i * 8)) & BigInt(0xff));
    }

    // Add to result array
    result.set(bytes, index * 32);
  });

  return result;
}

async function main() {
  // tage bridge address as an argument --bridge_address
  const l1_bridge_address = process.argv[2];
  console.log("ℹ️ L1 bridge address:", l1_bridge_address);
  const core_contract_address = process.argv[3];
  console.log("ℹ️ Core contract address:", core_contract_address);
  const l2_eth_token_address = process.argv[4];
  console.log("ℹ️ L2 ETH token address:", l2_eth_token_address);
  const l2_eth_bridge_address = process.argv[5];
  console.log("ℹ️ L2 ETH bridge address:", l2_eth_bridge_address);
  const bootstrapper_address =
    "0x4fe5eea46caa0a1f344fafce82b39d66b552f00d3cd12e89073ef4b4ab37860" ||
    process.argv[6];
  const bootstrapper_private_key = "0xabcd" || process.argv[7];

  // add funds to boostrapper account
  let bootstrapper_address_balance = await getAppChainBalance(
    bootstrapper_address
  );
  if (bootstrapper_address_balance < 10n ** 17n) {
    await bridgeToChain(l1_bridge_address, bootstrapper_address);
  } else {
    console.log("ℹ️ Bootstrapping account has enough funds, proceeding");
  }

  // upgrade ETH token to Cairo 1 as SNOS breaks otherwise
  const eth_token_class = await starknet_provider.getClassAt(
    l2_eth_token_address
  );
  if (eth_token_class.sierra_program) {
    console.log("ℹ️ Eth token is already upgraded, proceeding");
  } else {
    await upgradeETHToken(
      l2_eth_token_address,
      bootstrapper_private_key,
      bootstrapper_address
    );
  }

  // upgrade ETH bridge to Cairo 1 as well
  const l2_eth_bridge_class = await starknet_provider.getClassAt(
    l2_eth_bridge_address
  );
  if (l2_eth_bridge_class.sierra_program) {
    console.log("ℹ️ Eth bridge is already upgraded, proceeding");
  } else {
    await upgradeETHBridge(
      l2_eth_bridge_address,
      bootstrapper_private_key,
      bootstrapper_address
    );
  }

  // upgrade L1 ETH bridge
  const l1BridgeContract = new ethers.Contract(
    l1_bridge_address,
    ["function identify() external view returns (string)"],
    eth_provider
  );
  const identify = await l1BridgeContract.identify();
  console.log("ℹ️ L1 ETH bridge identify:", identify);
  if (
    identify.includes(
      // StarkWare_StarknetEthBridge_2023_1
      "StarkWare_StarknetEthBridge_2023_1"
    )
  ) {
    await upgradeL1EthBridge(l1_bridge_address);
  } else {
    console.log("ℹ️ L1 ETH bridge is already upgraded, proceeding");
  }

  const {
    address: starnet_expected_account_address,
    private_key: starknet_account_private_key,
    public_key: starknet_account_public_key,
  } = calculatePrefactualAccountAddress();
  console.log(
    "🏦 Starknet expected account address:",
    starnet_expected_account_address
  );

  await bridgeToChain(l1_bridge_address, starnet_expected_account_address);

  let block_number = await deployStarknetAccount(
    starknet_account_private_key,
    starnet_expected_account_address,
    starknet_account_public_key
  );

  // SNOS doesn't seem to be able to run on deploy account block
  await starknet_provider.waitForBlock(block_number + 1);

  block_number = await transfer(
    starknet_account_private_key,
    starnet_expected_account_address
  );

  await validateBlockPassesSnosChecks(block_number);

  // overriding core contract state to block_number - 1 so that
  // we can run orchestrator on block_number
  await overrideStateOnCoreContract(block_number - 1, core_contract_address);

  await setupMongoDb(block_number - 1);
}

main();
