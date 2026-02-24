export type RpcSemver = `${number}.${number}.${number}`;

export interface SpecBinding {
  rpcVersion: RpcSemver;
  specTag: string;
  specInfoVersion: string;
  files: {
    readApi: string;
    writeApi: string;
    traceApi: string;
    wsApi: string;
  };
}

export interface CapabilitySet {
  supportsPreConfirmedTag: boolean;
  supportsL1AcceptedTag: boolean;
  supportsStorageProofMethod: boolean;
  supportsWsSpec: boolean;
  isOfficialSpecMethod: (method: string) => boolean;
}

export interface RpcTarget {
  id: string;
  semver: RpcSemver;
  baseUrl: string;
  transport: "http";
  spec: SpecBinding;
  capabilities: CapabilitySet;
}

export interface JsonRpcEnvelopeSuccess<T> {
  jsonrpc: "2.0";
  id: string | number;
  result: T;
}

export interface JsonRpcEnvelopeError {
  jsonrpc: "2.0";
  id: string | number | null;
  error: { code: number; message: string; data?: unknown };
}

export interface RpcTransport {
  rawCall(
    method: string,
    params: unknown,
  ): Promise<JsonRpcEnvelopeSuccess<unknown> | JsonRpcEnvelopeError>;
  call<T>(method: string, params: unknown): Promise<T>;
}

export interface SpecRegistry {
  getMethodResultValidator(method: string): (x: unknown) => boolean;
  getMethodParamsValidator(method: string): (x: unknown) => boolean;
  listOfficialMethods(): string[];
}
