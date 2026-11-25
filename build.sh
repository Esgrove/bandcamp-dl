#!/bin/bash
set -eo pipefail

# Import common functions
DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=./common.sh
source "$DIR/common.sh"

USAGE="Usage: $0 [OPTIONS]

Build the Rust CLI tool and the Firefox extension.

OPTIONS: All options are optional
    -h | --help         Display these instructions.
    -c | --cli-only     Build only the Rust CLI (Bun is not needed).
    -w | --web-only     Build only the Firefox extension.
    -s | --sync-version Set the extension version to the Cargo version before building.
    -v | --verbose      Display commands being executed.
"
export USAGE

BUILD_CLI=true
BUILD_WEB=true
CLI_ONLY=false
WEB_ONLY=false
SYNC_VERSION=false
while [ $# -gt 0 ]; do
    case "$1" in
        -h | --help)
            print_usage_and_exit
            ;;
        -c | --cli-only)
            CLI_ONLY=true
            BUILD_WEB=false
            ;;
        -w | --web-only)
            WEB_ONLY=true
            BUILD_CLI=false
            ;;
        -s | --sync-version)
            SYNC_VERSION=true
            ;;
        -v | --verbose)
            set -x
            ;;
        *)
            print_usage_and_exit "Unknown option: $1"
            ;;
    esac
    shift
done

if [ "$CLI_ONLY" = true ] && [ "$WEB_ONLY" = true ]; then
    print_usage_and_exit "--cli-only and --web-only cannot be used together"
fi

# Check all required tools before building anything
if [ "$BUILD_CLI" = true ] && [ -z "$(command -v cargo)" ]; then
    print_error_and_exit "Cargo not found in path. Install with https://rustup.rs/"
fi
if [ "$BUILD_WEB" = true ]; then
    require_bun
fi

cd "$REPO_ROOT"

build_cli() {
    print_magenta "Building CLI..."
    cargo build --release

    executable=$(get_rust_executable_name)
    rm -f "$executable"
    mv ./target/release/"$executable" "$executable"
    ./"$executable" --version
    ./"$executable" -h || :
}

build_extension() {
    print_magenta "Building Firefox extension..."
    if [ "$SYNC_VERSION" = true ]; then
        set_extension_version "$EXTENSION_DIR/manifest.json" "$(get_cargo_version)"
    fi
    check_extension_version
    run_command bun install --frozen-lockfile
    run_command bun run test
    run_command bun run --bun lint
    run_command bun run --bun build --overwrite-dest

    local package
    package=$(find "$EXTENSION_ARTIFACTS_DIR" -maxdepth 1 -name "*-$(get_extension_version).zip" | head -n 1)
    print_green "Unsigned extension package: ${package:-$EXTENSION_ARTIFACTS_DIR}"
}

if [ "$BUILD_CLI" = true ]; then
    build_cli
fi
if [ "$BUILD_WEB" = true ]; then
    build_extension
fi
