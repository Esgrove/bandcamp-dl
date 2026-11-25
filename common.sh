#!/bin/bash
set -eo pipefail

# Get absolute path to repo root
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || (cd "$(dirname "${BASH_SOURCE[0]}")" && pwd))
export REPO_ROOT

# Check platform: BSD (Mac) and GNU (Linux & Git for Windows) coreutils implementations have some differences
# for example in the available command line options.
case "$(uname -s)" in
    "Darwin")
        BASH_PLATFORM="mac"
        ;;
    "MINGW"*)
        BASH_PLATFORM="windows"
        ;;
    *)
        BASH_PLATFORM="linux"
        ;;
esac
export BASH_PLATFORM

# Print a message with green color
print_green() {
    printf "\e[1;49;32m%s\e[0m\n" "$1"
}

# Print a message with magenta color
print_magenta() {
    printf "\e[1;49;35m%s\e[0m\n" "$1"
}

# Print a message with red color
print_red() {
    printf "\e[1;49;31m%s\e[0m\n" "$1"
}

# Print a message with yellow color
print_yellow() {
    printf "\e[1;49;33m%s\e[0m\n" "$1"
}

# Print an error and exit the program
print_error_and_exit() {
    print_red "ERROR: $1"
    # use exit code if given as second argument, otherwise default to 1
    exit "${2:-1}"
}

# Print usage and exit the program. An optional error message can be given as well.
print_usage_and_exit() {
    if [ $# -eq 1 ]; then
        print_red "ERROR: $1"
    fi
    if [ -z "$USAGE" ]; then
        print_red "No usage text provided in variable USAGE"
    else
        echo "$USAGE"
    fi
    # use exit code if given as second argument, otherwise default to 1
    exit "${2:-1}"
}

# Get the Rust executable name from Cargo.toml
get_rust_executable_name() {
    local executable
    executable=$(awk -F'=' '/\[\[bin\]\]/,/name/ {if($1 ~ /name/) print $2}' Cargo.toml | tr -d ' "' | head -n 1)
    # If no name under [[bin]], get the package name
    if [ -z "$executable" ]; then
        executable=$(awk -F'=' '/\[package\]/,/name/ {if($1 ~ /name/) print $2}' Cargo.toml | tr -d ' "' | head -n 1)
    fi
    if [ "$BASH_PLATFORM" = windows ]; then
        executable="${executable}.exe"
    fi
    echo "$executable"
}

# if DRYRUN or DRY_RUN has been set, only print commands instead of running them
run_command() {
    if [ "$DRY_RUN" = true ] || [ "$DRYRUN" = true ]; then
        echo "DRYRUN: $*"
    else
        echo "Running: $*"
        "$@"
    fi
}

# --- Firefox extension helpers ---

EXTENSION_DIR="$REPO_ROOT/extension"
EXTENSION_ARTIFACTS_DIR="$REPO_ROOT/web-ext-artifacts"
export EXTENSION_DIR EXTENSION_ARTIFACTS_DIR

# Exit with an install hint if Bun is not available
require_bun() {
    if [ -z "$(command -v bun)" ]; then
        print_error_and_exit "Bun not found in path. Install from https://bun.sh/ or use --cli-only to skip the Firefox extension."
    fi
}

# Get the package version from Cargo.toml
get_cargo_version() {
    awk -F'"' '/^version = / {print $2; exit}' "$REPO_ROOT/Cargo.toml"
}

# Get the version from an extension manifest (default: extension/manifest.json)
get_extension_version() {
    bun -e 'console.log(require(process.argv[1]).version)' "${1:-$EXTENSION_DIR/manifest.json}"
}

# Set the version in an extension manifest: set_extension_version <manifest> <version>
set_extension_version() {
    # shellcheck disable=SC2016 # JavaScript template literal, not shell expansion
    bun -e '
const fs = require("node:fs");
const [file, version] = process.argv.slice(1);
const text = fs.readFileSync(file, "utf8");
if (JSON.parse(text).version === undefined) throw new Error(`No version in ${file}`);
// Replace only the value to keep the manifest formatting intact.
fs.writeFileSync(file, text.replace(/("version"\s*:\s*)"[^"]*"/, `$1"${version}"`));
' "$1" "$2"
}

# Fail if the extension version does not match the Cargo package version
check_extension_version() {
    local cargo_version extension_version
    cargo_version=$(get_cargo_version)
    extension_version=$(get_extension_version)
    if [ "$cargo_version" != "$extension_version" ]; then
        print_error_and_exit "Extension version $extension_version does not match Cargo version $cargo_version. Run ./build.sh --sync-version"
    fi
}

# Print a SHA-256 hash of all extension source files (paths and contents)
get_extension_source_hash() {
    bun -e '
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const root = process.argv[1];
const files = [];
const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) walk(full);
        else files.push(full);
    }
};
walk(root);
const hash = crypto.createHash("sha256");
for (const file of files.sort()) {
    hash.update(path.relative(root, file).split(path.sep).join("/"));
    hash.update("\0");
    hash.update(fs.readFileSync(file));
    hash.update("\0");
}
console.log(hash.digest("hex"));
' "$EXTENSION_DIR"
}

# Open a file (such as a signed XPI) in Firefox
open_in_firefox() {
    local file="$1"
    case "$BASH_PLATFORM" in
        mac)
            run_command open -a Firefox "$file"
            ;;
        windows)
            run_command cmd.exe //c start "" firefox "$(cygpath -w "$file")"
            ;;
        *)
            if [ -z "$(command -v firefox)" ]; then
                print_yellow "Firefox not found in path. Open this file in Firefox manually: $file"
                return 0
            fi
            if [ "$DRY_RUN" = true ] || [ "$DRYRUN" = true ]; then
                echo "DRYRUN: firefox $file"
            else
                # A newly started Firefox would otherwise block this script
                nohup firefox "$file" > /dev/null 2>&1 &
            fi
            ;;
    esac
}

# Copy stdin to the clipboard. Returns non-zero if no clipboard tool is available.
copy_to_clipboard() {
    if [ -n "$(command -v pbcopy)" ]; then
        pbcopy
    elif [ -n "$(command -v clip.exe)" ]; then
        clip.exe
    elif [ -n "$WAYLAND_DISPLAY" ] && [ -n "$(command -v wl-copy)" ]; then
        wl-copy
    elif [ -n "$(command -v xclip)" ]; then
        xclip -selection clipboard
    else
        cat > /dev/null
        return 1
    fi
}
