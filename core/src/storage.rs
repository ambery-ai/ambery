//! Storage：append-only JSONL，启动 replay 恢复。

use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct JsonlStore {
    dir: PathBuf,
}

impl JsonlStore {
    pub fn new(dir: impl AsRef<Path>) -> std::io::Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir: dir.as_ref().to_path_buf(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn append<T: Serialize>(&self, file: &str, value: &T) -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(file))?;
        let line = serde_json::to_string(value).map_err(std::io::Error::other)?;
        writeln!(f, "{line}")
    }

    pub fn read_all<T: DeserializeOwned>(&self, file: &str) -> std::io::Result<Vec<T>> {
        let path = self.dir.join(file);
        if !path.exists() {
            return Ok(vec![]);
        }
        let f = File::open(path)?;
        let mut out = vec![];
        for line in BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            out.push(serde_json::from_str(&line).map_err(std::io::Error::other)?);
        }
        Ok(out)
    }
}
