# Bandcamp DL

Rust CLI tool for downloading all Bandcamp purchases automatically (or any other JSON array of URLs).
Downloads files concurrently, unzips any zip files to the download directory and removes all cover images.
Each file is written to a temporary `.part` file and renamed when complete, so an interrupted download
never leaves a partial file behind. Connection errors, stalled transfers, and HTTP 429/5xx responses are
retried twice. The exit status is nonzero if any download failed.

## Build

Using the provided scripts, which handle both the Rust CLI and the Firefox extension by default:

```shell
./build.sh
./install.sh
```

Add `--cli-only` or `--web-only` to either script to handle only one part.
Building the extension needs [Bun](https://bun.sh/); `--cli-only` only needs Cargo.

`./install.sh` signs the extension with Mozilla as an *unlisted* add-on (never published or listed)
so that regular Firefox can install it permanently, then opens the signed XPI in Firefox.
Signing needs AMO API credentials in the environment:

1. Generate them at <https://addons.mozilla.org/developers/addon/api/key/> (a free Mozilla account is needed).
2. Export them before running the script:

   ```shell
   export MOZILLA_ADDON_API_USER="user:12345:67"   # JWT issuer
   export MOZILLA_ADDON_API_TOKEN="..."            # JWT secret
   ./install.sh
   ```

The signed version is `<Cargo version>.<build number>`, so every rebuild with changed extension sources
gets a new version, as Mozilla requires. Unchanged sources reuse the previous signed XPI
(tracked in `web-ext-artifacts/signed.json`). If you have no credentials,
`./install.sh --dev` starts Firefox with the extension loaded temporarily.
The script also copies the pairing JSON to the clipboard and prints the remaining manual steps.

## Usage

Pass a URL/JSON array as before, or use `bcdl --browser` with the Firefox extension.

```console
CLI tool for downloading a list of URLS

Usage: bcdl [OPTIONS] [URLS]

Arguments:
  <URLS>  A single URL or JSON string array of URLs

Options:
  -f, --force          Overwrite existing files
  -o, --output <PATH>  Optional output directory
  -v, --verbose        Verbose output
  -h, --help           Print help
  -V, --version        Print version
```

Run `bcdl --help` for browser setup, timeout, configuration-file, and tab-selection options.

## Download directly from Firefox

The CLI starts the download and owns the output location, progress bars, and cancellation.
The extension only supplies links from an already logged-in Bandcamp download tab.
No browser cookies are exported and no always-running local service is required.

### One-time setup on each computer

1. Run `./install.sh` (see [Build](#build)), or install the CLI with `cargo install --path .`
   (works from PowerShell on Windows too) and the extension manually as described below.
2. To install the extension manually from `extension/` for development, open
   `about:debugging#/runtime/this-firefox`, choose **Load Temporary Add-on**, and select
   `extension/manifest.json`. Temporary installations disappear when Firefox restarts.
   Regular Firefox requires a **Mozilla-signed XPI** for permanent installation, including
   private use. This repository contains unsigned source, not a signed release.
   [Unlisted signing](https://extensionworkshop.com/documentation/publish/signing-and-distribution-overview/)
   allows private distribution without a public listing. Developer Edition/ESR can instead
   use unsigned extensions with the appropriate signature-check configuration.
3. Run `bcdl --browser-setup`. In Firefox's Add-ons Manager, open this extension's
   **Preferences/Options**, paste the printed JSON, and save it.
4. Open or reload your logged-in `https://bandcamp.com/download?...` page after installation.
   Select your preferred audio format and wait for its download links to be ready.

Firefox 142 or newer is required. The extension requests Bandcamp and localhost access
(it only reads download pages), clipboard writing, and local settings storage. It declares website
content/browsing-data transfer because it sends tab titles and download links to your
local CLI; it sends no telemetry or purchase data to the developer or a cloud service.
Each Firefox profile has its own settings; pair only profiles you intend to use.

### Start downloads from the terminal

```shell
# macOS (the destination must already exist)
bcdl --browser -o "$HOME/Music"
```

```powershell
# Windows PowerShell
bcdl --browser -o "$HOME\Music"
```

The extension checks for a CLI request every two seconds while a download tab is open,
and every half second while the CLI is talking to it.
The CLI briefly gathers available tabs, asks you to choose if there is more than one,
receives fresh links, closes its listener, and runs its normal download/extraction workflow.
You can close Firefox after the links have arrived. `--force` and `--verbose` work normally;
Ctrl+C stops waiting or downloading. Existing image cleanup and ZIP removal still apply.

```shell
bcdl --browser --browser-timeout 120
# For noninteractive selection, use the number printed in the CLI's tab list:
bcdl --browser --browser-tab 2
```

The default timeout is 60 seconds for each browser step: finding tabs, then receiving links.
Time spent at the tab-selection prompt is not counted. Links are deduplicated
without changing their signed query strings. The extension does not switch formats,
generate missing links, or combine different purchase tabs.

### Pairing and troubleshooting

`--browser-setup` shows existing settings without changing the token. Treat its JSON and
your purchase/download URLs as secrets; do not put them in issues, logs, or source control.
On Unix the pairing file is created with mode `0600`; on Windows it uses your user
directory's inherited access permissions.

The pairing file is `bandcamp-dl/browser.json` under your OS configuration directory
(typically `~/Library/Application Support` on macOS and `%APPDATA%` on Windows).
Use `--browser-config PATH` with **both setup and import** to override it.

- If import times out, check pairing, reload the download tab, grant the extension's
  site permissions, select an audio format, and wait for Bandcamp to prepare its links.
  The toolbar tooltip and extension settings show connection status.
- If links expired, refresh the Bandcamp page and retry. Browser-import download errors
  hide signed URL details.
- If port `47653` is occupied, stop the other CLI instance. To use another port,
  remove the old pairing file, run `bcdl --browser-setup --browser-port 47654`,
  and save the new JSON in the extension.
- To unpair, use **Remove pairing** in extension settings and delete the pairing file
  whose path is printed by `--browser-setup`. A new setup generates a new token.
- Multiple tabs are selected in the terminal, never combined automatically. Paired
  Firefox profiles announce their tabs during a brief discovery window; keep the
  intended profile running and avoid opening additional profiles during import.

Only authenticated requests to `127.0.0.1` are accepted. The listener is temporary,
does not accept commands or output paths from browser pages, and is closed before
downloads start. It never binds to your LAN address.

### Clipboard fallback

Click the extension's toolbar icon on a Bandcamp download page to copy the links as
a JSON array, without starting a download. This works even without CLI pairing.
The tooltip/badge confirms the copied count or explains an error. Clipboard history
and other applications may be able to read copied signed URLs.

Paste the JSON inside single quotes as shown below, or pipe it through your shell:

```shell
# macOS: clipboard to existing JSON input
bcdl "$(pbpaste)" -o "$HOME/Music"
```

```powershell
# PowerShell 7.3+ uses native argument passing that preserves JSON quotes.
bcdl (Get-Clipboard -Raw) -o "$HOME\Music"
```

For Windows PowerShell 5.1, prefer `--browser`; legacy native-command quoting can
strip JSON quotes. No cross-platform shell command is copied by the extension.

### Extension development

Use Bun 1.4.5 (pinned by `packageManager` in `package.json`; CI reads it from there) for JavaScript dependencies and commands. `./build.sh --web-only` runs all of these:

```shell
bun install
bun run test
bun run --bun lint
bun run --bun build
```

The extension version must match the Cargo package version; `./build.sh --sync-version` updates it.

The last command packages unsigned source in `web-ext-artifacts/`; it does not sign
or upload anything. CI installs the committed `bun.lock` with `--frozen-lockfile`.
No Bun tooling is needed to run the installed extension.
Automated fixtures cover extraction and bridge behavior; test your actual logged-in
purchase page and selected formats on each target device before relying on it.

## Download and unzip Bandcamp purchases

Alternatively, get Bandcamp download links manually with a browser developer console.
Run this to get all the links from the page:

```javascript
var links = Array.from(document.querySelectorAll("a"))
  .filter((link) => link.href.startsWith("https://p4.bcbits.com"))
  .map((link) => link.href);
console.log(links);
```

Then right-click -> _Copy object_ -> paste to terminal inside single quotes:

```shell
bcdl '[
    "https://p4.bcbits.com/...",
    "https://p4.bcbits.com/...",
    "https://p4.bcbits.com/...",
    "https://p4.bcbits.com/...",
    "https://p4.bcbits.com/...",
    "https://p4.bcbits.com/..."
]'
```

## Unzip utility

Separate binary for just unzipping all files under a given dir or current working dir if none given.

```console
Extract all zip files concurrently

Usage: bczip [OPTIONS] [INPUT]

Arguments:
  [INPUT]  Optional input path

Options:
  -f, --force      Overwrite existing files
  -r, --recursive  Get zip files recursively
  -v, --verbose    Verbose output
  -h, --help       Print help
  -V, --version    Print version
```

## TODO

- Unzip each downloaded zip immediately without waiting for all downloads to finish first
- More robust file count calculation method
