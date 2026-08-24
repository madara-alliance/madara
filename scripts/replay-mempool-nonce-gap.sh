#!/usr/bin/env bash

# Replay a contiguous nonce gap from Madara's MongoDB mempool outbox.
#
# This utility intentionally supports only fee-charging Invoke V3 transactions
# whose paymaster data and account deployment data are empty and whose data
# availability modes are L1. Those fields are not stored in the query-friendly
# MongoDB document, so the transaction hash returned by Madara is checked
# against the persisted hash after every submission.

set -euo pipefail

export LC_ALL=C

usage() {
  cat <<'EOF'
Replay Invoke V3 transactions from the MongoDB mempool outbox in nonce order.

Usage:
  scripts/replay-mempool-nonce-gap.sh [options]

Required options:
  --namespace NAMESPACE          Kubernetes namespace
  --mongo-secret SECRET         Secret containing the MongoDB URI
  --mongo-pod POD               Pod from which mongosh can reach MongoDB
  --mongo-database DATABASE     MongoDB outbox database
  --sender ADDRESS              Account address whose nonce gap is being repaired
  --from-nonce NONCE            First nonce to replay (hexadecimal)
  --to-nonce NONCE              Last nonce to replay, inclusive (hexadecimal)
  --rpc-url URL                 Madara Starknet JSON-RPC URL

Optional:
  --mongo-uri-key KEY           MongoDB URI secret key
                                (default: MADARA_EXTERNAL_DB_MONGODB_URI)
  --mongo-container CONTAINER   Container in the MongoDB pod
  --mongo-collection NAME       Outbox collection
                                (default: mempool_transactions)
  --admin-rpc-url URL           Madara admin RPC URL
  --wait-for-empty-mempool      Wait for the global mempool to become empty;
                                requires --admin-rpc-url
  --wait-timeout SECONDS        Maximum wait for nonce progress and mempool drain
                                (default: 600)
  --poll-interval SECONDS       Poll interval (default: 2)
  --max-transactions COUNT      Safety limit for a single replay (default: 1000)
  --execute                     Submit transactions; without this flag, dry-run only
  -h, --help                    Show this help

Example:
  kubectl -n devnet port-forward pod/madara-0 39944:9944 39943:9943

  scripts/replay-mempool-nonce-gap.sh \
    --namespace devnet \
    --mongo-secret madara-secrets \
    --mongo-pod mongodb-0 \
    --mongo-container mongod \
    --mongo-database network-outbox \
    --sender 0x1234 \
    --from-nonce 0x10 \
    --to-nonce 0x2f \
    --rpc-url http://127.0.0.1:39944/rpc/v0_10 \
    --admin-rpc-url http://127.0.0.1:39943/ \
    --wait-for-empty-mempool \
    --execute
EOF
}

die() {
  echo "Error: $*" >&2
  exit 1
}

require_value() {
  local option="$1"
  local value="${2:-}"
  [[ -n "$value" && "$value" != --* ]] || die "${option} requires a value"
}

NAMESPACE=""
MONGO_SECRET=""
MONGO_URI_KEY="MADARA_EXTERNAL_DB_MONGODB_URI"
MONGO_POD=""
MONGO_CONTAINER=""
MONGO_DATABASE=""
MONGO_COLLECTION="mempool_transactions"
SENDER=""
FROM_NONCE=""
TO_NONCE=""
RPC_URL=""
ADMIN_RPC_URL=""
WAIT_FOR_EMPTY_MEMPOOL=false
WAIT_TIMEOUT=600
POLL_INTERVAL=2
MAX_TRANSACTIONS=1000
EXECUTE=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace)
      require_value "$1" "${2:-}"
      NAMESPACE="$2"
      shift 2
      ;;
    --mongo-secret)
      require_value "$1" "${2:-}"
      MONGO_SECRET="$2"
      shift 2
      ;;
    --mongo-uri-key)
      require_value "$1" "${2:-}"
      MONGO_URI_KEY="$2"
      shift 2
      ;;
    --mongo-pod)
      require_value "$1" "${2:-}"
      MONGO_POD="$2"
      shift 2
      ;;
    --mongo-container)
      require_value "$1" "${2:-}"
      MONGO_CONTAINER="$2"
      shift 2
      ;;
    --mongo-database)
      require_value "$1" "${2:-}"
      MONGO_DATABASE="$2"
      shift 2
      ;;
    --mongo-collection)
      require_value "$1" "${2:-}"
      MONGO_COLLECTION="$2"
      shift 2
      ;;
    --sender)
      require_value "$1" "${2:-}"
      SENDER="$2"
      shift 2
      ;;
    --from-nonce)
      require_value "$1" "${2:-}"
      FROM_NONCE="$2"
      shift 2
      ;;
    --to-nonce)
      require_value "$1" "${2:-}"
      TO_NONCE="$2"
      shift 2
      ;;
    --rpc-url)
      require_value "$1" "${2:-}"
      RPC_URL="$2"
      shift 2
      ;;
    --admin-rpc-url)
      require_value "$1" "${2:-}"
      ADMIN_RPC_URL="$2"
      shift 2
      ;;
    --wait-for-empty-mempool)
      WAIT_FOR_EMPTY_MEMPOOL=true
      shift
      ;;
    --wait-timeout)
      require_value "$1" "${2:-}"
      WAIT_TIMEOUT="$2"
      shift 2
      ;;
    --poll-interval)
      require_value "$1" "${2:-}"
      POLL_INTERVAL="$2"
      shift 2
      ;;
    --max-transactions)
      require_value "$1" "${2:-}"
      MAX_TRANSACTIONS="$2"
      shift 2
      ;;
    --execute)
      EXECUTE=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

[[ -n "$NAMESPACE" ]] || die "--namespace is required"
[[ -n "$MONGO_SECRET" ]] || die "--mongo-secret is required"
[[ -n "$MONGO_POD" ]] || die "--mongo-pod is required"
[[ -n "$MONGO_DATABASE" ]] || die "--mongo-database is required"
[[ -n "$SENDER" ]] || die "--sender is required"
[[ -n "$FROM_NONCE" ]] || die "--from-nonce is required"
[[ -n "$TO_NONCE" ]] || die "--to-nonce is required"
[[ -n "$RPC_URL" ]] || die "--rpc-url is required"

[[ "$NAMESPACE" =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid Kubernetes namespace"
[[ "$MONGO_SECRET" =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid MongoDB secret name"
[[ "$MONGO_URI_KEY" =~ ^[A-Za-z0-9_-]+$ ]] || die "invalid MongoDB URI secret key"
[[ "$MONGO_POD" =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid MongoDB pod name"
[[ -z "$MONGO_CONTAINER" || "$MONGO_CONTAINER" =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid MongoDB container name"
[[ "$MONGO_DATABASE" =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid MongoDB database name"
[[ "$MONGO_COLLECTION" =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid MongoDB collection name"
[[ "$SENDER" =~ ^0x[0-9a-fA-F]+$ ]] || die "--sender must be hexadecimal"
[[ "$FROM_NONCE" =~ ^0x[0-9a-fA-F]+$ ]] || die "--from-nonce must be hexadecimal"
[[ "$TO_NONCE" =~ ^0x[0-9a-fA-F]+$ ]] || die "--to-nonce must be hexadecimal"
[[ "$WAIT_TIMEOUT" =~ ^[1-9][0-9]*$ ]] || die "--wait-timeout must be a positive integer"
[[ "$POLL_INTERVAL" =~ ^[1-9][0-9]*$ ]] || die "--poll-interval must be a positive integer"
[[ "$MAX_TRANSACTIONS" =~ ^[1-9][0-9]*$ ]] || die "--max-transactions must be a positive integer"

if [[ "$WAIT_FOR_EMPTY_MEMPOOL" == true && -z "$ADMIN_RPC_URL" ]]; then
  die "--wait-for-empty-mempool requires --admin-rpc-url"
fi

for command in base64 cat curl grep jq kubectl mktemp sed tr wc; do
  command -v "$command" >/dev/null 2>&1 || die "${command} is required"
done

normalize_hex() {
  local digits="${1#0x}"
  while [[ ${#digits} -gt 1 && "${digits:0:1}" == "0" ]]; do
    digits="${digits:1}"
  done
  digits=$(printf '%s' "$digits" | tr '[:upper:]' '[:lower:]')
  printf '0x%s\n' "$digits"
}

hex_less_than() {
  local left="${1#0x}"
  local right="${2#0x}"

  while [[ ${#left} -gt 1 && "${left:0:1}" == "0" ]]; do left="${left:1}"; done
  while [[ ${#right} -gt 1 && "${right:0:1}" == "0" ]]; do right="${right:1}"; done

  if [[ ${#left} -ne ${#right} ]]; then
    [[ ${#left} -lt ${#right} ]]
  else
    [[ "$left" < "$right" ]]
  fi
}

SENDER=$(normalize_hex "$SENDER")
FROM_NONCE=$(normalize_hex "$FROM_NONCE")
TO_NONCE=$(normalize_hex "$TO_NONCE")

hex_less_than "$TO_NONCE" "$FROM_NONCE" && die "--to-nonce must be greater than or equal to --from-nonce"

rpc_call() {
  local url="$1"
  local method="$2"
  local params="$3"

  jq -cn \
    --arg method "$method" \
    --argjson params "$params" \
    '{jsonrpc:"2.0",id:1,method:$method,params:$params}' |
    curl --silent --show-error --fail \
      --header 'content-type: application/json' \
      --data-binary @- \
      "$url"
}

get_pre_confirmed_nonce() {
  local params response
  params=$(jq -cn --arg address "$SENDER" '{block_id:"pre_confirmed",contract_address:$address}')
  response=$(rpc_call "$RPC_URL" starknet_getNonce "$params")
  printf '%s' "$response" | jq -er '.result'
}

get_transaction_status() {
  local hash="$1"
  local params
  params=$(jq -cn --arg hash "$hash" '{transaction_hash:$hash}')
  rpc_call "$RPC_URL" starknet_getTransactionStatus "$params"
}

get_mempool_count() {
  local response
  response=$(rpc_call "$ADMIN_RPC_URL" madara_getMempoolTxnHashes '[{"limit":10000}]')
  printf '%s' "$response" | jq -er '.result | length'
}

mongo_eval() {
  local javascript="$1"
  local container_args=()
  if [[ -n "$MONGO_CONTAINER" ]]; then
    container_args=(-c "$MONGO_CONTAINER")
  fi

  kubectl -n "$NAMESPACE" get secret "$MONGO_SECRET" \
    -o "jsonpath={.data.${MONGO_URI_KEY}}" |
    base64 -d |
    kubectl -n "$NAMESPACE" exec -i "$MONGO_POD" "${container_args[@]}" -- \
      env MONGO_REPLAY_EVAL="$javascript" sh -lc '
        IFS= read -r mongo_uri || true
        if [ -z "$mongo_uri" ]; then
          echo "MongoDB URI secret is empty" >&2
          exit 1
        fi
        mongosh "$mongo_uri" --quiet --eval "$MONGO_REPLAY_EVAL"
      '
}

METADATA_JS=$(cat <<EOF
const collection = db.getSiblingDB("${MONGO_DATABASE}").getCollection("${MONGO_COLLECTION}");
const sender = "${SENDER}";
const firstNonce = BigInt("${FROM_NONCE}");
const lastNonce = BigInt("${TO_NONCE}");
const count = lastNonce - firstNonce + 1n;
const maxTransactions = BigInt("${MAX_TRANSACTIONS}");

function emit(kind, value) {
  print("MADARA_REPLAY_" + kind + ":" + JSON.stringify(value));
}

function chainIdHex(chainId) {
  let result = "";
  for (let index = 0; index < chainId.length; index++) {
    const code = chainId.charCodeAt(index);
    if (code > 0x7f) throw new Error("chain_id must be ASCII");
    result += code.toString(16).padStart(2, "0");
  }
  return "0x" + result;
}

if (count > maxTransactions) {
  emit("ERROR", {message: "nonce range exceeds safety limit", count: count.toString(), max: maxTransactions.toString()});
} else {
  let expectedChainId = null;
  for (let nonceValue = firstNonce; nonceValue <= lastNonce; nonceValue++) {
    const nonce = "0x" + nonceValue.toString(16);
    const documents = collection.find(
      {sender_address: sender, nonce},
      {_id: 0, tx_hash: 1, tx_type: 1, tx_version: 1, chain_id: 1, charge_fee: 1, arrived_at: 1}
    ).sort({arrived_at: 1}).toArray();

    if (documents.length === 0) {
      emit("ERROR", {nonce, message: "transaction not found in MongoDB outbox"});
      continue;
    }

    const uniqueHashes = [...new Set(documents.map(document => document.tx_hash))];
    if (uniqueHashes.length !== 1) {
      emit("ERROR", {nonce, message: "ambiguous MongoDB documents", hashes: uniqueHashes});
      continue;
    }

    const document = documents[0];
    if (document.tx_type !== "INVOKE" || document.tx_version !== "V3" || document.charge_fee !== true) {
      emit("ERROR", {
        nonce,
        message: "only fee-charging Invoke V3 transactions are supported",
        tx_type: document.tx_type,
        tx_version: document.tx_version,
        charge_fee: document.charge_fee
      });
      continue;
    }

    const documentChainId = chainIdHex(document.chain_id);
    if (expectedChainId !== null && expectedChainId !== documentChainId) {
      emit("ERROR", {nonce, message: "nonce range contains multiple chain IDs"});
      continue;
    }
    expectedChainId = documentChainId;

    emit("META", {nonce, tx_hash: document.tx_hash});
  }

  emit("SUMMARY", {count: count.toString(), chain_id: expectedChainId});
}
EOF
)

echo "Validating MongoDB outbox records..."
METADATA_OUTPUT=$(mongo_eval "$METADATA_JS" | sed -n '/^MADARA_REPLAY_/p')

if printf '%s\n' "$METADATA_OUTPUT" | sed -n '/^MADARA_REPLAY_ERROR:/p' | grep -q .; then
  printf '%s\n' "$METADATA_OUTPUT" |
    sed -n 's/^MADARA_REPLAY_ERROR:/MongoDB validation error: /p' >&2
  exit 1
fi

SUMMARY=$(printf '%s\n' "$METADATA_OUTPUT" | sed -n 's/^MADARA_REPLAY_SUMMARY://p')
[[ -n "$SUMMARY" ]] || die "MongoDB validation did not return a summary"

EXPECTED_COUNT=$(printf '%s' "$SUMMARY" | jq -er '.count')
DOCUMENT_COUNT=$(printf '%s\n' "$METADATA_OUTPUT" | sed -n '/^MADARA_REPLAY_META:/p' | wc -l | tr -d ' ')
[[ "$DOCUMENT_COUNT" == "$EXPECTED_COUNT" ]] || die "validated ${DOCUMENT_COUNT} of ${EXPECTED_COUNT} requested transactions"

MONGO_CHAIN_ID=$(printf '%s' "$SUMMARY" | jq -er '.chain_id')
RPC_CHAIN_RESPONSE=$(rpc_call "$RPC_URL" starknet_chainId '[]')
RPC_CHAIN_ID=$(printf '%s' "$RPC_CHAIN_RESPONSE" | jq -er '.result')
[[ "$(normalize_hex "$MONGO_CHAIN_ID")" == "$(normalize_hex "$RPC_CHAIN_ID")" ]] ||
  die "MongoDB chain ID ${MONGO_CHAIN_ID} does not match Madara chain ID ${RPC_CHAIN_ID}"

CURRENT_NONCE=$(normalize_hex "$(get_pre_confirmed_nonce)")
echo "Validated ${DOCUMENT_COUNT} transactions for chain ${RPC_CHAIN_ID}."
echo "Account pre-confirmed nonce: ${CURRENT_NONCE}"

REPLAY_COUNT=0
while IFS= read -r metadata; do
  nonce=$(printf '%s' "$metadata" | jq -r '.nonce')
  hash=$(printf '%s' "$metadata" | jq -r '.tx_hash')
  if hex_less_than "$nonce" "$CURRENT_NONCE"; then
    echo "SKIP ${nonce} ${hash} (account nonce already advanced)"
  else
    echo "REPLAY ${nonce} ${hash}"
    REPLAY_COUNT=$((REPLAY_COUNT + 1))
  fi
done < <(printf '%s\n' "$METADATA_OUTPUT" | sed -n 's/^MADARA_REPLAY_META://p')

if [[ "$EXECUTE" != true ]]; then
  echo "Dry run complete: ${REPLAY_COUNT} transaction(s) would be submitted. Pass --execute to replay them."
  exit 0
fi

if [[ "$REPLAY_COUNT" -eq 0 ]]; then
  echo "Nothing to replay; the account nonce is already beyond the requested range."
  exit 0
fi

PAYLOAD_JS=$(cat <<EOF
const collection = db.getSiblingDB("${MONGO_DATABASE}").getCollection("${MONGO_COLLECTION}");
const sender = "${SENDER}";
const firstNonce = BigInt("${FROM_NONCE}");
const lastNonce = BigInt("${TO_NONCE}");
const currentNonce = BigInt("${CURRENT_NONCE}");

function hex(value) {
  return "0x" + BigInt(value.toString()).toString(16);
}

function emit(value) {
  print("MADARA_REPLAY_TX:" + JSON.stringify(value));
}

for (let nonceValue = firstNonce; nonceValue <= lastNonce; nonceValue++) {
  if (nonceValue < currentNonce) continue;

  const nonce = "0x" + nonceValue.toString(16);
  const documents = collection.find({sender_address: sender, nonce}).sort({arrived_at: 1}).limit(1).toArray();
  if (documents.length !== 1) throw new Error("missing transaction for nonce " + nonce);

  const document = documents[0];
  const bounds = document.resource_bounds;
  const transaction = {
    type: "INVOKE",
    version: "0x3",
    sender_address: document.sender_address,
    calldata: document.calldata,
    signature: document.signature,
    nonce: document.nonce,
    resource_bounds: {
      l1_gas: {
        max_amount: hex(bounds.l1_gas_max_amount),
        max_price_per_unit: bounds.l1_gas_max_price
      },
      l2_gas: {
        max_amount: hex(bounds.l2_gas_max_amount),
        max_price_per_unit: bounds.l2_gas_max_price
      },
      l1_data_gas: {
        max_amount: hex(bounds.l1_data_gas_max_amount ?? 0),
        max_price_per_unit: bounds.l1_data_gas_max_price ?? "0x0"
      }
    },
    tip: hex(document.tip),
    paymaster_data: [],
    account_deployment_data: [],
    nonce_data_availability_mode: "L1",
    fee_data_availability_mode: "L1"
  };

  emit({
    nonce,
    expected_hash: document.tx_hash,
    request: {
      jsonrpc: "2.0",
      id: Number(nonceValue - firstNonce + 1n),
      method: "starknet_addInvokeTransaction",
      params: {invoke_transaction: transaction}
    }
  });
}
EOF
)

PAYLOAD_FILE=$(mktemp "${TMPDIR:-/tmp}/madara-mempool-replay.XXXXXX")
chmod 600 "$PAYLOAD_FILE"
cleanup() {
  rm -f "$PAYLOAD_FILE"
}
trap cleanup EXIT

mongo_eval "$PAYLOAD_JS" | sed -n 's/^MADARA_REPLAY_TX://p' > "$PAYLOAD_FILE"
PAYLOAD_COUNT=$(wc -l < "$PAYLOAD_FILE" | tr -d ' ')
[[ "$PAYLOAD_COUNT" -eq "$REPLAY_COUNT" ]] || die "generated ${PAYLOAD_COUNT} of ${REPLAY_COUNT} replay payloads"

REPLAYED_NONCES=()
REPLAYED_HASHES=()

while IFS= read -r row; do
  nonce=$(printf '%s' "$row" | jq -er '.nonce')
  expected_hash=$(printf '%s' "$row" | jq -er '.expected_hash')
  echo "Submitting ${nonce} (${expected_hash})..."

  response=$(printf '%s' "$row" | jq -c '.request' |
    curl --silent --show-error --fail \
      --header 'content-type: application/json' \
      --data-binary @- \
      "$RPC_URL")
  returned_hash=$(printf '%s' "$response" | jq -r '.result.transaction_hash // empty')

  if [[ "$returned_hash" != "$expected_hash" ]]; then
    existing_status=$(get_transaction_status "$expected_hash")
    existing_finality=$(printf '%s' "$existing_status" | jq -r '.result.finality_status // empty')
    if [[ -n "$existing_finality" ]]; then
      echo "Already known: ${nonce} ${expected_hash} (${existing_finality})"
    else
      error=$(printf '%s' "$response" | jq -c '.error // .')
      die "submission failed for nonce ${nonce}; expected hash ${expected_hash}, response: ${error}"
    fi
  else
    echo "Accepted ${nonce} ${returned_hash}"
  fi

  REPLAYED_NONCES+=("$nonce")
  REPLAYED_HASHES+=("$expected_hash")
done < "$PAYLOAD_FILE"

echo "Waiting for the account nonce to advance beyond ${TO_NONCE}..."
deadline=$((SECONDS + WAIT_TIMEOUT))
while true; do
  CURRENT_NONCE=$(normalize_hex "$(get_pre_confirmed_nonce)")
  if hex_less_than "$TO_NONCE" "$CURRENT_NONCE"; then
    break
  fi
  if [[ $SECONDS -ge $deadline ]]; then
    die "timed out waiting for account nonce progress; current nonce is ${CURRENT_NONCE}"
  fi
  sleep "$POLL_INTERVAL"
done

FAILED_STATUSES=0
for index in "${!REPLAYED_HASHES[@]}"; do
  status=$(get_transaction_status "${REPLAYED_HASHES[$index]}")
  finality=$(printf '%s' "$status" | jq -r '.result.finality_status // "UNKNOWN"')
  execution=$(printf '%s' "$status" | jq -r '.result.execution_status // "UNKNOWN"')
  echo "STATUS ${REPLAYED_NONCES[$index]} ${finality} ${execution}"
  if [[ "$execution" != "SUCCEEDED" ]]; then
    FAILED_STATUSES=$((FAILED_STATUSES + 1))
  fi
done

[[ "$FAILED_STATUSES" -eq 0 ]] || die "${FAILED_STATUSES} replayed transaction(s) did not succeed"

if [[ "$WAIT_FOR_EMPTY_MEMPOOL" == true ]]; then
  echo "Waiting for the global mempool to become empty..."
  deadline=$((SECONDS + WAIT_TIMEOUT))
  while true; do
    MEMPOOL_COUNT=$(get_mempool_count)
    echo "Mempool transactions: ${MEMPOOL_COUNT}"
    if [[ "$MEMPOOL_COUNT" -eq 0 ]]; then
      break
    fi
    if [[ $SECONDS -ge $deadline ]]; then
      die "timed out waiting for the mempool to become empty (${MEMPOOL_COUNT} remain; count is capped at 10000)"
    fi
    sleep "$POLL_INTERVAL"
  done
fi

echo "Replay complete: ${#REPLAYED_HASHES[@]} transaction(s) succeeded; account nonce is ${CURRENT_NONCE}."
