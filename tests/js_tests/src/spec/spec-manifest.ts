/**
 * Pinned spec file URLs and SHA-256 checksums per RPC version.
 * When adding a new version, add entries here with verified checksums.
 *
 * URLs point to the `main` branch of the pathfinder repo because the
 * Starknet RPC spec doesn't publish tagged releases for individual versions.
 * The SHA-256 checksums act as the real pin — if upstream updates a spec file,
 * the checksum mismatch will fail explicitly, prompting a manual review and
 * re-pin of the new checksums.
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
          "https://raw.githubusercontent.com/eqlabs/pathfinder/main/specs/rpc/v10/starknet_api_openrpc.json",
        sha256:
          "f51ef570b31686f36db17973b2bd0e6612a8753246a5067086626c2a0f6c3792",
      },
      {
        file: "starknet_write_api.json",
        sourceUrl:
          "https://raw.githubusercontent.com/eqlabs/pathfinder/main/specs/rpc/v10/starknet_write_api.json",
        sha256:
          "41f47c083c3ba211dafd20e2934d52d6e19bfc54badce43b6a5406870ec3c6eb",
      },
      {
        file: "starknet_trace_api_openrpc.json",
        sourceUrl:
          "https://raw.githubusercontent.com/eqlabs/pathfinder/main/specs/rpc/v10/starknet_trace_api_openrpc.json",
        sha256:
          "ba07e54565e2de6ef25d64e4ba2454c2dbfd0133555557b75c14e5d8f22db993",
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
