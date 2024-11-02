#!/bin/bash

PID_FILE="$(pwd)/.pids.json"

jq -r --arg key "$1" '.[$key] // empty' "$PID_FILE"
