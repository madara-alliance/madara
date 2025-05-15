import { RpcProvider, Account, Contract, CallData, cairo } from "starknet";
import { RPC_URL, SIGNER_PRIVATE, SIGNER_CONTRACT_ADDRESS } from "./constant";
import {
  readContractSierra,
  readContractCasm,
  readContractSierraInArtifacts,
} from "./utils";
import { hash, stark, ec } from "starknet";

/**
 * Test order configuration, ideally the tests should be able to
 * run in parallel but given in the current case if we have 2 txns
 * from the same account, it throws an error of having same txn hash
 * because the nonce is not getting updated for the first txn because
 * the block was not produced yet.
 *
 * TODO: Remove this when we have a way to get the updated nonce
 */
const testOrder = [
  { name: "Declare Contract", fn: declareContract },
  { name: "Deploy Contract", fn: deployContract },
  { name: "Transfer Funds", fn: transferFunds },
  { name: "Deploy Account", fn: deployAccount },
];

/**
 * Test context interface
 */
interface TestContext {
  provider: RpcProvider;
  account: Account;
}

describe("Starknet Contract Tests", () => {
  let provider: RpcProvider;
  let account: Account;
  let nonce: number;

  beforeAll(async () => {
    // Initialize provider and account
    provider = new RpcProvider({ nodeUrl: RPC_URL });
    account = new Account(provider, SIGNER_CONTRACT_ADDRESS, SIGNER_PRIVATE);
  });

  // Run tests in specified order
  testOrder.forEach(({ name, fn }) => {
    test(name, async () => {
      const result = await fn({ provider, account });

      // TODO: this is a workaround to wait for the block to be produced
      // given we have block time of 3 seconds, we wait for 5 seconds so that
      // nonce gets updated. Remove this when we have a way to get the updated nonce
      // when there is a txn already in the pool from same account
      await new Promise((resolve) => setTimeout(resolve, 5000));
    });
  });
});

/**
 * Declares a contract on the Starknet network
 *
 * @param {TestContext} context - The test context
 * @returns {Promise<{classHash: string}>} The declared class hash
 */
async function declareContract({ provider, account }: TestContext) {
  // Read the Sierra and CASM representations of the HelloStarknet contract
  const sierra = readContractSierra("madara_contracts_HelloStarknet");
  const casm = readContractCasm("madara_contracts_HelloStarknet");

  // Declare the contract on the network
  const declareResponse = await account.declare({
    contract: sierra,
    casm: casm,
  });

  // Wait for the declaration transaction to be confirmed
  await provider.waitForTransaction(declareResponse.transaction_hash);

  // Add assertion to check if the class hash is valid
  expect(declareResponse.class_hash).toBeTruthy();

  // Retrieve the declared class from the network
  let response = await provider.getClass(declareResponse.class_hash);

  // Verify the retrieved class matches the declared contract
  if ("sierra_program" in response) {
    expect(response.sierra_program).toEqual(sierra.sierra_program);
    expect(response.abi).toEqual(sierra.abi);
    expect(response.contract_class_version).toEqual(
      sierra.contract_class_version,
    );
    expect(response.entry_points_by_type).toEqual(sierra.entry_points_by_type);
  } else {
    throw new Error("UnExpected CompiledSierra");
  }
}

/**
 * Deploys a contract to the Starknet network
 *
 * @param {TestContext} context - The test context
 */
async function deployContract({ provider, account }: TestContext) {
  // Read the Sierra representation of the HelloStarknet contract
  const sierra = readContractSierra("madara_contracts_HelloStarknet");
  // Compute the class hash of the contract
  let classHash = hash.computeContractClassHash(sierra);

  // Deploy the contract
  const deployResult = await account.deploy({
    classHash: classHash,
  });

  // Wait for the deployment transaction to be confirmed
  await provider.waitForTransaction(deployResult.transaction_hash);

  // Add assertion to check if the contract address is valid
  expect(deployResult.contract_address[0]).toBeTruthy();

  // Retrieve the class hash for the deployed contract
  let response = await provider.getClassHashAt(
    deployResult.contract_address[0],
  );

  // Verify that the retrieved class hash matches the computed class hash
  expect(response).toEqual(classHash);
}

/**
 * Deploys an account contract to the Starknet network
 *
 * @param {TestContext} context - The test context
 */
/**
 * Deploys an account contract to the Starknet network
 *
 * @param {TestContext} context - The test context containing provider, account, and nonce
 */
async function deployAccount({ provider, account }: TestContext) {
  // Read the Sierra contract class for the account
  const sierra = readContractSierraInArtifacts(
    "openzeppelin_AccountUpgradeable",
  );

  // Compute the class hash of the account contract
  let classHash = hash.computeContractClassHash(sierra);

  // Generate a new random private key for the account
  const privateKey = stark.randomAddress();

  // Derive the public key from the private key
  const publicKey = ec.starkCurve.getStarkKey(privateKey);

  // Prepare the constructor calldata with the public key
  const calldata = { publicKey: publicKey };

  // Calculate the future address of the account contract
  const accountAddress = hash.calculateContractAddressFromHash(
    publicKey,
    classHash,
    calldata,
    0,
  );

  // Create a new Account instance with the calculated address and private key
  const newAccount = new Account(provider, accountAddress, privateKey);

  // Deploy the account contract
  const { transaction_hash, contract_address } = await newAccount.deployAccount(
    {
      classHash: classHash,
      constructorCalldata: calldata,
      addressSalt: publicKey,
    },
    {
      maxFee: 0,
    },
  );

  // Wait for the transaction to be confirmed and get the receipt
  let transactionReceipt = await provider.waitForTransaction(transaction_hash);

  // Retrieve the class hash for the deployed account contract
  let response = await provider.getClassHashAt(contract_address);

  // Verify that the deployed contract's class hash matches the expected class hash
  expect(response).toEqual(classHash);
}

/**
 * Transfers funds between accounts using an ERC20 token contract
 *
 * @param {TestContext} context - The test context
 */
async function transferFunds({ provider, account }: TestContext) {
  // Constants for the transfer operation
  // Note: These addresses are specific to the devnet environment
  const RECEIVER_ADDRESS =
    "0x5e9e93c6235f8ae6c2f4f0069bd30753ec21b26fbad80cfbf5da2c1bc573d69";
  const TRANSFER_AMOUNT = 100000000000n;
  const ERC20_CONTRACT_ADDRESS =
    "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";

  // Read the ERC20 contract class
  const erc20ContractData = readContractSierraInArtifacts(
    "openzeppelin_ERC20Upgradeable",
  );

  // Create an instance of the ERC20 contract
  const erc20Instance = new Contract(
    erc20ContractData.abi,
    ERC20_CONTRACT_ADDRESS,
    provider,
  );

  // Connect the account to the ERC20 contract instance
  erc20Instance.connect(account);

  // Get the initial balances of sender and receiver
  const preTransactSenderBalance = await erc20Instance.balance_of(
    SIGNER_CONTRACT_ADDRESS,
  );
  const preTransactReceiverBalance =
    await erc20Instance.balance_of(RECEIVER_ADDRESS);

  // Execute the transfer
  // Note: We are setting maxFee to zero here
  const transferResponse = await account.execute(
    {
      contractAddress: ERC20_CONTRACT_ADDRESS,
      entrypoint: "transfer",
      calldata: CallData.compile({
        recipient: RECEIVER_ADDRESS,
        amount: cairo.uint256(TRANSFER_AMOUNT),
      }),
    },
    {
      maxFee: 0,
    },
  );

  // Wait for the transfer transaction to be confirmed
  await provider.waitForTransaction(transferResponse.transaction_hash);

  // Get the final balances of sender and receiver
  const postTransactSenderBalance = await erc20Instance.balance_of(
    SIGNER_CONTRACT_ADDRESS,
  );
  const postTransactReceiverBalance =
    await erc20Instance.balance_of(RECEIVER_ADDRESS);

  // Verify that the balances have been updated correctly
  // Note: In real world case, the sender balance would be
  // preTransactionSenderBalance - TRANSFER_AMOUNT - Fees
  // but we had fees set to zero while executing transaction
  expect(postTransactSenderBalance).toBe(
    preTransactSenderBalance - TRANSFER_AMOUNT,
  );
  expect(postTransactReceiverBalance).toBe(
    preTransactReceiverBalance + TRANSFER_AMOUNT,
  );
}
