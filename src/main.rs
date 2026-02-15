use clap::{Arg, ArgAction, command, crate_authors, crate_version};
use log::{error, info};
use std::env;
use std::io::{self};
use std::path::PathBuf;
use std::process::exit;
pub mod autodetect;
pub mod utils;

use ps3dec::DEFAULT_CHUNK;

// use crate::args::{DEFAULT_CHUNK, Ps3decargs};
use crate::autodetect::detect_key;
use crate::utils::key_validation;
pub use utils::{
    extract_regions, generate_iv, is_encrypted, read_exact_at, setup_logging, write_all_at,
};

// Either drag and drop which will auto-detect key, OR launch through CLI.
fn main() -> io::Result<()> {
    setup_logging().expect("Failed to setup logging");

    let matches = command!()
        .author(crate_authors!("\n"))
        .version(crate_version!())
        .about("PS3dec Remake is a remake of the original PS3 DISC decryption tool in rust")
        .long_about("PS3Dec is a tool to decrypt PS3 Redump ISOs...")
        .arg(
            Arg::new("iso")
                .help("Path to the PS3 ISO file to decrypt.")
                .required(true),
        )
        .arg(
            Arg::new("decryption_key")
                .short('k')
                .long("decryption-key")
                .help("Decryption key (32 hex).")
                .conflicts_with("auto"),
        )
        .arg(
            Arg::new("num_threads")
                .short('t')
                .long("num_threads")
                .help("Number of threads to use for decryption.")
                .default_value("16"),
        )
        .arg(
            Arg::new("auto")
                .long("auto")
                .help("Autodetect key from ISO name.")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("skip")
                .short('s')
                .long("skip")
                .help("Skip exit confirmation.")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("output_dir")
                .short('o')
                .long("output_dir")
                .help("Output directory."),
        )
        .arg(
            Arg::new("output_name")
                .short('n')
                .long("output_name")
                .help("Output filename (without extension)."),
        )
        .arg(
            Arg::new("chunk_size")
                .short('c')
                .long("chunk_size")
                .help("Chunk size in MiB."), // can't use default_value() because clap is ass
        )
        .get_matches();

    // Can't directly convert to PathBuf because yet again clap is ass.
    let Some(iso_path) = matches.get_one::<String>("iso") else {
        error!("The iso path must be a valid path");
        exit(1);
    };
    let iso_path = PathBuf::from(iso_path);

    let is_iso = iso_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("iso"))
        .unwrap_or(false);
    if !is_iso {
        error!("The file must be an ISO file with .iso extension");
        exit(1);
    }

    let filename = iso_path.file_stem().and_then(|f| f.to_str()).unwrap_or("");
    info!("ISO path: {}", iso_path.display());
    if let Ok(c) = iso_path.canonicalize() {
        info!("Canonical path: {}", c.display());
    }
    info!("Filename used for key lookup: {}", filename);

    let decryption_key = if matches.get_flag("auto") {
        if let Ok(Some(key)) = detect_key(filename) {
            info!("Auto-detected key for {}: {}", filename, key);
            key
        } else {
            error!("No key could be auto-detected for {}", filename);
            exit(1);
        }
    } else if let Some(key) = matches.get_one::<String>("decryption_key") {
        if key_validation(&key) {
            info!("Using provided decryption key: {}", key);
            key.clone()
        } else {
            error!("Invalid PS3 decryption key format.");
            exit(1);
        }
    } else {
        error!("Decryption key is required unless '--auto' is specified.");
        exit(1);
    };

    let num_threads = matches.get_one::<String>("num_threads").unwrap();
    let num_threads = usize::from_str_radix(num_threads, 10).unwrap();
    let output_dir = matches
        .get_one::<PathBuf>("output_dir")
        .map(|f| f.as_path());
    let output_name = matches
        .get_one::<PathBuf>("output_name")
        .map(|f| f.as_path());

    let chunk_bytes = match matches.get_one::<String>("chunk_size") {
        Some(s) => usize::from_str_radix(s, 10)
            .map(|mib| mib.saturating_mul(1024 * 1024))
            .unwrap(),
        None => DEFAULT_CHUNK,
    };

    ps3dec::decrypt(
        &iso_path,
        &decryption_key,
        num_threads,
        output_dir,
        output_name,
        chunk_bytes,
    )?;

    if !matches.get_flag("skip") {
        info!("Job done, press any button to exit...");
        let mut input_string = String::new();
        io::stdin()
            .read_line(&mut input_string)
            .expect("Failed to read line");
        info!("Ciao!");
    }

    Ok(())
}
