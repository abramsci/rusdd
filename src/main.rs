//! # rusdd - Really Useful Secure Digital Duplicator
//!
//! A simple tool for digitizing physical media into back-up images bit-by-bit.
//! Built with the focus on usability: error logging, smart truncation, etc.
//!
//! Author: Sergei Abramenkov
//! License: MIT
//! Version: 0.0.5
//! ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Instant;

// -------------------------- PROGRAM USAGE AND HELP --------------------------
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
      --limit-size <N>              Limit size (drive capacity hint)
      --chunk-size <N>              Reading block size (default: 32MiB)
                                    Must be power of 2 (for efficiency)
      --sector-size <N>             Physical sector size to read in bytes
                                    Possible values: [512, 1024, 2048, 4096]
                                    Enables recovery mode if provided";
const UNITS: &str = "
Note: --limit-size and --chunk-size accept unit suffixes (binary):
      B (bytes), K/KiB (1024 B), M/MiB (1024^2 B), G/GiB (1024^3 B)
      Examples: 512B, 256KiB, 32M, 4GiB";

/// Explicity does matter for eventual scaling
enum HelpLevel {
    Usage,
    Extended,
}

impl HelpLevel {
    fn display(&self) {
        println!("rusdd - Really Useful Secure Digital Duplicator");
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
    limit_size: u64,          // Physical drive hint (default: 32GiB)
    chunk_size: u64,          // Logical granularity (default: 32MiB)
    sector_size: Option<u16>, // None = no recovery mode
}

impl Config {
    /// Program welcome message (a header in case of STDOUT redirection)
    fn display(&self) {
        println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^");
        println!("rusdd - Really Useful Secure Digital Duplicator (ver.0.0.5)");
        println!("IMAGING FROM: {}", self.source.display());
        println!("  Read limit: {}", self.limit_size);
        println!("  Chunk size: {}", self.chunk_size);
        println!("  Inspection: {}", if self.inspect { "ON" } else { "OFF" });
        println!("  Truncation: {}", if self.truncate { "ON" } else { "OFF" });
        println!("IMAGING INTO: {}", self.destination.display());
        println!("***********************************************************");
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
        let mut limit: u64 = 32 * 1024 * 1024 * 1024; // => 32 GiB
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
                "--limit-size" => {
                    let v = cli.next().ok_or_else(|| missing("limit-size"))?;
                    limit = Self::parse_size_with_unit("limit size", &v)?;
                }
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
            limit_size: limit,
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

// -------------------------- AUXILIARY FUNCTIONS --------------------------
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

    /// Record an error, then return true if EOF is detected
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
        is_eof
    }

    /// Reset error saturation and last code but keep last offset
    fn reading_success(&mut self) {
        self.saturation = 0;
        self.last_code = None;
    }
}

/// Imaging can be long (with USB 2.0) so progress traking is essential
struct ProgressTracker {
    next_report: u64,
    report_interval: u64,
    start_time: Instant,
    last_offset: u64,
}
impl ProgressTracker {
    fn new(report_interval_bytes: u64) -> Self {
        ProgressTracker {
            next_report: report_interval_bytes,
            report_interval: report_interval_bytes,
            start_time: Instant::now(),
            last_offset: 0,
        }
    }

    fn update(&mut self, offset: u64) {
        if offset >= self.next_report {
            let elapsed = self.start_time.elapsed();
            let scanned = offset / (1024 * 1024);
            let speed = if elapsed.as_secs() > 0 {
                (offset as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64()
            } else {
                0.0
            };
            eprint!(
                "\rScanned: {} MiB | Speed {:.1} MiB/s | Time {:?}",
                scanned, speed, elapsed
            );
            self.next_report += self.report_interval;
            self.last_offset = offset;
        }
    }

    fn finish(&self, offset: u64) {
        let elapsed = self.start_time.elapsed();
        let scanned = offset / (1024 * 1024);
        eprint!(
            "\rScanned: {} MiB complete! | Time elapsed total {:?}",
            scanned, elapsed
        );
    }
}

// -------------------------- CORE DATA STRUCTURES --------------------------
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

    fn display(&self) -> String {
        match self {
            Location::Drive(way) => format!("{} (drive)", way),
            Location::Image(way) => format!("{} (file)", way.display()),
            Location::Void => format!("NONE (void)"),
        }
    }
}

/// Byte buffer status
#[derive(Clone, Debug, Eq, PartialEq)]
enum Status {
    Good, // Readable, reasonable, respectable
    Bad,  // Error in reading attempt (CRC-like)
    Ugly, // Full of the same byte pattern (ex. 0x00 or 0xFF)
}
impl From<&[u8]> for Status {
    fn from(buf: &[u8]) -> Self {
        if buf.is_empty() {
            return Status::Good;
        }
        let byte = buf[0];
        let all_same = buf.iter().all(|&b| b == byte);
        if all_same { Status::Ugly } else { Status::Good }
    }
}
impl Status {
    const PLACEHOLDER: u32 = 0xBECAFEDA;
    const BAD_UNKNOWN: u32 = 0xDEADCEEB;

    fn essence_from_buffer(&self, buf: &[u8]) -> u32 {
        match self {
            Status::Good => Self::PLACEHOLDER,
            Status::Bad => Self::BAD_UNKNOWN,
            Status::Ugly => (buf[0] as u32) * 0x01010101,
        }
    }
}

/// Continuous block with the same status
#[derive(Clone, Debug)]
struct Segment {
    offset: u64,  // Byte offset in the source
    length: u64,  // Length in bytes
    essence: u32, // Checksum (Good), error code (Bad) or pattern (Ugly)
    status: Status,
    last: bool,
    // 2-byte padding - room for more usable info
}

/// Location (memory) structure as a vector of continuous blocks
struct Layout {
    stride: u64,
    segments: Vec<Segment>,
}
impl Layout {
    /// Scanning function to figure out location layout
    fn scan(
        location: &mut (impl Read + Seek),
        stride: u64,
        limit: Option<u64>,
    ) -> io::Result<Self> {
        let mut segments = Vec::new();
        let mut buf = vec![0u8; stride as usize];
        let mut offset = 0u64;
        let mut current: Option<Segment> = None;
        let limit = limit.unwrap_or(u64::MAX);
        let mut eof = EofDetector::new();
        // Progress reporing each 64 MiB
        let mut progress = ProgressTracker::new(64 * 1024 * 1024);
        // A simplistic idea - looping over location step-by-step
        while offset < limit {
            let step = std::cmp::min(stride, limit - offset) as usize;
            let mut bytes_read = step as u64;
            let essence;
            progress.update(offset);
            // Actual reading attempt
            let status = match location.read_exact(&mut buf[..step]) {
                // Success - full step was read
                Ok(()) => {
                    eof.reading_success();
                    Status::from(&buf[..step])
                }
                // Legit filesystem end of file
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    // Partial read at the end of location
                    let actual = location.read(&mut buf[..step])?;
                    if actual == 0 {
                        break;
                    }
                    bytes_read = actual as u64;
                    Status::from(&buf[..actual])
                }
                // Reading errors (bad sectors) - using heuristics
                Err(e) => {
                    if eof.reading_error(&e, offset) {
                        break;
                    }
                    location.seek(SeekFrom::Current(step as i64))?;
                    Status::Bad
                }
            };
            essence = status.essence_from_buffer(&buf[..bytes_read as usize]);
            // Segment accumulation
            match &mut current {
                // Extending the current segment
                Some(seg) if seg.status == status && seg.essence == essence => {
                    seg.length += bytes_read as u64;
                }
                // Status changed -> taking ownership to push completed segment
                Some(_) => {
                    let completed = current.take().unwrap();
                    segments.push(completed);
                    // Initializing a new one
                    current = Some(Segment {
                        offset,
                        length: bytes_read as u64,
                        essence,
                        status,
                        last: false,
                    });
                }
                // No current segment (just starting) -> making a new one
                None => {
                    current = Some(Segment {
                        offset,
                        length: bytes_read as u64,
                        essence,
                        status,
                        last: false,
                    });
                }
            }
            offset += bytes_read as u64;
            if offset >= limit {
                break;
            }
        }
        progress.finish(offset);
        if let Some(mut seg) = current {
            seg.last = true;
            segments.push(seg);
        }
        Ok(Layout { stride, segments })
    }

    /// Figuring out effective stop point for the segmented layout
    /// Using reverse iteration (go backwards) to truncate the Ugly tail
    /// Therefore we look for the first segment that is not Ugly
    fn truncation_point(&self) -> u64 {
        let mut end = self
            .segments
            .last()
            .map(|s| s.offset + s.length)
            .unwrap_or(0);
        for seg in self.segments.iter().rev() {
            if seg.status != Status::Ugly {
                end = seg.offset + seg.length;
            }
        }
        end
    }

    /// Printing location layout as CSV-lines
    fn print_csv(&self) {
        for seg in self.segments.iter() {
            let comment = match seg.status {
                Status::Good => format!("'readable data'"),
                Status::Bad => format!("'error reading'"),
                Status::Ugly => format!("'bit-pattern: 0x{:08X}'", seg.essence),
            };
            println!("{},{},{}", seg.offset, seg.length, comment);
        }
    }
}

// -------------------------- HIGH-LEVEL WORKFLOW --------------------------
/// Actual logic of the imaging source into destination bit-by-bit
fn run(cli: &Config) -> io::Result<()> {
    // Workflow welcome displaying config we dealing with
    cli.display();
    // Pass 0 - source Location inspection
    let stride = cli.chunk_size;
    let mut src = cli.source.open_for_read()?;
    eprintln!("Inspecting source layout...");
    let src_layout = Layout::scan(&mut src, stride, None)?;
    src.rewind()?; // Return to position 0 before forgetting about it!
    eprintln!(
        "... complete. Segments found: {}",
        src_layout.segments.len()
    );
    // Truncation and operational mode decisions
    let stop = src_layout.truncation_point();
    eprintln!("Effective stop point: {}", stop);
    if cli.inspect {
        src_layout.print_csv();
        eprintln!("Inspection mode - no writes performed");
        return Ok(());
    }
    // Pass 1 - copy Good and Ugly segments
    // TO BE IMPLEMENTED
    // Pass 2 - forensic sector-by-sector recovery of Bad segments
    // TO BE IMPLEMENTED
    println!("***********************************************************");
    Ok(())
}

fn main() {
    let cli = Config::parse_cli().unwrap();
    if let Err(e) = run(&cli) {
        eprintln!("[ERROR] {}", e);
        std::process::exit(1);
    }
}

// -------------------------------- UNIT TESTS --------------------------------
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
}
