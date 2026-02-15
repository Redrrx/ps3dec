pub mod autodetect;
pub mod utils;
use aes::Aes128Dec;
use aes::cipher::generic_array::{GenericArray, typenum::U16};
use aes::cipher::{BlockDecryptMut, KeyInit};
use hex::decode;
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use utils::{extract_regions, generate_iv, is_encrypted};
const SECTOR_SIZE: usize = 2048;

pub const DEFAULT_CHUNK: usize = 16 * 1024 * 1024;

pub fn decrypt(
    file_path: &Path,
    decryption_key: &str,
    num_threads: usize,
    output_dir: Option<&Path>,
    output_name: Option<&Path>,
    chunk_bytes: usize,
) -> io::Result<()> {
    info!("Starting decryption process.");
    let start_time = Instant::now();

    rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads.max(1))
        .build_global()
        .unwrap_or_else(|e| info!("Failed to set thread count, using default: {}", e));

    let key_bytes = decode(decryption_key.trim()).map_err(|e| io::Error::other(e.to_string()))?;
    if key_bytes.len() != 16 {
        return Err(io::Error::other(
            "decryption key must be 16 bytes (32 hex chars)",
        ));
    }
    let key_ga: GenericArray<u8, U16> = GenericArray::clone_from_slice(&key_bytes);

    let input_file = File::open(&file_path)?;
    let total_size = input_file.metadata()?.len();
    if total_size % SECTOR_SIZE as u64 != 0 {
        return Err(io::Error::other(
            "input size is not a multiple of SECTOR_SIZE",
        ));
    }
    let total_sectors = (total_size / SECTOR_SIZE as u64) as usize;

    info!(
        "File size: {:.2} MB, Total sectors: {}",
        total_size as f64 / (1024.0 * 1024.0),
        total_sectors
    );

    let mut region_reader = BufReader::with_capacity(256 * 1024, File::open(&file_path)?);
    let regions = Arc::new(extract_regions(&mut region_reader)?);
    info!("Total regions detected: {}", regions.len());

    let in_file = Arc::new(input_file);
    let output_file_path = if let Some(dir) = output_dir {
        let file_name = if let Some(name) = output_name {
            name.with_extension("iso")
        } else {
            file_path.with_extension("dec.iso")
        };
        let output_dir_path = Path::new(&dir);
        if !output_dir_path.exists() {
            fs::create_dir_all(output_dir_path)?;
        }
        output_dir_path.join(file_name)
    } else if let Some(name) = output_name {
        name.with_extension("iso")
    } else {
        file_path.with_extension("dec.iso")
    };
    info!("Output will be written to: {}", output_file_path.display());

    let out_file =
        OpenOptions::create(OpenOptions::new().write(true), true).open(&output_file_path)?;
    out_file.set_len(total_size)?;
    let out_file = Arc::new(out_file);

    let progress_bar = Arc::new(ProgressBar::new(total_sectors as u64));
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.208} {elapsed_precise} |{bar:40.208/236}| {bytes:>8}/{total_bytes:8} ({bytes_per_sec}) ETA {eta_precise}")
            .unwrap()
            .progress_chars("█▓▒░"),
    );
    let chunk_sectors = (chunk_bytes / SECTOR_SIZE).max(1);
    let tasks: Vec<(usize, usize)> = (0..total_sectors)
        .step_by(chunk_sectors)
        .map(|s| (s, (s + chunk_sectors).min(total_sectors)))
        .collect();

    tasks
        .into_par_iter()
        .for_each(|(start_sector, end_sector)| {
            let mut buf = vec![0u8; SECTOR_SIZE * (end_sector - start_sector)];
            if let Err(e) = utils::read_exact_at(
                &in_file,
                &mut buf,
                (start_sector as u64) * (SECTOR_SIZE as u64),
            ) {
                eprintln!("Error reading data: {}", e);
                return;
            }

            let mut aes_core = Aes128Dec::new(&key_ga);
            for (i, sector) in buf.chunks_mut(SECTOR_SIZE).enumerate() {
                let sector_index = start_sector + i;
                if !is_encrypted(&regions, sector_index as u64, sector) {
                    continue;
                }

                let iv_ga = generate_iv(sector_index as u64);
                let mut prev = [0u8; 16];
                prev.copy_from_slice(iv_ga.as_slice());

                for block in sector.chunks_exact_mut(16) {
                    let mut cur = [0u8; 16];
                    cur.copy_from_slice(block);

                    aes_core.decrypt_block_mut(GenericArray::from_mut_slice(block));
                    for k in 0..16 {
                        block[k] ^= prev[k];
                    }
                    prev = cur;
                }
            }

            let off = (start_sector as u64) * (SECTOR_SIZE as u64);
            if let Err(e) = utils::write_all_at(&out_file, &buf, off) {
                eprintln!("Error writing data at offset {}: {}", off, e);
            }

            progress_bar.inc((end_sector - start_sector) as u64);
        });

    progress_bar.finish_with_message("Decryption completed");

    let elapsed = start_time.elapsed();
    info!(
        "Decryption completed in {:.2} seconds.",
        elapsed.as_secs_f64()
    );
    info!("Data written to {}", output_file_path.display());

    Ok(())
}
