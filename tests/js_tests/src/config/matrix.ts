import {
  normalizeSemver,
  parseSemver,
  toRpcPathVersion,
  type ParsedSemver,
} from "../core/semver";
import type { CapabilitySet, RpcSemver, RpcTarget, SpecBinding } from "./types";

const SPEC_FILE_NAMES = {
  readApi: "starknet_api_openrpc.json",
  writeApi: "starknet_write_api.json",
  traceApi: "starknet_trace_api_openrpc.json",
  wsApi: "starknet_ws_api.json",
} as const;

export const SPEC_BINDINGS: Record<RpcSemver, SpecBinding> = {
  "0.9.0": {
    rpcVersion: "0.9.0",
    specTag: "v0.9.0",
    specInfoVersion: "0.9.0",
    files: SPEC_FILE_NAMES,
  },
  "0.10.0": {
    rpcVersion: "0.10.0",
    specTag: "v0.10.0",
    specInfoVersion: "0.10.0",
    files: SPEC_FILE_NAMES,
  },
};

function emptyOfficialMethodMatcher(): CapabilitySet["isOfficialSpecMethod"] {
  return () => false;
}

function createBaseCapabilities(version: RpcSemver): CapabilitySet {
  const parsed = parseSemver(version);
  const supportsPreConfirmed = isAtLeast(parsed, { major: 0, minor: 9, patch: 0 });
  const supportsL1Accepted = supportsPreConfirmed;
  const supportsStorageProof = isAtLeast(parsed, { major: 0, minor: 9, patch: 0 });

  return {
    supportsPreConfirmedTag: supportsPreConfirmed,
    supportsL1AcceptedTag: supportsL1Accepted,
    supportsStorageProofMethod: supportsStorageProof,
    supportsWsSpec: true,
    isOfficialSpecMethod: emptyOfficialMethodMatcher(),
  };
}

function isAtLeast(a: ParsedSemver, b: ParsedSemver): boolean {
  if (a.major !== b.major) {
    return a.major > b.major;
  }
  if (a.minor !== b.minor) {
    return a.minor > b.minor;
  }
  return a.patch >= b.patch;
}

export function bindSpec(version: RpcSemver): SpecBinding {
  const direct = SPEC_BINDINGS[version];
  if (direct) {
    return direct;
  }

  if (version.startsWith("0.10.")) {
    return SPEC_BINDINGS["0.10.0"];
  }

  if (version.startsWith("0.9.")) {
    return SPEC_BINDINGS["0.9.0"];
  }
  throw new Error(
    `No Starknet spec binding configured for RPC version ${version}. Supported major/minor targets are 0.9.x and 0.10.x.`,
  );
}

export function resolveRpcRootUrlFromEnv(): string {
  return (
    process.env.RPC_ROOT_URL ||
    process.env.RPC_URL ||
    "http://127.0.0.1:9944"
  ).replace(/\/$/, "");
}

export function buildRpcTarget(versionInput: string, rpcRootUrl?: string): RpcTarget {
  const semver = normalizeSemver(versionInput);
  const spec = bindSpec(semver);
  const rpcRoot = (rpcRootUrl || resolveRpcRootUrlFromEnv()).replace(/\/$/, "");
  const id = toRpcPathVersion(semver);

  return {
    id,
    semver,
    baseUrl: `${rpcRoot}/rpc/${id}/`,
    transport: "http",
    spec,
    capabilities: createBaseCapabilities(semver),
  };
}

export function resolveMatrixFromEnv(defaults?: RpcSemver[]): RpcSemver[] {
  const single = process.env.RPC_VERSION?.trim();
  if (single) {
    return [normalizeSemver(single)];
  }

  const many = process.env.RPC_VERSIONS?.trim();
  if (many) {
    return many
      .split(",")
      .map((v) => v.trim())
      .filter(Boolean)
      .map(normalizeSemver);
  }

  if (defaults && defaults.length > 0) {
    return defaults;
  }

  return ["0.10.0"];
}
