/**
 * Raw JSON-RPC caller for Starknet RPC methods.
 * Provides both `call()` (unwraps result) and `rawCall()` (returns full envelope).
 */

export interface JsonRpcEnvelope {
  jsonrpc: string;
  id: number;
  result?: any;
  error?: { code: number; message: string; data?: any };
}

export class RpcCaller {
  private url: string;
  private requestId = 0;

  constructor(url: string) {
    this.url = url;
  }

  /**
   * Make a raw JSON-RPC call and return the full envelope (for envelope/error testing).
   */
  async rawCall(method: string, params: any): Promise<JsonRpcEnvelope> {
    const id = ++this.requestId;

    const body: any = {
      jsonrpc: "2.0",
      id,
      method,
    };

    if (Array.isArray(params)) {
      body.params = params;
    } else if (
      params &&
      typeof params === "object" &&
      Object.keys(params).length > 0
    ) {
      body.params = params;
    } else {
      body.params = [];
    }

    const response = await fetch(this.url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body, bigIntReplacer),
    });

    const json = await response.json();

    // Validate JSON-RPC 2.0 envelope structure
    assertValidEnvelope(json, id);

    return json as JsonRpcEnvelope;
  }

  /**
   * Make a JSON-RPC call and return the result, throwing on error.
   */
  async call(method: string, params: any): Promise<any> {
    const envelope = await this.rawCall(method, params);

    if (envelope.error) {
      throw new RpcError(
        method,
        envelope.error.code,
        envelope.error.message,
        envelope.error.data,
      );
    }

    return envelope.result;
  }
}

/**
 * Validate JSON-RPC 2.0 envelope structure.
 */
function assertValidEnvelope(json: any, expectedId: number): void {
  if (json.jsonrpc !== "2.0") {
    throw new Error(
      `Invalid JSON-RPC version: expected "2.0", got "${json.jsonrpc}"`,
    );
  }

  if (json.id !== expectedId) {
    throw new Error(
      `JSON-RPC id mismatch: expected ${expectedId}, got ${json.id}`,
    );
  }

  const hasResult = "result" in json;
  const hasError = "error" in json;

  if (!hasResult && !hasError) {
    throw new Error(
      "Invalid JSON-RPC envelope: neither result nor error present",
    );
  }

  if (hasError) {
    if (typeof json.error.code !== "number") {
      throw new Error(
        `Invalid JSON-RPC error: code must be a number, got ${typeof json.error.code}`,
      );
    }
    if (typeof json.error.message !== "string") {
      throw new Error(
        `Invalid JSON-RPC error: message must be a string, got ${typeof json.error.message}`,
      );
    }
  }
}

function bigIntReplacer(_key: string, value: any): any {
  if (typeof value === "bigint") {
    return "0x" + value.toString(16);
  }
  return value;
}

export class RpcError extends Error {
  code: number;
  data?: any;

  constructor(method: string, code: number, message: string, data?: any) {
    super(`RPC error [${method}]: ${message} (code: ${code})`);
    this.code = code;
    this.data = data;
  }
}
