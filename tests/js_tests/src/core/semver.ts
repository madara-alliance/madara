import type { RpcSemver } from "../config/types";

export interface ParsedSemver {
  major: number;
  minor: number;
  patch: number;
}

export function parseSemver(input: string): ParsedSemver {
  const normalized = input
    .trim()
    .replace(/^v/i, "")
    .replace(/_/g, ".")
    .replace(/\/$/, "");

  const parts = normalized.split(".").filter(Boolean);

  if (parts.length === 2) {
    parts.push("0");
  }

  if (parts.length !== 3 || parts.some((p) => !/^\d+$/.test(p))) {
    throw new Error(`Invalid semver string: ${input}`);
  }

  return {
    major: Number(parts[0]),
    minor: Number(parts[1]),
    patch: Number(parts[2]),
  };
}

export function toSemverString(parsed: ParsedSemver): RpcSemver {
  return `${parsed.major}.${parsed.minor}.${parsed.patch}`;
}

export function normalizeSemver(input: string): RpcSemver {
  return toSemverString(parseSemver(input));
}

export function compareSemver(a: string, b: string): number {
  const va = parseSemver(a);
  const vb = parseSemver(b);

  if (va.major !== vb.major) {
    return va.major - vb.major;
  }
  if (va.minor !== vb.minor) {
    return va.minor - vb.minor;
  }
  return va.patch - vb.patch;
}

export function gteSemver(a: string, b: string): boolean {
  return compareSemver(a, b) >= 0;
}

export function toRpcPathVersion(version: string): string {
  const normalized = normalizeSemver(version);
  return `v${normalized.replace(/\./g, "_")}`;
}
