[![License](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
![Rust](https://img.shields.io/badge/Rust-2024%20Edition-orange?logo=rust)

# rusdd - Really Useful Secure Digital Duplicator

A simple tool for digitizing physical media into back-up images bit-by-bit.
Can be used vice-versa to imprint back-up image file onto fresh SD-card.
Or alternatively as direct drive-to-drive or file-to-file copy/comparison.
These use cases are basically abstracted as 'SOURCE -> DESTINATION'.
Core workflow performs location inspection to build segmentation layout.
Designed with the balance of simplicity and usability in mind: 
+ comand line interface (CLI) with minimal bloat and maximum flexability
+ redirectable STDOUT with core info only (CSV-like segmentaion layout)
+ human-readable execution progress in STDERR without output pollution
+ option to truncate empty (repeated 4-byte pattern) trailing space
- ~~option to attempt sector-by-sector recovery for errored chunks~~
- ~~option to compare destination vs source layout and content~~

P.S. My first "learning via practical necessity" project in Rust.
