//! Default spec templates and init.

use std::path::Path;

use super::Spec;

pub const DEFAULT_TEMPLATES: &[(&str, &str)] = &[
    (
        "issue",
        r#"<!-- spec: issue -->
<!-- desc: 普通 issue（task/bug/chore）：拆分描述，无 Done when；验收与编排在 PR，审查 findings 回写本 issue 评论区 -->

# <issue 标题>

## Goal [req]
<!-- 这个 issue 要解决的目标（必填） -->

## Background [req]
<!-- 为什么现在做、之前的决定或链接（必填） -->

## Suspected areas [req]
<!-- 改动范围：文件/package/符号/workflow/文档（必填） -->

## Out of scope [opt]
<!-- 明确不应顺带纳入的工作（可选） -->

## How to observe success [opt]
<!-- 命令/页面状态/CI job/指标或前后对比（可选） -->

## Additional context [opt]
<!-- 无法放入上述字段的链接或脱敏说明（可选） -->
"#,
    ),
    (
        "epic",
        r#"<!-- spec: epic -->
<!-- desc: epic issue：必须有 Implement order，不能有 Done when -->
<!-- forbid: Done when -->

# <epic 标题>

## Description [req]
<!-- 里程碑目标：完成后应该存在什么能力（必填） -->

## Problem / use case [req]
<!-- 谁在当前流程中受阻、为什么拆这个里程碑（必填） -->

## Implement order [req] [checkbox]
<!-- 按顺序列出的实施步骤（必填；epic 用 Implement order，不用 Done when） -->
- [ ]

## Scope [req]
<!-- 单 PR 还是多 PR（必填） -->

## Non-goals [opt]
<!-- 不应顺带纳入的 API/协议/部署/架构改动（可选） -->

## Proposed approach [opt]
<!-- 高层方案（可选） -->

## Alternatives considered [opt]
<!-- 被拒绝的设计或当前 workaround（可选） -->

## Area [opt]
<!-- 负责该变化的 package/workflow/provider/区域（可选） -->

## Additional context [opt]
<!-- 链接/先例/脱敏说明（可选） -->
"#,
    ),
    (
        "pr",
        r#"<!-- spec: pr -->
<!-- desc: PR：必须有 Construction plan（≥2 checkbox） -->

# <PR 标题>

## What [req]
<!-- 合并后会发生什么变化（必填） -->

## Why [req]
<!-- 为什么做、根因/背景/设计决策（必填） -->

## Issue [req]
<!-- 主 Issue：Fixes #N 或说明无关联（必填） -->

## Construction plan [req] [checkbox]
<!-- 最小实现步骤（必填，≥2 个 checkbox） -->
- [ ]

## Delivery record [req]
<!-- Delivered / Verification / Follow-up（必填） -->

## How to test [req]
<!-- 评审者可复现的命令或步骤（必填） -->

## Checklist [req] [checkbox]
<!-- 提交前自检（必填） -->
- [ ]
"#,
    ),
    (
        "review",
        r#"<!-- spec: review -->
<!-- desc: review：CRG + ocr 双审查格式 -->

## Agent 🤖 - CRG Review: <english-title> [req]
<!-- CRG 审查标题（英文），发现按文件/严重度列出 -->

## ocr findings [req]
<!-- ocr AI 审查发现；无发现时写「无审查发现」 -->

## Conclusion [req]
<!-- 结论：无阻塞项 / 需修复项清单 -->
"#,
    ),
];

/// Write the four default templates into `<dir>/specs/` (idempotent — never
/// overwrites an existing file so user edits survive re-init).
pub fn write_default_specs(dir: &Path) -> Result<(), String> {
    let specs_dir = dir.join("specs");
    std::fs::create_dir_all(&specs_dir)
        .map_err(|e| format!("create specs dir {}: {e}", specs_dir.display()))?;
    for (name, content) in DEFAULT_TEMPLATES {
        let path = specs_dir.join(format!("{name}.md"));
        if !path.exists() {
            std::fs::write(&path, content)
                .map_err(|e| format!("write spec template {}: {e}", path.display()))?;
        }
    }
    Ok(())
}
