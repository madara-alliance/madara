import type { RpcSemver } from "../config/types";
import { resolveMatrixFromEnv } from "../config/matrix";

export function singleVersionFromEnv(): RpcSemver {
  return resolveMatrixFromEnv(["0.10.0"])[0];
}

export function preferredPendingLikeTag(
  supportsPreConfirmedTag: boolean,
): "pre_confirmed" | "pending" {
  return supportsPreConfirmedTag ? "pre_confirmed" : "pending";
}

export function uniqueNumbers(values: Array<number | undefined>): number[] {
  return Array.from(
    new Set(values.filter((value): value is number => typeof value === "number")),
  );
}

export function hex64(value: number): string {
  return `0x${value.toString(16).padStart(64, "0")}`;
}
