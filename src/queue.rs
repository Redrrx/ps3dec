use crate::args::Ps3decargs;
use crate::{
    autodetect::detect_key, cpu_pool, decrypt_file, logging::LogBuffer, output_path,
    progress_message, progress_row, progress_style, ui,
};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressState};
use log::{error, info};
use std::collections::HashSet;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub(crate) const REFRESH_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Waiting,
    Queued,
    Running,
    Pausing,
    Paused,
    Finished,
}

#[derive(Clone)]
pub(crate) struct Job {
    pub id: usize,
    pub input: PathBuf,
    pub progress: ProgressBar,
    state: State,
    stop: Arc<AtomicBool>,
    order: usize,
    failure: Option<io::ErrorKind>,
}

impl Job {
    fn new(input: PathBuf, index: usize, state: State) -> Self {
        let progress = progress_row(&input, index + 1, &progress_style());
        if state == State::Waiting {
            progress.set_message(progress_message("Waiting"));
        }
        Self {
            id: index,
            input,
            progress,
            state,
            stop: Arc::new(AtomicBool::new(false)),
            order: index,
            failure: None,
        }
    }

    pub fn active(&self) -> bool {
        matches!(self.state, State::Running | State::Pausing)
    }

    pub fn paused(&self) -> bool {
        self.state == State::Paused
    }

    fn movable(&self) -> bool {
        !self.active() && !matches!(self.status().as_str(), "Done" | "Already done")
    }

    pub fn status(&self) -> String {
        if self.state == State::Pausing {
            return "Pausing".to_owned();
        }
        if self.active() && self.stop.load(Ordering::Relaxed) {
            return "Stopping".to_owned();
        }
        console::strip_ansi_codes(&self.progress.message())
            .trim()
            .to_owned()
    }
}

pub(crate) struct Queue {
    pub directory: PathBuf,
    jobs: Mutex<Vec<Job>>,
    shutdown: AtomicBool,
}

impl Queue {
    fn new(args: &Ps3decargs) -> io::Result<Self> {
        let jobs = args
            .iso
            .iter()
            .enumerate()
            .map(|(index, input)| Job::new(PathBuf::from(input), index, State::Queued))
            .collect();
        Ok(Self {
            directory: std::env::current_dir()?,
            jobs: Mutex::new(jobs),
            shutdown: AtomicBool::new(false),
        })
    }

    pub fn snapshot(&self) -> Vec<Job> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        jobs.sort_by_key(|job| job.order);
        jobs
    }

    pub fn idle(&self) -> bool {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .all(|job| matches!(job.state, State::Waiting | State::Paused | State::Finished))
    }

    pub fn shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    pub fn start(&self, index: usize) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let Some(job) = jobs.get(index) else {
            return;
        };
        if self.shutting_down()
            || job.active()
            || matches!(job.status().as_str(), "Done" | "Already done")
        {
            return;
        }
        let job = &mut jobs[index];
        if matches!(job.state, State::Paused | State::Finished) {
            let position = job.progress.position();
            job.progress.reset();
            job.progress.set_position(position);
        }
        job.stop.store(false, Ordering::Relaxed);
        job.state = State::Queued;
        job.progress.set_message(progress_message("Queued"));
    }

    pub fn toggle_pause(&self, index: usize) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if self.shutting_down() {
            return;
        }
        let Some(job) = jobs.get_mut(index) else {
            return;
        };
        match job.state {
            State::Running if !job.progress.is_finished() => {
                job.state = State::Pausing;
                job.stop.store(true, Ordering::Relaxed);
            }
            State::Waiting | State::Queued => {
                job.state = State::Paused;
                job.progress
                    .abandon_with_message(progress_message("Paused"));
            }
            State::Paused => {
                drop(jobs);
                self.start(index);
            }
            _ => {}
        }
    }

    pub fn move_job(&self, index: usize, up: bool) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let Some(job) = jobs.get(index) else {
            return;
        };
        if self.shutting_down() || !job.movable() {
            return;
        }
        let order = job.order;
        let neighbor = jobs
            .iter()
            .enumerate()
            .filter(|(_, job)| {
                job.movable()
                    && if up {
                        job.order < order
                    } else {
                        job.order > order
                    }
            })
            .min_by_key(|(_, job)| job.order.abs_diff(order))
            .map(|(index, _)| index);
        if let Some(neighbor) = neighbor {
            jobs[index].order = jobs[neighbor].order;
            jobs[neighbor].order = order;
        }
    }

    pub fn cancel(&self, index: usize) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let Some(job) = jobs.get_mut(index) else {
            return;
        };
        match job.state {
            State::Running | State::Pausing if !job.progress.is_finished() => {
                job.state = State::Running;
                job.stop.store(true, Ordering::Relaxed);
            }
            State::Waiting | State::Queued | State::Paused => {
                job.state = State::Finished;
                job.progress
                    .abandon_with_message(progress_message("Stopped"));
            }
            _ => {}
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        for job in jobs.iter_mut() {
            match job.state {
                State::Running | State::Pausing => {
                    job.state = State::Running;
                    job.stop.store(true, Ordering::Relaxed);
                }
                State::Queued => {
                    job.state = State::Finished;
                    job.progress
                        .abandon_with_message(progress_message("Stopped"));
                }
                _ => {}
            }
        }
    }

    fn next(&self) -> Option<(usize, Job)> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if self.shutting_down() {
            return None;
        }
        let index = jobs
            .iter()
            .enumerate()
            .filter(|(_, job)| job.state == State::Queued)
            .min_by_key(|(_, job)| job.order)
            .map(|(index, _)| index)?;
        let job = &mut jobs[index];
        job.state = State::Running;
        job.progress.set_message(progress_message("Starting"));
        Some((index, job.clone()))
    }

    fn finish(&self, index: usize, result: io::Result<()>) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let job = &mut jobs[index];
        if job.state == State::Pausing
            && job.stop.load(Ordering::Relaxed)
            && result
                .as_ref()
                .is_err_and(|e| e.kind() == io::ErrorKind::Interrupted)
        {
            job.state = State::Paused;
            job.progress
                .abandon_with_message(progress_message("Paused"));
            let message = format!("Paused: {}", job.input.display());
            drop(jobs);
            info!("{message}");
            return;
        }
        job.state = State::Finished;
        job.failure = result.as_ref().err().map(io::Error::kind);
        if let Err(e) = result {
            let stopped =
                e.kind() == io::ErrorKind::Interrupted && job.stop.load(Ordering::Relaxed);
            job.progress
                .abandon_with_message(progress_message(if stopped { "Stopped" } else { "Failed" }));
            let message = format!("{}: {e}", job.input.display());
            drop(jobs);
            error!("{message}");
        }
    }

    pub fn discover(&self, args: &Ps3decargs) -> io::Result<()> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let kind = match entry.file_type() {
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                result => result?,
            };
            let path = entry.path();
            if kind.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("iso"))
                && !entry
                    .file_name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .ends_with("_decrypted.iso")
            {
                paths.push(path);
            }
        }
        paths.sort();
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        for input in paths {
            let source = normalized(&input);
            let output = normalized(&output_path(&input, args.output_dir.as_deref(), None));
            if jobs.iter().any(|job| {
                normalized(&job.input) == source
                    || normalized(&output_path(&job.input, args.output_dir.as_deref(), None))
                        == output
            }) {
                continue;
            }
            let index = jobs.len();
            jobs.push(Job::new(input, index, State::Waiting));
        }
        Ok(())
    }
}

pub fn run(
    args: &Ps3decargs,
    progress: &MultiProgress,
    logs: Option<&LogBuffer>,
) -> io::Result<()> {
    if args.iso.len() != 1 && (args.dk.is_some() || args.output_name.is_some()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--dk and --output-name require a single ISO; batches look up each key separately",
        ));
    }
    if args.tc == 0 || args.jobs == 0 || args.chunk_size == Some(0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "thread, job, and chunk counts must be positive",
        ));
    }
    if let Some(dir) = &args.output_dir {
        fs::create_dir_all(dir)?;
    }
    check_paths(args)?;
    let pool = cpu_pool(args.tc)?;

    if let Some(logs) = logs {
        progress.set_draw_target(ProgressDrawTarget::hidden());
        return run_tui(args, &pool, logs);
    }
    let interactive = io::stderr().is_terminal() && logs.is_none();
    let style = progress_style(); 
    let _header = interactive.then(|| {
        let header = ProgressBar::hidden();
        header.set_prefix(" #  ISO");
        header.set_message(progress_message("STATUS"));
        header.set_style(
            style
                .clone()
                .with_key("bar", |_: &ProgressState, out: &mut dyn std::fmt::Write| {
                    write!(out, "{:<16}", "PROGRESS").unwrap();
                })
                .with_key(
                    "percent",
                    |_: &ProgressState, _: &mut dyn std::fmt::Write| {},
                )
                .with_key(
                    "bytes_per_sec",
                    |_: &ProgressState, out: &mut dyn std::fmt::Write| {
                        write!(out, "SPEED").unwrap();
                    },
                )
                .with_key("eta", |_: &ProgressState, out: &mut dyn std::fmt::Write| {
                    write!(out, "ETA").unwrap();
                }),
        );
        let header = progress.add(header);
        header.finish();
        header
    });
    let rows: Vec<_> = args
        .iso
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let input = PathBuf::from(name);
            let bar = progress.add(progress_row(&input, index + 1, &style));
            bar.tick();
            (input, bar)
        })
        .collect();
    let stop = AtomicBool::new(false);
    let plain = !io::stderr().is_terminal();
    let errors = process_queue(args, &rows, &pool, &stop, plain);
    if !errors.is_empty() || stop.load(Ordering::Relaxed) {
        progress.clear()?;
    }
    if stop.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "decryption stopped; reopen PS3Dec with the same ISOs to resume unfinished files automatically",
        ));
    }
    if errors.is_empty() {
        return Ok(());
    }

    Err(io::Error::other(format!(
        "{} {} failed; reopen PS3Dec with the same ISOs to resume any unfinished files",
        errors.len(),
        if errors.len() == 1 { "ISO" } else { "ISOs" }
    )))
}

fn run_tui(args: &Ps3decargs, pool: &rayon::ThreadPool, logs: &LogBuffer) -> io::Result<()> {
    let queue = Queue::new(args)?;
    let count = if args.sequential { 1 } else { args.jobs };
    thread::scope(|scope| {
        let workers: Vec<_> = (0..count)
            .map(|_| {
                scope.spawn(|| {
                    while !queue.shutting_down() {
                        let Some((index, job)) = queue.next() else {
                            thread::sleep(REFRESH_INTERVAL);
                            continue;
                        };
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            run_file(&job.input, args, pool, &job.progress, &job.stop)
                        }))
                        .unwrap_or_else(|_| {
                            queue.shutdown();
                            Err(io::Error::other("decryption worker panicked"))
                        });
                        queue.finish(index, result);
                    }
                })
            })
            .collect();
        let display = ui::run(args, &queue, logs);
        queue.shutdown();
        let mut panicked = false;
        for worker in workers {
            if worker.join().is_err() {
                panicked = true;
            }
        }
        display?;
        if panicked {
            return Err(io::Error::other("decryption worker panicked"));
        }
        Ok(())
    })?;
    let rows = queue.snapshot();
    if rows
        .iter()
        .any(|job| job.failure == Some(io::ErrorKind::Interrupted))
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "decryption stopped; reopen PS3Dec with the same ISOs to resume unfinished files automatically",
        ));
    }
    let failures = rows.iter().filter(|job| job.failure.is_some()).count();
    if failures > 0 {
        return Err(io::Error::other(format!(
            "{failures} {} failed; reopen PS3Dec with the same ISOs to resume any unfinished files",
            if failures == 1 { "ISO" } else { "ISOs" }
        )));
    }
    Ok(())
}

fn process_queue(
    args: &Ps3decargs,
    rows: &[(PathBuf, ProgressBar)],
    pool: &rayon::ThreadPool,
    stop: &AtomicBool,
    plain: bool,
) -> Vec<String> {
    let next = AtomicUsize::new(0);
    let jobs = if args.sequential { 1 } else { args.jobs }.min(rows.len());
    let errors = thread::scope(|scope| {
        let workers: Vec<_> = (0..jobs)
            .map(|_| {
                scope.spawn(|| {
                    let mut errors = Vec::new();
                    while !stop.load(Ordering::Relaxed) {
                        let Some((input, bar)) = rows.get(next.fetch_add(1, Ordering::Relaxed))
                        else {
                            break;
                        };
                        if plain {
                            eprintln!("{}: starting", input.display());
                        }
                        match run_file(input, args, pool, bar, stop) {
                            Err(e) => {
                                let stopped = e.kind() == io::ErrorKind::Interrupted
                                    && stop.load(Ordering::Relaxed);
                                bar.abandon_with_message(progress_message(if stopped {
                                    "Stopped"
                                } else {
                                    "Failed"
                                }));
                                let message = format!("{}: {e}", input.display());
                                error!("{message}");
                                errors.push(message);
                            }
                            Ok(()) if plain => eprintln!("{}: {}", input.display(), bar.message()),
                            Ok(()) => {
                                // we good
                            }
                        }
                    }
                    errors
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker.join().unwrap_or_else(|_| {
                    error!("decryption worker panicked");
                    vec!["decryption worker panicked".to_owned()]
                })
            })
            .collect()
    });
    for (_, bar) in rows.iter().filter(|(_, bar)| !bar.is_finished()) {
        bar.abandon_with_message(progress_message(if stop.load(Ordering::Relaxed) {
            "Stopped"
        } else {
            "Failed"
        }));
    }
    errors
}

fn run_file(
    input: &Path,
    args: &Ps3decargs,
    pool: &rayon::ThreadPool,
    progress: &indicatif::ProgressBar,
    stop: &AtomicBool,
) -> io::Result<()> {
    if !input
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("iso"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "input must have an .iso extension",
        ));
    }
    let metadata = fs::metadata(input).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            let message = if input.is_relative() {
                let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                format!(
                    "file not found (current directory: {}); use an absolute path or change to the ISO directory",
                    directory.display()
                )
            } else {
                "file not found".to_owned()
            };
            io::Error::new(e.kind(), message)
        } else {
            e
        }
    })?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "input must be a regular file",
        ));
    }
    info!("ISO path: {}", input.display());
    let key = key_for(input, args)?;
    let output = output_path(
        input,
        args.output_dir.as_deref(),
        args.output_name.as_deref(),
    );
    decrypt_file(input, &key, &output, args.chunk_size, pool, progress, stop)
}

fn key_for(input: &Path, args: &Ps3decargs) -> io::Result<String> {
    if let Some(key) = &args.dk {
        return Ok(key.clone());
    }
    if !args.auto {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a key is required; use --dk or --auto",
        ));
    }
    let name = input.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "input filename is not valid UTF-8",
        )
    })?;
    detect_key(name.to_owned())
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no matching key in keys/"))
}

fn check_paths(args: &Ps3decargs) -> io::Result<()> {
    let mut inputs = HashSet::new();
    let mut outputs = HashSet::new();
    for name in &args.iso {
        let input = Path::new(name);
        let output = output_path(
            input,
            args.output_dir.as_deref(),
            args.output_name.as_deref(),
        );
        if !inputs.insert(normalized(input)) || !outputs.insert(normalized(&output)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("duplicate input or output path: {}", input.display()),
            ));
        }
    }
    Ok(())
}

fn normalized(path: &Path) -> PathBuf {
    if let Ok(path) = fs::canonicalize(path) {
        return path;
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    parent.join(path.file_name().unwrap_or(path.as_os_str()))
}
