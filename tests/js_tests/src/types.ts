// ---- State Setup JSON Schema ----

export interface StateSetup {
  version: string;
  steps: SetupStep[];
}

export type SetupStep =
  | DeclareStep
  | DeployStep
  | InvokeStep
  | DeployAccountStep
  | L1HandlerStep
  | CloseBlockStep;

interface BaseStep {
  id: string;
  close_block?: boolean;
}

export interface DeclareStep extends BaseStep {
  type: "declare";
  contract: string;
}

export interface DeployStep extends BaseStep {
  type: "deploy";
  class_hash: string;
  constructor_calldata: string[];
  salt: string;
  unique: boolean;
}

export interface InvokeStep extends BaseStep {
  type: "invoke";
  contract_address: string;
  entry_point: string;
  calldata: string[];
}

export interface DeployAccountStep extends BaseStep {
  type: "deploy_account";
  class_hash: string;
  private_key: string;
  constructor_calldata: string[];
  fund_amount: string;
}

export interface L1HandlerStep extends BaseStep {
  type: "l1_handler";
  contract_address: string;
  entry_point: string;
  calldata: string[];
  nonce: number;
  paid_fee_on_l1: string;
}

export interface CloseBlockStep {
  id: string;
  type: "close_block";
}

// ---- Step Results (captured during write phase) ----

export interface StepResult {
  transaction_hash?: string;
  class_hash?: string;
  contract_address?: string;
  block_number?: number;
  block_hash?: string;
}

// ---- Read Assertions JSON Schema ----

export interface ReadAssertions {
  version: string;
  assertions: Assertion[];
}

export interface Assertion {
  id: string;
  method: string;
  params: any;
  expected: any;
  description?: string;
  construct?: ConstructHint;
  skip?: boolean;
}

export interface ConstructHint {
  type: "invoke_v3" | "declare_v3" | "deploy_account_v3";
  sender_address?: string;
  contract_address?: string;
  entry_point?: string;
  calldata?: string[];
  contract?: string;
}

// ---- Error Assertions JSON Schema ----

export interface ErrorAssertions {
  version: string;
  assertions: ErrorAssertion[];
}

export interface ErrorAssertion {
  id: string;
  method: string;
  params: any;
  expected_error: { code: number };
  description?: string;
}

// ---- Test Context (shared between write and read phases) ----

export interface TestContext {
  rpcUrl: string;
  adminUrl: string;
  results: Map<string, StepResult>;
  assertionResults: Map<string, any>;
  nonceTracker: Map<string, bigint>;
  specRegistry?: import("./spec/spec-registry").SpecRegistry;
}
