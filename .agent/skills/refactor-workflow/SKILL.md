<!-- canon: hathawayANdRX105/canon @ d0a4b2c (synced 2026-09-23) -->
---
name: refactor-workflow
description: "重构/移植多模块工作流：主控负责铺地基（scaffold-dsh）→ 建 .wt 工作树 → 拆任务派子代理 → 代码审查验收。适用场景：用户说'重构/移植/复刻某模块'、'铺好骨架然后派子代理实现'、'拆任务'、'开 worktree'、'主控子代理分工'。本 skill 是主控（编排 agent）的操作手册；铺地基细节见 scaffold-dsh skill。"
license: MIT
---

# 路径约定

本文档中所有绝对路径以 `<repo根>` 为占位符，表示**使用该 skill 的仓库根目录**（ferrite 或 omenic，按使用方决定）。

示例对照（仅供参考，实际执行时替换为 `<repo根>`）：

| 占位符 | ferrite 实例 | omenic 实例 |
|---|---|---|
| `<repo根>` | `/home/hathaway/projects/ferrite` | `/home/hathaway/projects/omenic` |
| `<repo根>/.wt/<name>` | `/home/hathaway/projects/ferrite/.wt/<name>` | `/home/hathaway/projects/omenic/.wt/<name>` |
| `<repo根>/crates/harness/<module>/src` | `/home/hathaway/projects/ferrite/crates/harness/<module>/src` | `/home/hathaway/projects/omenic/crates/harness/<module>/src` |

---

# 重构工作流（主控 → 子代理）

## 适用场景

- 用户说："把这个模块重构/移植过来"、"铺好骨架然后派子代理实现"
- 需要跨多个文件/crate 的重构，单个会话做不完
- 需要子代理并行填写占位实现

**不适用**：纯文档修改、单一文件 bug fix、用户没提子代理的简单任务。

---

## 流程总览

```
阶段 0  铺地基（scaffold-dsh skill）
  │   主控在 .wt 工作树里完成：cargo check 绿 + 占位清单 + README
  ▼
阶段 1  拆任务 + 开 PR
  │   按 crate/文件拆子任务（每个 ≤5 文件、单一主题）
  │   建 draft PR，body 登记任务清单
  ▼
阶段 2  派子代理（dev）
  │   最多 2 个并行；同 crate 文件必须串行
  │   每个 prompt 写明：绝对路径、允许/禁止文件、goal、验收命令
  ▼
阶段 3  审查（audit）
  │   主控逐任务校验 diff 边界 + 跑验收命令
  │   必要时派校验子代理交叉确认
  ▼
阶段 4  CI 驱动（test）
  │   push PR → CI 动态选包 → 全部绿才继续
  │   CI 失败 → 提取日志回阶段 2 精准修复
  ▼
阶段 5  代码审查（tool review）
  │   code-review-graph update → detect-changes → ocr review（commit 级分批）
  │   发现 bug → 回阶段 2 修 → 再 review，直到干净
  ▼
阶段 6  验收（smoke）
  │   真实用户路径跑一遍，截图/OCR 对比
  ▼
阶段 7  收尾（tidy + report）
  │   gate 复检 + fmt + docs 同步 → PR comment
  ▼
阶段 8  合并 + 清理 worktree
      用户确认 → squash merge → git worktree remove
```

---

## 阶段 0：铺地基

> 本阶段依赖 scaffold-dsh skill；先读 .agent/skills/scaffold-dsh/SKILL.md，铺完地基（cargo check 绿）再开始阶段 1。

**主控做，不用子代理。**

1. 读所属域目录 `README.md`，确定本次要动的 crate 和依赖关系
2. `cd <repo根>`（仓库根，**不在 worktree 内**）
   执行 `git worktree add .wt/<branch-name> -b feat/<branch-name>`
3. 自检：`git worktree list` 新条目路径必须是 `<repo根>/.wt/<branch-name>`，
   路径含第二个 `.wt/` 即嵌套，立即撤销
4. 进入 worktree，按 **scaffold-dsh skill** 铺完占位骨架：
   - 所有相关 crate `cargo check -p <crate>` 绿
   - 占位清单写入 crate `README.md`
   - 提交 conventional commit：`scaffold(harness): add <module> skeleton with todo! placeholders`

**完成标志**：`cargo check -p <crate>` 绿，PR 里能看到所有 `todo!` 占位。

---

## 阶段 1：拆任务 + 开 PR

### 拆任务规则（硬性）

- 单个子任务 **≤ 5 个文件、单一主题、单一修改范围**
- 优先按文件拆；同文件内按修改范围拆；仍然太大就按主题/调用链/测试拆
- 能并行的子任务：不同 crate、或同 crate 内不交叉的文件
- **禁止**"一个子代理干完半个模块"——宁可拆成 3 个小任务也不合并

### 任务清单格式（PR body）

```markdown
## 任务清单

- [ ] T1: `crates/harness/core/src/run_status.rs` — 填 `RunStatus::from_str` 占位
      参考: deepseek-harness/packages/core/src/state.ts:88
      约束: 调用方 runtime/src/loop.rs:32 传 &str，返回 Result<RunStatus, ParseError>
      验收: `cpulimit -l 70 -i -- cargo check -p harness-core`  # 必须带 -p，禁止无 -p 全仓 check
- [ ] T2: `crates/harness/core/src/abort.rs` — 填 `AbortSignal::wait` 占位
      ...
```

### 开 PR

```bash
cd <repo根>/.wt/<name>
# 阶段 0 已提交 scaffold commit；此处确认工作区干净后直接推分支开 PR
git status --porcelain   # 应输出空（工作区干净）
git push origin <branch>
gh pr create --draft \
  --title "refactor(harness): scaffold <module> from deepseek-harness" \
  --body-file /dev/stdin <<'EOF'
<!-- 在这里放任务清单 + suspect area + 风险点 -->
EOF
```

记录 `base_sha`（`git rev-parse HEAD~1` 或 main 当前 sha），后续 CRG 用。

---

## 阶段 2：派子代理（dev）

### 并行规则

- 最多 **2 个**互不冲突的子任务并行
- 同 crate 同文件 → **必须串行**
- 子代理之间用 `hub` 消息协调（不要各写各的然后冲突）

### 子代理 prompt 格式（每个 prompt 必须含全部字段）

```
【工作目录】<repo根>/.wt/<branch>/
【所属分支】feat/<branch>
【允许修改的文件】crates/harness/core/src/run_status.rs
【禁止触碰的文件】crates/harness/core/src/abort.rs、crates/contract/**
【goal】填 RunStatus::from_str 的 todo! 占位（见 crates/harness/core/README.md 占位清单第 N 行）
【非目标】不改 trait 签名；不加新错误类型；不动 abort.rs
【参考根】/home/hathaway/projects/harness/deepseek-harness  # 参考项目绝对路径
【参考】packages/core/src/state.ts:88 — fromStr  # 相对于参考根
【验收命令】cd <repo根>/.wt/<branch>/ && \
            cpulimit -l 70 -i -- cargo check -p harness-core  # 必须带 -p，禁止无 -p 全仓 check
【约束】
  - 所有编译命令必须套 cpulimit -l 70 -i --
  - 测试放 tests/ 目录，不用 #[cfg(test)]
  - 完成时 commit：feat(harness-core): implement RunStatus::from_str
  - 禁止 git push（commit 即止，push 由阶段 4 主控执行）
  - 需动允许外文件 → 停，hub 报主控，等指示后再继续
  - 不要改 PR body（由主控更新）
```

**全局绝对路径**（`.wt/<branch>/...`）必须在 prompt 里写明，禁止让子代理自己推导路径。

**参考根**：`【参考根】` 给出参考项目的绝对路径，`【参考】` 字段写相对路径；子代理拼出完整路径后直接读文件，不再依赖主控转述。

---

## 阶段 3：审查（audit）

子代理完成后，**主控**做（不委托）：

1. **跑验收命令**（真跑，不只看子代理的输出）：
   ```bash
   cd <repo根>/.wt/<branch>
   cpulimit -l 70 -i -- cargo check -p harness-core  # 必须带 -p，禁止无 -p 全仓 check
   ```
2. **diff 边界检查**：`git diff --name-only` 确认改动只落在 prompt 声明的文件
3. **检查 root cause / 调用方 / 边界输入**：对照 scaffold 注释里的约束，确认实现语义对齐
4. 必要时再派一个**校验子代理**做交叉确认（读 diff + 对照参考实现，不写代码）

**失败** → 回阶段 2 重拆或修 prompt；连续 2 次同一子任务失败 → 主控自己接手该任务。

---

## 阶段 4：CI 驱动（test）

### 子代理 prompt 补充（测试相关）

子代理 prompt 里的【验收命令】字段，若涉及测试必须套 cpulimit：

```bash
# 编译验收
cpulimit -l 70 -i -- cargo check -p <crate>  # 必须带 -p，禁止无 -p 全仓 check

# 单测验收（3 秒内跑完的简单单测才本地跑；否则只 check，测试推 CI）
cpulimit -l 70 -i -- cargo test -p <crate> -- <test_name>
```

> 说明：所有 cargo 命令（check/test/clippy）必须带 `-p` 目标 crate，禁止无 `-p` 全仓 check。

规则：
- **所有集成测试/多 crate 联调/重型测试 → CI 跑**，本地不执行 `cargo test --all`
- 本地 `cargo test` 仅限「改动核心逻辑的单体单测且 3 秒内跑完」的调试场景
- 本地 clippy 必须先 `rustup update stable`（CI 用 stable，版本差会被新 lint 拦）

- 全部子任务 audit 通过 → push PR
- CI 动态选包跑（按目标仓库 AGENTS.md 的 CI 调度规则执行；有动态选包的仓跑选包；全量 CI 的仓更要本地克制，只跑 -p check）
- **CI 全绿前不得 closeout / merge**
- CI 失败 → 提取云端日志 → 回阶段 2 精准修复（把失败当新子任务）
- 本地只跑 `cargo check -p <crate>`（极小范围）；`cargo test` 全量禁止本地跑

---

## 阶段 5：代码审查（tool review）

```bash
# 先刷新图谱（scaffold 之后子代理的 commit 不在旧图里，必须 update）
code-review-graph update --brief

# 结构层（CRG）
code-review-graph detect-changes --base <base_sha>

# 规范层（ocr）—— commit 级分批：每个子任务一个独立 commit，ocr 按 commit 喂入
# 若子任务已各自 commit，逐个跑：
ocr review -c <task_commit_1_sha>
ocr review -c <task_commit_2_sha>
# 若整范围一起 review（较少用）：
# ocr review --from <base_sha> --to <branch>
```

发现 bug / 问题 → 回阶段 2 修 → 重新 review，直到干净。

**每轮（review + fix）写一条 PR comment**：发现 → 修复 commit SHA → 验证命令。

---

## 阶段 6：验收（smoke）

真实用户路径跑一遍：
- CLI 命令 / 真实 URL / 真实进程
- UI 截图或 ariaSnapshot 对照
- 发现问题 → 更新 PR 任务清单 → 回阶段 2 修

通过后在 PR 写一条 "smoke 验证通过 / 方法 / 结果" comment。

---

## 阶段 7：收尾（tidy + report）

1. **gate 复检**：`gate pre-commit` + `gate pre-push` 全量，FAIL 清零
2. **杂物检查**：`.wt/<branch>/` 里跟本次无关的临时文件 → `gio trash`（严禁 `rm`）
3. **格式**：`cargo fmt --all`（若改动文件，重跑最小验收 + tool review + smoke）
4. **docs 同步**：crate `README.md` 的"待实现占位清单"逐项打勾，过期注释更新

**第 5 步报告**：PR 发最终总结 comment，格式 = 改了哪些文件 / 跑了哪些测试 / CRG+ocr+smoke 结果 / PR 链接 / 剩余风险

---

## 阶段 8：合并 + 清理

**用户确认后**（不得自行合并）：

```bash
git worktree remove <repo根>/.wt/<name>
```

worktree 清理前检查：PR 已 squash merge、用户明确确认、`git worktree list` 确认目录对应。

---

## 关键门禁（不可绕过）

- PR-only：一切工作面走 PR，禁止新建 GitHub issue 改 epic
- 工作目录门禁：子代理只在 `.wt/<branch>/` 工作，prompt 写绝对路径
- 任务量门禁：单个子任务 ≤ 5 文件
- gate 拦截门：FAIL 不清零不得 push
- 合并门禁：squash merge 必须用户确认，不得自动执行
