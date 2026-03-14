export interface SpecFileLock {
  file: string;
  sourceUrl: string;
  sha256: string;
}

export interface SpecLockEntry {
  tag: string;
  infoVersion: string;
  fetchedAt: string;
  notes?: string;
  files: SpecFileLock[];
}

export const SPEC_LOCK_MANIFEST: SpecLockEntry[] = [
  {
    tag: "v0.9.0",
    infoVersion: "0.9.0",
    fetchedAt: "2026-02-24",
    files: [
      {
        file: "starknet_api_openrpc.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.9.0/api/starknet_api_openrpc.json",
        sha256:
          "84df808c90a41b51c24c1fddbcf4e57fa6841b34aeec0ae6cf44d43691f11da0",
      },
      {
        file: "starknet_write_api.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.9.0/api/starknet_write_api.json",
        sha256:
          "8dd987f5ad3ad6641ab90693998eb499bd99ca1979e1c819d90bb55118d2affe",
      },
      {
        file: "starknet_trace_api_openrpc.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.9.0/api/starknet_trace_api_openrpc.json",
        sha256:
          "f9add9e607cfc42299e652f6b3eaefec890a47960bc89eed0f981868b5fbd0c8",
      },
      {
        file: "starknet_ws_api.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.9.0/api/starknet_ws_api.json",
        sha256:
          "212b6c47c4d5629561ee1e3bc64be052c79e5f077f11d146bd55d12e6283adaf",
      },
    ],
  },
  {
    tag: "v0.10.0",
    infoVersion: "0.10.0",
    fetchedAt: "2026-02-24",
    files: [
      {
        file: "starknet_api_openrpc.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.10.0/api/starknet_api_openrpc.json",
        sha256:
          "d8d4dc6279d00b35be3414cf17997523dfa84fa2604619c2ebbf2fdf8dde4b77",
      },
      {
        file: "starknet_write_api.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.10.0/api/starknet_write_api.json",
        sha256:
          "3aa263e858870634103487856c91ea502527ff6248d318bc7143be9e6eb84145",
      },
      {
        file: "starknet_trace_api_openrpc.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.10.0/api/starknet_trace_api_openrpc.json",
        sha256:
          "9cd30bc979f7e17d84cc7212dca0fb80549f42bb46262e96942746647bbdbb5e",
      },
      {
        file: "starknet_ws_api.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.10.0/api/starknet_ws_api.json",
        sha256:
          "0f14dfb3ecc24b5b4e0c485e849b100df77b73501b454ecb15e0490d556f2a33",
      },
    ],
  },
];
