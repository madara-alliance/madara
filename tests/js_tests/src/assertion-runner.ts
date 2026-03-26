import {
  Account,
  RpcProvider,
  hash,
  ec,
  stark,
  CallData,
  cairo,
  Deployer,
} from "starknet";
import { RpcCaller } from "./rpc-caller";
import { resolveValue } from "./ref-resolver";
import { loadContractSierra, loadContractCasm } from "./contract-loader";
import {
  DEFAULT_ACCOUNT_ADDRESS,
  DEFAULT_PRIVATE_KEY,
  UDC_ADDRESS,
  ERC20_STRK_ADDRESS,
} from "./config";
import { Assertion, ConstructHint, TestContext } from "./types";

// We use raw JSON-RPC for ALL assertion calls to test the actual RPC spec,
// not starknet.js's abstraction layer which may transform params/responses.

export async function runAssertion(
  assertion: Assertion,
  ctx: TestContext,
): Promise<void> {
  const provider = new RpcProvider({ nodeUrl: ctx.rpcUrl });
  const rpcCaller = new RpcCaller(ctx.rpcUrl);

  // Resolve $ref and $computed in params
  const resolvedParams = resolveValue(assertion.params, ctx.results);

  // Build transaction if construct hint is present
  let finalParams = resolvedParams;
  let actual: any;

  if (assertion.construct) {
    finalParams = await buildConstructedParams(
      assertion,
      resolvedParams,
      ctx,
      provider,
    );
    // If construct returned a _raw_result, the RPC was already called
    if (finalParams && finalParams._raw_result !== undefined) {
      actual = deepSerialize(finalParams._raw_result);
    }
  }

  if (actual === undefined) {
    // Make the RPC call
    actual = await executeRpcCall(
      assertion.method,
      finalParams,
      provider,
      rpcCaller,
    );
  }

  // Store result for cross-validation
  ctx.assertionResults.set(assertion.id, actual);

  // Resolve $ref and $computed in expected
  const resolvedExpected = resolveValue(assertion.expected, ctx.results);

  // Match actual vs expected
  const errors = matchValue(resolvedExpected, actual, "$");
  if (errors.length > 0) {
    const errorMsg = errors
      .map((e) => `  ${e.path}: expected ${e.expected}, got ${e.actual}`)
      .join("\n");
    throw new Error(
      `Assertion "${assertion.id}" failed (${assertion.method}):\n${errorMsg}`,
    );
  }

  // Validate response shape against official OpenRPC spec (if registry available)
  if (ctx.specRegistry && !assertion.construct) {
    const specErrors = ctx.specRegistry.validateResult(
      assertion.method,
      actual,
    );
    if (specErrors && specErrors.length > 0) {
      throw new Error(
        `Spec schema validation failed for "${assertion.id}" (${assertion.method}):\n  ${specErrors.join("\n  ")}`,
      );
    }
  }
}

// ---- RPC Call Execution ----

async function executeRpcCall(
  method: string,
  params: any,
  _provider: RpcProvider,
  rpcCaller: RpcCaller,
): Promise<any> {
  // All assertion calls go through raw JSON-RPC to test the actual spec
  const rpcParams = formatRpcParams(method, params);
  return rpcCaller.call(method, rpcParams);
}

/**
 * Format params for JSON-RPC call according to the Starknet spec.
 * Most methods use named params (object), some use positional (array).
 */
function formatRpcParams(method: string, params: any): any {
  if (Array.isArray(params)) return params;
  if (!params || Object.keys(params).length === 0) return [];
  return params;
}

// ---- Construct Transaction Objects ----

async function buildConstructedParams(
  assertion: Assertion,
  resolvedParams: any,
  ctx: TestContext,
  provider: RpcProvider,
): Promise<any> {
  const hint = resolveValue(assertion.construct!, ctx.results) as ConstructHint;

  switch (hint.type) {
    case "invoke_v3":
      return buildInvokeV3Params(
        assertion,
        hint,
        resolvedParams,
        ctx,
        provider,
      );
    case "declare_v3":
      return buildDeclareV3Params(
        assertion,
        hint,
        resolvedParams,
        ctx,
        provider,
      );
    case "deploy_account_v3":
      return buildDeployAccountV3Params(
        assertion,
        hint,
        resolvedParams,
        ctx,
        provider,
      );
    default:
      throw new Error(`Unknown construct type: ${hint.type}`);
  }
}

async function buildInvokeV3Params(
  assertion: Assertion,
  hint: ConstructHint,
  resolvedParams: any,
  ctx: TestContext,
  provider: RpcProvider,
): Promise<any> {
  const deployer = new Deployer(UDC_ADDRESS, "deployContract");
  const account = new Account({
    provider,
    address: hint.sender_address || DEFAULT_ACCOUNT_ADDRESS,
    signer: DEFAULT_PRIVATE_KEY,
    deployer,
  });

  const calldata = hint.calldata || [];
  const call = {
    contractAddress: hint.contract_address!,
    entrypoint: hint.entry_point!,
    calldata,
  };

  const method = assertion.method;

  if (method === "starknet_addInvokeTransaction") {
    const response = await account.execute(call);
    return { _raw_result: response };
  }

  if (method === "starknet_estimateFee") {
    const feeEstimate = await account.estimateInvokeFee([call]);
    return { _raw_result: feeEstimate };
  }

  if (method === "starknet_simulateTransactions") {
    const typedCall = { type: "INVOKE" as const, ...call };
    const simulation = await account.simulateTransaction([typedCall], {
      skipValidate: (resolvedParams.simulation_flags || []).includes(
        "SKIP_VALIDATE",
      ),
    });
    return { _raw_result: simulation };
  }

  return resolvedParams;
}

async function buildDeclareV3Params(
  _assertion: Assertion,
  hint: ConstructHint,
  resolvedParams: any,
  _ctx: TestContext,
  provider: RpcProvider,
): Promise<any> {
  const deployer = new Deployer(UDC_ADDRESS, "deployContract");
  const account = new Account({
    provider,
    address: DEFAULT_ACCOUNT_ADDRESS,
    signer: DEFAULT_PRIVATE_KEY,
    deployer,
  });

  const sierra = loadContractSierra(hint.contract!);
  const casm = loadContractCasm(hint.contract!);

  const response = await account.declare({ contract: sierra, casm });
  return { _raw_result: response };
}

async function buildDeployAccountV3Params(
  _assertion: Assertion,
  _hint: ConstructHint,
  resolvedParams: any,
  ctx: TestContext,
  provider: RpcProvider,
): Promise<any> {
  const deployer = new Deployer(UDC_ADDRESS, "deployContract");
  const privateKey = stark.randomAddress();
  const publicKey = ec.starkCurve.getStarkKey(privateKey);

  // Use the same account class as the pre-deployed accounts
  const classHash = resolveValue(
    "$ref:declare_test_account.class_hash",
    ctx.results,
  ) as string;

  const constructorCalldata = CallData.compile({ publicKey });
  const contractAddress = hash.calculateContractAddressFromHash(
    publicKey,
    classHash,
    constructorCalldata,
    0,
  );

  // Fund the new account
  const fundAccount = new Account({
    provider,
    address: DEFAULT_ACCOUNT_ADDRESS,
    signer: DEFAULT_PRIVATE_KEY,
    deployer,
  });

  const fundResponse = await fundAccount.execute({
    contractAddress: ERC20_STRK_ADDRESS,
    entrypoint: "transfer",
    calldata: CallData.compile({
      recipient: contractAddress,
      amount: cairo.uint256("0x2386f26fc10000"),
    }),
  });
  await provider.waitForTransaction(fundResponse.transaction_hash);

  // Deploy the account
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

  return { _raw_result: response };
}

// ---- Serialization ----

/** Convert BigInts and other non-JSON types to JSON-safe values */
function deepSerialize(value: any): any {
  if (value === null || value === undefined) return value;
  if (typeof value === "bigint") return "0x" + value.toString(16);
  if (
    typeof value === "number" ||
    typeof value === "boolean" ||
    typeof value === "string"
  )
    return value;
  if (Array.isArray(value)) return value.map(deepSerialize);
  if (typeof value === "object") {
    const result: any = {};
    for (const [k, v] of Object.entries(value)) {
      result[k] = deepSerialize(v);
    }
    return result;
  }
  return value;
}

// ---- Assertion Matching ----

interface MatchError {
  path: string;
  expected: string;
  actual: string;
}

export function matchValue(
  expected: any,
  actual: any,
  path: string,
): MatchError[] {
  // Handle special assertion tokens
  if (typeof expected === "string") {
    return matchStringAssertion(expected, actual, path);
  }

  // Null / undefined
  if (expected === null || expected === undefined) {
    if (actual !== null && actual !== undefined) {
      return [
        { path, expected: "null/undefined", actual: JSON.stringify(actual) },
      ];
    }
    return [];
  }

  // Boolean
  if (typeof expected === "boolean") {
    if (actual !== expected) {
      return [{ path, expected: String(expected), actual: String(actual) }];
    }
    return [];
  }

  // Number
  if (typeof expected === "number") {
    if (actual !== expected) {
      return [{ path, expected: String(expected), actual: String(actual) }];
    }
    return [];
  }

  // Array
  if (Array.isArray(expected)) {
    return matchArray(expected, actual, path);
  }

  // Object (partial match)
  if (typeof expected === "object") {
    return matchObject(expected, actual, path);
  }

  // Fallback: exact match
  if (expected !== actual) {
    return [
      {
        path,
        expected: JSON.stringify(expected),
        actual: JSON.stringify(actual),
      },
    ];
  }
  return [];
}

function matchStringAssertion(
  expected: string,
  actual: any,
  path: string,
): MatchError[] {
  // $any - matches anything non-null
  if (expected === "$any") {
    if (actual === null || actual === undefined) {
      return [{ path, expected: "$any (non-null)", actual: String(actual) }];
    }
    return [];
  }

  // $exists - field must be present (even if null/0/empty)
  if (expected === "$exists") {
    if (actual === undefined) {
      return [{ path, expected: "$exists", actual: "undefined" }];
    }
    return [];
  }

  // $gt:N - greater than
  if (expected.startsWith("$gt:")) {
    const threshold = parseNumberOrHex(expected.slice(4));
    const actualNum = toNumber(actual);
    if (actualNum === null || actualNum <= threshold) {
      return [{ path, expected: `> ${threshold}`, actual: String(actual) }];
    }
    return [];
  }

  // $length:N - exact array length
  if (expected.startsWith("$length:")) {
    const expectedLen = parseInt(expected.slice(8), 10);
    if (!Array.isArray(actual)) {
      return [
        {
          path,
          expected: `array of length ${expectedLen}`,
          actual: typeof actual,
        },
      ];
    }
    if (actual.length !== expectedLen) {
      return [
        {
          path,
          expected: `length ${expectedLen}`,
          actual: `length ${actual.length}`,
        },
      ];
    }
    return [];
  }

  // $length_gte:N - minimum array length
  if (expected.startsWith("$length_gte:")) {
    const minLen = parseInt(expected.slice(12), 10);
    if (!Array.isArray(actual)) {
      return [
        {
          path,
          expected: `array of length >= ${minLen}`,
          actual: typeof actual,
        },
      ];
    }
    if (actual.length < minLen) {
      return [
        {
          path,
          expected: `length >= ${minLen}`,
          actual: `length ${actual.length}`,
        },
      ];
    }
    return [];
  }

  // Literal string comparison (hex values are compared case-insensitively)
  if (isHexString(expected) && isHexString(String(actual))) {
    if (normHex(expected) !== normHex(String(actual))) {
      return [{ path, expected, actual: String(actual) }];
    }
    return [];
  }

  if (String(actual) !== expected) {
    return [{ path, expected, actual: String(actual) }];
  }
  return [];
}

function matchArray(expected: any[], actual: any, path: string): MatchError[] {
  if (!Array.isArray(actual)) {
    return [
      { path, expected: `array[${expected.length}]`, actual: typeof actual },
    ];
  }

  if (actual.length < expected.length) {
    return [
      {
        path,
        expected: `array with at least ${expected.length} elements`,
        actual: `array with ${actual.length} elements`,
      },
    ];
  }

  const errors: MatchError[] = [];
  for (let i = 0; i < expected.length; i++) {
    errors.push(...matchValue(expected[i], actual[i], `${path}[${i}]`));
  }
  return errors;
}

function matchObject(
  expected: Record<string, any>,
  actual: any,
  path: string,
): MatchError[] {
  if (actual === null || actual === undefined || typeof actual !== "object") {
    return [{ path, expected: "object", actual: String(actual) }];
  }

  const errors: MatchError[] = [];
  for (const [key, expectedVal] of Object.entries(expected)) {
    const actualVal = actual[key];
    errors.push(...matchValue(expectedVal, actualVal, `${path}.${key}`));
  }
  return errors;
}

// ---- Helpers ----

function isHexString(s: string): boolean {
  return /^0x[0-9a-fA-F]+$/.test(s);
}

function normHex(s: string): string {
  if (!s.startsWith("0x")) return s.toLowerCase();
  // Remove leading zeros after 0x and lowercase
  const stripped = s.slice(2).replace(/^0+/, "") || "0";
  return "0x" + stripped.toLowerCase();
}

function parseNumberOrHex(s: string): number {
  if (s.startsWith("0x")) return parseInt(s, 16);
  return parseFloat(s);
}

function toNumber(v: any): number | null {
  if (typeof v === "number") return v;
  if (typeof v === "bigint") return Number(v);
  if (typeof v === "string") {
    if (v.startsWith("0x")) return parseInt(v, 16);
    const n = parseFloat(v);
    if (!isNaN(n)) return n;
  }
  return null;
}
