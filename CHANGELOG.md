## [0.0.x] - initial development phase

### [0.0.6] ("The Traversal Engine") - 2026-05-02 
+ ***Full architecture overhaul***
* **Redesigned core workflow** around a single `traverse` function that
  reads, classifies, and optionally copies in one pass:
  + Stride-by-stride traversal with `read_stride` as the atomic unit
  + Good strides written immediately; Ugly (uniform 4-byte pattern)
    strides deferred via `PatternRun` and flushed in bulk
  + Bad strides filled with `0xDEAFBEEB` placeholder for later recovery
  + Single function serves Inspect, Copy, and Compare modes via
    `Option<&mut File>` for the destination handle
* **Rolling Adler-32 checksum** implemented via `RollingChecksum` trait:
  + Per-segment checksum for Good segments (stored as `essence`)
  + Global checksum for whole-source fast comparison
  + Trait-based design allows swapping checksum algorithms later
* **Improved segment model:**
  + `Status::Ugly` now detects 4-byte repeating patterns (not 1-byte)
  + `Layout::push` merges adjacent Good and Bad segments automatically;
    Ugly segments merge only on matching pattern
  + `truncation_point` correctly keeps one stride as intentional hint
  + CSV output format standardized with 0x-prefixed hex, header lines,
    and human-readable size column
* **CLI changes:**
  + Removed `--limit-size` parameter (heuristic EOF is sufficient)
  + Added flag conflict detection at parse time
* **New abstractions:**
  + `RollingChecksum` trait with `Adler32` implementation
  + `PatternRun` struct for deferred uniform-stride writes
  + `format_byte_count` and `format_duration` helpers for readability
  + `open_for_write` on `Location` (drive vs image semantics)
* **Output improvements:**
  + `run()` now reports total scanned, truncation point, skipped bytes,
    trailing pattern, and effective image size
  + CSV output extracted to free function `print_layout_csv`
  + Progress reporting and errors on STDERR; layout CSV on STDOUT
* **Removed:**
  + `Layout::scan` method (replaced by `traverse`)
  + `Segment.last` field (unused)
  + 1-byte Ugly detection
  + `--limit-size` CLI parameter

### [0.0.5] - 2026-04-26
+ **Deliberate redesign** of the architecture with a cleaner logic:
  + *Pass 0 (SCAN)* - read-only chunk-by-chunk analysis of the `--source`:
    + Forms `Layout` as vector of contuous `Status` chunks - `Segment`: 
      + Status can be: `Good` (readable), `Bad` (error read), `Ugly` (same-byte)
    + Only prints source condition layout to STDOUT with `--inspection`
    + Enables informed desicions before any copy/recovery (write) implemented
  + *Truncation decision*:
    + If `--truncate` -> analysing map for trailing `Ugly` chunks
      + Calculate effective stop point = position after last non-`Ugly` chunk
      + Preserve first `Ugly` chunk as an indicator (visible in destination)
    + Else (default non-`--truncate`) effective stop point = source total size
  - ==NOT IMPLEMENTED YET== *Pass 1 (COPY)*
  - ==NOT IMPLEMENTED YET== *Pass 2 (RECOVER)*
* **CLI rework**:
  * Removed `--buffer-size` param (to avoid convoluted config)
  * Added `--limit-size` param to hint physical drive capacity (default 32 GiB)
  * Renamed `--smart` into `--truncate`
  * Added `--inspect` mode (does not require / ignores `--destination`)
  * Reworked `--sector-size` logic:
    * If present - enables tool recovery mode (==NOT IMPLEMENTED YET==)
    * If absent (default) - only Pass 0 (and Pass 1)
  * Solidified reasoning on the rest of flags and parameters:
    * `--truncate` solely controls truncation decision (effective stop point)
    * `--chunk-size` has a default value (32 MiB) and acts as a key parameter:
      * logical granularity of the imaging process (also imacts performance)
      * pattern length for the consequetive same-byte `Ugly` blocks 
+ **Additional improvements**:
  + Upgraded error-handling for `Location` struct
  + "Divide-and-conquer" output stream strategy:
    + STDOUT: `Config::display()` header + CSV-lines log of read errors
    + STDERR: execution progress report, program warnings and errors
    + This would allow to split and redirect info the way user prefers it

### [0.0.4] - 2026-04-07
* Rework of the core workflow function (`run()`)
+ Struct do describe regions where reading failed
+ Function signatures for three-pass imaging process at high-level:
  + `copy_optimistic(..)` - buffer-sized pass (only signature)
  + `copy_realistic(..)` - chunk-sized pass (only signature)
  + `copy_forensic(..)` - sector-sized pass (only signature)
* Fixed string with usage pattern in program welcome (2026-04-06 commit)
#### 2026-04-05
+ Info display enhancements and some logic simplification:
  + `print_help()` moved into `display()` implementation for `HelpLevel`
  + `print_cli()` moved into `display()` implementation for `Config`
  + `calc_sizes()` helper function
* Removed `--force` flag entirely (better not to have option to screw youself)
* Rework of required arguments for versatility (potential bidirectional):
  * `--physical-drive` & `--output-path` => `--source` & `--destination`
  * `Location` enum (`Drive(String)` and `Image(PathBuf)`) with implementaion:
    * `from_str(..)`, `display()`, `open_for_read()`, `open_for_write()`

### [0.0.3] - 2026-04-04
* Field-test for a micro-SD card on Windows platform:
  * `.\target\debug\rusdd.exe --physical-drive \\.\PHYSICALDRIVE3 --output-path output-test.dd`
  * Failed with user-level access == as expected
  * Success with admin-level access
  * Verified result correctness via brief inspection in [HxD editor](https://mh-nexus.de/en/hxd/)
+ Core *input/output* (I/O) skeleton == simplistic version without loop
  + Read a single buffer from a physical drive
  + Write the buffer to a destination filepath

### [0.0.2] - 2026-04-03
+ Printing function with respective HelpLvel enum: Usage, Extended  
+ String constants for respective CLI parts: `ARGUMENTS`, `FLAGS`, `PARAMS`

### [0.0.1] - 2026-04-02
+ Validation and tests for power-of-two size optional parameters
+ *Comand line interface* (CLI) skeleton
  + Required arguments: `--physical-drive`, `--output-path`
  + Logical flags: `--force`, `--smart`
  + Optional parameters: `--sector-size`, `--chunk-size`, `--buffer-size`
+ Description of the tool core intention
