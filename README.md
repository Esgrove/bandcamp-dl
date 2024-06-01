# Bandcamp DL

Rust CLI tool for downloading all Bandcamp purchases automatically (or any other JSON array of URLs).

# Usage

Get all Bandcamp download links from the purchases page with Chrome console:

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
