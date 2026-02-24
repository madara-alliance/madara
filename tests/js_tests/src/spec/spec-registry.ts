import fs from "node:fs";
import { resolveSpecFilePath } from "../config/matrix";
import type { SpecBinding, SpecRegistry } from "../config/types";
import {
  compileMethodParamsValidator,
  compileMethodResultValidator,
  createAjv,
  registerOpenRpcReferenceSchemas,
  type OpenRpcMethodDef,
} from "./schema-compiler";

interface OpenRpcDocument {
  methods?: Array<{
    name: string;
    params?: Array<{ name: string; required?: boolean; schema?: unknown }>;
    result?: { schema?: unknown };
  }>;
  components?: Record<string, unknown>;
}

type MethodSource = "read" | "write" | "trace" | "ws";

interface MethodEntry extends OpenRpcMethodDef {
  source: MethodSource;
}

export interface OpenRpcBundle {
  readDoc: OpenRpcDocument;
  writeDoc: OpenRpcDocument;
  traceDoc: OpenRpcDocument;
  wsDoc: OpenRpcDocument;
}

export function loadOpenRpcBundle(binding: SpecBinding): OpenRpcBundle {
  return {
    readDoc: loadOpenRpcDocument(binding.specTag, binding.files.readApi),
    writeDoc: loadOpenRpcDocument(binding.specTag, binding.files.writeApi),
    traceDoc: loadOpenRpcDocument(binding.specTag, binding.files.traceApi),
    wsDoc: loadOpenRpcDocument(binding.specTag, binding.files.wsApi),
  };
}

function loadOpenRpcDocument(specTag: string, fileName: string): OpenRpcDocument {
  const filePath = resolveSpecFilePath(specTag, fileName);
  const content = fs.readFileSync(filePath, "utf8");
  return JSON.parse(content) as OpenRpcDocument;
}

export class OpenRpcSpecRegistry implements SpecRegistry {
  private readonly methods = new Map<string, MethodEntry>();

  private readonly paramsValidatorCache = new Map<string, (x: unknown) => boolean>();

  private readonly resultValidatorCache = new Map<string, (x: unknown) => boolean>();

  private readonly ajv = createAjv();

  constructor(private readonly bundle: OpenRpcBundle) {
    registerOpenRpcReferenceSchemas(this.ajv, bundle);
    this.addMethods(bundle.readDoc, "read");
    this.addMethods(bundle.writeDoc, "write");
    this.addMethods(bundle.traceDoc, "trace");
  }

  private addMethods(doc: OpenRpcDocument, source: MethodSource): void {
    const methods = doc.methods || [];

    for (const method of methods) {
      if (!method.name) {
        continue;
      }

      this.methods.set(method.name, {
        name: method.name,
        params: method.params,
        result: method.result,
        components: doc.components,
        source,
      });
    }
  }

  listOfficialMethods(): string[] {
    return [...this.methods.keys()].sort();
  }

  getMethodResultValidator(method: string): (x: unknown) => boolean {
    const cached = this.resultValidatorCache.get(method);
    if (cached) {
      return cached;
    }

    const definition = this.getMethodOrThrow(method);
    const validator = compileMethodResultValidator(this.ajv, definition);

    const wrapped = (value: unknown): boolean => {
      const ok = validator({ result: value });
      return Boolean(ok);
    };

    this.resultValidatorCache.set(method, wrapped);
    return wrapped;
  }

  getMethodParamsValidator(method: string): (x: unknown) => boolean {
    const cached = this.paramsValidatorCache.get(method);
    if (cached) {
      return cached;
    }

    const definition = this.getMethodOrThrow(method);
    const validator = compileMethodParamsValidator(this.ajv, definition);

    const wrapped = (value: unknown): boolean => Boolean(validator(value));
    this.paramsValidatorCache.set(method, wrapped);
    return wrapped;
  }

  getMethodSource(method: string): MethodSource {
    return this.getMethodOrThrow(method).source;
  }

  private getMethodOrThrow(method: string): MethodEntry {
    const found = this.methods.get(method);
    if (!found) {
      throw new Error(`Method ${method} not found in official Starknet OpenRPC docs`);
    }
    return found;
  }
}

export function createSpecRegistry(binding: SpecBinding): OpenRpcSpecRegistry {
  return new OpenRpcSpecRegistry(loadOpenRpcBundle(binding));
}
