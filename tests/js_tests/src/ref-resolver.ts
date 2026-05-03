import { hash } from "starknet";
import { StepResult } from "./types";

/**
 * Recursively resolves $ref, $computed, and $self tokens in a JSON value.
 *
 * Supported tokens:
 * - "$ref:step_id.field" → looks up context.get(step_id)?.field
 * - "$computed:sn_keccak:name" → computes starknet_keccak(name)
 * - "$self.public_key" → looks up selfContext.public_key
 */
export function resolveValue(
  value: any,
  context: Map<string, StepResult>,
  selfContext?: Record<string, any>,
): any {
  if (typeof value === "string") {
    return resolveStringToken(value, context, selfContext);
  }

  if (Array.isArray(value)) {
    return value.map((item) => resolveValue(item, context, selfContext));
  }

  if (value !== null && typeof value === "object") {
    const resolved: Record<string, any> = {};
    for (const [k, v] of Object.entries(value)) {
      resolved[k] = resolveValue(v, context, selfContext);
    }
    return resolved;
  }

  return value;
}

function resolveStringToken(
  token: string,
  context: Map<string, StepResult>,
  selfContext?: Record<string, any>,
): any {
  if (token.startsWith("$ref:")) {
    return resolveRef(token, context);
  }

  if (token.startsWith("$computed:")) {
    return resolveComputed(token);
  }

  if (token.startsWith("$self.")) {
    const field = token.slice("$self.".length);
    if (!selfContext || !(field in selfContext)) {
      throw new Error(`$self.${field} not found in self context`);
    }
    return selfContext[field];
  }

  return token;
}

function resolveRef(token: string, context: Map<string, StepResult>): any {
  const refPath = token.slice("$ref:".length);
  const dotIndex = refPath.indexOf(".");
  if (dotIndex === -1) {
    throw new Error(
      `Invalid $ref format: "${token}". Expected "$ref:step_id.field"`,
    );
  }

  const stepId = refPath.slice(0, dotIndex);
  const field = refPath.slice(dotIndex + 1);

  const stepResult = context.get(stepId);
  if (!stepResult) {
    throw new Error(
      `$ref: step "${stepId}" not found in context. Available: [${Array.from(context.keys()).join(", ")}]`,
    );
  }

  const value = (stepResult as any)[field];
  if (value === undefined) {
    throw new Error(
      `$ref: field "${field}" not found in step "${stepId}". Available: [${Object.keys(stepResult).join(", ")}]`,
    );
  }

  return value;
}

function resolveComputed(token: string): string {
  const parts = token.slice("$computed:".length).split(":");
  if (parts[0] === "sn_keccak" && parts.length === 2) {
    return hash.getSelectorFromName(parts[1]);
  }

  throw new Error(`Unknown $computed function: "${token}"`);
}
