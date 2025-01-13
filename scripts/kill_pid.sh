#!/bin/bash

get_pid() {
  pgrep "$1"
}

pid=$(get_pid "$1")

if [ -n "$pid" ]; then
  kill "$pid" 2>/dev/null || true
  echo "Process $1 (PID: $pid) terminated."
fi
