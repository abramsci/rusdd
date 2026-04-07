//! # rusdd - Really Useful Secure Digital Duplicator
//!
//! A simple tool for digitizing physical media into back-up images bit-by-bit.
//! Built with the focus on usability: error logging, smart truncation, etc.
//!
//! Author: Sergei Abramenkov
//! License: MIT
//! Version: 0.0.4
//! ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

use std::env;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;

const ARGUMENTS: &str = "
Required arguments (must be named - not positional):
  -s, --source <LOCATION>           What to image
  -d, --destination <LOCATION>      Where to put";

const FLAGS: &str = "
Flags (boolean toggles - false by default):
  -h, --help                        Show extended help with examples
  -t, --smart                       SmarT Truncation (after *empty chunk)
                                      *consecutive 0x00 or 0xFF pattern";

const PARAMS: &str = "
Parameters (optional - rarely should be non-default):
      --sector-size <N>             Physical sector size in bytes (default: 512)
                                      Possible values: [512, 1024, 2048, 4096]
      --chunk-size <N>              Sectors per *empty chunk (default: 16384)
                                      Must be power of 2 (for efficiency)
      --buffer-size <N>             Chunks per buffer (default: 4)
                                      Must be power of 2 (defaults to 32 MiB)";

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
            }
        }
    }
}

/// A higher level abstraction useful for better versatility
/// "Imagine" this tool could be eventually used for both:
///   (A) imaging some SD-card from a datalogger to HDD backup as a file
///   (B) putting backup image "back" on the unit with a fresh SD-card
/// Initially I was thinking (and it is still the core goal) of only A
/// But if planned and architected smart - B could be feasible as well
enum Location {
    Drive(String),  // Actual drive: "\\.\PHYSICALDRIVE3" or /dev/sdc
    Image(PathBuf), // Image file: "sd-card.dd"
}

impl Location {
    fn from_str(way: String) -> Location {
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

    fn open_for_read(&self) -> Result<impl Read, String> {
        match self {
            Location::Drive(way) => OpenOptions::new()
                .read(true)
                .open(way)
                .map_err(|e| format!("Open drive {} for read: {}", way, e)),
            Location::Image(way) => OpenOptions::new()
                .read(true)
                .open(way)
                .map_err(|e| format!("Open {} for read: {}", way.display(), e)),
        }
    }

    fn open_for_write(&self) -> Result<impl Write, String> {
        match self {
            Location::Drive(way) => OpenOptions::new()
                .write(true)
                .open(way)
                .map_err(|e| format!("Open drive {} for write: {}", way, e)),
            Location::Image(way) => {
                if way.exists() {
                    return Err(format!(
                        "File ({}) already exist. Check and move (delete) it.",
                        way.display()
                    ));
                }
                OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(way)
                    .map_err(|e| format!("Cannot create image file: {}", e))
            }
        }
    }

    fn display(&self) -> String {
        match self {
            Location::Drive(way) => format!("{} (drive)", way),
            Location::Image(way) => format!("{} (file)", way.display()),
        }
    }
}

/// Command line interface configuration
struct Config {
    source: Location,
    destination: Location,
    smart: bool,      // Is smart stop trancation enabled?
    sector_size: u16, // Media physical sector size in bytes
    chunk_size: u16,  // Pattern-seeking chunk size in sectors
    buffer_size: u16, // Reading buffer size in chunks
}

impl Config {
    fn calc_sizes(&self) -> (usize, usize, usize) {
        let sector: usize = self.sector_size as usize;
        let chunk: usize = (self.chunk_size as usize) * sector;
        let buffer: usize = (self.buffer_size as usize) * chunk;
        (sector, chunk, buffer)
    }

    // Program welcome message (a header in case of STDOUT redirection)
    fn display(&self) {
        let (sector, chunk, buffer) = self.calc_sizes();
        println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^");
        println!("rusdd - Really Useful Secure Digital Duplicator (ver.0.0.4)");
        println!("Source:       {}", self.source.display());
        println!("Destination:  {}", self.destination.display());
        println!("Smart mode:   {}", if self.smart { "ON" } else { "OFF" });
        println!("Sector size: {} bytes", sector);
        println!("Chunk size: {} KiB", chunk / 1024);
        println!("Buffer size: {} MiB", buffer / 1024 / 1024);
        println!("***********************************************************");
    }
}

/// Enforcement for power of 2: shrinking parameter space + hidden optimization
fn parse_size_param(name: &str, value: String) -> Result<u16, String> {
    let parsed: u16 = value
        .parse()
        .map_err(|_| format!("Invalid {}: {}", name, value))?;
    if !parsed.is_power_of_two() {
        return Err(format!("{} must be power of 2", name));
    }
    Ok(parsed)
}

fn parse_cli_from_iter<I>(mut cli: I) -> Result<Config, String>
where
    I: Iterator<Item = String>,
{
    let mut source = None;
    let mut destination = None;
    // Assigning defaults for flags and params
    let mut smart = false; // Full drive image (forensic frendly)
    let mut sector_size: u16 = 512; // Typical physical sector size
    let mut chunk_size: u16 = 16384; // => 8 MiB (8_589_934_592 bytes)
    let mut buffer_size: u16 = 4; // => 32 MiB (34_359_738_368 bytes)

    while let Some(arg) = cli.next() {
        match arg.as_str() {
            "--source" | "-s" => {
                let value = cli.next().ok_or("Missing --source value")?;
                source = Some(Location::from_str(value));
            }
            "--destination" | "-d" => {
                let value = cli.next().ok_or("Missing --destination value")?;
                destination = Some(Location::from_str(value));
            }
            "--smart" | "-t" => smart = true,
            "--sector-size" => {
                let value = cli.next().ok_or("Missing --sector-size value")?;
                sector_size = parse_size_param("sector size", value)?;
            }
            "--chunk-size" => {
                let value = cli.next().ok_or("Missing --chunk-size value")?;
                chunk_size = parse_size_param("chunk size", value)?;
            }
            "--buffer-size" => {
                let value = cli.next().ok_or("Missing --buffer-size value")?;
                buffer_size = parse_size_param("buffer size", value)?;
            }
            "--help" | "-h" => {
                HelpLevel::Extended.display();
                std::process::exit(0);
            }
            _ => return Err(format!("Unknown argument: {}", arg)),
        }
    }

    Ok(Config {
        source: source.ok_or("--source is required")?,
        destination: destination.ok_or("--destination is required")?,
        smart,
        sector_size: sector_size,
        chunk_size: chunk_size,
        buffer_size: buffer_size,
    })
}

fn parse_cli() -> Result<Config, String> {
    if env::args().len() == 1 {
        HelpLevel::Usage.display();
        std::process::exit(0);
    } else {
        parse_cli_from_iter(env::args().skip(1))
    }
}

struct FailedRegion {
    offset: u64, // Byte offset from start
    size: usize, // Size in bytes (buffer_size, then chunk_size, then sector_size)
}

/// Pass 1: Optimistic - full buffer reads
/// Returns: Vec of failed regions (offsets and sizes)
fn copy_optimistic(
    source: &mut impl Read,
    destination: &mut impl Write,
    config: &Config,
) -> Result<Vec<FailedRegion>, String> {
    // Returns offsets where buffer reads failed
}

/// Pass 2: Realistic - chunk-sized reads for failed buffers
/// Modifies dest in-place, returns failed chunk offsets
fn copy_realistic(
    source: &mut impl Read,
    destination: &mut impl Write,
    config: &Config,
    failures: &[FailedRegion],
) -> Result<Vec<FailedRegion>, String> {
    // Returns failed chunk regions within the failed buffers
}

/// Pass 3: Forensic - sector-by-sector for failed chunks
/// Fills unrecoverable sectors with pattern, logs to CSV
fn copy_forensic(
    source: &mut impl Read,
    destination: &mut impl Write,
    config: &Config,
    failures: &[FailedRegion],
) -> Result<(), String> {
    // No return - any remaining failures get filled with the pattern
}

fn run() -> Result<(), String> {
    let cli = parse_cli()?;
    cli.display();

    // Actual logic of the imaging physical device into a file bit-by-bit
    let mut source = cli.source.open_for_read()?;
    let mut destination = cli.destination.open_for_write()?;

    // Pass 1: Optimistic
    eprintln!("Pass 1/3: Optimistic copy (buffer-sized I/O)");
    let failures = copy_optimistic(&mut source, &mut destination, &cli)?;

    if failures.is_empty() {
        eprintln!("Complete: No errors detected");
        return Ok(());
    }

    eprintln!("Pass 1 complete. {} failed regions.", failures.len());

    // Pass 2: Chunk recovery
    eprintln!("Pass 2/3: Chunk-level recovery");
    let chunk_failures = copy_realistic(&mut source, &mut destination, &cli, &failures)?;
    if chunk_failures.is_empty() {
        eprintln!("Complete: All regions recovered");
        return Ok(());
    }
    eprintln!(
        "Pass 2 complete. {} chunks still failing.",
        chunk_failures.len()
    );

    // Pass 3: Sector forensic
    eprintln!("Pass 3/3: Sector-level forensic recovery");
    copy_forensic(&mut source, &mut destination, &cli, &chunk_failures)?;

    println!("***********************************************************");
    // println!(
    //     "Successfully wrote {} bytes to {}",
    //     bytes_read,
    //     cli.destination.display()
    // );
    eprintln!("Copy complete. Unrecoverable sectors filled with the pattern.");

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("[ERROR] {}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_param_power_of_two() {
        assert_eq!(parse_size_param("test", "512".to_string()), Ok(512));
        assert_eq!(parse_size_param("test", "1024".to_string()), Ok(1024));
        assert_eq!(parse_size_param("test", "32768".to_string()), Ok(32768));
    }

    #[test]
    fn test_parse_size_param_not_power_of_two() {
        let result = parse_size_param("test", "2077".to_string());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be power of 2"));
    }
}
