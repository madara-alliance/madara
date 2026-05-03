import Ajv, { ValidateFunction } from "ajv";
import { ensureSpecCached, loadCachedSpec } from "./spec-cache";
import {
  createSpecAjv,
  compileResultValidator,
  OpenRpcMethodDef,
} from "./schema-compiler";

interface MethodEntry {
  name: string;
  source: "read" | "write" | "trace";
  def: OpenRpcMethodDef;
}

export class SpecRegistry {
  private methods: Map<string, MethodEntry> = new Map();
  private validators: Map<string, ValidateFunction> = new Map();

  constructor(
    methods: Map<string, MethodEntry>,
    validators: Map<string, ValidateFunction>,
  ) {
    this.methods = methods;
    this.validators = validators;
  }

  /**
   * Get all method names defined in the spec.
   */
  getMethodNames(): string[] {
    return Array.from(this.methods.keys()).sort();
  }

  /**
   * Validate an RPC result against the spec schema.
   * Returns null if method has no result schema, otherwise returns
   * validation errors or empty array.
   */
  validateResult(method: string, result: unknown): string[] | null {
    const validator = this.validators.get(method);
    if (!validator) return null;

    const valid = validator(result);
    if (valid) return [];

    const errors = validator.errors || [];

    // Tolerate oneOf ambiguity (AJV reports multiple branches matching)
    if (errors.every((e: any) => e.keyword === "oneOf")) return [];

    // Filter out known spec quirks:
    // - continuation_token: spec says string, Madara returns null when absent
    const filtered = errors.filter(
      (e: any) =>
        !(
          e.instancePath === "/continuation_token" &&
          e.keyword === "type" &&
          e.message === "must be string"
        ),
    );
    if (filtered.length === 0) return [];

    return filtered.map(
      (e: any) => `${e.instancePath || "$"}: ${e.message} (${e.keyword})`,
    );
  }
}

/**
 * Create a SpecRegistry by downloading, caching, and indexing the spec.
 * All validators are compiled eagerly — if any method's schema fails to
 * compile, this function throws immediately rather than silently skipping.
 */
export async function createSpecRegistry(
  versionTag: string,
): Promise<SpecRegistry> {
  await ensureSpecCached(versionTag);

  const read = loadCachedSpec(versionTag, "starknet_api_openrpc.json");
  const write = loadCachedSpec(versionTag, "starknet_write_api.json");
  const trace = loadCachedSpec(versionTag, "starknet_trace_api_openrpc.json");

  const ajv = createSpecAjv({ read, write, trace });

  const methods = new Map<string, MethodEntry>();
  const validators = new Map<string, ValidateFunction>();

  const indexMethods = (doc: any, source: "read" | "write" | "trace") => {
    for (const method of doc.methods || []) {
      const def: OpenRpcMethodDef = {
        ...method,
        components: doc.components,
      };
      methods.set(method.name, { name: method.name, source, def });

      // Eagerly compile validator — throws on failure
      const validator = compileResultValidator(ajv, def);
      if (validator) {
        validators.set(method.name, validator);
      }
    }
  };

  indexMethods(read, "read");
  indexMethods(write, "write");
  indexMethods(trace, "trace");

  return new SpecRegistry(methods, validators);
}
