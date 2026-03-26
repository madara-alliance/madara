/**
 * Raw JSON-RPC caller for Starknet methods not directly exposed by starknet.js
 * (e.g., getStorageProof, getCompiledCasm, traceBlockTransactions, getMessagesStatus).
 */
export class RpcCaller {
  private url: string;
  private requestId = 0;

  constructor(url: string) {
    this.url = url;
  }

  async call(method: string, params: any): Promise<any> {
    const id = ++this.requestId;

    const body: any = {
      jsonrpc: "2.0",
      id,
      method,
    };

    // Handle both array params and object params
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

    if (json.error) {
      throw new RpcError(
        method,
        json.error.code,
        json.error.message,
        json.error.data,
      );
    }

    return json.result;
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
