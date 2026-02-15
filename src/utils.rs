use aes::cipher::consts::U16;
use aes::cipher::generic_array::GenericArray;
use chrono::Local;
use fern::Dispatch;
use log::{LevelFilter, info, warn};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::{fs, io};

#[cfg(unix)]
use std::os::unix::fs::FileExt as _;
#[cfg(windows)]
use std::os::windows::fs::FileExt as _;
use std::path::Path;

#[cfg(unix)]
pub fn read_exact_at(file: &File, mut buf: &mut [u8], mut off: u64) -> io::Result<()> {
    while !buf.is_empty() {
        let n = file.read_at(buf, off)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read_at=0"));
        }
        buf = &mut buf[n..];
        off += n as u64;
    }
    Ok(())
}
#[cfg(windows)]
pub fn read_exact_at(file: &File, mut buf: &mut [u8], mut off: u64) -> io::Result<()> {
    while !buf.is_empty() {
        let n = file.seek_read(buf, off)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "seek_read=0"));
        }
        buf = &mut buf[n..];
        off += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
pub fn write_all_at(file: &File, mut buf: &[u8], mut off: u64) -> io::Result<()> {
    while !buf.is_empty() {
        let n = file.write_at(buf, off)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "write_at=0"));
        }
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}
#[cfg(windows)]
pub fn write_all_at(file: &File, mut buf: &[u8], mut off: u64) -> io::Result<()> {
    while !buf.is_empty() {
        let n = file.seek_write(buf, off)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "seek_write=0"));
        }
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}

pub struct Region {
    start: u64,
    end: u64,
}

// Reliability is uncertain on key validation.
pub fn key_validation(key: &str) -> bool {
    let stripped_key: String = key.chars().filter(|c| !c.is_whitespace()).collect();

    if stripped_key.len() != 32 {
        println!("Invalid key length: {}", stripped_key.len());
        warn!("Invalid key length: {}", stripped_key.len());
        return false;
    }

    if !stripped_key.chars().all(|c| c.is_ascii_hexdigit()) {
        println!("Key contains invalid characters");
        warn!("Key contains invalid characters");
        return false;
    }

    info!("Key is valid");
    true
}

// Making an init vector
#[inline(always)]
pub fn generate_iv(sector: u64) -> GenericArray<u8, U16> {
    let mut iv_bytes = [0u8; 16];
    iv_bytes[12] = (sector >> 24) as u8;
    iv_bytes[13] = (sector >> 16) as u8;
    iv_bytes[14] = (sector >> 8) as u8;
    iv_bytes[15] = sector as u8;
    GenericArray::clone_from_slice(&iv_bytes)
}
#[inline(always)]
pub fn is_encrypted(regions: &[Region], sector: u64, sector_data: &[u8]) -> bool {
    if sector_data.iter().all(|&b| b == 0) {
        return false;
    }
    regions.iter().any(|r| sector >= r.start && sector < r.end)
}

// Splitting the cake
pub fn extract_regions<R: Read + Seek>(reader: &mut R) -> io::Result<Vec<Region>> {
    let mut header = [0u8; 4096];
    reader.seek(SeekFrom::Start(0))?;
    reader.read_exact(&mut header)?;
    let num_normal_regions = u32::from_be_bytes(header[0..4].try_into().unwrap()) as usize;
    let regions_count = (num_normal_regions * 2) - 1;
    let mut regions = Vec::with_capacity(regions_count);

    let mut is_encrypted = false;
    for i in 0..regions_count {
        let region_offset = 4 + i * 8;
        let start_sector =
            u32::from_be_bytes(header[region_offset..region_offset + 4].try_into().unwrap());
        let end_sector = u32::from_be_bytes(
            header[region_offset + 4..region_offset + 8]
                .try_into()
                .unwrap(),
        );

        regions.push(Region {
            start: start_sector as u64,
            end: end_sector as u64,
        });

        is_encrypted = !is_encrypted;
    }

    Ok(regions)
}

pub fn setup_logging() -> Result<(), Box<dyn std::error::Error>> {
    let log_dir = Path::new("log");
    fs::create_dir_all(log_dir)?;
    let now = Local::now();
    let log_file_name = format!("log/{}.log", now.format("%Y-%m-%d_%H-%M-%S"));

    Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{} [{}] - {}",
                now.format("%Y-%m-%d_%H-%M-%S"),
                record.level(),
                message
            ))
        })
        .level(LevelFilter::Debug)
        .chain(std::io::stdout())
        .chain(fern::log_file(log_file_name)?)
        .apply()?;

    Ok(())
}
