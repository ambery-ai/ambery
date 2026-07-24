//! Timer（concepts §1a，docs/timer.md）：每实例独立兜底扫描，错峰分布；Hook 是主通道。

use std::collections::HashMap;

/// 确定性哈希（同一实例每次偏移相同）
fn stable_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub struct TimerWheel {
    pub interval_ms: i64,
    pub stagger_ms: i64,
    next_due: HashMap<String, i64>,
}

impl TimerWheel {
    pub fn new(interval_ms: i64, stagger_ms: i64) -> Self {
        Self {
            interval_ms,
            stagger_ms,
            next_due: HashMap::new(),
        }
    }

    fn schedule(&self, instance: &str, from: i64) -> i64 {
        let offset = if self.stagger_ms > 0 {
            (stable_hash(instance) % self.stagger_ms as u64) as i64
        } else {
            0
        };
        from + self.interval_ms + offset
    }

    /// 实例注册 / Hook 到达：重新计时（近期有 Hook 的实例不该被补扫）
    pub fn reset(&mut self, instance: &str, now: i64) {
        let due = self.schedule(instance, now);
        self.next_due.insert(instance.into(), due);
    }

    /// 实例移除
    pub fn remove(&mut self, instance: &str) {
        self.next_due.remove(instance);
    }

    /// 提取到期实例（最多 batch 个），取走即重排；剩余保持到期下一 tick 再取
    pub fn due(&mut self, now: i64, batch: usize) -> Vec<String> {
        let mut due: Vec<(String, i64)> = self
            .next_due
            .iter()
            .filter(|(_, &t)| t <= now)
            .map(|(k, t)| (k.clone(), *t))
            .collect();
        due.sort_by_key(|(_, t)| *t); // 先到期的先扫
        let picked: Vec<String> = due.into_iter().take(batch).map(|(k, _)| k).collect();
        for inst in &picked {
            let next = self.schedule(inst, now);
            self.next_due.insert(inst.clone(), next);
        }
        picked
    }

    #[cfg(test)]
    fn due_at(&self, instance: &str) -> Option<i64> {
        self.next_due.get(instance).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_schedules_with_stagger() {
        let mut w = TimerWheel::new(300_000, 30_000);
        w.reset("a", 0);
        w.reset("b", 0);
        let da = w.due_at("a").unwrap();
        let db = w.due_at("b").unwrap();
        assert!(da >= 300_000 && da < 330_000);
        assert!(db >= 300_000 && db < 330_000);
        // 错峰确定性：同一实例重算偏移相同
        let mut w2 = TimerWheel::new(300_000, 30_000);
        w2.reset("a", 0);
        assert_eq!(w2.due_at("a").unwrap(), da);
    }

    #[test]
    fn due_reschedules_and_respects_batch() {
        let mut w = TimerWheel::new(100, 10);
        w.reset("a", 0);
        w.reset("b", 0);
        w.reset("c", 0);
        // 三实例错开（stagger 10ms），tick at 110 全部到期
        let picked = w.due(110, 2);
        assert_eq!(picked.len(), 2); // batch 上限错峰
        // 取走的已重排到未来
        for inst in &picked {
            assert!(w.due_at(inst).unwrap() >= 210);
        }
        // 剩余一个下一 tick 可取
        let rest = w.due(110, 2);
        assert_eq!(rest.len(), 1);
    }

    #[test]
    fn not_due_before_interval() {
        let mut w = TimerWheel::new(300_000, 30_000);
        w.reset("a", 0);
        assert!(w.due(299_000, 10).is_empty());
    }

    #[test]
    fn hook_reset_postpones_scan() {
        let mut w = TimerWheel::new(300_000, 0);
        w.reset("a", 0);
        assert_eq!(w.due_at("a").unwrap(), 300_000);
        w.reset("a", 250_000); // Hook 到达
        assert_eq!(w.due_at("a").unwrap(), 550_000);
    }
}
