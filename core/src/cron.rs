//! Cron（concepts §10g，docs/cron.md）：Harness 的持久化计划与延时调度。
//! CronScheduler 是唯一调度实现：entries（cron.jsonl append-only，replay 折叠）
//! + waiters（sleep 非持久化 oneshot，独立共享句柄——sleep 占用 Queue 串行点时
//! 调度任务仍能到点唤醒，无死锁）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

pub const CRON_FILE: &str = "cron.jsonl";
/// 设计常量（docs/cron.md）：sleep 上限 5 分钟（Queue 串行点占用防呆）
pub const MAX_SLEEP_MS: u64 = 300_000;
/// 设计常量（docs/cron.md）：every_ms 上限 30 天（防溢出回绕成永久刷屏计划）
pub const MAX_EVERY_MS: u64 = 2_592_000_000;

/// 任务调度（docs/cron.md §任务表示）：二选一
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Schedule {
    /// 一次性（epoch ms）
    At(i64),
    /// 间隔周期（锚定创建时刻：首次到期 = 创建 + every_ms）
    EveryMs(u64),
}

/// cron.jsonl 事件行（append-only，replay 折叠）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum CronLine {
    Create {
        id: String,
        schedule: Schedule,
        message: String,
        next_due: Option<i64>,
        ts: i64,
    },
    Fire { id: String, next_due: Option<i64>, ts: i64 },
    Delete { id: String, ts: i64 },
}

/// 内存投影中的计划条目
#[derive(Debug, Clone, PartialEq)]
pub struct CronEntry {
    pub id: String,
    pub schedule: Schedule,
    pub message: String,
    /// None = 完成态（at 已发放），不再调度
    pub next_due: Option<i64>,
}

/// sleep 等待者共享句柄（独立锁，不经 AmberyBackend 锁——sleep 持 Queue 串行点
/// 等待时，调度任务经此句柄到点唤醒，docs/cron.md §调度实现）
#[derive(Clone)]
pub struct WaiterHandle {
    waiters: Arc<Mutex<Vec<(i64, oneshot::Sender<()>)>>>,
}

impl WaiterHandle {
    /// 注册一次性等待（非持久化，崩溃丢失）
    pub fn register(&self, fire_ts: i64) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().unwrap().push((fire_ts, tx));
        rx
    }

    /// 到点唤醒（调度任务调用）
    pub fn fire_due(&self, now: i64) {
        let mut waiters = self.waiters.lock().unwrap();
        let (due, pending): (Vec<_>, Vec<_>) = waiters.drain(..).partition(|(ts, _)| *ts <= now);
        *waiters = pending;
        for (_, tx) in due {
            let _ = tx.send(());
        }
    }
}

/// Cron 调度器（Harness 挂载；entries 持久化 + waiters 共享句柄）
pub struct CronScheduler {
    entries: Vec<CronEntry>,
    path: PathBuf,
    waiters: WaiterHandle,
    /// 进程内递增计数（id 生成混入：create→delete→create 同毫秒不撞 id）
    seq: u64,
}

impl CronScheduler {
    /// 启动：replay cron.jsonl 折叠为当前计划集
    pub fn load(storage_dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(storage_dir)?;
        let path = storage_dir.join(CRON_FILE);
        let mut entries: Vec<CronEntry> = Vec::new();
        if path.exists() {
            for line in std::fs::read_to_string(&path)?.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(row) = serde_json::from_str::<CronLine>(line) else {
                    continue; // 坏行跳过（append-only 日志，单行病灶不带倒整体）
                };
                apply_line(&mut entries, row);
            }
        }
        Ok(Self {
            entries,
            path,
            waiters: WaiterHandle {
                waiters: Arc::new(Mutex::new(Vec::new())),
            },
            seq: 0,
        })
    }

    pub fn waiter_handle(&self) -> WaiterHandle {
        self.waiters.clone()
    }

    /// 当前计划集（观测/测试）
    pub fn entries(&self) -> &[CronEntry] {
        &self.entries
    }

    fn append(&self, line: &CronLine) -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(line).map_err(std::io::Error::other)?)?;
        Ok(())
    }

    /// 创建持久化计划（cron_create）。now 用于 every_ms 锚定与 id 生成
    pub fn create(&mut self, schedule: Schedule, message: &str, now: i64) -> Result<String, String> {
        if message.trim().is_empty() {
            return Err("message 必填且非空（到点注入 Queue 的 system 输入）".into());
        }
        let next_due = match schedule {
            Schedule::At(ts) => {
                if ts <= now {
                    return Err("schedule.at 须大于当前时刻".into());
                }
                ts
            }
            Schedule::EveryMs(ms) => {
                if ms == 0 {
                    return Err("schedule.every_ms 须 > 0".into());
                }
                if ms > MAX_EVERY_MS {
                    return Err(format!(
                        "schedule.every_ms 超上限 {MAX_EVERY_MS}（30 天，设计常量）"
                    ));
                }
                now.saturating_add(ms as i64)
            }
        };
        // id：时间戳 + 进程内递增计数短 hash（create→delete→create 同毫秒不撞）
        self.seq += 1;
        let id = format!("{:08x}", fnv1a(format!("{now}:{}", self.seq).as_bytes()));
        let entry = CronEntry {
            id: id.clone(),
            schedule,
            message: message.to_string(),
            next_due: Some(next_due),
        };
        self.append(&CronLine::Create {
            id: id.clone(),
            schedule,
            message: message.to_string(),
            next_due: Some(next_due),
            ts: now,
        })
        .map_err(|e| format!("cron.jsonl 写入失败：{e}"))?;
        self.entries.push(entry);
        Ok(id)
    }

    /// 删除计划（tombstone）——先落盘后改内存（append-only 顺序不分叉）
    pub fn delete(&mut self, id: &str, now: i64) -> Result<(), String> {
        if !self.entries.iter().any(|e| e.id == id) {
            return Err(format!(
                "计划 '{id}' 不存在（cron 无 list tool；id 见 create 返回或 cron.jsonl）"
            ));
        }
        self.append(&CronLine::Delete {
            id: id.to_string(),
            ts: now,
        })
        .map_err(|e| format!("cron.jsonl 写入失败：{e}"))?;
        self.entries.retain(|e| e.id != id);
        Ok(())
    }

    /// 取到期 message 并落 fire 行（every_ms 重排，at 完成态）——先落盘后改内存
    pub fn due(&mut self, now: i64) -> std::io::Result<Vec<String>> {
        let fired: Vec<(String, String, Option<i64>)> = self
            .entries
            .iter()
            .filter(|e| e.next_due.is_some_and(|d| d <= now))
            .map(|e| {
                let due = e.next_due.unwrap();
                let new_next = match e.schedule {
                    Schedule::At(_) => None,
                    Schedule::EveryMs(ms) => Some(due.saturating_add(ms as i64)),
                };
                (e.id.clone(), e.message.clone(), new_next)
            })
            .collect();
        // 先落盘（append-only：任一 fire 行失败则内存不动，下次 tick 重试）
        for (id, _, new_next) in &fired {
            self.append(&CronLine::Fire {
                id: id.clone(),
                next_due: *new_next,
                ts: now,
            })?;
        }
        for (id, _, new_next) in &fired {
            if let Some(e) = self.entries.iter_mut().find(|e| &e.id == id) {
                e.next_due = *new_next;
            }
        }
        Ok(fired.into_iter().map(|(_, m, _)| m).collect())
    }
}

fn apply_line(entries: &mut Vec<CronEntry>, row: CronLine) {
    match row {
        CronLine::Create {
            id,
            schedule,
            message,
            next_due,
            ..
        } => {
            entries.retain(|e| e.id != id); // 同 id 重建（防御）
            entries.push(CronEntry {
                id,
                schedule,
                message,
                next_due,
            });
        }
        CronLine::Fire { id, next_due, .. } => {
            if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                e.next_due = next_due;
            }
        }
        CronLine::Delete { id, .. } => entries.retain(|e| e.id != id),
    }
}

fn fnv1a(s: &[u8]) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in s {
        h ^= *b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ambery-cron-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn create_due_fire_reschedule_and_replay() {
        let dir = tmp("basic");
        let mut s = CronScheduler::load(&dir).unwrap();
        // every_ms：首次到期 = 创建 + every_ms
        let id = s.create(Schedule::EveryMs(60_000), "日报", 1000).unwrap();
        assert!(s.due(60_999).unwrap().is_empty());
        assert_eq!(s.due(61_000).unwrap(), vec!["日报".to_string()]);
        // fire 后重排 next_due += every_ms
        let e = s.entries().iter().find(|e| e.id == id).unwrap();
        assert_eq!(e.next_due, Some(121_000));
        // at 一次性：fire 后完成态（every 已重排到 121s，此刻只有 at 到期）
        let id2 = s.create(Schedule::At(70_000), "一次性", 61_000).unwrap();
        assert_eq!(s.due(70_000).unwrap(), vec!["一次性".to_string()]);
        let e2 = s.entries().iter().find(|e| e.id == id2).unwrap();
        assert_eq!(e2.next_due, None);
        // 完成态不再调度
        assert!(s.due(200_000).unwrap().iter().all(|m| m != "一次性"));
        // replay 折叠：create/fire/delete 全留痕，状态一致
        let s2 = CronScheduler::load(&dir).unwrap();
        assert_eq!(s2.entries().len(), 2);
        assert_eq!(s2.entries().iter().find(|e| e.id == id2).unwrap().next_due, None);
        // delete + tombstone replay（id2 完成态保留在日志与投影）
        let mut s3 = s2;
        s3.delete(&id, 300_000).unwrap();
        assert_eq!(s3.entries().len(), 1);
        assert!(s3.entries().iter().all(|e| e.id != id));
        let s4 = CronScheduler::load(&dir).unwrap();
        assert_eq!(s4.entries().len(), 1);
        assert!(s4.entries().iter().all(|e| e.id != id));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_validations() {
        let dir = tmp("valid");
        let mut s = CronScheduler::load(&dir).unwrap();
        assert!(s.create(Schedule::At(500), "x", 1000).is_err()); // at 已过
        assert!(s.create(Schedule::EveryMs(0), "x", 1000).is_err());
        assert!(s.create(Schedule::EveryMs(MAX_EVERY_MS + 1), "x", 1000).is_err()); // 超 30 天上限
        assert!(s.create(Schedule::EveryMs(1), "  ", 1000).is_err()); // 空 message
        assert!(s.create(Schedule::EveryMs(1), "ok", 1000).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_ids_unique_on_create_delete_create_same_ms() {
        // id 混入进程内递增计数：同毫秒 create→delete→create 不撞 id
        let dir = tmp("ids");
        let mut s = CronScheduler::load(&dir).unwrap();
        let id1 = s.create(Schedule::EveryMs(60_000), "一", 1000).unwrap();
        s.delete(&id1, 1000).unwrap();
        let id2 = s.create(Schedule::EveryMs(60_000), "二", 1000).unwrap();
        assert_ne!(id1, id2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn sleep_waiter_fires_via_handle() {
        let dir = tmp("sleep");
        let s = CronScheduler::load(&dir).unwrap();
        let h = s.waiter_handle();
        let mut rx = h.register(1000);
        // 未到点不唤醒
        h.fire_due(999);
        assert!(rx.try_recv().is_err());
        // 到点唤醒
        h.fire_due(1000);
        assert!(rx.await.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
