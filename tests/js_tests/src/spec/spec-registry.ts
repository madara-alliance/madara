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
  private ajv: Ajv;
  private methods: Map<string, MethodEntry> = new Map();
  private validatorCache: Map<string, ValidateFunction | null> = new Map();

  constructor(ajv: Ajv, methods: Map<string, MethodEntry>) {
    this.ajv = ajv;
    this.methods = methods;
  }

  /**
   * Get all method names defined in the spec.
   */
  getMethodNames(): string[] {
    return Array.from(this.methods.keys()).sort();
  }

  /**
   * Validate an RPC result against the spec schema.
   * Returns null if no validator exists, otherwise returns validation errors or empty array.
   */
  validateResult(method: string, result: unknown): string[] | null {
    const entry = this.methods.get(method);
    if (!entry) return null;

    let validator = this.validatorCache.get(method);
    if (validator === undefined) {
      validator = compileResultValidator(this.ajv, entry.def);
      this.validatorCache.set(method, validator);
    }

    if (validator === null) return null;

    const valid = validator(result);
    if (valid) return [];

    const errors = validator.errors || [];

    // Tolerate oneOf ambiguity (AJV reports multiple branches matching)
    if (errors.every((e: any) => e.keyword === "oneOf")) return [];

    return errors.map(
      (e: any) => `${e.instancePath || "$"}: ${e.message} (${e.keyword})`,
    );
  }
}

/**
 * Create a SpecRegistry by downloading, caching, and indexing the spec.
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

  const indexMethods = (doc: any, source: "read" | "write" | "trace") => {
    for (const method of doc.methods || []) {
      // Attach the document-level components to each method for $ref resolution
      const def: OpenRpcMethodDef = {
        ...method,
        components: doc.components,
      };
      methods.set(method.name, { name: method.name, source, def });
    }
  };

  indexMethods(read, "read");
  indexMethods(write, "write");
  indexMethods(trace, "trace");

  return new SpecRegistry(ajv, methods);
}
