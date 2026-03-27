import Ajv, { ValidateFunction } from "ajv";
import addFormats from "ajv-formats";

export interface OpenRpcMethodDef {
  name: string;
  params?: Array<{ name: string; schema?: any; required?: boolean }>;
  result?: { schema?: any };
  components?: any;
}

interface OpenRpcDocument {
  methods?: OpenRpcMethodDef[];
  components?: any;
}

/**
 * Create an AJV instance with all spec document components registered
 * so that both internal (#/components/...) and cross-document
 * (./api/starknet_api_openrpc.json#/components/...) $ref pointers resolve.
 */
export function createSpecAjv(docs: {
  read: OpenRpcDocument;
  write: OpenRpcDocument;
  trace: OpenRpcDocument;
}): Ajv {
  const ajv = new Ajv({
    strict: false,
    allErrors: true,
    validateFormats: false,
  });
  addFormats(ajv);

  // Register each document's components under alias IDs that cross-document
  // $refs use. For example, the trace spec references:
  //   ./api/starknet_api_openrpc.json#/components/schemas/FELT
  // So the read doc's components must be findable under that $id.
  const docAliases: Record<string, string[]> = {
    read: [
      "starknet_api_openrpc.json",
      "./api/starknet_api_openrpc.json",
      "api/starknet_api_openrpc.json",
    ],
    write: [
      "starknet_write_api.json",
      "./api/starknet_write_api.json",
      "api/starknet_write_api.json",
    ],
    trace: [
      "starknet_trace_api_openrpc.json",
      "./api/starknet_trace_api_openrpc.json",
      "api/starknet_trace_api_openrpc.json",
    ],
  };

  for (const [key, aliases] of Object.entries(docAliases)) {
    const doc = docs[key as keyof typeof docs];
    if (!doc?.components) continue;

    for (const alias of aliases) {
      try {
        ajv.addSchema({ $id: alias, components: doc.components });
      } catch {
        // Already registered
      }
    }
  }

  return ajv;
}

/**
 * Compile a validator for a method's result schema.
 *
 * The result schema may contain:
 * - Internal $refs: "#/components/schemas/CHAIN_ID" (resolved via components at root)
 * - Cross-doc $refs: "./api/starknet_api_openrpc.json#/components/schemas/FELT"
 *   (resolved via alias schemas registered in createSpecAjv)
 */
export function compileResultValidator(
  ajv: Ajv,
  method: OpenRpcMethodDef,
): ValidateFunction | null {
  const resultSchema = method.result?.schema;
  if (!resultSchema) return null;

  // Include components at root so internal #/components/schemas/... refs resolve
  const wrappedSchema: any = {
    ...resultSchema,
    components: method.components || {},
  };

  return ajv.compile(wrappedSchema);
}
