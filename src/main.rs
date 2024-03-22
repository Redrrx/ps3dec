pub mod args;
pub mod autodetect;
pub mod utils;
use crate::autodetect::detect_key;
use crate::utils::key_validation;
use args::Ps3decargs;
use chrono::Local;
use clap::Parser;
use log::{debug, error, info, LevelFilter};
use log4rs::{
    append::{console::ConsoleAppender, file::FileAppender},
    config::{Appender, Config, Logger, Root},
    encode::pattern::PatternEncoder,
};

use ps3decremake::decrypt;
use std::fs::File;
use std::io::Write;
use std::io::{self};
use std::path::Path;
use std::{env, fs};

fn setup_logging() -> Result<(), Box<dyn std::error::Error>> {
    let log_dir = Path::new("log");
    fs::create_dir_all(log_dir)?;
    let now = Local::now();
    let log_file_name = format!("log/{}.log", now.format("%Y-%m-%d_%H-%M-%S"));

    let stdout = ConsoleAppender::builder()
        .encoder(Box::new(PatternEncoder::new("{d} [{l}] - {m}\n")))
        .build();

    let logfile = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new("{d} [{l}] - {m}\n")))
        .build(log_file_name)?;

    let config = Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .build(
            Root::builder()
                .appender("stdout")
                .appender("logfile")
                .build(LevelFilter::Trace),
        )?;

    log4rs::init_config(config)?;

    info!("Logging is up.");

    Ok(())
}

// Either drag and drop which will auto detect key, OR launch through CLI.
fn main() -> io::Result<()> {
    env::set_var("RUST_LOG", "info");
    setup_logging().expect("yikes");
    let args: Vec<String> = env::args().collect();
    println!("{:?}", &args);
    if args.len() == 2 {
        let maybe_dragdrop_path = env::args().nth(1);
        if let Some(dragdrop) = maybe_dragdrop_path {
            let path = Path::new(&dragdrop);
            info!("{:?}", &path);
            if dragdrop.ends_with(".iso") {
                let filename_str = path.file_stem().and_then(|f| f.to_str()).unwrap_or("");
                debug!("Received drag-and-drop file name: {}", filename_str);
                debug!("{:?}", &dragdrop);
                if let Ok(Some(key)) = detect_key(filename_str.to_string()) {
                    decrypt(dragdrop, &key, 64)?;
                } else {
                    error!("No key found for {}", filename_str);
                }
            }
        }
    } else {
        let args = Ps3decargs::parse();
        if args.auto {
            let split = &args.iso.split(".iso").next().unwrap_or("");
            if let Ok(Some(key)) = detect_key(split.to_string()) {
                decrypt(args.iso, &key, args.tc)?;
            }
        } else {
            if let Some(dk) = args.dk {
                if key_validation(&dk) {
                    decrypt(args.iso, &dk, args.tc)?;
                } else {
                    error!("Error: Invalid PS3 decryption key format.");
                }
            } else {
                error!("Error: Decryption key is required unless '--auto' is specified.");
            }
        }
    }

    info!("Job done, press any button to exit...");
    let mut input_string = String::new();
    io::stdin()
        .read_line(&mut input_string)
        .expect("Failed to read line");
    info!("Ciao!");
    Ok(())
}
