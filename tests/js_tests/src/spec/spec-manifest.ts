/**
 * Pinned spec file URLs and SHA-256 checksums per RPC version.
 * When adding a new version, add entries here with verified checksums.
 *
 * URLs point to immutable upstream tags where available.
 * The SHA-256 checksums act as a second pin — if upstream content ever changes
 * or a URL is repointed, the checksum mismatch fails explicitly and forces a
 * manual review.
 */
export interface SpecFileLock {
  file: string;
  sourceUrl: string;
  sha256: string;
}

export interface SpecLockEntry {
  tag: string;
  files: SpecFileLock[];
}

export const SPEC_LOCK_MANIFEST: SpecLockEntry[] = [
  {
    tag: "v0.10.0",
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
    ],
  },
  {
    tag: "v0.10.2",
    files: [
      {
        file: "starknet_api_openrpc.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.10.2/api/starknet_api_openrpc.json",
        sha256:
          "f51ef570b31686f36db17973b2bd0e6612a8753246a5067086626c2a0f6c3792",
      },
      {
        file: "starknet_write_api.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.10.2/api/starknet_write_api.json",
        sha256:
          "41f47c083c3ba211dafd20e2934d52d6e19bfc54badce43b6a5406870ec3c6eb",
      },
      {
        file: "starknet_trace_api_openrpc.json",
        sourceUrl:
          "https://raw.githubusercontent.com/starkware-libs/starknet-specs/v0.10.2/api/starknet_trace_api_openrpc.json",
        sha256:
          "ba07e54565e2de6ef25d64e4ba2454c2dbfd0133555557b75c14e5d8f22db993",
      },
    ],
  },
];

export function getSpecLock(tag: string): SpecLockEntry | undefined {
  return SPEC_LOCK_MANIFEST.find((e) => e.tag === tag);
}
