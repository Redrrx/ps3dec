pub mod autodetect;
pub mod utils;

use aes::cipher::{consts::U16, generic_array::GenericArray, KeyIvInit};
use aes::Aes128Dec;
use cbc::Decryptor;
use hex::decode;
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use utils::{decrypt_sector, extract_regions, generate_iv, is_encrypted};

const SECTOR_SIZE: usize = 2048;
const CHUNK_SIZE: usize = 8 * 1024 * 1024;

type Aes128CbcDec = Decryptor<Aes128Dec>;


trait WriteAllAt: Write + Seek {
    fn write_all_at(&mut self, buf: &[u8], offset: u64) -> io::Result<()> {
        self.seek(SeekFrom::Start(offset))?;
        self.write_all(buf)
    }
}

impl WriteAllAt for File {}

pub fn decrypt(file_path: String, decryption_key: &str, thread_count: usize) -> io::Result<()> {
    info!("Starting decryption process.");
    let start_time = Instant::now();

    rayon::ThreadPoolBuilder::new().num_threads(thread_count).build_global().unwrap();

    let key_bytes = decode(decryption_key).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let key: GenericArray<u8, U16> = GenericArray::clone_from_slice(&key_bytes);

    let input_file = File::open(&file_path)?;
    let total_size = input_file.metadata()?.len();
    let total_sectors = total_size / SECTOR_SIZE as u64;

    info!("File size: {:.2} MB, Total sectors: {}", total_size as f64 / 1_048_576.0, total_sectors);

    let reader = Arc::new(Mutex::new(BufReader::with_capacity(CHUNK_SIZE, input_file)));
    let output_file_path = format!("{}_decrypted.iso", file_path);
    let output_file = Arc::new(Mutex::new(OpenOptions::new().write(true).create(true).open(&output_file_path)?));

    let regions = Arc::new(extract_regions(&mut reader.lock().unwrap())?);
    info!("Total regions detected: {}", regions.len());

    let progress_bar = Arc::new(ProgressBar::new(total_sectors));
    progress_bar.set_style(ProgressStyle::default_bar()
        .template("Estimated time left: {eta} [{bar:40.cyan/blue}] {pos:>7}/{len:7} sectors ({percent}%)")
        .unwrap()
        .progress_chars("=>-"));

    let chunk_size = CHUNK_SIZE / SECTOR_SIZE;
    let sectors: Vec<u64> = (0..total_sectors).collect();
    let chunks = sectors.chunks(chunk_size);

    chunks.par_bridge().for_each(|chunk| {
        let start_sector = *chunk.first().unwrap() as usize;
        let end_sector = (*chunk.last().unwrap() as usize + 1).min(total_sectors as usize);
        let mut chunk_data = vec![0u8; SECTOR_SIZE * (end_sector - start_sector)];

        {
            let mut reader = reader.lock().unwrap();
            reader.seek(SeekFrom::Start(start_sector as u64 * SECTOR_SIZE as u64)).unwrap();
            reader.read_exact(&mut chunk_data).unwrap();
        }

        chunk_data.par_chunks_mut(SECTOR_SIZE).enumerate().for_each(|(offset, sector_data)| {
            let sector_index = start_sector + offset;
            if is_encrypted(&regions, sector_index as u64, sector_data) {
                let iv = generate_iv(sector_index as u64);
                let mut cipher = Aes128CbcDec::new(&key, &iv);
                decrypt_sector(&mut cipher, sector_data).unwrap();
            }
        });

        let offset = start_sector as u64 * SECTOR_SIZE as u64;
        output_file.lock().unwrap().write_all_at(&chunk_data, offset).unwrap();

        progress_bar.inc((end_sector - start_sector) as u64);
    });

    progress_bar.finish_with_message("Decryption completed");

    let elapsed = start_time.elapsed();
    info!("Decryption completed in {:.2} seconds.", elapsed.as_secs_f64());
    info!("Data written to {}", output_file_path);

    Ok(())
}