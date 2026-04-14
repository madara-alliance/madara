cleanup_artifacts() {
  rm -rf "./build-artifacts/argent"
  rm -rf "./build-artifacts/braavos"
  rm -rf "./build-artifacts/cairo_lang"
  rm -rf "./build-artifacts/js_tests"
  rm -rf "./build-artifacts/orchestrator_tests"
  rm -rf "./build-artifacts/starkgate_latest"
  rm -rf "./build-artifacts/starkgate_legacy"
  rm -rf "./build-artifacts/bootstrapper"
}

export_artifacts() {
  docker build --platform=linux/amd64 -f ./build-artifacts/build.docker -t contracts .
  ID=$(docker create contracts do-nothing) || return 1
  docker cp "$ID:/artifacts/." ./build-artifacts || {
    docker rm "$ID" >/dev/null 2>&1 || true
    return 1
  }
  docker rm "$ID" >/dev/null
}

if [ -d "./build-artifacts/argent" ] ||
  [ -d "./build-artifacts/braavos" ] ||
  [ -d "./build-artifacts/cairo_lang" ] ||
  [ -d "./build-artifacts/js_tests" ] ||
  [ -d "./build-artifacts/orchestrator_tests" ] ||
  [ -d "./build-artifacts/starkgate_latest" ] ||
  [ -d "./build-artifacts/starkgate_legacy" ] ||
  [ -d "./build-artifacts/bootstrapper" ]; then
  echo -e "\033[2;3;37martifacts already exists, do you want to remove it?\033[0m \033[1;32m[y/N] \033[0m"
  read -r ans
  case "$ans" in
  [yY]*)
    cleanup_artifacts &&
      export_artifacts
    ;;
  *)
    exit 0
    ;;
  esac
else
  cleanup_artifacts &&
    export_artifacts
fi
