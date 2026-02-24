import Ajv, { type ValidateFunction } from "ajv";

export interface OpenRpcMethodDef {
  name: string;
  params?: Array<{ name: string; required?: boolean; schema?: unknown }>;
  result?: { schema?: unknown };
  components?: Record<string, unknown>;
}

export interface OpenRpcReferenceSchemas {
  readDoc: unknown;
  writeDoc: unknown;
  traceDoc: unknown;
  wsDoc: unknown;
}

const OPENRPC_REF_IDS = {
  read: [
    "./api/starknet_api_openrpc.json",
    "api/starknet_api_openrpc.json",
    "starknet_api_openrpc.json",
  ],
  write: [
    "./api/starknet_write_api.json",
    "api/starknet_write_api.json",
    "starknet_write_api.json",
  ],
  trace: [
    "./api/starknet_trace_api_openrpc.json",
    "api/starknet_trace_api_openrpc.json",
    "starknet_trace_api_openrpc.json",
  ],
  ws: [
    "./api/starknet_ws_api.json",
    "api/starknet_ws_api.json",
    "starknet_ws_api.json",
  ],
} as const;

export function createAjv(): Ajv {
  return new Ajv({
    strict: false,
    allErrors: true,
    allowUnionTypes: true,
  });
}

function withSchemaId(doc: unknown, id: string): Record<string, unknown> {
  if (doc && typeof doc === "object") {
    return { ...(doc as Record<string, unknown>), $id: id };
  }

  return { $id: id };
}

function registerAliases(
  ajv: Ajv,
  ids: readonly string[],
  schema: Record<string, unknown>,
): void {
  for (const id of ids) {
    ajv.addSchema(withSchemaId(schema, id), id);
  }
}

export function registerOpenRpcReferenceSchemas(
  ajv: Ajv,
  schemas: OpenRpcReferenceSchemas,
): void {
  registerAliases(ajv, OPENRPC_REF_IDS.read, schemas.readDoc as Record<string, unknown>);
  registerAliases(ajv, OPENRPC_REF_IDS.write, schemas.writeDoc as Record<string, unknown>);
  registerAliases(ajv, OPENRPC_REF_IDS.trace, schemas.traceDoc as Record<string, unknown>);
  registerAliases(ajv, OPENRPC_REF_IDS.ws, schemas.wsDoc as Record<string, unknown>);
}

export function compileMethodResultValidator(
  ajv: Ajv,
  method: OpenRpcMethodDef,
): ValidateFunction {
  const resultSchema = method.result?.schema;
  if (!resultSchema) {
    return ajv.compile(true);
  }

  const wrapped = {
    type: "object",
    properties: {
      result: resultSchema,
    },
    required: ["result"],
    additionalProperties: true,
    components: method.components || {},
  };

  return ajv.compile(wrapped);
}

export function compileMethodParamsValidator(
  ajv: Ajv,
  method: OpenRpcMethodDef,
): ValidateFunction {
  const params = method.params ?? [];

  if (params.length === 0) {
    const wrappedNoParams = {
      oneOf: [
        {
          type: "array",
          minItems: 0,
          maxItems: 0,
        },
        {
          type: "object",
          additionalProperties: false,
          maxProperties: 0,
        },
      ],
      components: method.components || {},
    };
    return ajv.compile(wrappedNoParams);
  }

  const properties: Record<string, unknown> = {};
  const required: string[] = [];

  for (const param of params) {
    if (!param?.name) {
      continue;
    }
    properties[param.name] = param.schema || true;
    if (param.required) {
      required.push(param.name);
    }
  }

  const namedParamsSchema = {
    type: "object",
    properties,
    required,
    additionalProperties: false,
  };

  const positionalParamsSchema = {
    type: "array",
    prefixItems: params.map((param) => param.schema || true),
    minItems: required.length,
    maxItems: params.length,
  };

  return ajv.compile({
    oneOf: [namedParamsSchema, positionalParamsSchema],
    components: method.components || {},
  });
}
