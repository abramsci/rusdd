[![License](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
![Rust](https://img.shields.io/badge/Rust-2024%20Edition-orange?logo=rust)
[![Crates.io Version](https://img.shields.io/crates/v/rusdd)](https://crates.io/crates/rusdd)

# rusdd - "Really Useful" Secure Digital (SD-card) Duplicator

A simple tool for digitizing physical media into back-up images bit-by-bit.
~~Can be used vice-versa to imprint back-up image file onto fresh SD-card.~~
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

## Download and try if you feel edgy (pre-build for Windows 10/11 x86_64)
[0.1.0](https://github.com/abramsci/rusdd/releases/tag/v0.1.0) - core functionality: imaging via traversal engine and layout inspection (chunk-by-chunk)

## Install via cargo (crates.io)
Just install and try (requires crust toolchain installed)
```
cargo install rusdd
```

## Build from source (this GitHub repo)
To get the code locally and play around with it
```
git clone https://github.com/abramsci/rusdd
cd rusdd
cargo build --release
```

## Platform-specific notes

### Windows
- Requires **admin privileges** to access physical drives.
* Use `\\.\PHYSICALDRIVEX` syntax. Find the drive number via Disk Management.
+ Example: `target\release\rusdd.exe --truncate --source \\.\PHYSICALDRIVE3 --destination D:\backup.dd --chunk-size 32K`

### Linux
- Requires **root** or `disk` group membership for raw device access.
* Use `/dev/sdX` syntax. Find the device (or partition) via `lsblk` command.
+ Example: `sudo target/release/rusdd -s /dev/sdc -d backup.dd`

### macOS (untested)
- Use something like `/dev/rdiskN` (raw device)?
- Example: `sudo target/release/rusdd -s /dev/rdisk2 -d backup.dd`

## Output
`rusdd` prints a CSV segment map to STDOUT and progress information to STDERR.
Redirect the things you want to capture accordingly:
`rusdd -s /dev/sd -d backup.dd 2>progress.log 1>layout.csv`



## How it works
See [DESIGN.md](doc/DESIGN.md) for architecture decisions and principles.
Do not trust anything especially if it asks for root privileges.
The tool actually can be used without these to simply copy or inspect files.
In any case - check the code and stay cyber-safe!

*P.S. My first "learning via practical necessity" project in Rust.*
