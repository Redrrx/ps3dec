use chrono::Local;
use indicatif::MultiProgress;
use log::{LevelFilter, Log, Metadata, Record};
use log4rs::{
    append::{
        console::{ConsoleAppender, Target},
        file::FileAppender,
    },
    config::{Appender, Config, Logger, Root},
    encode::pattern::PatternEncoder,
};
use std::collections::VecDeque;
use std::fs;
use std::sync::{Arc, Mutex};

pub type LogBuffer = Arc<Mutex<VecDeque<String>>>;

pub fn setup_logging(
    progress: MultiProgress,
    logs: Option<LogBuffer>,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all("log")?;
    let log_file_name = format!("log/{}.log", Local::now().format("%Y-%m-%d_%H-%M-%S"));
    let fmt = "{d(%Y-%m-%d %H:%M:%S)} [{l}] - {m}\n";
    let logfile = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new(fmt)))
        .build(log_file_name)?;

    let mut config =
        Config::builder().appender(Appender::builder().build("logfile", Box::new(logfile)));
    let mut root = Root::builder().appender("logfile");
    if logs.is_some() {
        config = config
            .logger(Logger::builder().build("mio", LevelFilter::Warn))
            .logger(Logger::builder().build("crossterm", LevelFilter::Warn));
    } else {
        let stdout = ConsoleAppender::builder()
            .target(Target::Stderr)
            .encoder(Box::new(PatternEncoder::new(fmt)))
            .build();
        config = config.appender(Appender::builder().build("stdout", Box::new(stdout)));
        root = root.appender("stdout");
    }
    let logger = log4rs::Logger::new(config.build(root.build(LevelFilter::Trace))?);
    let level = logger.max_log_level();
    log::set_boxed_logger(Box::new(ProgressLogger {
        logger,
        progress,
        logs,
    }))?;
    log::set_max_level(level);
    Ok(())
}

struct ProgressLogger {
    logger: log4rs::Logger,
    progress: MultiProgress,
    logs: Option<LogBuffer>,
}

impl Log for ProgressLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.logger.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if let Some(logs) = &self.logs {
            self.logger.log(record);
            let line = format!(
                "{} [{}] - {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.args()
            );
            let mut logs = logs.lock().unwrap_or_else(|e| e.into_inner());
            if logs.len() == 200 {
                logs.pop_front();
            }
            logs.push_back(line);
        } else {
            self.progress.suspend(|| self.logger.log(record));
        }
    }

    fn flush(&self) {
        self.logger.flush();
    }
}
