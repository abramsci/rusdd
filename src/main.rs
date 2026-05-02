//! # rusdd - Really Useful Secure Digital Duplicator
//!
//! A simple tool for digitizing physical media into back-up images bit-by-bit.
//! Can be used vice-versa to imprint back-up image file onto fresh SD-card.
//! Or alternatively as direct drive-to-drive or file-to-file copy/comparison.
//! These use cases are basically abstracted as 'SOURCE -> DESTINATION'.
//! Core workflow performs location inspection to build segmentation layout.
//! Designed primarily with the balance of simplicity and usability in mind:
//! + comand line interface (CLI) with minimal bloat and maximum flexability
//! + redirectable STDOUT with core info only (CSV-like segmentaion layout)
//! + human-readable execution progress in STDERR without output pollution
//! + option to truncate empty (repeated 4-byte pattern) trailing space
//! - ~~option to attempt sector-by-sector recovery for errored chunks~~
//! - ~~option to compare destination vs source layout and content~~
//!
//! Author: Sergei Abramenkov
//! License: MIT
//! Version: 0.1.0
//! ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
const VERSION: &str = "0.1.0";

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

// -----------------------------------------------------------------------------
// Comand Line Interface (CLI): Help text, Config struct and argument parsing
// -----------------------------------------------------------------------------

const ARGUMENTS: &str = "
Major arguments (must be named - not positional):
  -s, --source <LOCATION>           What to image (drive or file) [required]
  -d, --destination <LOCATION>      Where to put (file or drive)";

const FLAGS: &str = "
Flags (boolean toggles - false by default):
  -h, --help                        Show extended help with examples
  -i, --inspect                     Only inspect and print source layout
  -t, --truncate                    Truncate trailing same-byte patterns";

const PARAMS: &str = "
Parameters (optional):
      --chunk-size <N>              Reading block size (default: 32MiB)
                                    Must be power of 2 (for efficiency)
      --sector-size <N>             Physical sector size to read in bytes
                                    Possible values: [512, 1024, 2048, 4096]
            NOT IMPLEMENTED YET!    Enables recovery mode if provided";
const UNITS: &str = "
Note: --chunk-size accepts unit suffixes (binary):
      B (bytes), K/KiB (1024 B), M/MiB (1024^2 B), G/GiB (1024^3 B)
      Examples: 512B, 256KiB, 32M, 1GiB";

/// Explicity does matter for eventual scaling
enum HelpLevel {
    Usage,
    Extended,
}
impl HelpLevel {
    fn display(&self) {
        println!(
            "rusdd - Really Useful Secure Digital Duplicator (ver.{})",
            VERSION
        );
        println!("updates on: https://github.com/abramsci/rusdd");
        println!("\nUsage: rusdd [FLAGS] -s <SOURCE> -d <DEST> [PARAMS]");
        match self {
            HelpLevel::Usage => {
                println!("{}", ARGUMENTS);
                println!("{}", FLAGS);
                println!("Try 'rusdd -h' for more information.");
            }
            HelpLevel::Extended => {
                println!("{}", ARGUMENTS);
                println!("{}", FLAGS);
                println!("{}", PARAMS);
                println!("{}", UNITS);
            }
        }
    }
}

fn cli_error(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

fn missing(arg: &str) -> io::Error {
    cli_error(&format!("Missign --{} value", arg))
}

fn invalid(name: &str, value: &str) -> io::Error {
    cli_error(&format!("Invalid {} format: {}", name, value))
}

/// Command line interface configuration
struct Config {
    source: Location,
    destination: Location,
    inspect: bool,            // Only inspect source without imaging?
    truncate: bool,           // Is trailing-empty trancation enabled?
    chunk_size: u64,          // Logical granularity (default: 32MiB)
    sector_size: Option<u16>, // None = no recovery mode
}
impl Config {
    /// Program welcome message (a header in case of STDOUT redirection)
    fn display(&self) {
        println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^");
        println!("Really Useful Secure Digital Duplicator (v.{})", VERSION);
        println!("  IMAGING FROM: {}", self.source.display());
        println!("  Chunk size: {}", format_byte_count(self.chunk_size));
        println!("  Inspection: {}", if self.inspect { "ON" } else { "OFF" });
        println!("  Truncation: {}", if self.truncate { "ON" } else { "OFF" });
        println!("  IMAGING INTO: {}", self.destination.display());
        println!("**************************************************");
    }

    fn parse_size_with_unit(name: &str, value: &str) -> io::Result<u64> {
        let input = value.trim();
        // Split number from suffix
        let num_str = input
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>();
        let suffix = &input[num_str.len()..].to_uppercase();
        let num: u64 = num_str.parse().map_err(|_| invalid(name, &num_str))?;
        // Convert to number of bytes
        let bytes = match suffix.as_str() {
            "" | "B" => num,
            "K" | "KB" | "KiB" => num * 1024,
            "M" | "MB" | "MiB" => num * 1024 * 1024,
            "G" | "GB" | "GiB" => num * 1024 * 1024 * 1024,
            _ => {
                return Err(cli_error(&format!("Unknown unit for {}: {}", name, suffix)));
            }
        };
        // Enforcement for power of 2: shrink param space -> hidden optimization
        if !bytes.is_power_of_two() {
            return Err(cli_error(&format!(
                "{} must be power of two but got {}",
                name, bytes
            )));
        }
        Ok(bytes)
    }

    fn parse_cli_from_iter<I>(mut cli: I) -> io::Result<Self>
    where
        I: Iterator<Item = String>,
    {
        let mut source = Location::Void;
        let mut destination = Location::Void;
        // Assigning defaults for flags and params
        let mut inspect = false;
        let mut truncate = false; // Full drive image (forensic frendly)
        let mut chunk: u64 = 32 * 1024 * 1024; // 32 MiB (33_554_432 bytes)
        let mut sector: Option<u16> = None; // No sector-level recovery

        while let Some(arg) = cli.next() {
            match arg.as_str() {
                "--source" | "-s" => {
                    let value = cli.next().ok_or_else(|| missing("source"))?;
                    source = Location::from_str(value);
                }
                "--destination" | "-d" => {
                    let v = cli.next().ok_or_else(|| missing("destination"))?;
                    destination = Location::from_str(v);
                }
                "--inspect" | "-i" => inspect = true,
                "--truncate" | "-t" => truncate = true,
                "--chunk-size" => {
                    let v = cli.next().ok_or_else(|| missing("chunk-size"))?;
                    chunk = Self::parse_size_with_unit("chunk size", &v)?;
                }
                "--sector-size" => {
                    let v = cli.next().ok_or_else(|| missing("sector-size"))?;
                    let size = Self::parse_size_with_unit("sector size", &v)?;
                    match size {
                        512 | 1024 | 2048 | 4096 => sector = Some(size as u16),
                        _ => return Err(invalid("sector size", &v)),
                    }
                }
                "--help" | "-h" => {
                    HelpLevel::Extended.display();
                    std::process::exit(0);
                }
                _ => return Err(cli_error(&format!("Unknown arg: {}", arg))),
            }
        }
        // Terminate with InvalidInput if source (required arg) was not provided
        if matches!(source, Location::Void) {
            return Err(cli_error("--source is required"));
        }
        // In non-inspection mode - destination also must be provided
        if !inspect && matches!(destination, Location::Void) {
            return Err(cli_error("--destination is requied unless --inspect"));
        }
        Ok(Self {
            source,
            destination,
            inspect,
            truncate,
            chunk_size: chunk,
            sector_size: sector,
        })
    }

    fn parse_cli() -> io::Result<Self> {
        if env::args().len() == 1 {
            HelpLevel::Usage.display();
            std::process::exit(0);
        } else {
            Self::parse_cli_from_iter(env::args().skip(1))
        }
    }
}

// -----------------------------------------------------------------------------
// Auxiliary: progress report and formatting
// -----------------------------------------------------------------------------

const PROGRESS_FREQUENCY: f64 = 1.0; // How often we want to print progress

/// Tracks and reports scan/copy progress to STDERR at regular time intervals.
struct ProgressTracker {
    next: Instant, // when to emit the next progress line
    interval: f64, // in seconds
    start: Instant,
    offset: u64,
}
impl ProgressTracker {
    // Make a tracker reporting every `period` seconds (ex. 2.5)
    fn new(period: f64) -> Self {
        ProgressTracker {
            next: Instant::now() + Duration::from_secs_f64(period),
            interval: period,
            start: Instant::now(),
            offset: 0,
        }
    }

    fn update(&mut self, offset: u64) {
        let now = Instant::now();
        if now >= self.next {
            let elapsed = self.start.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 {
                offset as f64 / elapsed
            } else {
                0.0
            };
            eprint!(
                "\rScanned: {} in {} ({}/s on average)",
                format_byte_count(offset),
                format_duration(elapsed),
                format_byte_count(speed as u64)
            );
            self.next = now + Duration::from_secs_f64(self.interval);
            self.offset = offset;
        }
    }

    fn finish(&self, offset: u64) {
        let elapsed = self.start.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            offset as f64 / elapsed
        } else {
            0.0
        };
        eprint!(
            "\rCompleted in {} ({} at {}/s speed)",
            format_duration(elapsed),
            format_byte_count(offset),
            format_byte_count(speed as u64)
        );
    }
}

/// Format a duration in seconds as human-readable string with suffix
fn format_duration(t: f64) -> String {
    if t >= 7200.0 {
        format!("{:.2} hours", t / (2.0 * 60.0 * 60.0))
    } else if t >= 120.0 {
        format!("{:.2} minutes", t / (2.0 * 60.0))
    } else {
        format!("{:.1} seconds", t)
    }
}

/// Format a byte count as human-readable string with suffix
fn format_byte_count(n: u64) -> String {
    if n >= 1024 * 1024 * 1024 {
        format!("{:.2} GiB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if n >= 1024 * 1024 {
        format!("{:.2} MiB", n as f64 / (1024.0 * 1024.0))
    } else if n >= 1024 {
        format!("{:.2} KiB", n as f64 / 1024.0)
    } else {
        format!("{} B", n)
    }
}

/// Printing location layout as CSV-lines
fn print_layout_csv(layout: &Layout) {
    println!(
        "# rusdd layout (at {} stide)",
        format_byte_count(layout.stride)
    );
    println!("# 0xSTART, 0xEND, 0xLEN, STATUS, COMMENT");
    for seg in layout.segments.iter() {
        let status_char = match seg.status {
            Status::Good => 'G',
            Status::Bad => 'B',
            Status::Ugly => 'U',
        };
        let comment = match seg.status {
            Status::Good => format!(
                "'readable data [{}] ({})'",
                seg.essence,
                format_byte_count(seg.length)
            ),
            Status::Bad => format!(
                "'error reading [{}] ({})'",
                seg.essence,
                format_byte_count(seg.length)
            ),
            Status::Ugly => format!(
                "'pattern: 0x{:08X} ({})'",
                seg.essence,
                format_byte_count(seg.length)
            ),
        };
        println!(
            "0x{:0X},0x{:0X},0x{:0X},0x{:08X},{},{}",
            seg.offset,
            seg.offset + seg.length,
            seg.length,
            seg.essence,
            status_char,
            comment,
        );
    }
    println!("**************************************************");
}

// -----------------------------------------------------------------------------
// Core data types and methods
// -----------------------------------------------------------------------------

/// Byte buffer status:
/// G == General (Good readable data)
/// B == Broken ( CRC-like error in reading attempt)
/// U == Uniform (repeating 4-byte pattern like 0x00000000 or 0xFFEEEEDD)
#[derive(Clone, Debug, Eq, PartialEq)]
enum Status {
    Good,
    Bad,
    Ugly,
}
impl From<&[u8]> for Status {
    /// Classifies a buffer: if all 4-byte words equal -> Ugly, else Good
    /// Never returns Bad since it comes from I/O error and not content
    fn from(buf: &[u8]) -> Self {
        // Might need to rethink and force the caller to check it?
        if buf.len() < 4 {
            return Status::Good;
        }
        let pattern = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let uniform = buf
            .chunks_exact(4)
            .all(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) == pattern);
        if uniform { Status::Ugly } else { Status::Good }
    }
}
impl Status {
    /// Produces the u32 essence value for this status, given the source buffer
    ///
    /// # Panics if `self` is `Ugly` and `buf has fewer* than 4 bytes
    /// *this should never happen because as of now `Status::from` checks it
    fn hash(&self, buf: &[u8], sum: &dyn RollingChecksum) -> u32 {
        match self {
            Status::Good => sum.value(),
            Status::Bad => 0xDEAFBEEB,
            Status::Ugly => {
                assert!(buf.len() >= 4, "uniform buf is at least 4 bytes long");
                let word = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                word
            }
        }
    }
}

/// Continuous block of memory tagged with the same status
#[derive(Clone, Debug)]
struct Segment {
    offset: u64,  // Byte offset in the source
    length: u64,  // Length in bytes
    essence: u32, // Checksum (Good), error code (Bad) or pattern (Ugly)
    status: Status,
    // 3-byte padding - room for more usable info
}

/// Ordered sequence of `Segment`s mapping `Location` with `stride` granularity
struct Layout {
    stride: u64,
    essence: u32,
    segments: Vec<Segment>,
}
impl Layout {
    fn new(stride: u64) -> Self {
        Layout {
            stride,
            essence: 0x00000000u32,
            segments: Vec::new(),
        }
    }

    /// Append a segment or merging stride with the same status previous one
    /// (for Ugly to merge the pattern must also match).
    fn push(&mut self, offset: u64, length: u64, state: Status, essence: u32) {
        if let Some(last) = self.segments.last_mut() {
            let can_merge = match (&last.status, &state) {
                (Status::Good, Status::Good) => true,
                (Status::Bad, Status::Bad) => true,
                (Status::Ugly, Status::Ugly) => last.essence == essence,
                _ => false,
            };
            if can_merge {
                last.length += length;
                last.essence = essence; // update local checksum basically
                return;
            }
        }
        self.segments.push(Segment {
            offset,
            length,
            essence,
            status: state,
        });
    }

    /// Borrow all segments currently marked as Bad (for recovery pass)
    #[allow(dead_code)]
    fn broken_segments(&self) -> Vec<&Segment> {
        self.segments
            .iter()
            .filter(|s| s.status == Status::Bad)
            .collect()
    }

    /// Compute the effective stop offset when `--truncate` is enabled.
    ///
    /// Finds the last non-Ugly segment. If Ugly segments exist after,
    /// one Ugly `stride` is preserved as hint of intentional truncation.
    fn truncation_point(&self) -> u64 {
        let n = self.segments.len();
        // Edge case of completely empty layout
        if n == 0 {
            return 0;
        }
        // Last non-Ugly segment and its position in the Layout vector
        let mut idx: Option<usize> = None;
        for (i, seg) in self.segments.iter().enumerate().rev() {
            if seg.status != Status::Ugly {
                idx = Some(i);
                break;
            }
        }
        // Decision depending on layout structure
        if idx.is_none() {
            return 0; // everything Ugly
        } else {
            let i = idx.unwrap() as usize;
            let stop = self.segments[i].offset + self.segments[i].length;
            if i + 1 < n && self.segments[i + 1].status == Status::Ugly {
                return stop + self.stride; // hint chunk
            } else {
                return stop;
            }
        }
    }
}

/// Heuristic EOF is chosen due to Windows behaivor with different media.
/// During testing USB 3.0 flash drive essentially EOFed with code 27
/// but SD-card USB 2.0 reader had reached EOF-like end with code 23.
/// Since for the time being I want to limit myself to stdlib only
/// this is a scrapy but high-XP gain way to learn and build.
/// EOF heuristics so far:
///   1. Errors at the same offset (drive stuck)
///   2. Eight consecutive errors (threshold - likely end of drive)
struct EofDetector {
    saturation: u32,
    last_code: Option<i32>,
    last_offset: u64,
}
impl EofDetector {
    const MAX_SATURATION: u32 = 8; // Bad chunks in a row means we done

    fn new() -> Self {
        EofDetector {
            saturation: 0,
            last_code: None,
            last_offset: 0,
        }
    }

    /// Record an error, then return false (not a error) if EOF is detected
    fn reading_error(&mut self, error: &io::Error, offset: u64) -> bool {
        let code = error.raw_os_error();
        // Drive stuck - same error at same offset?
        let same_offset = offset == self.last_offset;
        if self.last_code == code || same_offset {
            self.saturation += 1;
        } else {
            self.saturation = 1;
            self.last_code = code;
            self.last_offset = offset;
        }
        let is_eof = self.saturation >= Self::MAX_SATURATION;
        if is_eof {
            eprintln!("\n[HEURISTIC] EOF detected at offset {}", offset);
            eprintln!("[HEURISTIC] {} errors: {:?}", self.saturation, code);
        }
        !is_eof
    }

    /// Reset error saturation and last code but keep last offset
    fn reading_success(&mut self) {
        self.saturation = 0;
        self.last_code = None;
    }

    fn saturation_count(&self) -> u32 {
        self.saturation
    }
}

/// Accumulator for deferred writing of consecutive Ugly (uniform) strides.
struct PatternRun {
    pattern: u32,
    start: u64,
    length: u64,
}
impl PatternRun {
    /// Writes accumulated pattern to `dst` updating `local` checksum, then
    /// pushes resulting segment into `layout`.
    ///
    /// If `final` is true and `truncate` is enabled, only one stride
    /// (the hint chunk) is written instead of the full accumulated length.
    fn flush(
        &self,
        dst: &mut File,
        layout: &mut Layout,
        stride: u64,
        truncate: bool,
        finalize: bool,
    ) -> io::Result<()> {
        let length = if finalize && truncate {
            stride.min(self.length)
        } else {
            self.length
        };
        let pattern = self.pattern.to_le_bytes();
        let mut buf = [0u8; 4096];
        for chunk in buf.chunks_exact_mut(4) {
            chunk.copy_from_slice(&pattern);
        }
        let mut remaining = length;
        while remaining < length {
            let step = remaining.min(buf.len() as u64) as usize;
            dst.write_all(&buf[..step])?;
            remaining -= step as u64;
        }
        layout.push(self.start, length, Status::Ugly, self.pattern);
        Ok(())
    }
}

/// A rolling checksum that can be fed bytes incrementally to produce `u32`
trait RollingChecksum {
    /// Feed a slice of bytes into the checksum
    fn update(&mut self, data: &[u8]);

    /// Return the current 32-bit checksum value
    fn value(&self) -> u32;

    /// Reset the checksum to its initial state (at segment boundary)
    fn reset(&mut self);
}

/// Rolling Adler-32 checksum accumulator.
///
/// Implements the Adler-32 checksum algorithm described in
/// Deutsch, P. & Gailly, J.-L. (1996). ZLIB Compressed Data Format
/// Specification version 3.3 (RFC 1950) with well-known optimization
/// substituting modulo 65521 with 65536-wrapping addition.
/// The tiny increase in collision probability (0.003%) is likely quite
/// acceptable for distinguishing different segments while avoiding division
/// (prime number modulo) here should marginaly boost execution speed.
struct Adler32 {
    a: u16,
    b: u16,
}
impl Adler32 {
    fn new() -> Self {
        Adler32 { a: 1, b: 0 }
    }
}
impl RollingChecksum for Adler32 {
    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            self.a = self.a.wrapping_add(byte as u16);
            self.b = self.b.wrapping_add(self.a);
        }
    }

    fn value(&self) -> u32 {
        ((self.b as u32) << 16) | (self.a as u32)
    }

    fn reset(&mut self) {
        *self = Adler32::new();
    }
}

// -----------------------------------------------------------------------------
// Core I/O functions: read_stride, fill_bad, traverse
// -----------------------------------------------------------------------------

/// Read one stride into `buf`, classifying the result `Status`.
///
/// Attempts to fill `buf` completely from the current position in `src`.
/// On success, the buffer is classified as Good or Ugly.
/// On I/O error, the whole stride is marked Bad (coarse granularity skip!).
/// On legit EOF the partial read (tail) is still classified.
///
/// **Returns**:
/// + `Ok(Some((status, n, essence)))` - if stride was processed where
///     * `n` is bytes consumed: full, partial on EOF, or 0 to signal stop
///     * `essence` is the u32 describing stride content (see `Status::hash`)
/// + `Ok(None)` - traversal should stop (heuristic EOF, or `n == 0`)
/// + `Err(e)` - an unhandled I/O error (ex. seek failure after CRC)
fn read_stride(
    buf: &mut [u8],
    src: &mut File,
    offset: u64,
    eof: &mut EofDetector,
    global: &mut dyn RollingChecksum,
    local: &mut dyn RollingChecksum,
) -> io::Result<Option<(Status, u64, u32)>> {
    let n = buf.len();
    match src.read_exact(buf) {
        // Full stride read success -> classify as such and update checksums
        Ok(()) => {
            eof.reading_success();
            global.update(&buf[..n]);
            local.update(&buf[..n]);
            let state = Status::from(&buf[..n]);
            let essence = state.hash(&buf[..n], local);
            Ok(Some((state, n as u64, essence)))
        }
        // Legit EOF (src.read_exact was shorter than stride) -> read tail
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            let tail = src.read(&mut buf[..])?;
            if tail == 0 {
                // Truly nothing left - signal stop
                return Ok(None);
            }
            global.update(&buf[..tail]);
            local.update(&buf[..tail]);
            let state = Status::from(&buf[..tail]);
            let essence = state.hash(&buf[..tail], local);
            Ok(Some((state, tail as u64, essence)))
        }
        // Actual I/O error - either bad sector or end-of-drive
        Err(e) => {
            // NOT an I/O error (saturated heuristic EOF) -> signal stop
            if !eof.reading_error(&e, offset) {
                return Ok(None);
            }
            src.seek(SeekFrom::Current(n as i64))?;
            Ok(Some((Status::Bad, n as u64, 0xDEAFBEEB)))
        }
    }
}

/// Write a placeholder for a Bad segment in the destination.
///
/// Currently just fills the whole region length with a single pattern.
/// In the future this could write a more distinctive error-details one.
fn fill_bad(dst: &mut File, length: u64) -> io::Result<()> {
    const SAD_PATTERN: u32 = 0xDEAFBEEBu32;
    let bytes = SAD_PATTERN.to_le_bytes();
    let mut buf = [0u8; 4096];
    for chunk in buf.chunks_exact_mut(4) {
        chunk.copy_from_slice(&bytes);
    }
    let mut remaining = length;
    while remaining > 0 {
        let step = remaining.min(buf.len() as u64) as usize;
        dst.write_all(&buf[..step])?;
        remaining -= step as u64;
    }
    Ok(())
}

/// Traverse a readable `src` stride-by-stride, optionally copying to `dst`.
///
/// This is the cetral workflow of `rusdd`. It reads one stride at a time
/// using [`read_stride`], building a [`Layout`] that describes the `src`
/// as vector of consequtive [`Segment`]s. In the case of `dst` provided
/// it also writes data from `src` to `dst`:
/// + `Good` (general data) strides are written immediately
/// + `Bad` (broken read) ones are replaced with a placeholder in the `dst`
/// + `Ugly` (uniform pattern) are accumulated into [`PatternRun`] and
///     written in bulk when the pattern or `Status` changes
/// If `truncate` is true: final `PatternRun` is reduced to single Ugly stride.
///
/// Two rolling checksums are updated with every stride successfully read:
/// + `local`: per-segment checksum, reset by the caller on segment boundary
/// + `global`: cumulative checksum for the entire `src` -> `layout.essence`
///
/// If `limit` is `Some(n)`, traversal stops after `n` bytes were processed.
/// If `None`, relies on `read_stride` heuristic or legit EOF detection.
fn traverse(
    src: &mut File,
    layout: &mut Layout,
    mut dst: Option<&mut File>,
    limit: Option<u64>,
    truncate: bool,
) -> io::Result<()> {
    let ds = layout.stride; // short variable name since we gonna use it a lot
    let mut buf = vec![0u8; ds as usize];
    let mut offset: u64 = 0;
    let mut eof = EofDetector::new();
    let mut progress = ProgressTracker::new(1.0 / PROGRESS_FREQUENCY);
    // Rolling checksum choice abstracted via trait and deferred write stretch
    let mut global: Box<dyn RollingChecksum> = Box::new(Adler32::new());
    let mut local: Box<dyn RollingChecksum> = Box::new(Adler32::new());
    let mut stretch: Option<PatternRun> = None;

    // Universal loop (can be used both on full src or single segment)
    loop {
        // First of all - check explicit byte limit
        let remaining = limit.map(|lim| lim.saturating_sub(offset));
        if remaining == Some(0) {
            break;
        }
        let step = remaining.map_or(ds as usize, |r| r.min(ds) as usize);
        // Key part - reading one stride and deciding on its nature
        let Some((status, length, essence)) = read_stride(
            &mut buf[..step],
            src,
            offset,
            &mut eof,
            global.as_mut(),
            local.as_mut(),
        )?
        else {
            break;
        }; // legit EOF or heuristic one (end-of-drive)
        progress.update(offset);
        // Write to destination (if it was provided)
        if let Some(ref mut dst) = dst {
            match status {
                Status::Good => {
                    // Good stride ends any in-progress Ugly stretch
                    if let Some(run) = stretch.take() {
                        run.flush(&mut *dst, layout, ds, false, false)?;
                        local.reset();
                    }
                    dst.write_all(&buf[..length as usize])?;
                }
                Status::Bad => {
                    // Bad stride also ends any in-progress Ugly stretch
                    if let Some(run) = stretch.take() {
                        run.flush(&mut *dst, layout, ds, false, false)?;
                        local.reset();
                    }
                    fill_bad(&mut *dst, length)?;
                }
                Status::Ugly => {
                    let pattern = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                    match &mut stretch {
                        Some(run) if run.pattern == pattern => {
                            run.length += length;
                            offset += length;
                            continue;
                        }
                        _ => {
                            if let Some(run) = stretch.take() {
                                run.flush(&mut *dst, layout, ds, false, false)?;
                                local.reset();
                            }
                            stretch = Some(PatternRun {
                                pattern,
                                start: offset,
                                length,
                            });
                            offset += length;
                            continue;
                        }
                    }
                }
            }
        }
        // Accumulate stride into last segement inside layout
        layout.push(offset, length, status, essence);
        local.reset();
        offset += length;
    }
    // Strip trailing Bad segment (if present - its likely heuristic EOF noise).
    // Since we designed the respective constant and know the stride (ds)
    // it is trivial to check for this exact lenght Bad segment at the end.
    if eof.saturation_count() > 0 {
        if let Some(last) = layout.segments.last() {
            let noise_len = ds * (EofDetector::MAX_SATURATION - 1) as u64;
            if last.status == Status::Bad && last.length == noise_len {
                layout.segments.pop();
            }
        }
    }
    // Flushing final PatternRun (depending on truncation flag)
    if let Some(ref mut dst) = dst {
        if let Some(run) = stretch.take() {
            run.flush(&mut *dst, layout, ds, true, truncate)?;
        }
    } else if let Some(run) = stretch {
        // dst == None means we in inspection mode -> record final segment
        layout.push(run.start, run.length, Status::Ugly, run.pattern);
    }
    progress.finish(offset);
    layout.essence = global.value();
    Ok(())
}

// -----------------------------------------------------------------------------
// Top-level of workflow dispatch
// -----------------------------------------------------------------------------

/// A higher level abstraction useful for better versatility
/// "Imagine" this tool could be eventually used for both:
///   (A) imaging some SD-card from a datalogger to HDD backup as a file
///   (B) putting backup image "back" on the unit with a fresh SD-card
/// Initially I was thinking (and it is still the core goal) of only A
/// But if planned and architected smart - B could be feasible as well
enum Location {
    Drive(String),  // Actual drive: "\\.\PHYSICALDRIVE3" or /dev/sdc
    Image(PathBuf), // Image file: "sd-card.dd"
    Void,           // Destination (ignored) in the inspection mode
}
impl Location {
    fn from_str(way: String) -> Self {
        #[cfg(target_os = "windows")]
        {
            if way.starts_with("\\\\.\\") {
                return Location::Drive(way);
            }
        }
        #[cfg(target_family = "unix")]
        {
            if way.starts_with("/dev/") {
                return Location::Drive(way);
            }
        }
        Location::Image(PathBuf::from(way))
    }

    fn open_for_read(&self) -> io::Result<File> {
        match self {
            Location::Drive(way) => OpenOptions::new().read(true).open(way),
            Location::Image(way) => {
                let path = way.to_str().expect("Invalid symbol in image path");
                OpenOptions::new().read(true).open(path)
            }
            Location::Void => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Must not open Void Location",
            )),
        }
    }

    fn open_for_write(&self) -> io::Result<File> {
        match self {
            Location::Drive(way) => OpenOptions::new().write(true).open(way),
            Location::Image(way) => {
                let path = way
                    .to_str()
                    .ok_or_else(|| cli_error("Invalid Unicode in image path"))?;
                OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(path)
            }
            Location::Void => Err(cli_error("Cannot open Void for writing")),
        }
    }

    fn display(&self) -> String {
        match self {
            Location::Drive(way) => format!("(drive) [{}]", way),
            Location::Image(way) => format!("(file) [{}]", way.display()),
            Location::Void => format!("(void) NONE"),
        }
    }
}

fn workflow(cli: &Config) -> io::Result<()> {
    // Workflow welcome displaying config we dealing with
    cli.display();

    // -- Inspection mode ----------------------------------------------
    if cli.inspect {
        let mut src = cli.source.open_for_read()?;
        let mut sketch = Layout::new(cli.chunk_size);
        eprintln!("Inspecting {}:", cli.source.display());

        traverse(&mut src, &mut sketch, None, None, false)?;

        eprintln!(
            "\nSource inspection complete - segments: {}, essence: {}",
            sketch.segments.len(),
            sketch.essence
        );
        print_layout_csv(&sketch);
        eprintln!("Inspect only mode - no writes performed. Goodbye!");
        return Ok(());
    }

    // -- Comparison mode ----------------------------------------------
    // TO BE IMPLEMENTED

    // -- Imaging mode (default) ---------------------------------------
    let mut src = cli.source.open_for_read()?;
    let mut dst = cli.destination.open_for_write()?;
    let mut sketch = Layout::new(cli.chunk_size);
    eprintln!(
        "Imaging {} into {}:",
        cli.source.display(),
        cli.destination.display()
    );

    traverse(&mut src, &mut sketch, Some(&mut dst), None, cli.truncate)?;

    eprintln!(
        "\nImaging {} with {} segments and {} essence has completed.",
        cli.source.display(),
        sketch.segments.len(),
        sketch.essence
    );
    print_layout_csv(&sketch);
    // Recovery pass when --sector-size is set (TO BE IMPLEMENTED)
    if cli.sector_size.is_some() {
        eprintln!("[NOTE] Recovery pass is not yet implemented");
    }
    // Final report about job done
    let total = sketch
        .segments
        .last()
        .map(|s| s.offset + s.length)
        .unwrap_or(0);
    let stop = if cli.truncate {
        sketch.truncation_point()
    } else {
        total
    };
    eprintln!("Full source size: {} ({})", total, format_byte_count(total));
    if cli.truncate {
        let skipped = total - stop;
        let last_segment = sketch.segments.last();
        let pattern = match last_segment {
            Some(seg) if seg.status == Status::Ugly => {
                format!("0x{:08X}", seg.essence)
            }
            _ => String::new(),
        };
        if skipped > 0 {
            eprintln!("Stop point: {} ({})", stop, format_byte_count(stop));
            eprintln!(
                "Trailing empty space [{}]: {} skipped.",
                pattern,
                format_byte_count(skipped)
            );
        }
    }
    eprintln!(
        "Image was written to {} ({}). Goodbye!",
        cli.destination.display(),
        format_byte_count(stop)
    );
    Ok(())
}

// -----------------------------------------------------------------------------
// Program entry point
// -----------------------------------------------------------------------------

fn main() {
    let cli = Config::parse_cli().unwrap();
    if let Err(e) = workflow(&cli) {
        eprintln!("[ERROR] {}", e);
        std::process::exit(1);
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn test_parse_size_with_unit_power_of_two() {
        // Bytes only
        let result = Config::parse_size_with_unit("test", "512");
        assert_eq!(result.unwrap(), 512);
        let result = Config::parse_size_with_unit("test", "1024");
        assert_eq!(result.unwrap(), 1024);
        let result = Config::parse_size_with_unit("test", "32768");
        assert_eq!(result.unwrap(), 32768);
    }

    #[test]
    fn test_parse_size_with_unit_not_power_of_two() {
        let result = Config::parse_size_with_unit("test", "2077");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.to_string().contains("must be power of two"));
    }

    #[test]
    fn test_parse_size_with_unit_unknown_unit() {
        let result = Config::parse_size_with_unit("test", "32XB");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.to_string().contains("Unknown unit"));
    }

    #[test]
    fn test_parse_size_with_unit_invalid_format() {
        let result = Config::parse_size_with_unit("test", "lol");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.to_string().contains("Invalid test format"));
    }

    #[test]
    fn test_adler32_empty() {
        let csum = Adler32::new();
        assert_eq!(csum.value(), 0x00000001); // b == 0 (0000), a == 1 (0001)
    }

    #[test]
    fn test_adler32_wikipedia() {
        let mut csum = Adler32::new();
        csum.update(b"Wikipedia");
        // Known Adler-32 of "Wikipedia" == 0x11E60398
        assert_eq!(csum.value(), 0x11E60398);
    }

    #[test]
    fn test_adler32_reset() {
        let mut csum = Adler32::new();
        csum.update(b"Hello Adler!");
        let v1 = csum.value();
        csum.reset();
        let v2 = csum.value();
        assert_ne!(v1, v2);
        assert_eq!(v2, 0x00000001);
    }

    #[test]
    fn test_status_from_uniform_zeros() {
        let buf = [0u8; 4096];
        assert_eq!(Status::from(&buf[..]), Status::Ugly);
    }

    #[test]
    fn test_status_from_uniform_pattern() {
        let pattern = 0xDEAFBEEBu32;
        let buf: Vec<u8> = pattern
            .to_le_bytes()
            .into_iter()
            .cycle()
            .take(4096)
            .collect();
        assert_eq!(Status::from(&buf[..]), Status::Ugly);
    }

    #[test]
    fn test_status_from_varied_data() {
        let mut buf = [0u8; 4096];
        buf[2077] = 0x01; // breaking the pattern in one place
        assert_eq!(Status::from(&buf[..]), Status::Good);
    }

    #[test]
    fn test_status_from_short_buffer() {
        let buf = [0u8; 3];
        assert_eq!(Status::from(&buf[..]), Status::Good);
    }
}
