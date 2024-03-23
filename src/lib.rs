pub mod autodetect;
pub mod utils;
use aes::cipher::consts::U16;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::KeyIvInit;
use aes::Aes128Dec;
use cbc::Decryptor;
use hex::decode;
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::ThreadPoolBuilder;
use std::fs::File;
use std::io;
use std::io::{BufReader, BufWriter, SeekFrom};
use std::io::{Read, Seek, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use utils::{decrypt_sector, extract_regions, generate_iv, is_encrypted};

type Aes128CbcDec = Decryptor<Aes128Dec>;

pub fn decrypt(file_path: String, decryption_key: &str, thread_count: usize) -> io::Result<()> {
    info!("Starting decryption process.");

    let now = SystemTime::now();
    ThreadPoolBuilder::new()
        .num_threads(thread_count)
        .build_global()
        .unwrap();
    let key_bytes =
        decode(decryption_key).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let key: GenericArray<u8, U16> = GenericArray::clone_from_slice(&key_bytes);

    let input_file = File::open(&file_path)?;
    let metadata = input_file.metadata()?;
    let total_size = metadata.len();
    let total_sectors = total_size / 2048;
    info!(
        "File size: {:.2} MB, Total sectors: {}",
        total_size as f64 / 1_048_576.0,
        total_sectors
    );

    let reader = Arc::new(Mutex::new(BufReader::new(input_file)));

    let output_file_path = format!("{}_decrypted.iso", file_path);
    let output_file = File::create(&output_file_path)?;
    let writer = Arc::new(Mutex::new(BufWriter::new(output_file)));

    let regions = Arc::new(extract_regions(&mut reader.lock().unwrap())?);
    info!("Total regions detected: {}", regions.len());

    let processed_sectors = Arc::new(AtomicUsize::new(0));

    let progress_bar = ProgressBar::new(total_sectors as u64);
    progress_bar.set_style(ProgressStyle::default_bar()
        .template("Going to take ({eta} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos:>7}/{len:7} )")
        .unwrap().progress_chars("#>-"));
    let batch_size = 1024usize;
    let total_batches = (total_sectors as usize + batch_size - 1) / batch_size;
    info!(
        "Processing in batches of {}, Total batches: {}",
        batch_size, total_batches
    );

    rayon::scope(|s| {
        for batch_index in 0..total_batches {
            let reader_clone = Arc::clone(&reader);
            let writer_clone = Arc::clone(&writer);
            let regions_clone = Arc::clone(&regions);
            let processed_sectors_clone = Arc::clone(&processed_sectors);
            let key_clone = key.clone();
            let progress_bar_clone = progress_bar.clone();

            s.spawn(move |_| {
                let start_sector = batch_index * batch_size;
                let end_sector = ((batch_index + 1) * batch_size).min(total_sectors as usize);
                let mut batch_data = vec![0u8; 2048 * (end_sector - start_sector)];

                {
                    let mut rdr = reader_clone.lock().unwrap();
                    rdr.seek(SeekFrom::Start(start_sector as u64 * 2048))
                        .expect("Failed to seek to start of batch");
                    rdr.read_exact(&mut batch_data)
                        .expect("Failed to read batch");
                }

                for sector_offset in 0..(end_sector - start_sector) {
                    let sector_index = start_sector + sector_offset;
                    let sector_data =
                        &mut batch_data[sector_offset * 2048..(sector_offset + 1) * 2048];

                    if is_encrypted(&*regions_clone, sector_index as u64, sector_data) {
                        let iv = generate_iv(sector_index as u64);
                        let mut cipher = Aes128CbcDec::new(&key_clone, &iv);
                        decrypt_sector(&mut cipher, sector_data).expect("Failed to decrypt sector");
                    }
                }

                {
                    let mut wrtr = writer_clone.lock().unwrap();
                    wrtr.seek(SeekFrom::Start(start_sector as u64 * 2048))
                        .expect("Failed to seek in output file");
                    wrtr.write_all(&batch_data)
                        .expect("Failed to write decrypted batch");
                }

                let sectors_processed = end_sector - start_sector;
                processed_sectors_clone.fetch_add(sectors_processed, Ordering::SeqCst);
                progress_bar_clone.inc(sectors_processed as u64);
            });
        }
    });

    progress_bar.finish_with_message("Decryption completed.");
    writer.lock().unwrap().flush()?;

    let elapsed = now.elapsed().expect("Failed to measure time");
    info!("Decryption completed in {} seconds.", elapsed.as_secs());
    info!("Data written to {}.", output_file_path);

    Ok(())
}
