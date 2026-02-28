# Repository Guidelines

## Project Overview

**AgentsChatGroup** is a multi-agent conversation platform where multiple AI agents (Claude Code, Gemini CLI, Codex, QWen Coder, etc.) can collaborate in shared chat sessions like a real team. Key features:
- **Multi-agent chat sessions** with real-time streaming
- **Context synchronization** across agents
- **Agent execution tracking** with diffs and logs
- **Session archive/restore** functionality
- **Permission-based access control**

### Supported AI Agents
- Claude Code (`@anthropic-ai/claude-code`)
- Gemini CLI (`@google/gemini-cli`)
- Codex (`@openai/codex`)
- QWen Coder (`@qwen-code/qwen-code`)
- Amp
- More coming soon

## Project Structure & Module Organization

### Backend (Rust Workspace)
- `crates/db/`: SQLx database models and migrations
  - Core chat models: `chat_session.rs`, `chat_agent.rs`, `chat_session_agent.rs`, `chat_message.rs`, `chat_run.rs`, `chat_permission.rs`, `chat_artifact.rs`
  - Legacy models: `task.rs`, `workspace.rs`, `session.rs` (legacy), `image.rs`
  - Migrations: `migrations/20260205190000_add_chat_tables.sql`
- `crates/server/`: API server and binaries
  - Chat routes: `src/routes/chat/` (sessions.rs, agents.rs, messages.rs, runs.rs)
  - Type generation: `src/bin/generate_types.rs`
- `crates/services/`: Business logic
  - `chat.rs`: Message parsing, mentions, attachments
  - `chat_runner.rs`: Agent execution orchestration, WebSocket streaming, workspace-scoped chat context/run artifacts
- `crates/executors/`, `crates/utils/`, `crates/deployment/`, `crates/local-deployment/`, `crates/git/`, `crates/review/`: Supporting crates

### Frontend
- `frontend/`: Main React + TypeScript application (Vite, Tailwind)
  - `src/pages/ui-new/`: **New design** pages (ChatSessions.tsx, Workspaces.tsx, WorkspacesLanding.tsx, ProjectKanban.tsx, VSCodeWorkspacePage.tsx, MigratePage.tsx)
  - `src/pages/`: **Legacy design** pages (Projects.tsx, ProjectTasks.tsx - wrapped in LegacyDesignScope)
  - `src/components/ui-new/`: New design system components
    - `primitives/`: ChatBoxBase, CreateChatBox, SessionChatBox
    - `primitives/conversation/`: Message renderers (ChatUserMessage, ChatAssistantMessage, ChatSystemMessage, ChatMarkdown, ChatAggregatedDiffEntries, etc.)
    - `containers/`: State management wrappers
  - `src/components/ui/`: Legacy design system components (still used by legacy pages)
  - `src/lib/api.ts`: API client with chatApi methods
  - `src/styles/`: CSS for both design systems
    - `new/index.css` + `tailwind.new.config.js` (scoped to `.new-design`)
    - `legacy/index.css` + `tailwind.legacy.config.js` (scoped to `.legacy-design`)
- `remote-frontend/`: Lightweight remote deployment frontend
- `shared/`: Generated TypeScript types from Rust (`shared/types.ts` - auto-generated, do not edit)
- `agents-chatgroup-npx/`: NPM CLI package (`agents-chatgroup`) for cross-platform installer and runner
- `scripts/`: Development helpers (port management, DB preparation, desktop packaging)
- `docs/`: Documentation (Mintlify setup)
- `src-tauri/`: Tauri desktop application configuration
- `assets/`, `dev_assets_seed/`, `dev_assets/`: Packaged and local dev assets

## Chat System Architecture

### Database Schema
The chat system uses 7 core entities:
- **ChatSession**: Conversation containers with title, status (active/archived), timestamps
- **ChatAgent**: Agent definitions with name, runner_type, system_prompt, tools_enabled
- **ChatSessionAgent**: Join table linking agents to sessions; tracks state (idle/running/waiting_approval/dead), workspace_path
- **ChatMessage**: Messages with sender_type (user/agent/system), mentions, metadata
- **ChatRun**: Agent execution runs per session_agent; includes run_index, run_dir, log/output paths
- **ChatPermission**: Access control with capability, scope, TTL
- **ChatArtifact**: Files/artifacts pinned to sessions

### API Routes (`/chat`)
```
/chat
├── /sessions (list, create)
│   ├── /{session_id}
│   │   ├── / (get, update, delete)
│   │   ├── /archive (POST), /restore (POST)
│   │   ├── /stream (WebSocket - real-time events)
│   │   ├── /agents (list, create session agents)
│   │   ├── /agents/{session_agent_id} (update, delete)
│   │   ├── /messages (list, create)
│   │   └── /messages/{message_id} (get, delete, upload attachments)
├── /agents (list, create, update, delete)
└── /runs/{run_id} (log, diff, untracked files)
```

### Frontend Chat Components
- **Main Page**: `frontend/src/pages/ui-new/ChatSessions.tsx` (93KB)
  - Lists active/archived sessions
  - Create new sessions with agent members
  - Real-time WebSocket message streaming
  - @mentions autocomplete
  - Diff viewer and run output display
- **UI Primitives**: `frontend/src/components/ui-new/primitives/`
  - `ChatBoxBase.tsx`, `CreateChatBox.tsx`, `SessionChatBox.tsx`
  - `conversation/`: 15+ specialized message renderers
- **API Client**: `frontend/src/lib/api.ts` exports `chatApi` object

### Data Flow
1. User creates session → API → DB (chat_sessions)
2. User adds agents → API → DB (chat_session_agents)
3. User sends message → API → mention parsing → DB
4. Agent executes → chat_runner orchestrates → WebSocket streams events
5. Run artifacts captured → stored with diffs/logs

### Runtime Storage (Workspace-Scoped)
- Agents are restricted to their configured workspace path for file access.
- Chat context file path:
  - `<workspace>/.agents_chatgroup/context/<session_id>/messages.jsonl`
- Chat run records path:
  - `<workspace>/.agents_chatgroup/runs/<session_id>/run_records/...`
- Per-run context snapshot path:
  - `<run_dir>/context.jsonl`
- Internal `.agents_chatgroup/` files should be treated as runtime artifacts, not user source files.

## Design System Status

The project currently maintains two design systems during transition:
- **New Design** (`.new-design` scope): Used in `pages/ui-new/` - ChatSessions, Workspaces, ProjectKanban
  - Components: `components/ui-new/`
  - Styles: `styles/new/index.css` + `tailwind.new.config.js`
- **Legacy Design** (`.legacy-design` scope): Used in `pages/` - Projects, ProjectTasks
  - Components: `components/ui/`, `components/legacy-design/`
  - Styles: `styles/legacy/index.css` + `tailwind.legacy.config.js`

Both scopes coexist via CSS class scoping. New features should use the **new design system**.

## Managing Shared Types Between Rust and TypeScript

ts-rs allows you to derive TypeScript types from Rust structs/enums. By annotating your Rust types with #[derive(TS)] and related macros, ts-rs will generate .ts declaration files for those types.

**Key shared types** exported in `shared/types.ts`:
- Chat: `ChatSession`, `ChatMessage`, `ChatAgent`, `ChatSessionAgent`, `ChatRun`, `ChatPermission`, `ChatArtifact`
- Stream: `ChatStreamEvent` (message_new, agent_delta, agent_state)
- Enums: `ChatSessionStatus` (active/archived), `ChatSenderType` (user/agent/system), `ChatSessionAgentState` (idle/running/waiting_approval/dead)

When making changes to the types, you can regenerate them using `pnpm run generate-types`
Do not manually edit shared/types.ts, instead edit crates/server/src/bin/generate_types.rs

## Build, Test, and Development Commands
- Install: `pnpm i`
- Run dev (frontend + backend with ports auto-assigned): `pnpm run dev`
- Backend (watch): `pnpm run backend:dev:watch`
- Frontend (dev): `pnpm run frontend:dev`
- Type checks: `pnpm run check` (frontend) and `pnpm run backend:check` (Rust cargo check)
- Rust tests: `cargo test --workspace`
- Generate TS types from Rust: `pnpm run generate-types` (or `generate-types:check` in CI)
- Prepare SQLx (offline): `pnpm run prepare-db`
- Prepare SQLx (remote package, postgres): `pnpm run remote:prepare-db`
- Local NPX build: `pnpm run build:npx` then `pnpm pack` in `agents-chatgroup-npx/`

## Coding Style & Naming Conventions
- Rust: `rustfmt` enforced (`rustfmt.toml`); group imports by crate; snake_case modules, PascalCase types.
- TypeScript/React: ESLint + Prettier (2 spaces, single quotes, 80 cols). PascalCase components, camelCase vars/functions, kebab-case file names where practical.
- Keep functions small, add `Debug`/`Serialize`/`Deserialize` where useful.

## Testing Guidelines
- Rust: prefer unit tests alongside code (`#[cfg(test)]`), run `cargo test --workspace`. Add tests for new logic and edge cases.
- Frontend: ensure `pnpm run check` and `pnpm run lint` pass. If adding runtime logic, include lightweight tests (e.g., Vitest) in the same directory.

## Security & Config Tips
- Use `.env` for local overrides; never commit secrets. Key envs: `FRONTEND_PORT`, `BACKEND_PORT`, `HOST` 
- Dev ports and assets are managed by `scripts/setup-dev-environment.js`.
