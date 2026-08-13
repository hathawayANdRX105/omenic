# omenic 审查手册 (Review Handbook)

Agent 和维护者审查者共用此清单。Agent 在 PR diff 行上发现问题时, comment 必须以下列约定前缀开头; 无问题不留 comment。审查结论只发 `COMMENT`, 不发 `APPROVE` / `REQUEST_CHANGES`, 最终二审由维护者完成。

---

## 1. Shell / Bash 脚本

- [ ] 所有 shell 脚本以 `#!/usr/bin/env bash` 开头, 不硬编码 `/bin/bash` 路径。
- [ ] 公开可调用脚本设置 `set -uo pipefail` 或 `set -euo pipefail`; 不在已设置 pipefail 的脚本里用裸 `grep` 不带 `|| true` (避免 grep 无匹配 exit 1 反转为整体失败)。
- [ ] `source` 路径基于 `$(dirname "${BASH_SOURCE[0]}")` 或 `BASH_SOURCE` 推导, 不依赖调用者 cwd。
- [ ] 复用函数必须 source 共享 lib (`lib/*.sh`); 不在多个脚本里粘贴同一段实现。
- [ ] `local` 变量只在函数内使用, 函数外用普通赋值。
- [ ] 注释说明 **why**, 不写外部标识符 (issue 编号、规则编号、章节号 `2.10`)。下一个读者看不懂这些编号。
- [ ] 网络类调用必须经过带重试的封装 (`gh_api_get`), 不裸调 `gh api` / `curl`。
- [ ] 退出码与 stderr 信息一致: 失败时把诊断信息打到 stderr 并返回非零。

## 2. YAML / 配置

- [ ] 配置文件放 `.github/` 下, 不散落到多个目录。
- [ ] 配置 schema 改动必须同步更新读取该配置的脚本与 lib helper。
- [ ] 配置加载失败时脚本要有 sensible fallback 或显式报错, 不静默吞错。

## 3. 依赖与构建产物

- [ ] 删除依赖时同步 `package.json` / `bun.lock` / `Cargo.toml` / `requirements.txt` 等, 并确认源码无直接 import; transitive 残留要说明来源。
- [ ] 构建产物 (target/ / node_modules/ / dist/ / build/) 不进 commit; lib/regex.sh 的 `is_genpath_hit` 黑名单覆盖这些路径。
- [ ] 体积/性能 PR 必须有前后证据 (同一 base、同一命令、同一 worktree); 没有改善不能把 checklist 勾成完成。

## 4. 安全

- [ ] 无硬编码密钥、真实配置、生产 token、私有 endpoint 或本机代理配置。
- [ ] 日志、PR、Issue、comment 不暴露密钥或用户隐私; lib/regex.sh 的 `is_secret_hit` 自动扫描常见泄密模式。
- [ ] `.env` / `.git/` / `.ssh/` 不进 staged diff (pre-commit hook 已强制)。

## 5. 测试与验证

- [ ] Bug fix 需要能复现原问题并证明不再触发; 功能/行为改动用现有测试或最小新增测试覆盖可观察契约。
- [ ] 改动至少跑一次相关 validator: `bash bin/validate_pr.sh` / `bash bin/validate_issue.sh` / `bash bin/merge_pr.sh --dry-run`。
- [ ] 验证输出要能复现: 命令、cwd、失败/通过结果都写清楚。

## 6. PR / Issue 卫生

- [ ] PR 标题为英文 (允许 Conventional Commits, 但仓库模板不强制); Issue 标题为中文 (repo 约定)。
- [ ] PR body 使用仓库模板; Issue/PR 正文 markdown heading 用英文, 内容用中文。
- [ ] 维护者要求不关闭时, 只用 `Related #N`, 不写 `Fixes #N` / closing keyword。
- [ ] Construction plan 的已完成 step 勾选; Issue Done when 只勾当前证据已满足的项; Done when 必须全勾才能关闭 sub-issue (脚本已强制)。
- [ ] CRG 记录写 PR comment (`Agent 🤖 - CRG Review:` 开头), 不写 PR body。

## 7. Agent review 评论格式

- [ ] Inline review (绑 path+line): `Agent 🤖 - Inline Review P0|P1|P2|P3: <内容>`
- [ ] CRG review (PR 顶层会话): `Agent 🤖 - CRG Review: <内容>`
- [ ] Reply (回复已有 finding): `Agent 🤖 - Fix: <理由>` / `Agent 🤖 - Block: <理由>` / `Agent 🤖 - Note: <理由>` 等任意英文意图词 + 理由。
- [ ] Review comment 内部结构: 二级标题 (`##`) 作主分类 → 三级标题 (`###`) 作子分类 → 中文内容。
- [ ] Review comment 严禁用 checkbox (`- [ ]` / `- [x]`); checkbox 只用于 Done when 与 Construction plan。
- [ ] 已撤回的 review comment 在 body 开头写 `[superseded — <理由>]` / `[withdrawn — <理由>]` / `[deprecated — <理由>]`, validator 会过滤掉。

---

## 语言要求

- PR 与 Issue 标题结构 (What / Why / How to test / Issue / Checklist 等) 可用英文, 内容用中文。
- 同一文档不混用中英文表述; 技术名词 (包名、函数名、env 变量、文件路径) 保持原文。
- Markdown heading 一律英文 (含 Issue body 与 review comment)。

## Agent 审查流程

1. 读取 PR file changes / `git diff origin/main...HEAD`, 只审当前 PR 相关文件。
2. 跑该 PR 对应的最小验证: `bash bin/validate_pr.sh <repo> <pr>` / `bash bin/validate_issue.sh <repo> <issue>` / `bash bin/merge_pr.sh <repo> <pr> --dry-run`。
3. 按上述清单逐项检查 diff。
4. 只对发现的问题在对应文件行号留 inline comment; 无问题不留。
5. 修复每条 finding 后, 在原 thread 回复 `Agent 🤖 - Fix: <理由>` / `Note: <理由>` 等, 写明提交与验证。
6. 更新 Issue Done when 与 PR Construction plan / Delivery record 的 checkbox。
7. 在 PR 会话区发一条 `Agent 🤖 - CRG Review:` summary review (用二级标题 + 三级标题 + 中文内容, 不要 checkbox)。
8. 维护者做最终二审。
