use aes::cipher::consts::U16;
use aes::cipher::generic_array::GenericArray;
use log::{info, warn};
use std::fs::File;
use std::io;
use std::io::{Read, Seek, SeekFrom};

#[cfg(unix)]
use std::os::unix::fs::FileExt as _;
#[cfg(windows)]
use std::os::windows::fs::FileExt as _;

#[cfg(unix)]
pub fn read_exact_at(file: &File, mut buf: &mut [u8], mut off: u64) -> io::Result<()> {
    while !buf.is_empty() {
        let n = match file.read_at(buf, off) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
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
        let n = match file.seek_read(buf, off) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
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
        let n = match file.write_at(buf, off) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
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
        let n = match file.seek_write(buf, off) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
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
    let mut header = [0u8; 2048];
    reader.seek(SeekFrom::Start(0))?;
    reader.read_exact(&mut header)?;
    let num_normal_regions = u32::from_be_bytes(header[0..4].try_into().unwrap()) as usize;
    if num_normal_regions == 0 {
        return Err(io::Error::other("invalid region map: zero regions"));
    }
    if num_normal_regions > (header.len() - 8) / 8 {
        return Err(io::Error::other(
            "region map too large for 2048-byte sector",
        ));
    }
    let mut regions = Vec::with_capacity(num_normal_regions - 1);

    let mut previous_end = 0u64;
    for i in 0..num_normal_regions {
        let region_offset = 8 + i * 8;
        let start =
            u32::from_be_bytes(header[region_offset..region_offset + 4].try_into().unwrap()) as u64;
        let inclusive_end = u32::from_be_bytes(
            header[region_offset + 4..region_offset + 8]
                .try_into()
                .unwrap(),
        ) as u64;
        let end = inclusive_end + 1;

        if start >= end || (i > 0 && start < previous_end) {
            return Err(io::Error::other(
                "invalid region map: reversed or overlapping regions",
            ));
        }
        if i > 0 && previous_end < start {
            regions.push(Region {
                start: previous_end,
                end: start,
            });
        }
        previous_end = end;
    }

    Ok(regions)
}
