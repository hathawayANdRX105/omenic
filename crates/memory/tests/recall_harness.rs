//! MEM-0 recall harness: a fixed bilingual corpus + query set with ground
//! truth, measuring recall@5. This is the acceptance gate for every future
//! retrieval change (jcode drove recall 0.0 → 0.53 → 0.75 with this shape).
//!
//! The floor below is a regression gate, not a target ceiling — improvements
//! (embeddings, rerank) must keep the harness green or raise the floor.

use memory::{MemoryEntry, MemoryGraph};

/// (id, text, tags) — 24 entries across four topic clusters, mixing English
/// and Chinese, plus a superseded pair and a contradiction pair so the
/// graph cascade has real edges to walk.
fn corpus() -> Vec<MemoryEntry> {
    let rows: &[(u64, &str, &[&str])] = &[
        (
            1,
            "deploy target is fly.io; staging on fly-east",
            &["deploy"],
        ),
        (
            2,
            "rust toolchain pinned to 1.85 via rust-toolchain.toml",
            &["tooling"],
        ),
        (3, "uses zsh with starship prompt", &["shell"]),
        (4, "editor is helix, theme everforest", &["editor"]),
        (
            5,
            "repo CI runs cargo test --workspace on every PR",
            &["ci"],
        ),
        (6, "分支策略:只在 feature 分支开发,禁止直推 main", &["git"]),
        (7, "合并必须走 squash PR,标题用英文", &["git"]),
        (
            8,
            "API key rotation happens monthly via vault",
            &["security"],
        ),
        (9, "prefers ripgrep over grep for search", &["tooling"]),
        (10, "会议记录都在 obsidian vault 里", &["notes"]),
        (
            11,
            "daemon socket lives under XDG_CONFIG_HOME/omenic",
            &["runtime"],
        ),
        (
            12,
            "sessions db is sqlite at sessions.db, OMENIC_SESSION_DB overrides",
            &["runtime"],
        ),
        (13, "deploy pipeline blocks on green CI", &["deploy", "ci"]),
        (14, "日志输出统一 tracing,不直接 println", &["code-style"]),
        (
            15,
            " rustfmt 团队规范:cargo fmt --all 提交前必跑",
            &["code-style"],
        ),
        (16, "用户偏好中文回复", &["language"]),
        (17, "模型渠道走 openai 兼容 /v1/chat/completions", &["llm"]),
        (18, "embedding 用 384 维 MiniLM,内联存储", &["llm"]),
        (19, "测试约定:tui 用 TestBackend 断言,不截图", &["testing"]),
        (20, "web 端口默认 8026,PORT 可覆盖", &["runtime"]),
        (21, "构建产物在 target/debug,不要提交进 git", &["git"]),
        (
            22,
            "敏感词过滤列表在 sensitive-word 仓库维护",
            &["security"],
        ),
        (23, "记忆系统默认关闭,enabled=true 才生效", &["memory"]),
        (
            24,
            "调优:compaction 字符预算 120k,保留最近 30k",
            &["memory"],
        ),
    ];
    let mut out: Vec<MemoryEntry> = rows
        .iter()
        .map(|(id, text, tags)| {
            let mut e = MemoryEntry::new(*text);
            e.id = *id;
            e.ts = "2026-01-01T00:00:00Z".into();
            e.tags = tags.iter().map(|t| t.to_string()).collect();
            e
        })
        .collect();
    // Supersede pair: #1 replaced an old heroku note that is not in the
    // corpus rows (id 0 doesn't exist), so instead chain #7 over an extra
    // legacy entry appended here.
    let mut legacy = MemoryEntry::new("merge is done by local merge to main");
    legacy.id = 25;
    legacy.ts = "2025-06-01T00:00:00Z".into();
    legacy.tags = vec!["git".into()];
    legacy.superseded_by = Some(7);
    legacy.active = false;
    out.push(legacy);
    // Contradict pair: #20 vs a stale port note.
    let mut stale = MemoryEntry::new("web 端口默认 3000,PORT 可覆盖");
    stale.id = 26;
    stale.ts = "2025-08-01T00:00:00Z".into();
    stale.tags = vec!["runtime".into()];
    stale.contradicts = Some(20);
    out.push(stale);
    out
}

/// (query, expected ids that must appear in top-5)
fn queries() -> Vec<(&'static str, Vec<u64>)> {
    vec![
        ("deploy 部署目标", vec![1, 13]),
        ("deploy target", vec![1]),
        ("CI 怎么跑测试", vec![5, 13]),
        ("commit 到 main 可以吗", vec![6, 7]),
        ("merge 流程", vec![7]),
        ("ripgrep 还是 grep", vec![9]),
        ("sessions sqlite 路径", vec![12]),
        ("web 默认端口", vec![20]),
        ("端口配置", vec![20]),
        ("记笔记用什么", vec![10]),
        ("日志规范", vec![14]),
        ("代码风格要求", vec![14, 15]),
        ("回复语言偏好", vec![16]),
        ("llm 接口协议", vec![17]),
        ("embedding 模型", vec![18]),
        ("ui 测试怎么做", vec![19]),
        ("记忆开关", vec![23]),
        ("compaction 预算", vec![24]),
        ("密钥怎么管理", vec![8]),
        ("git 提交规范", vec![6, 7, 21]),
    ]
}

/// Fraction of expected ids present in the top-`k` hits, averaged over all
/// queries.
fn recall_at_k(hits_per_query: &[Vec<u64>], expected: &[(&str, Vec<u64>)], k: usize) -> f32 {
    let mut total = 0f32;
    for (hits, (_, want)) in hits_per_query.iter().zip(expected) {
        let top: std::collections::HashSet<u64> = hits.iter().take(k).copied().collect();
        let found = want.iter().filter(|id| top.contains(id)).count();
        total += found as f32 / want.len() as f32;
    }
    total / expected.len() as f32
}

#[test]
fn recall_at_5_meets_the_regression_floor() {
    let graph = MemoryGraph::build(&corpus());
    let q = queries();

    let hits: Vec<Vec<u64>> = q
        .iter()
        .map(|(query, _)| {
            memory::recall::recall(&graph, query, 5)
                .iter()
                .map(|h| h.id)
                .collect()
        })
        .collect();

    let r5 = recall_at_k(&hits, &q, 5);
    eprintln!("corpus=26 queries={} recall@5={r5:.3}", q.len());
    for ((query, want), got) in q.iter().zip(&hits) {
        eprintln!("  {query:20} want={want:?} got={got:?}");
    }
    assert!(
        r5 >= 0.70,
        "recall@5 regressed: {r5:.3} < 0.70 — retrieval changes must not lose ground"
    );
}

#[test]
fn superseded_entries_do_not_outrank_their_replacement() {
    let graph = MemoryGraph::build(&corpus());
    let hits = memory::recall::recall(&graph, "merge 到 main 的方式", 5);
    let rank_of = |id: u64| hits.iter().position(|h| h.id == id);
    let old = rank_of(25);
    let new = rank_of(7);
    match (old, new) {
        (Some(o), Some(n)) => assert!(
            n < o,
            "live replacement (#7, rank {n:?}) must outrank superseded #25 (rank {o:?})"
        ),
        (Some(_), None) => panic!("replacement missing while superseded entry present"),
        _ => {} // neither surfaced — fine, the query missed both
    }
}

#[test]
fn contradict_pair_stays_queryable_for_triage() {
    let graph = MemoryGraph::build(&corpus());
    let hits = memory::recall::recall(&graph, "web 端口", 6);
    assert!(
        hits.iter().any(|h| h.id == 20),
        "live entry must surface: {hits:?}"
    );
    // The contradicted stale entry may appear but never above the live one.
    if let (Some(stale), Some(live)) = (
        hits.iter().position(|h| h.id == 26),
        hits.iter().position(|h| h.id == 20),
    ) {
        assert!(live < stale, "contradicted claim must not outrank live");
    }
}
