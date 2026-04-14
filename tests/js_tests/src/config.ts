export const DEFAULT_ACCOUNT_ADDRESS =
  "0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d";
export const DEFAULT_PRIVATE_KEY =
  "0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07";
export const ERC20_STRK_ADDRESS =
  "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";
export const ERC20_ETH_ADDRESS =
  "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";
export const UDC_ADDRESS =
  "0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf";

export const RPC_PORT = 9944;
export const ADMIN_PORT = 9943;

export function getRpcUrl(version: string): string {
  const versionPath = version.replace(/\./g, "_");
  return `http://127.0.0.1:${RPC_PORT}/rpc/v${versionPath}/`;
}

export function getAdminUrl(): string {
  return `http://127.0.0.1:${ADMIN_PORT}/`;
}

export const BLOCK_POLL_INTERVAL_MS = 500;
export const BLOCK_POLL_TIMEOUT_MS = 30_000;
