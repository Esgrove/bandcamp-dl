#!/bin/bash
set -eo pipefail

# Import common functions
DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=./common.sh
source "$DIR/common.sh"

USAGE="Usage: $0 [OPTIONS]

Build the Rust CLI tool.

OPTIONS: All options are optional
    --help
        Display these instructions.

    --verbose
        Display commands being executed."

while [ $# -gt 0 ]; do
    case "$1" in
        --help)
            echo "$USAGE"
            exit 1
            ;;
        --verbose)
            set -x
            ;;
    esac
    shift
done

if [ -z "$(command -v cargo)" ]; then
    print_error_and_exit "Cargo not found in path. Maybe install rustup?"
fi

cd "$REPO_ROOT"

cargo build --release

executable=$(get_rust_executable_name)
rm -f "$executable"
mv ./target/release/"$executable" "$executable"
./"$executable" --version
./"$executable" -h || :
