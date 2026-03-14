import type {
  JsonRpcEnvelopeError,
  JsonRpcEnvelopeSuccess,
  RpcTransport,
} from "../config/types";
import { RpcCallError } from "./errors";

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isJsonRpcSuccess(
  value: unknown,
): value is JsonRpcEnvelopeSuccess<unknown> {
  return (
    isObject(value) &&
    value.jsonrpc === "2.0" &&
    "id" in value &&
    "result" in value &&
    !("error" in value)
  );
}

function isJsonRpcError(value: unknown): value is JsonRpcEnvelopeError {
  return (
    isObject(value) &&
    value.jsonrpc === "2.0" &&
    "id" in value &&
    isObject(value.error) &&
    typeof value.error.code === "number" &&
    typeof value.error.message === "string"
  );
}

export class HttpJsonRpcTransport implements RpcTransport {
  private requestId = 0;

  constructor(private readonly endpoint: string) {}

  async rawCall(
    method: string,
    params: unknown,
  ): Promise<JsonRpcEnvelopeSuccess<unknown> | JsonRpcEnvelopeError> {
    const id = ++this.requestId;

    const response = await fetch(this.endpoint, {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({
        jsonrpc: "2.0",
        method,
        params,
        id,
      }),
    });

    const json = (await response.json()) as unknown;

    if (isJsonRpcSuccess(json) || isJsonRpcError(json)) {
      return json;
    }

    throw new Error(
      `Invalid JSON-RPC envelope returned by ${method}: ${JSON.stringify(json)}`,
    );
  }

  async call<T>(method: string, params: unknown): Promise<T> {
    const envelope = await this.rawCall(method, params);

    if ("error" in envelope) {
      throw new RpcCallError(
        method,
        envelope.error.code,
        envelope.error.message,
        envelope.error.data,
      );
    }

    return envelope.result as T;
  }
}
