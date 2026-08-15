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
        let f = File::open(&path)?;
        let mut out = vec![];
        let mut bad_count = 0usize;
        for line in BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(value) => out.push(value),
                Err(err) => {
                    bad_count += 1;
                    eprintln!(
                        "[storage] {} 第 {bad_count} 条坏行已隔离到 {file}.bad：{err}",
                        path.display()
                    );
                    // 隔离而非丢弃：坏行原样归档，日志神圣原则下不原地改写原文件
                    let bad_path = format!("{}.bad", path.display());
                    let _ = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&bad_path)
                        .and_then(|mut f| writeln!(f, "{line}"));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_all_skips_bad_lines_and_quarantines_them() {
        let dir = std::env::temp_dir().join(format!("ambery-storage-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = JsonlStore::new(&dir).unwrap();
        store.append("nums.jsonl", &serde_json::json!(1)).unwrap();
        fs::write(dir.join("nums.jsonl"), "1\n{\"broken\":\n\n2\n").unwrap();

        let values = store.read_all::<serde_json::Value>("nums.jsonl").unwrap();
        assert_eq!(values, vec![serde_json::json!(1), serde_json::json!(2)]);
        let bad = fs::read_to_string(dir.join("nums.jsonl.bad")).unwrap();
        assert_eq!(bad, "{\"broken\":\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_all_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("ambery-storage-miss-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = JsonlStore::new(&dir).unwrap();
        assert!(store.read_all::<serde_json::Value>("none.jsonl").unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
