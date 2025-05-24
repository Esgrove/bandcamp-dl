# Bandcamp DL

Rust CLI tool for downloading all Bandcamp purchases automatically (or any other JSON array of URLs).
Downloads files concurrently, unzips any zip files to the download directory and removes all cover images.

## Build

Using the provided scripts:

```shell
./build.sh

./install.sh
```

## Usage

```console
CLI tool for downloading a list of URLS

Usage: bcdl [OPTIONS] <URLS>

Arguments:
  <URLS>  A single URL or JSON string array of URLs

Options:
  -f, --force          Overwrite existing files
  -o, --output <PATH>  Optional output directory
  -v, --verbose        Verbose output
  -h, --help           Print help
  -V, --version        Print version
```

## Download and unzip Bandcamp purchases

First get all Bandcamp download links from the purchase download page with a browser developer console.
Run this to get all the links from the page:

```javascript
var links = Array.from(document.querySelectorAll('a'))
    .filter(link => link.href.startsWith('https://p4.bcbits.com'))
    .map(link => link.href);
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

Usage: bcdl-zip [OPTIONS] [INPUT]

Arguments:
  [INPUT]  Optional input path

Options:
  -f, --force    Overwrite existing files
  -v, --verbose  Verbose output
  -h, --help     Print help
  -V, --version  Print version
```

## TODO

- Unzip each downloaded zip immediately without waiting for all downloads to finish first
- More robust file count calculation method
