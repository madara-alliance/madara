import {
  Account,
  RpcProvider,
  hash,
  ec,
  CallData,
  cairo,
  Deployer,
  num,
} from "starknet";
import { AdminClient } from "./admin-client";
import { BlockTracker } from "./block-tracker";
import { loadContractSierra, loadContractCasm } from "./contract-loader";
import { resolveValue } from "./ref-resolver";
import {
  DEFAULT_ACCOUNT_ADDRESS,
  DEFAULT_PRIVATE_KEY,
  UDC_ADDRESS,
  ERC20_STRK_ADDRESS,
} from "./config";
import {
  StateSetup,
  SetupStep,
  StepResult,
  TestContext,
  DeclareStep,
  DeployStep,
  InvokeStep,
  DeployAccountStep,
  L1HandlerStep,
} from "./types";

export async function executeStateSetup(
  setup: StateSetup,
  ctx: TestContext,
): Promise<void> {
  const provider = new RpcProvider({ nodeUrl: ctx.rpcUrl });
  const deployer = new Deployer(UDC_ADDRESS, "deployContract");
  const account = new Account({
    provider,
    address: DEFAULT_ACCOUNT_ADDRESS,
    signer: DEFAULT_PRIVATE_KEY,
    deployer,
  });
  const admin = new AdminClient(ctx.adminUrl);
  const blockTracker = new BlockTracker(provider, admin);

  for (const step of setup.steps) {
    console.log(`[setup] Executing step: ${step.id} (${step.type})`);
    try {
      await executeStep(step, ctx, provider, account, admin, blockTracker);
      console.log(`[setup] Step ${step.id} completed`);
    } catch (err) {
      console.error(`[setup] Step ${step.id} FAILED:`, err);
      throw new Error(`State setup failed at step "${step.id}": ${err}`);
    }
  }
}

async function getNextNonce(
  provider: RpcProvider,
  address: string,
  ctx: TestContext,
): Promise<string> {
  // Use the local nonce tracker if available (for multi-tx in same block)
  const tracked = ctx.nonceTracker.get(address);
  if (tracked !== undefined) {
    return num.toHex(tracked);
  }
  // Otherwise fetch from node using pre_confirmed block tag
  const nonce = await provider.getNonceForAddress(address, "pre_confirmed");
  return nonce;
}

function incrementNonce(
  ctx: TestContext,
  address: string,
  currentNonce: string,
): void {
  const n = BigInt(currentNonce);
  ctx.nonceTracker.set(address, n + 1n);
}

function resetNonce(ctx: TestContext, address: string): void {
  ctx.nonceTracker.delete(address);
}

async function executeStep(
  step: SetupStep,
  ctx: TestContext,
  provider: RpcProvider,
  account: Account,
  admin: AdminClient,
  blockTracker: BlockTracker,
): Promise<void> {
  let result: StepResult;

  switch (step.type) {
    case "declare":
      result = await executeDeclare(step, ctx, provider, account);
      break;
    case "deploy":
      result = await executeDeploy(step, ctx, provider, account);
      break;
    case "invoke":
      result = await executeInvoke(step, ctx, provider, account);
      break;
    case "deploy_account":
      result = await executeDeployAccount(step, ctx, provider, account);
      break;
    case "l1_handler":
      result = await executeL1Handler(step, ctx, admin);
      break;
    case "close_block": {
      const blockInfo = await blockTracker.closeBlockAndTrack();
      ctx.results.set(step.id, {
        block_number: blockInfo.block_number,
        block_hash: blockInfo.block_hash,
      });
      // Reset nonce trackers on block close (on-chain nonces are now canonical)
      ctx.nonceTracker.clear();
      return;
    }
    default:
      throw new Error(`Unknown step type: ${(step as any).type}`);
  }

  ctx.results.set(step.id, result);

  if ("close_block" in step && step.close_block) {
    const blockInfo = await blockTracker.closeBlockAndTrack();
    result.block_number = blockInfo.block_number;
    result.block_hash = blockInfo.block_hash;
    ctx.results.set(step.id, result);
    // Reset nonce trackers on block close
    ctx.nonceTracker.clear();
  }
}

async function executeDeclare(
  step: DeclareStep,
  ctx: TestContext,
  provider: RpcProvider,
  account: Account,
): Promise<StepResult> {
  const sierra = loadContractSierra(step.contract);
  const casm = loadContractCasm(step.contract);

  const nonce = await getNextNonce(provider, DEFAULT_ACCOUNT_ADDRESS, ctx);

  const response = await account.declare(
    {
      contract: sierra,
      casm,
    },
    { nonce },
  );

  incrementNonce(ctx, DEFAULT_ACCOUNT_ADDRESS, nonce);
  await provider.waitForTransaction(response.transaction_hash);

  return {
    transaction_hash: response.transaction_hash,
    class_hash: response.class_hash,
  };
}

async function executeDeploy(
  step: DeployStep,
  ctx: TestContext,
  provider: RpcProvider,
  account: Account,
): Promise<StepResult> {
  const classHash = resolveValue(step.class_hash, ctx.results) as string;
  const constructorCalldata = resolveValue(
    step.constructor_calldata,
    ctx.results,
  ) as string[];

  const nonce = await getNextNonce(provider, DEFAULT_ACCOUNT_ADDRESS, ctx);

  const response = await account.deploy(
    {
      classHash,
      constructorCalldata,
      salt: step.salt,
      unique: step.unique,
    },
    { nonce },
  );

  incrementNonce(ctx, DEFAULT_ACCOUNT_ADDRESS, nonce);
  await provider.waitForTransaction(response.transaction_hash);

  return {
    transaction_hash: response.transaction_hash,
    contract_address: response.contract_address[0],
  };
}

async function executeInvoke(
  step: InvokeStep,
  ctx: TestContext,
  provider: RpcProvider,
  account: Account,
): Promise<StepResult> {
  const contractAddress = resolveValue(
    step.contract_address,
    ctx.results,
  ) as string;
  const calldata = resolveValue(step.calldata, ctx.results) as string[];

  const nonce = await getNextNonce(provider, DEFAULT_ACCOUNT_ADDRESS, ctx);

  // If nonce tracker is active (multi-tx in same block), we need explicit
  // resource bounds because fee estimation uses confirmed state where
  // the pending tx's nonce hasn't advanced yet.
  const hasPendingTx = ctx.nonceTracker.has(DEFAULT_ACCOUNT_ADDRESS);
  const details: any = { nonce };
  if (hasPendingTx) {
    details.resourceBounds = {
      l2_gas: { max_amount: 0x1000000n, max_price_per_unit: 0x100000n },
      l1_gas: { max_amount: 0x1000n, max_price_per_unit: 0x100000000n },
      l1_data_gas: { max_amount: 0x1000n, max_price_per_unit: 0x100000000n },
    };
  }

  const response = await account.execute(
    {
      contractAddress,
      entrypoint: step.entry_point,
      calldata,
    },
    details,
  );

  incrementNonce(ctx, DEFAULT_ACCOUNT_ADDRESS, nonce);
  await provider.waitForTransaction(response.transaction_hash);

  return {
    transaction_hash: response.transaction_hash,
  };
}

async function executeDeployAccount(
  step: DeployAccountStep,
  ctx: TestContext,
  provider: RpcProvider,
  defaultAccount: Account,
): Promise<StepResult> {
  const classHash = resolveValue(step.class_hash, ctx.results) as string;
  const privateKey = step.private_key;
  const publicKey = ec.starkCurve.getStarkKey(privateKey);

  const selfContext = { public_key: publicKey };
  const constructorCalldata = resolveValue(
    step.constructor_calldata,
    ctx.results,
    selfContext,
  ) as string[];

  const contractAddress = hash.calculateContractAddressFromHash(
    publicKey,
    classHash,
    constructorCalldata,
    0,
  );

  // Fund the new account address before deploying
  if (step.fund_amount && step.fund_amount !== "0x0") {
    const fundNonce = await getNextNonce(
      provider,
      DEFAULT_ACCOUNT_ADDRESS,
      ctx,
    );
    const fundResponse = await defaultAccount.execute(
      {
        contractAddress: ERC20_STRK_ADDRESS,
        entrypoint: "transfer",
        calldata: CallData.compile({
          recipient: contractAddress,
          amount: cairo.uint256(step.fund_amount),
        }),
      },
      { nonce: fundNonce },
    );
    incrementNonce(ctx, DEFAULT_ACCOUNT_ADDRESS, fundNonce);
    await provider.waitForTransaction(fundResponse.transaction_hash);
  }

  // Create account instance and deploy
  const deployer = new Deployer(UDC_ADDRESS, "deployContract");
  const newAccount = new Account({
    provider,
    address: contractAddress,
    signer: privateKey,
    deployer,
  });

  const response = await newAccount.deployAccount({
    classHash,
    constructorCalldata,
    addressSalt: publicKey,
  });

  await provider.waitForTransaction(response.transaction_hash);

  return {
    transaction_hash: response.transaction_hash,
    contract_address: response.contract_address,
  };
}

async function executeL1Handler(
  step: L1HandlerStep,
  ctx: TestContext,
  admin: AdminClient,
): Promise<StepResult> {
  const contractAddress = resolveValue(
    step.contract_address,
    ctx.results,
  ) as string;
  const calldata = resolveValue(step.calldata, ctx.results) as string[];
  const entryPointSelector = hash.getSelectorFromName(step.entry_point);

  const response = await admin.addL1HandlerMessage({
    contract_address: contractAddress,
    entry_point_selector: entryPointSelector,
    calldata,
    nonce: step.nonce,
    paid_fee_on_l1: step.paid_fee_on_l1,
  });

  return {
    transaction_hash: response.transaction_hash,
  };
}
