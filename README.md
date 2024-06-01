# Bandcamp DL

Rust CLI tool for downloading all Bandcamp purchases automatically (or any other JSON array of URLs).
Downloads files concurrently, unzips any zip files to the download directory and removes all cover images.

# Usage

```console
CLI tool for downloading a list of URLS

Usage: bcdl [OPTIONS] <URLS>

Arguments:
  <URLS>  JSON string containing an array of URLs

Options:
  -f, --force          Overwrite existing files
  -o, --output <PATH>  Optional output directory
  -v, --verbose        Verbose output
  -h, --help           Print help
  -V, --version        Print version
```

## Download and unzip Bandcamp purchases

First get all Bandcamp download links from the purchase download page with Chrome DevTools.
Open Chrome DevTools console and run this to get all the links from the page:

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
