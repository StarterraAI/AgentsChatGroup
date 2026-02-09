import type {
  ChatAgent,
  ChatSessionAgent,
} from 'shared/types';

export type StreamRun = {
  agentId: string;
  content: string;
  isFinal: boolean;
};

export type AgentStateInfo = {
  state: import('shared/types').ChatSessionAgentState;
  startedAt: string | null;
};

export type MentionStatus = 'received' | 'running' | 'completed' | 'failed';

export type RunHistoryItem = {
  runId: string;
  agentId: string;
  createdAt: string;
  content: string;
};

export type SessionMember = {
  agent: ChatAgent;
  sessionAgent: ChatSessionAgent;
};

export type DiffMeta = {
  runId: string | null;
  preview: string | null;
  truncated: boolean;
  available: boolean;
  untrackedFiles: string[];
};

export type ChatAttachment = {
  id: string;
  name: string;
  mime_type?: string | null;
  size_bytes?: number;
  kind?: string;
  relative_path?: string;
};

export type DiffFileEntry = {
  path: string;
  patch: string;
  additions: number;
  deletions: number;
};

export type RunDiffState = {
  loading: boolean;
  error: string | null;
  files: DiffFileEntry[];
};

export type UntrackedFileState = {
  loading: boolean;
  error: string | null;
  content: string | null;
  open: boolean;
};

export type MessageTone = {
  bg: string;
  border: string;
};
