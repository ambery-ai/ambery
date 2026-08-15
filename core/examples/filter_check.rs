//! 用真实采集的 UIA 文本验证 Filter：`cargo run --example filter_check -- <file>...`
//! （真实样本含工作内容，仅存临时目录，不入库）

use ambery_core::filter::{Change, Filter};

fn main() {
    let f = ambery_core::filter::by_name("claude").expect("claude filter");
    let mut prev: Option<String> = None;
    for path in std::env::args().skip(1) {
        let raw = std::fs::read_to_string(&path).expect("read file");
        let out = f.apply(&raw);
        println!("=== {path}");
        println!(
            "  raw: {} 行 {} 字 → filtered: {} 行 {} 字",
            raw.lines().count(),
            raw.chars().count(),
            out.lines().count(),
            out.chars().count()
        );
        if let Some(p) = &prev {
            let change = f.detect_change(p, &out);
            println!(
                "  change vs prev: {}",
                match change {
                    Change::Unchanged => "Unchanged".into(),
                    Change::Minor(s) => format!("Minor({s:.2})"),
                    Change::Substantive(s) => format!("Substantive({s:.2})"),
                }
            );
        }
        println!("--- filtered ---\n{out}\n");
        prev = Some(out);
    }
}
