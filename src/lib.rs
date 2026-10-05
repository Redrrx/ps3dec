pub mod args;
pub mod autodetect;
pub mod logging;
pub mod queue;
mod recover;
mod ui;
pub mod utils;

use aes::Aes128Dec;
use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::NoPadding};
use console::{Alignment, Term, measure_text_width, pad_str, style, truncate_str};
use indicatif::{
    HumanBytes, HumanDuration, ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle,
};
use log::info;
use rayon::{ThreadPool, ThreadPoolBuilder, prelude::*};
use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use args::DEFAULT_CHUNK;
use recover::Recovery;
use utils::{extract_regions, generate_iv, is_encrypted};

const SECTOR_SIZE: usize = 2048;

pub fn decrypt(
    file_path: String,
    decryption_key: &str,
    thread_count: usize,
    output_dir: Option<String>,
    output_name: Option<String>,
    chunk_size: Option<usize>,
) -> io::Result<()> {
    let pool = cpu_pool(thread_count)?;
    let input = Path::new(&file_path);
    let output = output_path(input, output_dir.as_deref(), output_name.as_deref());
    let progress = progress_row(input, 1, &progress_style());
    progress.set_draw_target(ProgressDrawTarget::stderr());
    let result = decrypt_file(
        input,
        decryption_key,
        &output,
        chunk_size,
        &pool,
        &progress,
        &AtomicBool::new(false),
    );
    if result.is_err() {
        progress.abandon_with_message(progress_message("Failed"));
    }
    result
}

pub(crate) fn cpu_pool(threads: usize) -> io::Result<ThreadPool> {
    ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()
        .map_err(io::Error::other)
}

pub(crate) fn output_path(input: &Path, directory: Option<&str>, name: Option<&str>) -> PathBuf {
    let filename = match (directory, name) {
        (_, Some(name)) => format!("{name}.iso"),
        (Some(_), None) => {
            let stem = input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("decrypted");
            format!("{stem}_decrypted.iso")
        }
        (None, None) => {
            let mut path = input.as_os_str().to_os_string();
            path.push("_decrypted.iso");
            return PathBuf::from(path);
        }
    };
    directory.map_or_else(
        || PathBuf::from(&filename),
        |dir| Path::new(dir).join(&filename),
    )
}

fn progress_name_width() -> usize {
    usize::from(Term::stderr().size().1)
        .saturating_sub(65)
        .clamp(12, 60)
}

pub(crate) fn progress_style() -> ProgressStyle {
    let name_width = progress_name_width();
    ProgressStyle::with_template(&format!(
        "{{prefix:{name_width}}} │ {{msg}} │ {{bar:16.cyan/black}} {{percent:>3}}% │ {{bytes_per_sec:>12}} │ {{eta:>8}}",
    ))
    .expect("valid progress template")
    .progress_chars("━━─")
    .with_key("eta", |state: &ProgressState, out: &mut dyn std::fmt::Write| {
        let incomplete = state.pos() < state.len().unwrap_or(0);
        if state.per_sec() == 0.0 || (state.is_finished() && incomplete) {
            write!(out, "--").unwrap();
        } else {
            write!(out, "{:#}", HumanDuration(state.eta())).unwrap();
        }
    })
}

pub(crate) fn progress_message(message: &str) -> String {
    let text = pad_str(message, 12, Alignment::Left, Some("…"));
    let text = match message {
        "STATUS" => style(text).bold().dim(),
        "Queued" => style(text).dim(),
        "Failed" => style(text).red().bold(),
        "Done" | "Already done" => style(text).green(),
        "Resuming" | "Pausing" | "Paused" | "Stopped" => style(text).yellow(),
        _ => style(text).cyan(),
    };
    text.to_string()
}

pub(crate) fn progress_row(input: &Path, index: usize, style: &ProgressStyle) -> ProgressBar {
    let progress = ProgressBar::with_draw_target(Some(1), ProgressDrawTarget::hidden());
    let name = input
        .file_name()
        .unwrap_or(input.as_os_str())
        .to_string_lossy();
    let name = format!("{index:>2}  {name}").replace(char::is_control, " ");
    let name_width = progress_name_width();
    let mut width = 0;
    let tail = name
        .char_indices()
        .rev()
        .take_while(|(_, c)| {
            width += measure_text_width(&c.to_string());
            width <= name_width / 3
        })
        .last()
        .map_or("", |(index, _)| &name[index..]);
    progress.set_prefix(truncate_str(&name, name_width, &format!("…{tail}")).into_owned());
    progress.set_message(progress_message("Queued"));
    progress.set_style(style.clone());
    progress
}

pub(crate) fn decrypt_file(
    input_path: &Path,
    decryption_key: &str,
    output: &Path,
    chunk_size: Option<usize>,
    pool: &ThreadPool,
    progress: &ProgressBar,
    stop: &AtomicBool,
) -> io::Result<()> {
    let start = Instant::now();
    let chunk_bytes = chunk_size
        .map(|mib| mib.checked_mul(1024 * 1024).filter(|&size| size > 0))
        .unwrap_or(Some(DEFAULT_CHUNK))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "chunk size must be positive and fit in memory addressing",
            )
        })?;
    let key = hex::decode(decryption_key.trim())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let key: [u8; 16] = key.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "decryption key must be 32 hex characters",
        )
    })?;
    let input = File::open(input_path)?;
    let total_size = input.metadata()?.len();
    if total_size % SECTOR_SIZE as u64 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "input size is not a multiple of 2048",
        ));
    }
    let regions = extract_regions(&mut BufReader::new(input.try_clone()?))?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut recovery = Recovery::open(input_path, &input, output, &key)?;
    progress.set_length(total_size);
    progress.set_position(recovery.offset());
    if recovery.is_complete() {
        progress.finish_with_message(progress_message("Already done"));
        info!("Already decrypted: {}", output.display());
        return Ok(());
    }
    let remaining = total_size - recovery.offset();
    let available = fs2::available_space(parent).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("cannot check free space in {}: {e}", parent.display()),
        )
    })?;
    if available < remaining {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            format!(
                "not enough free space in {}: need {} more, only {} available; free space and reopen PS3Dec to resume",
                parent.display(),
                HumanBytes(remaining),
                HumanBytes(available)
            ),
        ));
    }
    progress.reset_elapsed();
    progress.set_message(progress_message(if recovery.offset() == 0 {
        "Decrypting"
    } else {
        "Resuming"
    }));
    info!(
        "Decrypting {} from byte {}",
        input_path.display(),
        recovery.offset()
    );

    let capacity = remaining.min(chunk_bytes as u64) as usize;
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(capacity).map_err(|e| {
        io::Error::new(
            io::ErrorKind::OutOfMemory,
            format!("cannot allocate {capacity} chunk bytes: {e}"),
        )
    })?;
    let key = (&key).into();
    while recovery.offset() < total_size {
        if stop.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "decryption stopped; output is incomplete and kept for recovery",
            ));
        }
        let offset = recovery.offset();
        let length = (total_size - offset).min(chunk_bytes as u64) as usize;
        buffer.resize(length, 0);
        utils::read_exact_at(&input, &mut buffer, offset).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("read {} at byte {offset}: {e}", input_path.display()),
            )
        })?;
        pool.install(|| {
            buffer
                .par_chunks_mut(SECTOR_SIZE)
                .enumerate()
                .try_for_each(|(i, sector)| {
                    let index = offset / SECTOR_SIZE as u64 + i as u64;
                    if !is_encrypted(&regions, index, sector) {
                        return Ok(());
                    }
                    cbc::Decryptor::<Aes128Dec>::new(key, &generate_iv(index))
                        .decrypt_padded_mut::<NoPadding>(sector)
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("invalid CBC block length in sector {index}"),
                            )
                        })?;
                    Ok::<(), io::Error>(())
                })
        })?;
        utils::write_all_at(recovery.file(), &buffer, offset).map_err(|e| {
            io::Error::new(
                e.kind(),
                if e.kind() == io::ErrorKind::StorageFull {
                    format!(
                        "disk full while writing {}.part at byte {offset}; free space and reopen PS3Dec to resume",
                        output.display()
                    )
                } else {
                    format!(
                        "write {}.part at byte {offset}: {e}; output is incomplete and kept for recovery",
                        output.display()
                    )
                },
            )
        })?;
        recovery.checkpoint(offset + length as u64, &buffer)?;
        progress.set_position(recovery.offset());
    }
    progress.set_message(progress_message("Finishing"));
    recovery.finish()?;
    progress.finish_with_message(progress_message("Done"));
    info!(
        "Decryption completed in {:.2}s: {}",
        start.elapsed().as_secs_f64(),
        output.display()
    );
    Ok(())
}
