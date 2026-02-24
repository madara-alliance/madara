import type { CapabilitySet, RpcSemver } from "./types";

export function withOfficialMethodMatcher(
  base: CapabilitySet,
  officialMethods: Iterable<string>,
): CapabilitySet {
  const set = new Set(officialMethods);
  return {
    ...base,
    isOfficialSpecMethod: (method: string) => set.has(method),
  };
}

export function isLatestTarget(version: RpcSemver): boolean {
  return version === "0.10.0";
}
