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
 * Create an AJV instance with all spec documents registered as schemas
 * so that cross-file $ref references resolve correctly.
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

  // Register each document under multiple alias IDs for $ref resolution
  const aliases: Record<string, string[]> = {
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

  for (const [key, ids] of Object.entries(aliases)) {
    const doc = docs[key as keyof typeof docs];
    if (!doc) continue;

    // Build a schema document with the components from the spec
    const schemaDoc: any = {
      $id: ids[0],
      components: doc.components || {},
    };

    try {
      ajv.addSchema(schemaDoc);
    } catch {
      // Schema might already be added
    }

    // Register additional aliases
    for (let i = 1; i < ids.length; i++) {
      try {
        ajv.addSchema({ ...schemaDoc, $id: ids[i] });
      } catch {
        // Ignore duplicates
      }
    }
  }

  return ajv;
}

/**
 * Compile a validator for a method's result schema.
 */
export function compileResultValidator(
  ajv: Ajv,
  method: OpenRpcMethodDef,
): ValidateFunction | null {
  const resultSchema = method.result?.schema;
  if (!resultSchema) return null;

  try {
    return ajv.compile(resultSchema);
  } catch (err) {
    console.warn(
      `[schema-compiler] Failed to compile result validator for ${method.name}: ${err}`,
    );
    return null;
  }
}
