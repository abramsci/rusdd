//! # rusdd - Really Useful Secure Digital Duplicator
//!
//! A simple tool for digitizing physical media into back-up images bit-by-bit.
//! Built with the focus on usability: error logging, smart truncation, etc.
//!
//! Author: Sergei Abramenkov
//! License: MIT
//! Version: 0.0.3
//! ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

const ARGUMENTS: &str = "
Required arguments (must be named - not positional):
  -d, --physical-drive <DRIVE>      Device identifier
  -o, --output-path <PATH>          Output image file";

const FLAGS: &str = "
Flags (boolean toggles - false by default):
  -h, --help                        Show extended help with examples
  -f, --force                       Force overwrite output if exists
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

/// Command line interface configuration
struct Config {
    physical_drive: String,
    output_path: PathBuf,
    force: bool,      // Force overwrite of the output path?
    smart: bool,      // Is smart stop trancation enabled?
    sector_size: u16, // Media physical sector size in bytes
    chunk_size: u16,  // Pattern-seeking chunk size in sectors
    buffer_size: u16, // Reading buffer size in chunks
}

/// Print help in STDERR so it would not pollute STDOUT
fn print_help(level: HelpLevel) {
    eprintln!("rusdd - Really Useful Secure Digital Duplicator");
    eprintln!("\nUsage: rusdd [FLAGS] -d <DRIVE> -o <PATH> [PARAMS]");

    match level {
        HelpLevel::Usage => {
            eprintln!("{}", ARGUMENTS);
            eprintln!("{}", FLAGS);
            eprintln!("Try 'rusdd -h' for more information.");
        }
        HelpLevel::Extended => {
            eprintln!("{}", ARGUMENTS);
            eprintln!("{}", FLAGS);
            eprintln!("{}", PARAMS);
        }
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
    let mut physical_drive = None;
    let mut output_path = None;
    // Assigning defaults for flags and params
    let mut force = false; // Better safe than sorry (warn user)
    let mut smart = false; // Full drive image (forensic frendly)
    let mut sector_size: u16 = 512; // Typical physical sector size
    let mut chunk_size: u16 = 16384; // => 8 MiB (8_589_934_592 bytes)
    let mut buffer_size: u16 = 4; // => 32 MiB (34_359_738_368 bytes)

    while let Some(arg) = cli.next() {
        match arg.as_str() {
            "--physical-drive" | "-d" => {
                let value = cli.next().ok_or("Missing --physical-drive value")?;
                physical_drive = Some(value);
            }
            "--output-path" | "-o" => {
                let value = cli.next().ok_or("Missing --output-path value")?;
                output_path = Some(PathBuf::from(value));
            }
            "--force" | "-f" => force = true,
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
                print_help(HelpLevel::Extended);
                std::process::exit(0);
            }
            _ => return Err(format!("Unknown argument: {}", arg)),
        }
    }

    Ok(Config {
        physical_drive: physical_drive.ok_or("--physical-drive is required")?,
        output_path: output_path.ok_or("--output-path is required")?,
        force,
        smart,
        sector_size: sector_size,
        chunk_size: chunk_size,
        buffer_size: buffer_size,
    })
}

fn parse_cli() -> Result<Config, String> {
    if env::args().len() == 1 {
        print_help(HelpLevel::Usage);
        std::process::exit(0);
    } else {
        parse_cli_from_iter(env::args().skip(1))
    }
}

fn run() -> Result<(), String> {
    let cli = parse_cli()?;
    let chunk: usize = (cli.chunk_size as usize) * (cli.sector_size as usize);
    let buffer: usize = (cli.buffer_size as usize) * chunk;

    // Program welcome - a header for CSV log in case of STDOUT redirection
    println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^");
    println!("rusdd - Really Useful Secure Digital Duplicator (ver.0.0.3)");
    println!("Output Path: {}", cli.output_path.to_string_lossy());
    if cli.output_path.exists() && !cli.force {
        return Err(format!(
            "Output Path ({}) does already exist. \
            Be careful to check it and back-up if needed. \
            Then use --force flag if you sure to overwrite it.",
            cli.output_path.display()
        ));
    }
    if cli.smart {
        println!(
            "Smart Truncation Stop: ENABLED (after {} empty sectors)",
            cli.chunk_size
        );
        println!("  Detects both 0x00 and 0xFF empty patterns");
    } else {
        println!("Smart Truncation Stop: DISABLED (full copy)");
    }
    println!("Physical Drive: {}", cli.physical_drive);
    println!("Sector size: {} bytes", cli.sector_size);
    println!("Buffer size: {} MiB", buffer / 1024 / 1024);
    println!("************************************************************");

    // Actual logic of the imaging physical device into a file bit-by-bit
    let mut source = File::open(&cli.physical_drive)
        .map_err(|e| format!("Cannot open {}: {}", cli.physical_drive, e))?;
    let mut destination = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&cli.output_path)
        .map_err(|e| format!("Failed {}: {}", cli.output_path.display(), e))?;
    let mut buf = vec![0u8; buffer];

    // Starting small - read and write only one buffer
    let bytes_read = source
        .read(&mut buf)
        .map_err(|e| format!("Read error: {}", e))?;
    if bytes_read == 0 {
        return Err("Source device is empty".to_string());
    }
    eprintln!("Read {} bytes", bytes_read);
    destination
        .write_all(&buf[..bytes_read])
        .map_err(|e| format!("Write error: {}", e))?;
    println!("************************************************************");
    println!(
        "Successfully wrote {} bytes to {}",
        bytes_read,
        cli.output_path.display()
    );

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
