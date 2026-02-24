import { Account, Deployer, RpcProvider } from "starknet";
import type { RpcTarget } from "../config/types";
import {
  SIGNER_CONTRACT_ADDRESS,
  SIGNER_PRIVATE,
  UDC_ADDRESS,
} from "./constants";

export interface AccountFixture {
  provider: RpcProvider;
  account: Account;
}

export function createAccountFixture(target: RpcTarget): AccountFixture {
  const provider = new RpcProvider({ nodeUrl: target.baseUrl });
  const madaraDeployer = new Deployer(UDC_ADDRESS, "deployContract");
  const account = new Account({
    provider,
    address: SIGNER_CONTRACT_ADDRESS,
    signer: SIGNER_PRIVATE,
    deployer: madaraDeployer,
  });

  return { provider, account };
}
