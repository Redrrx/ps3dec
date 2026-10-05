use atomicwrites::{AllowOverwrite, AtomicFile, move_atomic};
use fs2::FileExt;
use same_file::Handle;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::utils::read_exact_at;

#[derive(Deserialize, Serialize, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
struct Source {
    path: PathBuf,
    size: u64,
    modified: (u64, u32),
    header: String,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
struct Chunk {
    start: u64,
    digest: String,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    source: Source,
    output: PathBuf,
    key: String,
    next_offset: u64,
    chunk: Option<Chunk>,
}

pub(crate) struct Recovery {
    input: File,
    file: File,
    part: PathBuf,
    resume: PathBuf,
    record: Record,
    complete: bool,
}

impl Recovery {
    pub(crate) fn open(
        input_path: &Path,
        input: &File,
        output: &Path,
        key: &[u8],
    ) -> io::Result<Self> {
        Self::open_inner(input_path, input, output, key).map_err(|e| context(output, e))
    }

    fn open_inner(input_path: &Path, input: &File, output: &Path, key: &[u8]) -> io::Result<Self> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let output = fs::canonicalize(parent)?.join(
            output
                .file_name()
                .ok_or_else(|| invalid("output has no filename"))?,
        );
        let part = append(&output, ".part");
        let resume = append(&output, ".resume");
        let has_part = regular(&part)?;
        let has_final = regular(&output)?;
        let has_resume = regular(&resume)?;
        let source = source(&fs::canonicalize(input_path)?, input)?;
        let key = hex::encode(Sha256::digest(key));
        if !has_part && !has_final && has_resume {
            return Err(invalid("checkpoint exists without an output"));
        }
        let fresh = !has_part && !has_final;
        let path = if has_final { &output } else { &part };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(fresh)
            .open(path)?;
        // Lock the data inode before reading or changing its checkpoint.
        FileExt::try_lock_exclusive(&file)?;
        verify_path(path, &file)?;
        if same_file(&file, input)? {
            return Err(invalid("output is a hardlink to the input"));
        }
        // Recheck names after acquiring the lock: another writer may have published.
        if regular(&part)? != (has_part || fresh) || regular(&output)? != has_final {
            return Err(invalid(
                "output paths changed while acquiring the lock; retry",
            ));
        }
        if regular(&resume)? && fresh {
            return Err(invalid(
                "checkpoint appeared while creating the partial output",
            ));
        }
        let record = if fresh {
            Record {
                version: 1,
                source,
                output,
                key,
                next_offset: 0,
                chunk: None,
            }
        } else {
            let record: Record = serde_json::from_reader(File::open(&resume)?.take(64 * 1024))
                .map_err(|e| invalid(format!("invalid checkpoint {}: {e}", resume.display())))?;
            if record.version != 1
                || record.source != source
                || record.output != output
                || record.key != key
            {
                return Err(invalid(
                    "checkpoint does not match source, output, version, or key",
                ));
            }
            record
        };
        let recovery = Self {
            input: input.try_clone()?,
            file,
            part,
            resume,
            record,
            complete: has_final,
        };
        recovery.validate_output()?;
        if has_final && has_part {
            let alias = File::open(&recovery.part)?;
            if !same_file(&recovery.file, &alias)? {
                return Err(invalid("both partial and final outputs exist"));
            }
            verify_path(&recovery.part, &alias)?;
            // move_atomic may have linked the destination before unlinking the source.
            fs::remove_file(&recovery.part)?;
        }
        if !has_final {
            recovery.file.set_len(recovery.offset())?;
        }
        recovery.file.sync_all()?;
        recovery.check_source()?;
        // Also makes a recovered publication (and alias removal) directory-durable.
        recovery.save(&recovery.record)?;
        Ok(recovery)
    }

    pub(crate) fn file(&self) -> &File {
        &self.file
    }
    pub(crate) fn offset(&self) -> u64 {
        self.record.next_offset
    }
    pub(crate) fn is_complete(&self) -> bool {
        self.complete
    }

    pub(crate) fn checkpoint(
        &mut self,
        next_offset: u64,
        plaintext_chunk: &[u8],
    ) -> io::Result<()> {
        self.commit(next_offset, plaintext_chunk)
            .map_err(|e| context(&self.resume, e))
    }

    fn commit(&mut self, next_offset: u64, plaintext_chunk: &[u8]) -> io::Result<()> {
        if self.complete
            || next_offset <= self.offset()
            || next_offset > self.record.source.size
            || !next_offset.is_multiple_of(2048)
            || next_offset - self.offset() != plaintext_chunk.len() as u64
        {
            return Err(invalid("invalid checkpoint offset or chunk length"));
        }
        self.check_source()?;
        verify_path(&self.part, &self.file)?;
        if regular(&self.record.output)? {
            return Err(invalid("final output appeared during recovery"));
        }
        let len = self.file.metadata()?.len();
        if len < next_offset || len > self.record.source.size {
            return Err(invalid(
                "output length does not contain the committed chunk",
            ));
        }
        let chunk = Chunk {
            start: self.offset(),
            digest: hex::encode(Sha256::digest(plaintext_chunk)),
        };

        self.file.sync_all()?;
        self.check_source()?;
        let mut record = self.record.clone();
        record.next_offset = next_offset;
        record.chunk = Some(chunk);
        self.save(&record)?;
        self.record = record;
        Ok(())
    }

    pub(crate) fn finish(self) -> io::Result<()> {
        let output = self.record.output.clone();
        self.publish().map_err(|e| context(&output, e))
    }

    fn publish(self) -> io::Result<()> {
        self.check_source()?;
        if self.offset() != self.record.source.size {
            return Err(invalid("cannot publish an incomplete output"));
        }
        self.validate_output()?;
        self.file.sync_all()?;
        if self.complete {
            verify_path(&self.record.output, &self.file)?;
            return self.save(&self.record);
        }
        verify_path(&self.part, &self.file)?;
        if regular(&self.record.output)? {
            return Err(invalid("final output already exists"));
        }
        move_atomic(&self.part, &self.record.output)?;
        self.save(&self.record)
    }

    fn validate_output(&self) -> io::Result<()> {
        let offset = self.offset();
        let len = self.file.metadata()?.len();
        if !offset.is_multiple_of(2048)
            || offset > self.record.source.size
            || len < offset
            || len > self.record.source.size
        {
            return Err(invalid("invalid checkpoint offset or truncated output"));
        }
        if self.complete && (offset != self.record.source.size || len != offset) {
            return Err(invalid("final output does not have a complete checkpoint"));
        }
        let Some(chunk) = &self.record.chunk else {
            if offset != 0 {
                return Err(invalid("checkpoint is missing its committed chunk"));
            }
            return Ok(());
        };
        if chunk.start % 2048 != 0
            || chunk.start >= offset
            || hash_range(&self.file, chunk.start, offset - chunk.start)? != chunk.digest
        {
            return Err(invalid("last committed chunk is invalid or has changed"));
        }
        Ok(())
    }

    fn check_source(&self) -> io::Result<()> {
        if source(&self.record.source.path, &self.input)? != self.record.source {
            return Err(invalid("source changed during recovery"));
        }
        Ok(())
    }

    fn save(&self, record: &Record) -> io::Result<()> {
        regular(&self.resume)?;
        AtomicFile::new(&self.resume, AllowOverwrite)
            .write(|file| -> io::Result<()> {
                serde_json::to_writer(file, record).map_err(|e| {
                    io::Error::new(e.io_error_kind().unwrap_or(io::ErrorKind::InvalidData), e)
                })
            })
            .map_err(|e| context(&self.resume, io::Error::from(e)))
    }
}

fn source(path: &Path, file: &File) -> io::Result<Source> {
    verify_path(path, file)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() % 2048 != 0 {
        return Err(invalid("source must be a regular, sector-aligned file"));
    }
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    let header = hash_range(file, 0, metadata.len().min(64 * 1024))?;
    let after = file.metadata()?;
    if after.len() != metadata.len() || after.modified()? != metadata.modified()? {
        return Err(invalid("source changed while checking its header"));
    }
    Ok(Source {
        path: path.to_path_buf(),
        size: metadata.len(),
        modified: (modified.as_secs(), modified.subsec_nanos()),
        header,
    })
}

fn hash_range(file: &File, mut start: u64, mut len: u64) -> io::Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0; 8192];
    while len != 0 {
        let count = len.min(buffer.len() as u64) as usize;
        read_exact_at(file, &mut buffer[..count], start)?;
        hash.update(&buffer[..count]);
        start += count as u64;
        len -= count as u64;
    }
    Ok(hex::encode(hash.finalize()))
}

fn regular(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(invalid(format!(
            "refusing non-regular path {}",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(context(path, e)),
    }
}

fn verify_path(path: &Path, file: &File) -> io::Result<()> {
    if !regular(path)? || Handle::from_path(path)? != Handle::from_file(file.try_clone()?)? {
        return Err(invalid(format!(
            "file identity changed: {}",
            path.display()
        )));
    }
    Ok(())
}

fn same_file(a: &File, b: &File) -> io::Result<bool> {
    Ok(Handle::from_file(a.try_clone()?)? == Handle::from_file(b.try_clone()?)?)
}

fn append(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn context(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        if error.kind() == io::ErrorKind::StorageFull {
            format!(
                "disk full while saving recovery data for {}; free space and reopen PS3Dec to resume",
                path.display()
            )
        } else {
            format!("recovery {}: {error}", path.display())
        },
    )
}
