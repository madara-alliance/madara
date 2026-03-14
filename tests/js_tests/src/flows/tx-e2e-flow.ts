import {
  Account,
  CallData,
  Contract,
  RpcProvider,
  cairo,
  ec,
  hash,
  stark,
} from "starknet";
import {
  ERC20_CONTRACT_ADDRESS,
  SIGNER_CONTRACT_ADDRESS,
} from "../fixtures/constants";
import { waitFor } from "../core/poller";
import {
  readContractCasm,
  readContractSierra,
  readContractSierraInArtifacts,
} from "../fixtures/artifacts";

export interface E2ETxRef {
  txHash: string;
}

export interface DeclaredContractRef extends E2ETxRef {
  classHash: string;
}

export interface DeployedContractRef extends E2ETxRef {
  contractAddress: string;
}

export interface DeployedAccountRef extends E2ETxRef {
  classHash: string;
  contractAddress: string;
  privateKey: string;
}

export class TxE2EFlow {
  private readonly pendingLikeTag: "pending" | "pre_confirmed";

  constructor(
    public readonly provider: RpcProvider,
    public readonly account: Account,
    options?: { supportsPreConfirmedTag?: boolean },
  ) {
    this.pendingLikeTag = options?.supportsPreConfirmedTag
      ? "pre_confirmed"
      : "pending";
  }

  async waitForTransaction(txHash: string): Promise<any> {
    const receipt = await waitFor(
      async () => {
        try {
          return await this.provider.getTransactionReceipt(txHash);
        } catch (error) {
          if (isTxNotFoundError(error)) {
            return null;
          }
          throw error;
        }
      },
      (value) => {
        if (!value) {
          return false;
        }

        const finality = getReceiptFinalityStatus(value);
        const execution = getReceiptExecutionStatus(value);

        if (finality === "REJECTED" || execution === "REVERTED") {
          return true;
        }

        return (
          finality === "ACCEPTED_ON_L2" ||
          finality === "ACCEPTED_ON_L1" ||
          finality === "ACCEPTED_ONCHAIN"
        );
      },
      { timeoutMs: 180_000, intervalMs: 1000 },
    );

    if (!receipt) {
      throw new Error(`Transaction ${txHash} was not found before timeout`);
    }

    const finality = getReceiptFinalityStatus(receipt);
    const execution = getReceiptExecutionStatus(receipt);

    if (finality === "REJECTED") {
      throw new Error(
        `Transaction ${txHash} was rejected: ${JSON.stringify(receipt)}`,
      );
    }

    if (execution === "REVERTED") {
      const revertReason =
        typeof (receipt as any).revert_reason === "string"
          ? (receipt as any).revert_reason
          : JSON.stringify(receipt);
      throw new Error(`Transaction ${txHash} reverted: ${revertReason}`);
    }

    return receipt;
  }

  async declareContract(
    name: string,
    options?: { allowAlreadyDeclared?: boolean },
  ): Promise<DeclaredContractRef> {
    const sierra = readContractSierra(name);
    const casm = readContractCasm(name);
    const expectedClassHash = hash.computeContractClassHash(sierra);
    try {
      const response = await this.withNonceRetry((nonce) =>
        this.account.declare(
          {
            contract: sierra,
            casm,
          },
          { nonce },
        ),
      );

      await this.waitForTransaction(response.transaction_hash);

      return {
        txHash: response.transaction_hash,
        classHash: response.class_hash,
      };
    } catch (error) {
      if (options?.allowAlreadyDeclared && isClassAlreadyDeclaredError(error)) {
        return {
          txHash: "0x0",
          classHash: expectedClassHash,
        };
      }

      throw error;
    }
  }

  async deployFromDeclared(classHash: string): Promise<DeployedContractRef> {
    const deployResult = await this.withNonceRetry((nonce) =>
      this.account.deploy({ classHash }, { nonce }),
    );
    await this.waitForTransaction(deployResult.transaction_hash);

    const address = Array.isArray(deployResult.contract_address)
      ? deployResult.contract_address[0]
      : deployResult.contract_address;

    return {
      txHash: deployResult.transaction_hash,
      contractAddress: address,
    };
  }

  async deployFreshAccount(fundAmount: bigint = 1_000_000_000_000_000n): Promise<DeployedAccountRef> {
    const sierra = readContractSierraInArtifacts("openzeppelin_AccountUpgradeable");
    const classHash = hash.computeContractClassHash(sierra);

    const privateKey = stark.randomAddress();
    const publicKey = ec.starkCurve.getStarkKey(privateKey);
    const constructorCalldata = CallData.compile({ publicKey });

    const accountAddress = hash.calculateContractAddressFromHash(
      publicKey,
      classHash,
      constructorCalldata,
      0,
    );

    await this.transferKnownToken(accountAddress, fundAmount);

    const newAccount = new Account({
      provider: this.provider,
      address: accountAddress,
      signer: privateKey,
    });

    const deployNonce = await getNonceOrZero(this.provider, accountAddress);

    const response = await newAccount.deployAccount({
      classHash,
      constructorCalldata,
      addressSalt: publicKey,
    }, { nonce: deployNonce });

    await this.waitForTransaction(response.transaction_hash);
    const deployedAddress = Array.isArray(response.contract_address)
      ? response.contract_address[0]
      : response.contract_address;

    return {
      txHash: response.transaction_hash,
      classHash,
      contractAddress: deployedAddress,
      privateKey,
    };
  }

  async transferKnownToken(
    recipient: string,
    amount: bigint,
  ): Promise<E2ETxRef> {
    const transferResponse = await this.withNonceRetry((nonce) =>
      this.account.execute(
        {
          contractAddress: ERC20_CONTRACT_ADDRESS,
          entrypoint: "transfer",
          calldata: CallData.compile({
            recipient,
            amount: cairo.uint256(amount),
          }),
        },
        { nonce },
      ),
    );

    await this.waitForTransaction(transferResponse.transaction_hash);
    return { txHash: transferResponse.transaction_hash };
  }

  async getClassHashAt(address: string): Promise<string> {
    return this.provider.getClassHashAt(address, "latest");
  }

  async getClass(classHash: string): Promise<any> {
    return this.provider.getClass(classHash, "latest");
  }

  async getClassAt(address: string): Promise<any> {
    return this.provider.getClassAt(address, "latest");
  }

  async estimateTransferFee(
    recipient: string,
    amount: bigint,
  ): Promise<{ overall_fee: bigint }> {
    return this.withNonceRetry((nonce) =>
      this.account.estimateInvokeFee(
        {
          contractAddress: ERC20_CONTRACT_ADDRESS,
          entrypoint: "transfer",
          calldata: CallData.compile({
            recipient,
            amount: cairo.uint256(amount),
          }),
        },
        { nonce },
      ),
    );
  }

  async getReceiverBalance(address: string): Promise<bigint> {
    const erc20Class = readContractSierraInArtifacts("openzeppelin_ERC20Upgradeable");

    const contract = new Contract({
      abi: erc20Class.abi,
      address: ERC20_CONTRACT_ADDRESS,
      providerOrAccount: this.account,
    });

    const balance = await contract.balance_of(address);
    return toBigintFromUint256(balance);
  }

  async getSenderBalance(): Promise<bigint> {
    return this.getReceiverBalance(SIGNER_CONTRACT_ADDRESS);
  }

  computeClassHash(contractName: string): string {
    const sierra = readContractSierra(contractName);
    return hash.computeContractClassHash(sierra);
  }

  private async resolveNonceForSubmission(): Promise<string> {
    const latest = await this.getNonceAt("latest");
    const pendingLike = await this.getPendingLikeNonceOrLatest(latest);

    const readyNonce = pendingLike > latest
      ? await waitFor(
          async () => this.getNonceAt("latest"),
          (value) => value >= pendingLike,
          { timeoutMs: 90_000, intervalMs: 1000 },
        )
      : latest;

    return `0x${readyNonce.toString(16)}`;
  }

  private async getNonceAt(
    blockTag: "latest" | "pending" | "pre_confirmed",
  ): Promise<bigint> {
    const value = await this.provider.getNonceForAddress(
      this.account.address,
      blockTag as any,
    );
    return BigInt(value);
  }

  private async getPendingLikeNonceOrLatest(latest: bigint): Promise<bigint> {
    try {
      return await this.getNonceAt(this.pendingLikeTag);
    } catch {
      return latest;
    }
  }

  private async withNonceRetry<T>(
    operation: (nonceHex: string) => Promise<T>,
  ): Promise<T> {
    const nonce = await this.resolveNonceForSubmission();

    try {
      return await operation(nonce);
    } catch (error) {
      const expected = extractExpectedAccountNonceHex(error);
      if (!expected || normalizeHex(nonce) === normalizeHex(expected)) {
        throw error;
      }

      return operation(expected);
    }
  }
}

function getReceiptFinalityStatus(
  receipt: any,
): string | undefined {
  if (!receipt || typeof receipt !== "object") {
    return undefined;
  }

  return receipt.finality_status ?? receipt.status;
}

function getReceiptExecutionStatus(
  receipt: any,
): string | undefined {
  if (!receipt || typeof receipt !== "object") {
    return undefined;
  }

  return receipt.execution_status;
}

function isTxNotFoundError(error: unknown): boolean {
  const message = String(
    (error as { message?: string } | undefined)?.message ?? error,
  );
  return (
    message.includes("Transaction hash not found") ||
    message.includes("TRANSACTION_HASH_NOT_FOUND")
  );
}

function extractExpectedAccountNonceHex(error: unknown): string | null {
  const message = String(
    (error as { message?: string } | undefined)?.message ?? error,
  );
  const match = message.match(
    /Account nonce:\s*(0x[0-9a-fA-F]+)\s*;\s*got:\s*(0x[0-9a-fA-F]+)/,
  );
  if (!match) {
    return null;
  }

  return normalizeHex(match[1]);
}

function normalizeHex(value: string): string {
  return `0x${BigInt(value).toString(16)}`;
}

function isClassAlreadyDeclaredError(error: unknown): boolean {
  const message = String(
    (error as { message?: string } | undefined)?.message ?? error,
  );

  return (
    message.includes("already declared") ||
    message.includes("CLASS_ALREADY_DECLARED")
  );
}

async function getNonceOrZero(
  provider: RpcProvider,
  address: string,
): Promise<string> {
  try {
    return await provider.getNonceForAddress(address, "latest");
  } catch {
    return "0x0";
  }
}

function toBigintFromUint256(value: any): bigint {
  if (typeof value === "bigint") {
    return value;
  }

  if (typeof value === "string") {
    return BigInt(value);
  }

  if (Array.isArray(value) && value.length >= 2) {
    const low = BigInt(value[0]);
    const high = BigInt(value[1]);
    return low + (high << 128n);
  }

  if (value && typeof value === "object" && "low" in value && "high" in value) {
    const low = BigInt((value as any).low);
    const high = BigInt((value as any).high);
    return low + (high << 128n);
  }

  throw new Error(`Could not convert ERC20 balance to bigint: ${JSON.stringify(value)}`);
}
