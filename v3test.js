const starknet = require("starknet");
const casm1 = require("any.compiled_contract_class.json");
const sierra1 = require("any.contract_class.json");
const provider = new starknet.RpcProvider({
  nodeUrl: "http://localhost:9944",
});

const accountLocal = new starknet.Account(
  provider,
  "0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d",
  "0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07",
);

const accountLocalV3 = new starknet.Account(
  provider,
  "0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d",
  "0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07",
  undefined,
  starknet.constants.TRANSACTION_VERSION.V3,
);

async function declare() {
  console.log("sierra1 - ");
  try {
    console.log("sierra1 - ");
    const declareResponse = await accountLocalV3.declare({
      contract: sierra1,
      casm: casm1,
    });
    console.log(
      "Test Contract declared with classHash =",
      declareResponse.class_hash,
    );
    await provider.waitForTransaction(declareResponse.transaction_hash);
    console.log(":white_check_mark: Test Completed.");
  } catch (err) {
    console.log("Contract is already declared", err);
  }
}

async function deploy() {
  try {
    const deployResult = await accountLocalV3.deploy({
      classHash: starknet.hash.computeContractClassHash(sierra1),
    });

    console.log("This is the deploy result - ", deployResult);
    return deployResult.contract_address[0];
  } catch (err) {
    console.log("Error in deployment", err);
  }
}

async function getTransactionReceipt(txnHash) {
  const result = await provider.getTransactionReceipt(txnHash);

  console.log("This is the transaction receipt - ", result);
  console.log(result.events.keys);
  console.log(result.events.data);
}
async function main() {
  let contract = await declare();
  console.log(contract);
  let contractAddress = await deploy();
  console.log(contractAddress);
}

main();
