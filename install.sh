#!/bin/bash
set -eo pipefail

# Install the Rust CLI tool to path and the Firefox extension to Firefox.

# Import common functions
DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=./common.sh
source "$DIR/common.sh"

USAGE="Usage: $0 [OPTIONS]

Install the Rust CLI tool and the Firefox extension.

The extension is signed by Mozilla as an unlisted (private, never published) add-on,
so it can be permanently installed in regular Firefox. This needs AMO API credentials
in the environment (see below), or use --dev for a temporary installation.

OPTIONS: All options are optional
    -h | --help                 Display these instructions.
    -c | --cli-only             Install only the Rust CLI (Bun is not needed).
    -w | --web-only             Install only the Firefox extension.
    -d | --dev                  Skip signing: run Firefox with a temporarily loaded extension.
    -p | --firefox-profile PATH Use this Firefox profile with --dev instead of a temporary one.
    -x | --xpi PATH             Open this already signed XPI instead of building and signing.
    -n | --no-pair              Do not copy the pairing settings to the clipboard.
    -v | --verbose              Display commands being executed.

ENVIRONMENT:
    MOZILLA_ADDON_API_USER   AMO API key (JWT issuer), e.g. user:12345:67. Required for signing.
    MOZILLA_ADDON_API_TOKEN  AMO API secret (JWT secret). Required for signing.
    EXTENSION_BUILD_NUMBER   Override the signed build number (4th version part).
"
export USAGE

INSTALL_CLI=true
INSTALL_WEB=true
CLI_ONLY=false
WEB_ONLY=false
DEV=false
FIREFOX_PROFILE=""
XPI=""
PAIR=true
while [ $# -gt 0 ]; do
    case "$1" in
        -h | --help)
            print_usage_and_exit
            ;;
        -c | --cli-only)
            CLI_ONLY=true
            INSTALL_WEB=false
            ;;
        -w | --web-only)
            WEB_ONLY=true
            INSTALL_CLI=false
            ;;
        -d | --dev)
            DEV=true
            ;;
        -p | --firefox-profile)
            [ -n "$2" ] || print_usage_and_exit "$1 requires a path"
            FIREFOX_PROFILE="$2"
            shift
            ;;
        -x | --xpi)
            [ -n "$2" ] || print_usage_and_exit "$1 requires a path"
            XPI="$2"
            shift
            ;;
        -n | --no-pair)
            PAIR=false
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
if [ "$DEV" = true ] && [ -n "$XPI" ]; then
    print_usage_and_exit "--dev and --xpi cannot be used together"
fi
if [ -n "$FIREFOX_PROFILE" ] && [ "$DEV" != true ]; then
    print_usage_and_exit "--firefox-profile requires --dev"
fi
if [ -n "$XPI" ] && [ ! -f "$XPI" ]; then
    print_error_and_exit "XPI not found: $XPI"
fi

if [ "$INSTALL_CLI" = true ] && [ -z "$(command -v cargo)" ]; then
    print_error_and_exit "Cargo not found in path. Install with https://rustup.rs/"
fi
if [ "$INSTALL_WEB" = true ]; then
    require_bun
fi

cd "$REPO_ROOT"

SIGNED_DIR="$EXTENSION_ARTIFACTS_DIR/signed"
SIGNED_STATE="$EXTENSION_ARTIFACTS_DIR/signed.json"
STAGING_DIR="$EXTENSION_ARTIFACTS_DIR/staging"

install_cli() {
    print_magenta "Installing CLI..."
    run_command cargo install --path "$REPO_ROOT"

    executable=$(get_rust_executable_name)
    if [ "$DRY_RUN" = true ] || [ "$DRYRUN" = true ]; then
        return 0
    fi
    if [ -z "$(command -v "$executable")" ]; then
        print_error_and_exit "Binary $executable not found. Is the Cargo install directory in path?"
    fi
    echo "$($executable --version) from $(which "$executable")"
}

# Exit with setup instructions unless both AMO credentials are set and non-empty.
# Only lengths are tested so --verbose tracing never prints the secret values.
require_amo_credentials() {
    local missing=()
    [ "${#MOZILLA_ADDON_API_USER}" -gt 0 ] || missing+=(MOZILLA_ADDON_API_USER)
    [ "${#MOZILLA_ADDON_API_TOKEN}" -gt 0 ] || missing+=(MOZILLA_ADDON_API_TOKEN)
    if [ ${#missing[@]} -eq 0 ]; then
        return 0
    fi
    print_red "ERROR: Missing or empty environment variables for signing: ${missing[*]}"
    echo "Regular Firefox only installs extensions signed by Mozilla, even for private use.
The add-on is signed as unlisted: it is never published or listed.
  1. Generate API credentials at https://addons.mozilla.org/developers/addon/api/key/
  2. export MOZILLA_ADDON_API_USER=<JWT issuer, e.g. user:12345:67>
     export MOZILLA_ADDON_API_TOKEN=<JWT secret>

Or run '$0 --dev' to load the extension temporarily without signing."
    exit 1
}

# Print the signed XPI recorded for a source hash, if it still exists
find_signed_xpi() {
    [ -f "$SIGNED_STATE" ] || return 0
    bun -e '
const fs = require("node:fs");
const [file, hash] = process.argv.slice(1);
const build = JSON.parse(fs.readFileSync(file, "utf8")).builds.find((b) => b.hash === hash);
if (build && fs.existsSync(build.xpi)) console.log(build.xpi);
' "$SIGNED_STATE" "$1"
}

# Print the next unused build number for a Cargo version
next_build_number() {
    if [ -n "$EXTENSION_BUILD_NUMBER" ]; then
        echo "$EXTENSION_BUILD_NUMBER"
        return 0
    fi
    if [ ! -f "$SIGNED_STATE" ]; then
        echo 1
        return 0
    fi
    # shellcheck disable=SC2016 # JavaScript template literal, not shell expansion
    bun -e '
const fs = require("node:fs");
const [file, base] = process.argv.slice(1);
const numbers = JSON.parse(fs.readFileSync(file, "utf8")).builds
    .filter((b) => b.version.startsWith(`${base}.`))
    .map((b) => Number(b.version.slice(base.length + 1)));
console.log(Math.max(0, ...numbers) + 1);
' "$SIGNED_STATE" "$1"
}

record_signed_build() {
    bun -e '
const fs = require("node:fs");
const [file, hash, version, xpi] = process.argv.slice(1);
const state = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : { builds: [] };
state.builds.push({ hash, version, xpi, signed: new Date().toISOString() });
fs.writeFileSync(file, JSON.stringify(state, null, 2) + "\n");
' "$SIGNED_STATE" "$1" "$2" "$3"
}

# Sign the extension as an unlisted add-on, reusing an earlier signature for unchanged sources.
# Sets SIGNED_XPI.
sign_extension() {
    local hash cargo_version version
    hash=$(get_extension_source_hash)
    SIGNED_XPI=$(find_signed_xpi "$hash")
    if [ -n "$SIGNED_XPI" ]; then
        print_green "Extension sources unchanged; reusing signed $SIGNED_XPI"
        return 0
    fi

    cargo_version=$(get_cargo_version)
    version="$cargo_version.$(next_build_number "$cargo_version")"
    print_magenta "Signing extension version $version (unlisted)..."
    rm -rf "$STAGING_DIR"
    mkdir -p "$SIGNED_DIR"
    cp -R "$EXTENSION_DIR" "$STAGING_DIR"
    set_extension_version "$STAGING_DIR/manifest.json" "$version"

    # web-ext reads its credentials from these variables; export them without tracing the values
    local tracing=false
    case "$-" in *x*) tracing=true ;; esac
    { set +x; } 2> /dev/null
    export WEB_EXT_API_KEY="$MOZILLA_ADDON_API_USER"
    export WEB_EXT_API_SECRET="$MOZILLA_ADDON_API_TOKEN"
    if [ "$tracing" = true ]; then
        set -x
    fi

    if ! run_command bunx --bun web-ext sign --channel unlisted \
        --source-dir "$STAGING_DIR" --artifacts-dir "$SIGNED_DIR"; then
        print_red "Signing failed."
        echo "If AMO reports that version $version already exists (for example after deleting"
        echo "$SIGNED_STATE), retry with a higher number: EXTENSION_BUILD_NUMBER=<n> $0"
        exit 1
    fi
    if [ "$DRY_RUN" = true ] || [ "$DRYRUN" = true ]; then
        SIGNED_XPI="$SIGNED_DIR/<signed>.xpi"
        return 0
    fi

    SIGNED_XPI=$(find "$SIGNED_DIR" -maxdepth 1 -name "*-$version.xpi" | head -n 1)
    if [ -z "$SIGNED_XPI" ]; then
        print_error_and_exit "Signing succeeded but no XPI for $version was found in $SIGNED_DIR"
    fi
    record_signed_build "$hash" "$version" "$SIGNED_XPI"
    rm -rf "$STAGING_DIR"
    print_green "Signed extension: $SIGNED_XPI"
}

# Copy the pairing JSON (port and token) to the clipboard for the extension settings
copy_pairing() {
    local executable
    executable=$(get_rust_executable_name)
    if [ -z "$(command -v "$executable")" ]; then
        print_yellow "$executable not found in path; run '$executable --browser-setup' after installing the CLI."
        return 0
    fi
    if [ "$DRY_RUN" = true ] || [ "$DRYRUN" = true ]; then
        echo "DRYRUN: $executable --browser-setup | <clipboard>"
        PAIRING_COPIED=true
        return 0
    fi
    local pairing
    pairing=$("$executable" --browser-setup 2> /dev/null)
    if printf '%s' "$pairing" | copy_to_clipboard; then
        PAIRING_COPIED=true
        print_green "Copied pairing settings to the clipboard (keep them private)."
    else
        print_yellow "No clipboard tool found; run '$executable --browser-setup' and copy its JSON."
    fi
}

print_checklist() {
    local step=1
    echo
    print_magenta "Remaining manual steps:"
    if [ "$DEV" != true ]; then
        echo "  $step. Confirm the add-on installation prompt in Firefox."
        step=$((step + 1))
    fi
    echo "  $step. Open about:addons, select 'Bandcamp DL Link Provider' > Preferences (or '...' > Options)."
    step=$((step + 1))
    if [ "$PAIRING_COPIED" = true ]; then
        echo "  $step. Paste the pairing JSON from the clipboard and click 'Save pairing'."
    else
        echo "  $step. Paste the output of '$(get_rust_executable_name) --browser-setup' and click 'Save pairing'."
    fi
    step=$((step + 1))
    echo "  $step. Reload your Bandcamp download page, then run: $(get_rust_executable_name) --browser -o <output dir>"
}

install_extension() {
    if [ -n "$XPI" ]; then
        open_in_firefox "$XPI"
    elif [ "$DEV" = true ]; then
        "$REPO_ROOT/build.sh" --web-only
    else
        require_amo_credentials
        "$REPO_ROOT/build.sh" --web-only
        sign_extension
        open_in_firefox "$SIGNED_XPI"
    fi

    if [ "$PAIR" = true ]; then
        copy_pairing
    fi
    print_checklist

    if [ "$DEV" = true ]; then
        echo
        print_yellow "Starting Firefox with the extension loaded temporarily. It is removed when Firefox closes."
        local args=(--source-dir "$EXTENSION_DIR")
        if [ -n "$FIREFOX_PROFILE" ]; then
            args+=(--firefox-profile "$FIREFOX_PROFILE" --keep-profile-changes)
        fi
        run_command bunx --bun web-ext run "${args[@]}"
    fi
}

PAIRING_COPIED=false
if [ "$INSTALL_CLI" = true ]; then
    install_cli
fi
if [ "$INSTALL_WEB" = true ]; then
    print_magenta "Installing Firefox extension..."
    install_extension
fi
