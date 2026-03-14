export class RpcCallError extends Error {
  constructor(
    public readonly method: string,
    public readonly code: number,
    message: string,
    public readonly data?: unknown,
  ) {
    super(`${method} failed (${code}): ${message}`);
  }
}
