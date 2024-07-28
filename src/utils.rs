use aes::cipher::consts::U16;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::BlockDecryptMut;
use aes::Aes128Dec;
use cbc::Decryptor;
use log::{info, warn};
use std::fs::File;
use std::io;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::sync::MutexGuard;

type Aes128CbcDec = Decryptor<Aes128Dec>;

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

    if !stripped_key.chars().all(|c| c.is_digit(16)) {
        println!("Key contains invalid characters");
        warn!("Key contains invalid characters");
        return false;
    }

    info!("Key is valid");
    true
}

// Making an init vector
pub fn generate_iv(sector: u64) -> GenericArray<u8, U16> {
    let mut iv_bytes = [0u8; 16];
    iv_bytes[12] = ((sector & 0xFF000000) >> 24) as u8;
    iv_bytes[13] = ((sector & 0x00FF0000) >> 16) as u8;
    iv_bytes[14] = ((sector & 0x0000FF00) >> 8) as u8;
    iv_bytes[15] = (sector & 0x000000FF >> 0) as u8;
    GenericArray::clone_from_slice(&iv_bytes)
}

pub fn is_encrypted(regions: &[Region], sector: u64, sector_data: &[u8]) -> bool {
    if sector_data.iter().all(|&b| b == 0) {
        return false;
    }
    regions.iter().any(|r| sector >= r.start && sector < r.end)
}

pub fn decrypt_sector(cipher: &mut Aes128CbcDec, sector_data: &mut [u8]) -> io::Result<()> {
    for chunk in sector_data.chunks_exact_mut(16) {
        cipher.decrypt_block_mut(GenericArray::from_mut_slice(chunk));
    }
    Ok(())
}

// Splitting the cake
pub fn extract_regions(reader: &mut MutexGuard<BufReader<File>>) -> io::Result<Vec<Region>> {
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
