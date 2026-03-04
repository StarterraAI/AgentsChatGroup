<div align="center">
  <img src="frontend/public/agent-chatgroup-logo-white.png" alt="AgentsChatGroup" width="320">

  <h1>AgentsChatGroup</h1>

  <p><strong>Run a team of AI agents in one group chat - they @mention each other, share context, and work in parallel.</strong></p>

  <p>
    <a href="https://www.npmjs.com/package/agents-chatgroup"><img alt="npm" src="https://img.shields.io/npm/v/agents-chatgroup?style=flat-square" /></a>
    <a href="https://github.com/StarterraAI/AgentsChatGroup/actions/workflows/pre-release.yml"><img alt="Build" src="https://github.com/StarterraAI/AgentsChatGroup/actions/workflows/pre-release.yml/badge.svg" /></a>
    <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" /></a>
    <a href="https://discord.gg/MbgNFJeWDc"><img alt="Discord" src="https://img.shields.io/badge/Discord-Join%20Chat-5865F2?style=flat-square&logo=discord&logoColor=white" /></a>
    <a href="https://docs.agentschatgroup.com/getting-started"><img alt="Platforms" src="https://img.shields.io/badge/Platforms-Windows%20%7C%20macOS%20%7C%20Linux%20%7C%20Web-2EA44F?style=flat-square" /></a>
  </p>

  <p>
    <a href="https://your-demo-link.com">Watch Demo</a> |
    <a href="#quick-start">Quick Start</a> |
    <a href="https://docs.agentschatgroup.com">Docs</a>
  </p>

  <p align="center">
    <a href="./README_zh.md">中文</a>
  </p>
  
</div>

---

![AgentsChatGroup Demo](docs/images/demo.gif)

---


## Quick Start

### Option A: Run with npx

```bash
# web
npx agents-chatgroup
```

### Option B: Download Desktop App

[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D6?style=for-the-badge&logo=windows)](https://github.com/StarterraAI/AgentsChatGroup/releases/latest)
[![Download for macOS](https://img.shields.io/badge/Download-macOS-000000?style=for-the-badge&logo=apple)](https://github.com/StarterraAI/AgentsChatGroup/releases/latest)
[![Download for Linux](https://img.shields.io/badge/Download-Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)](https://github.com/StarterraAI/AgentsChatGroup/releases/latest)

**You'll need at least one AI agent installed:**

| Agent | Install |
|-------|---------|
| [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `npm i -g @anthropic-ai/claude-code` |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `npm i -g @google/gemini-cli` |
| [Codex](https://github.com/openai/codex) | `npm i -g @openai/codex` |
| [QWen Coder](https://qwenlm.github.io/qwen-code-docs/en/users/overview/) | `npm i -g @qwen-code/qwen-code@latest` |

📚 [Guides for Installing More Agents →](https://docs.agentschatgroup.com/getting-started)

## Why develop this app?

You might be using Claude Code, Gemini CLI, and Codex every day. Have you run into the following issues?

- **you are the middleman.** you copy outputs from one agent and paste them into another, manually.
- **No parallelism.** Tasks queue up. One agent finishes, then the next one starts.
- **Lost context.** Every new agent conversation starts from zero.
- **Context switching kills flow.** Jumping between multiple chat windows is exhausting.

**AI is getting more powerful, but we developers are getting more exhausted.**

## The Solution

AgentsChatGroup puts all your AI agents into **one chat group**. They share context, `@mention` each other to hand off tasks, and work in parallel like a real team.


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

You assign the task once. The agents figure out the rest.

## How It's Different

| Capability | 🧍 Traditional Single Agent | 🪟 Multi-window Workflow | 🤖 Claude Code-Agent Team | 🧩 AgentsChatGroup |
|--|--|--|--|--|
| Parallelism | ❌ No (sequential) | ⚠️ Partial (manual) | ✅ Yes (Claude subagents) | ✅ Yes (automatic) |
| Shared context | ❌ No | ❌ No (copy-paste) | ⚠️ Partial (split subagent contexts) | ✅ Yes (always in sync) |
| Multi-model collaboration | ❌ No | ⚠️ Partial (manual switching) | ❌ No (Claude-only) | ✅ Yes (Claude + Gemini + Codex + more) |
| Agent handoff | ❌ No | ❌ No (you orchestrate) | ⚠️ Partial (delegated inside Claude) | ✅ Yes (@mention) |
| Your effort | 🔺 High | 🔺 Very high | ◼️ Medium | 🔹 Low |

## Features

| Category | Details |
|----------|---------|
| **Agent Support** | Claude Code, Gemini CLI, Codex, Amp, QWen Coder, and other popular agents |
| **Collaboration** | Group chat, shared context, @mention handoff, task tracking, session archive |
| **Team Presets** | 22 built-in member presets, 8 built-in team presets, one-click team import, custom presets with full CRUD |
| **Config** | Unified MCP config, flexible environment variables |
| **Platform** | Desktop app (Windows / macOS / Linux), web app (npx) |
| **Coming Soon** | Compact context optimization, more agent integrations |


## AI Team Presets

Stop configuring the same agents over and over. AgentsChatGroup includes **22 built-in member presets** and **8 built-in team presets** you can import in one click.

![AgentsChatGroup_AddTeam](docs/images/add_team_en.png)

You can also build and save your own custom presets. When importing a team, a preview shows exactly which members will be created, reused, or renamed - so there are no surprises.

📚 [Team Presets documentation](https://docs.agentschatgroup.com/core-features/ai-team-presets)

---

### Use Cases

**Full-stack Development Team**
> Architect designs the schema -> Coder implements -> Reviewer checks security -> Tester writes coverage. All in one chat, in parallel.

**Content Production Team**
> Researcher gathers sources -> Writer drafts -> Editor polishes. No copy-pasting between chats.

**Codebase Audit**
> Multiple agents scan different modules simultaneously. Get a full report in a fraction of the time.

**Data Pipeline**
> Cleaner preps data -> Analyst runs queries -> Visualizer builds charts. Each agent picks up where the last left off.

Assemble more AI teams. The future is yours to build.


## Tech Stack

| Layer | Technology |
|-------|-----------|
| Frontend | React + TypeScript + Vite + Tailwind CSS |
| Backend | Rust |
| Desktop | Tauri |

## Local Development

#### Mac/Linux

```bash
# 1. Clone the repository
git clone https://github.com/StarterraAI/AgentsChatGroup.git
cd agents-chatgroup

# 2. Install dependencies
pnpm i

# 3. Start the development server (runs Rust backend + React frontend)
pnpm run dev

# 4. Build frontend
pnpm --filter frontend build

# 5. Build desktop app
pnpm desktop:build
```

#### Windows (PowerShell): Start backend and frontend separately

`pnpm run dev` cannot run in Windows PowerShell. Use the following commands to run backend and frontend separately.

```bash
# 1. Clone the repository
git clone https://github.com/StarterraAI/AgentsChatGroup.git
cd AgentsChatGroup

# 2. Install dependencies
pnpm i

# 3. generate typescript type
pnpm run generate-types

# 4. run database migrate
pnpm run prepare-db
```

**Terminal A (backend)**

```powershell
$env:FRONTEND_PORT = node scripts/setup-dev-environment.js frontend
$env:BACKEND_PORT = node scripts/setup-dev-environment.js backend
$env:RUST_LOG = "debug"
cargo run --bin server
```

**Terminal B (frontend)**

```powershell
$env:FRONTEND_PORT = <frontend port generated from terminal A>
$env:BACKEND_PORT = <backend port generated from terminal A>
cd frontend
pnpm dev -- --port $env:FRONTEND_PORT --host
```

Open the frontend page at `http://localhost:<FRONTEND_PORT>` (example: `http://localhost:3001`).

## Contributing

We welcome contributions! Check what's needed in [Issues](https://github.com/StarterraAI/AgentsChatGroup/issues) or start a [Discussion](https://github.com/StarterraAI/AgentsChatGroup/discussions).

1. Fork -> feature branch -> PR
2. Please open an issue before large changes
3. Please follow our [Code of Conduct](./CODE_OF_CONDUCT.md)

## Community

| | |
|--|--|
| **Bug Reports** | [GitHub Issues](https://github.com/StarterraAI/AgentsChatGroup/issues) |
| **Discussions** | [GitHub Discussions](https://github.com/StarterraAI/AgentsChatGroup/discussions) |
| **Community Chat** | [Discord](https://discord.gg/MbgNFJeWDc) |

## Acknowledgements

Built on top of [Vibe Kanban](https://www.vibekanban.com/) - thanks to their team for the excellent open source foundation.
