import { ChatSessionAgentState } from 'shared/types';
import type { MessageTone } from './types';

// Detect an active mention query at the input tail. The token before `@`
// can be start-of-text or any character so `hello@` also opens suggestions.
export const mentionRegex = /(^|[\s\S])@([\p{L}\p{N}_-]*)$/u;
export const mentionTokenRegex = /(^|\s)@([\p{L}\p{N}_-]+)/gu;
export const mentionAllKeyword = 'all';
export const mentionAllAliases = [
  mentionAllKeyword,
  'everyone',
  'allmembers',
  'all_member',
  '所有人',
  '全体',
] as const;
const mentionAllAliasSet = new Set(
  mentionAllAliases.map((alias) => alias.toLowerCase())
);
export const isMentionAllAlias = (value: string): boolean =>
  mentionAllAliasSet.has(value.trim().toLowerCase());
export const memberNameRegex = /^[\p{L}\p{N}_-]+$/u;
export const MAX_MEMBER_NAME_LENGTH = 30;
export const getMemberNameLength = (value: string) =>
  Array.from(value.trim()).length;

export const fallbackRunnerTypes = [
  'CLAUDE_CODE',
  'CODEX',
  'AMP',
  'GEMINI',
  'OPENCODE',
  'CURSOR_AGENT',
  'QWEN_CODE',
  'COPILOT',
  'DROID',
  'KIMI_CODE',
];

export const agentStateLabels: Record<ChatSessionAgentState, string> = {
  idle: 'Idle',
  running: 'Running',
  waitingapproval: 'Waiting approval',
  dead: 'Dead',
};

export const agentStateDotClass: Record<ChatSessionAgentState, string> = {
  idle: 'bg-low',
  running: 'bg-brand',
  waitingapproval: 'bg-brand-secondary',
  dead: 'bg-error',
};

export const messagePalette: MessageTone[] = [
  { bg: 'rgba(226, 239, 255, 0.8)', border: 'rgba(176, 205, 242, 0.7)' },
  { bg: 'rgba(231, 249, 239, 0.85)', border: 'rgba(176, 223, 198, 0.7)' },
  { bg: 'rgba(255, 243, 227, 0.85)', border: 'rgba(231, 204, 173, 0.7)' },
  { bg: 'rgba(245, 236, 255, 0.8)', border: 'rgba(209, 187, 234, 0.7)' },
  { bg: 'rgba(255, 236, 242, 0.8)', border: 'rgba(232, 184, 198, 0.7)' },
  { bg: 'rgba(238, 244, 248, 0.85)', border: 'rgba(189, 205, 217, 0.7)' },
];

export const userMessageTone: MessageTone = {
  bg: 'rgba(219, 238, 255, 0.9)',
  border: 'rgba(156, 197, 237, 0.8)',
};
