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
  constructor(
    public readonly provider: RpcProvider,
    public readonly account: Account,
  ) {}

  async waitForTransaction(txHash: string): Promise<any> {
    return this.provider.waitForTransaction(txHash);
  }

  async declareContract(name: string): Promise<DeclaredContractRef> {
    const sierra = readContractSierra(name);
    const casm = readContractCasm(name);

    const response = await this.account.declare({
      contract: sierra,
      casm,
    });

    await this.waitForTransaction(response.transaction_hash);

    return {
      txHash: response.transaction_hash,
      classHash: response.class_hash,
    };
  }

  async deployFromDeclared(classHash: string): Promise<DeployedContractRef> {
    const deployResult = await this.account.deploy({ classHash });
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

    const response = await newAccount.deployAccount({
      classHash,
      constructorCalldata,
      addressSalt: publicKey,
    });

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
    const transferResponse = await this.account.execute({
      contractAddress: ERC20_CONTRACT_ADDRESS,
      entrypoint: "transfer",
      calldata: CallData.compile({
        recipient,
        amount: cairo.uint256(amount),
      }),
    });

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
    return this.account.estimateInvokeFee({
      contractAddress: ERC20_CONTRACT_ADDRESS,
      entrypoint: "transfer",
      calldata: CallData.compile({
        recipient,
        amount: cairo.uint256(amount),
      }),
    });
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
