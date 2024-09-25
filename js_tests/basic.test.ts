import { RpcProvider, Account, Contract, CallData, cairo } from "starknet";
import { SIGNER_PRIVATE, SIGNER_PUBLIC } from "./constant";
import { readContractSierra, readContractSierraCasm, readContract } from "./utils";
import { hash } from "starknet";

/**
 * Test order configuration
 * Each test is defined with a name and corresponding function
 */
const testOrder = [
  { name: 'Declare Contract', fn: declareContract },
  { name: 'Deploy Contract', fn: deployContract },
  { name: 'Deploy Account', fn: deployAccount },
  { name: 'Fund Transfer', fn: transferFunds },
];

/**
 * Test context interface
 */
interface TestContext {
  provider: RpcProvider;
  account: Account;
  nonce: number;
}

describe('Starknet Contract Tests', () => {
  let provider: RpcProvider;
  let account: Account;
  let nonce: number;

  beforeAll(async () => {
    // Initialize provider and account
    provider = new RpcProvider({ nodeUrl: "http://localhost:9944" });
    account = new Account(provider, SIGNER_PUBLIC, SIGNER_PRIVATE);
    nonce = 0;
  });

  // Run tests in specified order
  testOrder.forEach(({ name, fn }) => {
    test(name, async () => {
      const result = await fn({ provider, account, nonce });
      nonce++;  // Increment nonce after each test

      // TODO: this is a workaround to wait for the block to be produced
      // given we have block time of 3 seconds, we wait for 5 seconds so that 
      // nonce gets updated. Remove this when we have a way to get the updated nonce
      // when there is a txn already in the pool from same account
      await new Promise(resolve => setTimeout(resolve, 5000));
    });
  });
});

/**
 * Declares a contract on the Starknet network
 * 
 * @param {TestContext} context - The test context
 * @returns {Promise<{classHash: string}>} The declared class hash
 */
async function declareContract({ provider, account, nonce }: TestContext) {
  const sierra = readContractSierra("madara_contracts_HelloStarknet");
  const casm = readContractSierraCasm("madara_contracts_HelloStarknet");
  
  const declareResponse = await account.declare({
    contract: sierra,
    casm: casm,
  }, {
    nonce,
  });

  await provider.waitForTransaction(declareResponse.transaction_hash);
  
  // Add assertion to check if the class hash is valid
  expect(declareResponse.class_hash).toBeTruthy();
}

/**
 * Deploys a contract to the Starknet network
 * 
 * @param {TestContext} context - The test context
 */
async function deployContract({ provider, account, nonce }: TestContext) {
  const deployResult = await account.deploy({
    classHash: hash.computeContractClassHash(readContractSierra("madara_contracts_HelloStarknet")),
  }, {
    nonce
  });

  // Add assertion to check if the contract address is valid
  expect(deployResult.contract_address[0]).toBeTruthy();
}

/**
 * Deploys an account contract to the Starknet network
 * 
 * @param {TestContext} context - The test context
 */
async function deployAccount({ provider, account, nonce }: TestContext) {
  const deployResult = await account.deploy({
    classHash: hash.computeContractClassHash(readContractSierra("madara_contracts_AccountUpgradeable")),
  }, {
    nonce,
  });

  // Add assertion to check if the account address is valid
  expect(deployResult.contract_address[0]).toBeTruthy();
}

/**
 * Transfers funds between accounts using an ERC20 token contract
 * 
 * @param {TestContext} context - The test context
 */
async function transferFunds({ provider, account, nonce }: TestContext) {
  const RECEIVER_ADDRESS = "0x5e9e93c6235f8ae6c2f4f0069bd30753ec21b26fbad80cfbf5da2c1bc573d69";
  const TRANSFER_AMOUNT = 100000000000n;
  const ERC20_CONTRACT_ADDRESS = "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";

  const erc20ContractData = readContract("ERC20");
  const erc20Instance = new Contract(
    erc20ContractData.abi,
    ERC20_CONTRACT_ADDRESS,
    provider,
  );

  erc20Instance.connect(account);

  const preTransactSenderBalance = (await erc20Instance.balanceOf(SIGNER_PUBLIC));
  const preTransactReceiverBalance = (await erc20Instance.balanceOf(RECEIVER_ADDRESS));


  const transferResponse = await account.execute({
    contractAddress: ERC20_CONTRACT_ADDRESS,
    entrypoint: 'transfer',
    calldata: CallData.compile({
      recipient: RECEIVER_ADDRESS,
      amount: cairo.uint256(TRANSFER_AMOUNT),
    }),
  }, {
    nonce,
    maxFee: 0,
  });

  await provider.waitForTransaction(
    transferResponse.transaction_hash,
  );
  const postTransactSenderBalance = (await erc20Instance.balanceOf(SIGNER_PUBLIC));
  const postTransactReceiverBalance = (await erc20Instance.balanceOf(RECEIVER_ADDRESS));


  expect(postTransactSenderBalance.balance.low).toBe(
    preTransactSenderBalance.balance.low -
      TRANSFER_AMOUNT
  );
  expect(postTransactReceiverBalance.balance.low).toBe(
    preTransactReceiverBalance.balance.low + TRANSFER_AMOUNT,
  );
}
