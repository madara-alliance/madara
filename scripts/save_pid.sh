#!/bin/bash

PID_FILE="$(pwd)/.pids.json"

if [ ! -f "$PID_FILE" ]; then
  echo '{}' > "$PID_FILE"
fi

jq --arg key "$1" --arg val "$2" '.[$key] = $val' "$PID_FILE" > "${PID_FILE}.tmp" && \
mv "${PID_FILE}.tmp" "$PID_FILE"