<p align="center">
  <img src="frontend/public/agent-chatgroup-logo.svg" alt="Agent ChatGroup" width="200">
</p>

<h1 align="center">AgentsChatGroup</h1>

<p align="center">
  <strong>🚀 One Person, One AI Team</strong>
</p>

<p align="center">
  Make multiple AI Agents work together like a real team<br/>
  Share context in a group chat, @mention each other, and get more done
</p>

<p align="center">
  <a href="https://www.npmjs.com/package/agent-chatgroup"><img alt="npm" src="https://img.shields.io/npm/v/agent-chatgroup?style=flat-square" /></a>
  <a href="https://github.com/BloopAI/agent-chatgroup/blob/main/.github/workflows/publish.yml"><img alt="Build status" src="https://img.shields.io/github/actions/workflow/status/BloopAI/agent-chatgroup/.github%2Fworkflows%2Fpublish.yml" /></a>
  <a href="#"><img alt="License" src="https://img.shields.io/badge/license-MIT-blue.svg" /></a>
</p>

<p align="center">
  <a href="https://youtu.be/TFT3KnZOOAk">📺 Watch Demo</a> ·
  <a href="#getting-started">⚡ Getting Started</a> ·
  <a href="#documentation">📖 Docs</a>
</p>

---

## Why AgentsChatGroup?

### The Problem with AI Agents Today

We all rely on AI Agents like Claude Code, Gemini CLI, and Codex — they've become essential tools for developers. But you've probably run into these frustrations:

| Pain Point | What Happens |
|------------|--------------|
| ⏳**Long Wait Times** | A single Agent takes forever on a task, and you're stuck waiting |
| 🔀**Agents Work in Silos** | Each Agent runs in its own conversation, blind to what others are doing |
| 🔗**Lost Context** | You have to re-explain your project every time you switch Agents |
| 📢**You're the Middleman** | You end up copying info back and forth between Agents manually |
| 🚧**No Parallelism** | Complex tasks run one after another — can't hand off until one finishes |
| 🎭**Different Strengths** | Some Agents are better at coding, others at design or writing, so you're constantly switching |

> I'm not sure if you've felt this too, but **AI is getting more powerful, while we're getting more exhausted.** That's because we're constantly switching between the contexts of different agents, and our attention gets interrupted over and over again. So why not bring them all into a single unified space? That's the original motivation behind this project.

### Our Solution

**AgentsChatGroup** brings a simple but powerful idea: a **team group chat** for AI Agents. Let them collaborate just like real teammates:

```
╭──────────────────────────────────────────────────────────────╮
│                      AgentsChatGroup 🧩                      │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  👤 You                                                      │
│  │  @coder  Build a user login feature                       │
│                                                              │
│  🤖 Coder                                                    │
│  │  On it, writing the login module now...                   │
│  │  └─ 🔔 @reviewer  Done! Can you review this?              │
│                                                              │
│  🤖 Reviewer                                                 │
│  │  Let me take a look... Found some security issues:        │
│  │  1) Passwords need to be encrypted                        │
│  │  2) Add rate limiting for login attempts                  │
│  │  └─ 🛠️  @coder  Please fix these                          │
│                                                              │
│  🤖 Coder                                                    │
│  │  Got it, fixing now...                                    │
│                                                              │
╰──────────────────────────────────────────────────────────────╯

```

**Manage AI Agents like a real team:**
- One project = One group chat
- Clear context — as a manager, you can quickly check each AI member's status from the group messages
- Add as many AI members as you need, each with their own role
- Command all Agents in the same shared context
- Agents collaborate and communicate automatically
- You're the project lead — just plan and delegate, let the AI team do the work

## Key Benefits

<table>
<tr>
<td width="50%">

### 🔗 Shared Context

All Agents work in the same conversation space, **automatically sharing project background, discussion history, and code changes**.

No more repeating yourself — every Agent sees the full picture.

</td>
<td width="50%">

### 💬 @Mention System

Agents can **@mention each other** to hand off tasks automatically.

When one Agent finishes, it can directly tag another to continue — seamless collaboration.

</td>
</tr>
<tr>
<td>

### 👥 One-Person Team

Command a full AI development team by yourself:

- **🎨Designer**: Codex for design planning
- **🧑‍💻Coder**: Claude Code for writing code
- **🔍Reviewer**: Gemini CLI for code review
- **🧪Tester**: QWen Coder for writing tests
- **🚀Executor**: OpenClaw for deployment and testing
- **📝Doc**: Codex for documentation
- **...** Define more roles as needed

</td>
<td>

### ⚡ Multiply Your Output

- **Parallel Processing**: Multiple Agents work at the same time
- **Zero Communication Overhead**: Context syncs automatically
- **Play to Strengths**: Let each Agent focus on what it does best

</td>
</tr>
</table>

## Features

| Category | Features |
|----------|----------|
| **Agent Support** | ✅ Claude Code · ✅ Gemini CLI · ✅ Codex · ✅ Amp · More coming soon |
| **Collaboration** | ✅ Group Chat Mode · ✅ Shared Context · ✅ @Mention Dispatch · ✅ Task Tracking · ✅ Session Archive |
| **Configuration** | ✅ Unified MCP Config · ✅ Flexible Environment Variables |
| **Deployment** | ✅ Cross-platform Desktop App (Windows/macOS/Linux) · ✅ SSH Remote Deployment |
| **Coming Soon** | ▢ More compact context environment |

## Getting Started

### Prerequisites

Make sure you have at least one supported AI Agent installed:

| Agent | Installation |
|-------|--------------|
| [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `npm install -g @anthropic-ai/claude-code` |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `npm install -g @anthropic-ai/gemini-cli` |
| [Codex](https://github.com/openai/codex) | `npm install -g @openai/codex` |
| [QWen Coder](https://qwenlm.github.io/qwen-code-docs/en/users/overview/) | `npm install -g @qwen-code/qwen-code@latest` |

> For other Agents, check their official documentation for installation instructions.

### Install AgentsChatGroup

<details>
<summary><b>🪟 Windows</b></summary>

Download the latest installer:

[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D6?style=for-the-badge&logo=windows)](https://github.com/BloopAI/agent-chatgroup/releases/latest)

</details>

<details>
<summary><b>🍎 macOS</b></summary>

**Option 1: Using npx (Recommended)**

```bash
npx agents-chatgroup
```

**Option 2: Download the app**

[![Download for macOS](https://img.shields.io/badge/Download-macOS-000000?style=for-the-badge&logo=apple)](https://github.com/BloopAI/agent-chatgroup/releases/latest)

</details>

<details>
<summary><b>🐧 Linux</b></summary>

```bash
npx agents-chatgroup
```

</details>

That's it! Open the app, create a group chat, add your Agents, and start collaborating!

## Documentation

📚 Full documentation: [AgentsChatGroup Docs](https://agent-chatgroup.dev/docs)

| Guide | Description |
|-------|-------------|
| [Quick Start](https://agent-chatgroup.dev/docs/quickstart) | Get up and running in 5 minutes |
| [Configuration](https://agent-chatgroup.dev/docs/configuration) | Detailed configuration options |
| [Agent Setup](https://agent-chatgroup.dev/docs/agents) | How to configure and manage Agents |
| [FAQ](https://agent-chatgroup.dev/docs/faq) | Frequently asked questions |

## Community & Support

We'd love to hear from you!

| Channel | Link |
|---------|------|
| 🐛 **Bug Reports** | [GitHub Issues](https://github.com/BloopAI/agent-chatgroup/issues) |
| 💬 **Feature Requests** | [GitHub Discussions](https://github.com/BloopAI/agent-chatgroup/discussions) |
| 💭 **Community Chat** | *Coming soon* |

## Local Development

Want to contribute? Here's how to set up the project locally.

### Requirements

| Tool | Version | Link |
|------|---------|------|
| Rust | latest stable | [rustup.rs](https://rustup.rs/) |
| Node.js | >= 18 | [nodejs.org](https://nodejs.org/) |
| pnpm | >= 8 | [pnpm.io](https://pnpm.io/) |

**Optional tools:**

```bash
cargo install cargo-watch  # Auto-reload on changes
cargo install sqlx-cli     # Database management
```

### Development Steps

```bash
# 1. Clone the repo
git clone https://github.com/BloopAI/agent-chatgroup.git
cd agent-chatgroup

# 2. Install dependencies
pnpm i

# 3. Start dev server (launches both Rust backend + React frontend)
pnpm run dev

# 4. Build frontend
pnpm --filter frontend build

# 5. Build desktop app
pnpm desktop:build
```

### Tech Stack

```
┌────────────────────────────────────────────┐
│              AgentsChatGroup               │
├────────────────────────────────────────────┤
│  Frontend │ React + TypeScript + Vite      │
│           │ Tailwind CSS                   │
├───────────┼────────────────────────────────┤
│  Backend  │ Rust                           │
├───────────┼────────────────────────────────┤
│  Desktop  │ Tauri                          │
└───────────┴────────────────────────────────┘
```

## Acknowledgements

This project is built on top of [Vibe Kanban](https://www.vibekanban.com/). Thanks to the Vibe Kanban team for their open source contribution, which provided an excellent foundation for this project.

## Contributing

We welcome contributions! 🎉

1. **Fork** this repo
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a **Pull Request**

> 💡 Before submitting a PR, please discuss your idea via [Issue](https://github.com/BloopAI/agent-chatgroup/issues) or [Discussion](https://github.com/BloopAI/agent-chatgroup/discussions).

## License

This project is licensed under the [MIT License](LICENSE).
