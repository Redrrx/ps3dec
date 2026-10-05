use clap::Parser;
use log::info;
use std::{
    env,
    io::{self, IsTerminal},
    process::ExitCode,
};

use args::Ps3decargs;
pub use ps3dec::{args, autodetect, logging, utils};
pub use utils::{extract_regions, generate_iv, is_encrypted, read_exact_at, write_all_at};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let mut args = Ps3decargs::parse();
    let bare_paths = env::args_os().len() == args.iso.len() + 1;
    if bare_paths {
        for path in &mut args.iso {
            *path = path.trim_matches(|c| c == '"' || c == '\'').to_owned();
        }
    }
    if args.dk.is_none() && (bare_paths || args.iso.len() != 1) {
        args.auto = true;
    }
    let fullscreen = args.iso.len() != 1
        && io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && io::stderr().is_terminal();
    if args.iso.is_empty() && !fullscreen {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "provide an ISO path, or open PS3Dec in a terminal to use the directory queue",
        ));
    }
    let logs = fullscreen.then(logging::LogBuffer::default);
    let progress = indicatif::MultiProgress::new();
    logging::setup_logging(progress.clone(), logs.clone())
        .map_err(|e| io::Error::other(e.to_string()))?;
    ps3dec::queue::run(&args, &progress, logs.as_ref())?;
    if !args.skip && !fullscreen {
        info!("Job done, press Enter to exit...");

        io::stdin().read_line(&mut String::new())?;
    }
    Ok(())
}
