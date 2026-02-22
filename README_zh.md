<div align="center">
  <img src="frontend/public/agent-chatgroup-logo-white.png" alt="AgentsChatGroup" width="320">

  <h1>AgentsChatGroup</h1>

  <p><strong>在一个群聊中运行一支 AI Agent 团队，它们可以相互 @、共享上下文并行工作。</strong></p>

  <p>
    <a href="https://www.npmjs.com/package/agents-chatgroup"><img alt="npm" src="https://img.shields.io/npm/v/agents-chatgroup?style=flat-square" /></a>
    <a href="https://github.com/StarterraAI/AgentsChatGroup/actions/workflows/pre-release.yml"><img alt="Build" src="https://github.com/StarterraAI/AgentsChatGroup/actions/workflows/pre-release.yml/badge.svg" /></a>
    <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-MIT-blue.svg" /></a>
    <a href="https://github.com/StarterraAI/AgentsChatGroup/stargazers"><img alt="Stars" src="https://img.shields.io/github/stars/StarterraAI/AgentsChatGroup?style=flat-square" /></a>
  </p>

  <p>
    <a href="https://your-demo-link.com">📺 观看演示</a> ·
    <a href="#快速开始">⚡ 快速开始</a> ·
    <a href="https://docs.agentschatgroup.com">📖 文档</a>
  </p>

  <p align="center">
    <a href="./README.md">🇬🇧 English</a>
  </p>
  
</div>

---

<!-- 🔴 最高优先级：把这里换成动态 GIF，展示多 Agent 并行工作的聊天画面 -->
![AgentsChatGroup Demo](docs/images/demo.gif)

> **Claude Code 编写代码 → 自动 @ Gemini CLI 进行评审 → 评审结果回流到群聊，全程无需你手动传话。**

---

## 问题

你可能每天都在使用 Claude Code、Gemini CLI 和 Codex，但你一定遇到过这些问题：

- **你成了“中间人”。** 需要手动把一个 Agent 的输出复制粘贴给另一个。
- **无法并行。** 任务只能排队执行，一个完成后下一个才能开始。
- **上下文丢失。** 每次切换到新的 Agent 会话都要从零重新解释。
- **频繁切窗口打断思路。** 在 4 个聊天窗口之间来回跳转非常消耗精力。

**AI 越来越强，但开发者却越来越累。**

## 解决方案

AgentsChatGroup 把所有 AI Agent 放进**同一个群聊**。它们共享上下文、通过 @mention 自动交接任务，并行协作，就像一支真实团队。

```
╭─────────────────────────────────────────────────────────────╮
│                    AgentsChatGroup 🧩                       │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  👤 You                                                     │
│  │  @coder Build a user login feature                       │
│                                                             │
│  🤖 Coder                                    [parallel ⚡] │
│  │  Writing the login module...                             │
│  │  └─ @reviewer Done! Please review this.                  │
│                                                             │
│  🤖 Reviewer                                 [parallel ⚡] │
│  │  Found 2 security issues:                                │
│  │  1. Passwords need hashing                               │
│  │  2. Add rate limiting                                    │
│  │  └─ @coder Please fix these.                             │
│                                                             │
│  🤖 Coder                                                   │
│  │  Fixed. Pushing now...                                   │
│                                                             │
╰─────────────────────────────────────────────────────────────╯
```

你只需分配一次任务，剩下的由 Agent 团队自行协作完成。

## 快速开始

### 方案 A：使用 npx 运行

```bash
# web
npx agents-chatgroup
```

### 方案 B：下载桌面应用

[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D6?style=for-the-badge&logo=windows)](https://github.com/StarterraAI/AgentsChatGroup/releases/latest)
[![Download for macOS](https://img.shields.io/badge/Download-macOS-000000?style=for-the-badge&logo=apple)](https://github.com/StarterraAI/AgentsChatGroup/releases/latest)
[![Download for Linux](https://img.shields.io/badge/Download-Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)](https://github.com/StarterraAI/AgentsChatGroup/releases/latest)

**你至少需要安装一个 AI Agent：**

| Agent | 安装命令 |
|-------|---------|
| [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `npm i -g @anthropic-ai/claude-code` |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `npm i -g @google/gemini-cli` |
| [Codex](https://github.com/openai/codex) | `npm i -g @openai/codex` |
| [QWen Coder](https://qwenlm.github.io/qwen-code-docs/en/users/overview/) | `npm i -g @qwen-code/qwen-code@latest` |

📚 [完整安装指南 →](https://docs.agentschatgroup.com/getting-started)

## 使用场景示例

**🧑‍💻 全栈开发团队**
> 架构师设计 Schema → 开发 Agent 实现功能 → 审查 Agent 检查安全性 → 测试 Agent 编写覆盖。全流程在一个群聊里并行推进。

**📝 内容生产团队**
> 研究 Agent 收集资料 → 写作 Agent 起草内容 → 编辑 Agent 润色。无需在多个聊天窗口之间复制粘贴。

**🔍 代码库审计团队**
> 多个 Agent 同时扫描不同模块，在更短时间内产出完整报告。

**📊 数据管道协作团队**
> 清洗 Agent 预处理数据 → 分析 Agent 执行查询 → 可视化 Agent 生成图表。每个 Agent 都能无缝承接上一步结果。

更多AI团队等你来建设。

## 有何不同

| | 传统单 Agent | 多窗口工作流 | Claude Code-Agent Team | AgentsChatGroup |
|--|--|--|--|--|
| 并行能力 | ❌ 串行 | ⚠️ 手动并行 | ✅ Claude 子代理 | ✅ 自动并行 |
| 上下文共享 | ❌ | ❌ 需要手动复制粘贴 | ⚠️ 子代理上下文分裂 | ✅ 始终同步 |
| 多模型协作 | ❌ | ⚠️ 手动切换 | ❌ 仅 Claude | ✅ Claude + Gemini + Codex + 更多 |
| Agent 任务交接 | ❌ | ❌ 由你手动编排 | ⚠️ 仅在 Claude 内部委派 | ✅ @mention 自动交接 |
| 你的投入成本 | 高 | 很高 | 中 | 低 |

## 功能特性

| 类别 | 详情 |
|----------|---------|
| **Agent 支持** | Claude Code · Gemini CLI · Codex · Amp · QWen Coder · 其他热门 Agent |
| **协作能力** | 群聊 · 上下文共享 · @Mention 交接 · 任务追踪 · 会话归档 |
| **配置管理** | 统一 MCP 配置 · 灵活环境变量 |
| **平台支持** | 桌面应用（Windows / macOS / Linux） · Web页面应用 |
| **即将推出** | 更紧凑的上下文优化 · 更多 Agent 集成 |

## 技术栈

| 层级 | 技术 |
|-------|-----------|
| 前端 | React + TypeScript + Vite + Tailwind CSS |
| 后端 | Rust |
| 桌面端 | Tauri |

## 本地开发

#### Mac/Linux

```bash
# 1. 克隆仓库
git clone https://github.com/StarterraAI/AgentsChatGroup.git
cd agents-chatgroup

# 2. 安装依赖
pnpm i

# 3. 启动开发服务器（同时启动 Rust 后端 + React 前端）
pnpm run dev

# 4. 构建前端
pnpm --filter frontend build

# 5. 构建桌面应用
pnpm desktop:build
```

#### Windows (PowerShell)：前后端分开启动

`pnpm run dev` 无法在 Windows PowerShell 中运行。请使用以下命令分别启动后端和前端。

```bash
# 1. 克隆仓库
git clone https://github.com/StarterraAI/AgentsChatGroup.git
cd AgentsChatGroup

# 2. 安装依赖
pnpm i
```

**终端 A（后端）**

```powershell
$env:FRONTEND_PORT = node scripts/setup-dev-environment.js frontend
$env:BACKEND_PORT = node scripts/setup-dev-environment.js backend
$env:VK_ALLOWED_ORIGINS = "http://localhost:$env:FRONTEND_PORT"
$env:DISABLE_WORKTREE_CLEANUP = "1"
$env:RUST_LOG = "debug"
cargo run --bin server
```

**终端 B（前端）**

```powershell
$env:FRONTEND_PORT = node scripts/setup-dev-environment.js frontend
cd frontend
pnpm dev -- --port $env:FRONTEND_PORT --host
```

打开前端页面：`http://localhost:<FRONTEND_PORT>`（例如：`http://localhost:3001`）。

## 贡献

欢迎贡献！你可以先在 [Issues](https://github.com/StarterraAI/AgentsChatGroup/issues) 查看待办，或在 [Discussions](https://github.com/StarterraAI/AgentsChatGroup/discussions) 发起讨论。

1. Fork → feature branch → PR
2. 大改动请先开 Issue 讨论

## 社区

| | |
|--|--|
| 🐛 **Bug 反馈** | [GitHub Issues](https://github.com/StarterraAI/AgentsChatGroup/issues) |
| 💬 **讨论区** | [GitHub Discussions](https://github.com/StarterraAI/AgentsChatGroup/discussions) |
| 💭 **社区群聊** | *即将上线* |

## 致谢

本项目基于 [Vibe Kanban](https://www.vibekanban.com/) 构建，感谢其团队提供优秀的开源基础。
