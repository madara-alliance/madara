/**
 * Raw JSON-RPC client for the Madara admin API (port 9943).
 * These are Madara-specific methods not in starknet.js.
 */
export class AdminClient {
  private url: string;
  private requestId = 0;

  constructor(url: string) {
    this.url = url;
  }

  private async call(method: string, params: any = []): Promise<any> {
    const id = ++this.requestId;
    const response = await fetch(this.url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id,
        method,
        params,
      }),
    });

    const json = await response.json();
    if (json.error) {
      throw new Error(
        `Admin RPC error [${method}]: ${json.error.message} (code: ${json.error.code})`,
      );
    }
    return json.result;
  }

  async ping(): Promise<string> {
    return this.call("madara_ping");
  }

  async closeBlock(): Promise<void> {
    await this.call("madara_closeBlock");
  }

  async addL1HandlerMessage(params: {
    contract_address: string;
    entry_point_selector: string;
    calldata: string[];
    nonce: number;
    paid_fee_on_l1: string;
    version?: string;
  }): Promise<{ transaction_hash: string }> {
    // The RPC expects L1HandlerTransactionWithFee { tx: L1HandlerTransaction, paid_fee_on_l1: u128 }
    const message = {
      tx: {
        version: params.version || "0x0",
        nonce: params.nonce,
        contract_address: params.contract_address,
        entry_point_selector: params.entry_point_selector,
        calldata: params.calldata,
      },
      paid_fee_on_l1: parseInt(params.paid_fee_on_l1),
    };
    return this.call("madara_addL1HandlerMessage", [message]);
  }
}
