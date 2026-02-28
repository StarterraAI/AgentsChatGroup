import { parseDiffStats } from '@/utils/diffStatsParser';
import type { TFunction } from 'i18next';
import type {
  ChatMemberPreset,
  ChatTeamPreset,
  JsonValue,
} from 'shared/types';
import { mentionTokenRegex, messagePalette, userMessageTone } from './constants';
import type {
  ChatAttachment,
  DiffFileEntry,
  DiffMeta,
  MessageTone,
  SessionMember,
} from './types';

export function hashKey(value: string): number {
  let hash = 0;
  for (let i = 0; i < value.length; i += 1) {
    hash = (hash << 5) - hash + value.charCodeAt(i);
    hash |= 0;
  }
  return Math.abs(hash);
}

export function getMessageTone(key: string, isUser: boolean): MessageTone {
  if (isUser) {
    return userMessageTone;
  }
  const index = hashKey(key) % messagePalette.length;
  return messagePalette[index];
}

export function extractMentions(content: string): Set<string> {
  const mentions = new Set<string>();
  for (const match of content.matchAll(mentionTokenRegex)) {
    const name = match[2];
    if (name) mentions.add(name);
  }
  return mentions;
}

export function extractRunId(meta: unknown): string | null {
  if (!meta || typeof meta !== 'object') return null;
  const runId = (meta as { run_id?: unknown }).run_id;
  return typeof runId === 'string' ? runId : null;
}

export function sanitizeHandle(value: string | null | undefined): string {
  if (!value) return 'you';
  const sanitized = value
    .split('@')[0]
    .split(' ')[0]
    .trim()
    .replace(/[^a-zA-Z0-9_-]/g, '');
  return sanitized.length > 0 ? sanitized : 'you';
}

export function extractDiffMeta(meta: unknown): DiffMeta {
  if (!meta || typeof meta !== 'object') {
    return {
      runId: null,
      preview: null,
      truncated: false,
      available: false,
      untrackedFiles: [],
    };
  }
  const raw = meta as {
    run_id?: unknown;
    diff_preview?: unknown;
    diff_truncated?: unknown;
    diff_available?: unknown;
    untracked_files?: unknown;
  };
  const runId = typeof raw.run_id === 'string' ? raw.run_id : null;
  const preview = typeof raw.diff_preview === 'string' ? raw.diff_preview : null;
  const truncated = raw.diff_truncated === true;
  const available = raw.diff_available === true || preview !== null;
  const untrackedFiles = Array.isArray(raw.untracked_files)
    ? raw.untracked_files.filter((item) => typeof item === 'string')
    : [];
  return { runId, preview, truncated, available, untrackedFiles };
}

export function extractReferenceId(meta: unknown): string | null {
  if (!meta || typeof meta !== 'object') return null;
  const raw = meta as {
    reference?: { message_id?: unknown };
    reference_message_id?: unknown;
  };
  if (raw.reference && typeof raw.reference === 'object') {
    const value = raw.reference.message_id;
    if (typeof value === 'string') return value;
  }
  return typeof raw.reference_message_id === 'string'
    ? raw.reference_message_id
    : null;
}

export function extractAttachments(meta: unknown): ChatAttachment[] {
  if (!meta || typeof meta !== 'object') return [];
  const raw = meta as { attachments?: unknown };
  if (!Array.isArray(raw.attachments)) return [];
  return raw.attachments
    .filter((item) => item && typeof item === 'object')
    .map((item) => item as ChatAttachment)
    .filter((item) => typeof item.id === 'string');
}

export function formatBytes(value?: number | null): string {
  if (!value || value <= 0) return '';
  const units = ['B', 'KB', 'MB', 'GB'];
  let size = value;
  let unitIndex = 0;
  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex += 1;
  }
  const decimals = size >= 10 || unitIndex === 0 ? 0 : 1;
  return `${size.toFixed(decimals)} ${units[unitIndex]}`;
}

export function truncateText(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  return `${value.slice(0, maxLength - 1)}…`;
}

/**
 * Detect if message content contains common API error patterns
 * Returns an error type string if detected, or null otherwise
 * Supports: Claude, OpenAI/Codex, QWen Coder, Azure OpenAI, Google AI, and other providers
 */
export function detectApiError(content: string): {
  type: 'quota_exceeded' | 'rate_limit' | 'overloaded' | 'credit_exhausted' | 'auth_error' | 'context_limit' | 'other_error';
  message: string;
  provider?: string;
} | null {
  const lowered = content.toLowerCase();

  // === Anthropic/Claude specific errors ===
  if (
    lowered.includes('anthropic') ||
    lowered.includes('claude')
  ) {
    if (lowered.includes('credit balance') || lowered.includes('credit exhausted')) {
      return { type: 'credit_exhausted', message: 'Claude credit balance exhausted', provider: 'Anthropic' };
    }
    if (lowered.includes('rate limit') || lowered.includes('rate_limit')) {
      return { type: 'rate_limit', message: 'Claude API rate limit exceeded', provider: 'Anthropic' };
    }
    if (lowered.includes('overloaded')) {
      return { type: 'overloaded', message: 'Claude API is overloaded', provider: 'Anthropic' };
    }
  }

  // === OpenAI/Codex specific errors ===
  if (
    lowered.includes('openai') ||
    lowered.includes('codex') ||
    lowered.includes('gpt-4') ||
    lowered.includes('gpt-3') ||
    lowered.includes('o1-') ||
    lowered.includes('o3-')
  ) {
    if (
      lowered.includes('billing hard limit') ||
      lowered.includes('exceeded your current quota') ||
      lowered.includes('insufficient_quota')
    ) {
      return { type: 'quota_exceeded', message: 'OpenAI quota exceeded', provider: 'OpenAI' };
    }
    if (lowered.includes('rate limit') || lowered.includes('rate_limit_exceeded')) {
      return { type: 'rate_limit', message: 'OpenAI API rate limit exceeded', provider: 'OpenAI' };
    }
    if (lowered.includes('context_length_exceeded') || lowered.includes('maximum context length')) {
      return { type: 'context_limit', message: 'OpenAI context length exceeded', provider: 'OpenAI' };
    }
    if (lowered.includes('invalid_api_key') || lowered.includes('incorrect api key')) {
      return { type: 'auth_error', message: 'OpenAI API key invalid', provider: 'OpenAI' };
    }
  }

  // === Alibaba Cloud / QWen Coder specific errors ===
  if (
    lowered.includes('qwen') ||
    lowered.includes('tongyi') ||
    lowered.includes('dashscope') ||
    lowered.includes('aliyun') ||
    lowered.includes('alibaba')
  ) {
    if (
      lowered.includes('quota') ||
      lowered.includes('余额不足') ||
      lowered.includes('账户余额') ||
      lowered.includes('免费额度')
    ) {
      return { type: 'quota_exceeded', message: 'QWen API 额度已用尽', provider: 'Alibaba' };
    }
    if (
      lowered.includes('rate limit') ||
      lowered.includes('限流') ||
      lowered.includes('请求过于频繁') ||
      lowered.includes('qps')
    ) {
      return { type: 'rate_limit', message: 'QWen API 请求频率超限', provider: 'Alibaba' };
    }
    if (lowered.includes('accessdenied') || lowered.includes('invalidaccesskey')) {
      return { type: 'auth_error', message: 'QWen API 密钥无效', provider: 'Alibaba' };
    }
  }

  // === Azure OpenAI specific errors ===
  if (lowered.includes('azure') && lowered.includes('openai')) {
    if (lowered.includes('quota') || lowered.includes('tokens per minute')) {
      return { type: 'quota_exceeded', message: 'Azure OpenAI quota exceeded', provider: 'Azure' };
    }
    if (lowered.includes('rate limit') || lowered.includes('429')) {
      return { type: 'rate_limit', message: 'Azure OpenAI rate limit exceeded', provider: 'Azure' };
    }
  }

  // === Google AI / Gemini specific errors ===
  if (
    lowered.includes('google') ||
    lowered.includes('gemini') ||
    lowered.includes('palm') ||
    lowered.includes('vertex')
  ) {
    if (lowered.includes('quota') || lowered.includes('resource_exhausted')) {
      return { type: 'quota_exceeded', message: 'Google AI quota exceeded', provider: 'Google' };
    }
    if (lowered.includes('rate limit') || lowered.includes('429')) {
      return { type: 'rate_limit', message: 'Google AI rate limit exceeded', provider: 'Google' };
    }
  }

  // === DeepSeek specific errors ===
  if (lowered.includes('deepseek')) {
    if (lowered.includes('quota') || lowered.includes('balance')) {
      return { type: 'quota_exceeded', message: 'DeepSeek 额度已用尽', provider: 'DeepSeek' };
    }
    if (lowered.includes('rate limit') || lowered.includes('429')) {
      return { type: 'rate_limit', message: 'DeepSeek API 请求频率超限', provider: 'DeepSeek' };
    }
  }

  // === Generic quota/credit exhaustion (fallback) ===
  if (
    lowered.includes('quota exceeded') ||
    lowered.includes('quota_exceeded') ||
    lowered.includes('credit balance') ||
    lowered.includes('credit exhausted') ||
    lowered.includes('insufficient credit') ||
    lowered.includes('insufficient_quota') ||
    (lowered.includes('billing') && lowered.includes('limit')) ||
    lowered.includes('余额不足') ||
    lowered.includes('额度') && (lowered.includes('用尽') || lowered.includes('不足'))
  ) {
    return {
      type: 'quota_exceeded',
      message: 'API quota or credit limit reached',
    };
  }

  // === Generic rate limiting (fallback) ===
  if (
    lowered.includes('rate limit') ||
    lowered.includes('rate_limit') ||
    lowered.includes('too many requests') ||
    lowered.includes('429') ||
    lowered.includes('请求过于频繁') ||
    lowered.includes('限流')
  ) {
    return {
      type: 'rate_limit',
      message: 'API rate limit exceeded',
    };
  }

  // === Generic server overload (fallback) ===
  if (
    lowered.includes('overloaded') ||
    lowered.includes('server is busy') ||
    lowered.includes('503') ||
    lowered.includes('service unavailable') ||
    lowered.includes('服务繁忙') ||
    lowered.includes('系统繁忙')
  ) {
    return {
      type: 'overloaded',
      message: 'API server is overloaded',
    };
  }

  // === Generic authentication errors (fallback) ===
  if (
    lowered.includes('invalid api key') ||
    lowered.includes('invalid_api_key') ||
    lowered.includes('authentication failed') ||
    lowered.includes('unauthorized') ||
    lowered.includes('401') ||
    lowered.includes('密钥无效') ||
    lowered.includes('认证失败')
  ) {
    return {
      type: 'auth_error',
      message: 'API authentication failed',
    };
  }

  // === Context/token limit errors (fallback) ===
  if (
    lowered.includes('context length') ||
    lowered.includes('context_length') ||
    lowered.includes('token limit') ||
    lowered.includes('maximum.*tokens') ||
    lowered.includes('上下文长度') ||
    lowered.includes('超出最大')
  ) {
    return {
      type: 'context_limit',
      message: 'Context or token limit exceeded',
    };
  }

  return null;
}

export function splitUnifiedDiff(patch: string): DiffFileEntry[] {
  const lines = patch.split('\n');
  const entries: DiffFileEntry[] = [];
  let current: string[] = [];
  let currentPath = '';

  const flush = () => {
    if (current.length === 0) return;
    const content = current.join('\n');
    const path = currentPath || 'unknown';
    const stats = parseDiffStats(content);
    entries.push({
      path,
      patch: content,
      additions: stats.additions,
      deletions: stats.deletions,
    });
  };

  for (const line of lines) {
    if (line.startsWith('diff --git ')) {
      flush();
      current = [line];
      const match = /^diff --git a\/(.+?) b\/(.+)$/.exec(line);
      currentPath = match?.[2] || line.replace('diff --git ', '').trim();
      continue;
    }
    if (current.length > 0) {
      current.push(line);
    }
  }

  flush();
  return entries;
}

function stableStringifyJson(value: unknown): string {
  if (value === null || typeof value !== 'object') {
    return JSON.stringify(value);
  }

  if (Array.isArray(value)) {
    return `[${value.map((item) => stableStringifyJson(item)).join(',')}]`;
  }

  const entries = Object.entries(value as Record<string, unknown>).sort(
    ([a], [b]) => a.localeCompare(b)
  );
  return `{${entries
    .map(([key, item]) => `${JSON.stringify(key)}:${stableStringifyJson(item)}`)
    .join(',')}}`;
}

export function normalizePresetToolsEnabled(
  value: unknown
): JsonValue {
  if (value === null || value === undefined) {
    return {};
  }
  if (typeof value !== 'object' || Array.isArray(value)) {
    return {};
  }
  return value as JsonValue;
}

export function areToolsEnabledEqual(a: unknown, b: unknown): boolean {
  return stableStringifyJson(normalizePresetToolsEnabled(a)) ===
    stableStringifyJson(normalizePresetToolsEnabled(b));
}

export function resolvePresetRunnerType({
  presetRunnerType,
  defaultRunnerType,
  enabledRunnerTypes,
  availableRunnerTypes,
}: {
  presetRunnerType: string | null | undefined;
  defaultRunnerType: string | null | undefined;
  enabledRunnerTypes: string[];
  availableRunnerTypes: string[];
}): string | null {
  const trimmedPresetRunner = presetRunnerType?.trim();
  if (
    trimmedPresetRunner &&
    (enabledRunnerTypes.includes(trimmedPresetRunner) ||
      availableRunnerTypes.includes(trimmedPresetRunner))
  ) {
    return trimmedPresetRunner;
  }

  const trimmedDefaultRunner = defaultRunnerType?.trim();
  if (
    trimmedDefaultRunner &&
    enabledRunnerTypes.includes(trimmedDefaultRunner)
  ) {
    return trimmedDefaultRunner;
  }

  if (enabledRunnerTypes.length > 0) {
    return enabledRunnerTypes[0];
  }
  if (availableRunnerTypes.length > 0) {
    return availableRunnerTypes[0];
  }
  return null;
}

export function resolveUniqueAgentName(
  requestedName: string,
  takenNamesLowercase: Set<string>
): string {
  const trimmed = requestedName.trim();
  const baseName = trimmed.length > 0 ? trimmed : 'agent';
  let candidate = baseName;
  let suffix = 2;
  while (takenNamesLowercase.has(candidate.toLowerCase())) {
    candidate = `${baseName}_${suffix}`;
    suffix += 1;
  }
  takenNamesLowercase.add(candidate.toLowerCase());
  return candidate;
}

export function getSessionWorkspacePath(
  sessionId: string,
  agentName: string
): string {
  return `chat/session_${sessionId}/agents/${agentName}`;
}

export function validateWorkspacePath(path: string): string | null {
  const trimmed = path.trim();
  if (!trimmed) {
    return 'Workspace path is required.';
  }

  if (trimmed.includes('\0')) {
    return 'Workspace path contains invalid characters.';
  }

  const normalized = trimmed.replace(/\\/g, '/');
  const segments = normalized.split('/').filter((segment) => segment.length > 0);
  if (segments.includes('..')) {
    return "Workspace path cannot contain '..'.";
  }

  return null;
}

export type MemberPresetImportAction = 'create' | 'reuse' | 'skip';

export interface MemberPresetImportPlan {
  presetId: string;
  presetName: string;
  runnerType: string;
  finalName: string;
  systemPrompt: string;
  toolsEnabled: JsonValue;
  action: MemberPresetImportAction;
  reason: string;
  agentId: string | null;
  workspacePath: string;
}

export function getLocalizedMemberPresetName(
  preset: Pick<ChatMemberPreset, 'id' | 'name' | 'is_builtin'>,
  t: TFunction<'chat'>
): string {
  if (!preset.is_builtin) return preset.name;
  return t(`members.presetDisplay.members.${preset.id}`, {
    defaultValue: preset.name,
  });
}

export function getLocalizedTeamPresetName(
  preset: Pick<ChatTeamPreset, 'id' | 'name' | 'is_builtin'>,
  t: TFunction<'chat'>
): string {
  if (!preset.is_builtin) return preset.name;
  return t(`members.presetDisplay.teams.${preset.id}`, {
    defaultValue: preset.name,
  });
}

export function getLocalizedMemberPresetNameById(
  presetId: string,
  fallbackName: string,
  t: TFunction<'chat'>
): string {
  return t(`members.presetDisplay.members.${presetId}`, {
    defaultValue: fallbackName,
  });
}

export function buildMemberPresetImportPlan({
  preset,
  sessionId,
  sessionMembers,
  defaultRunnerType,
  enabledRunnerTypes,
  availableRunnerTypes,
  takenNamesLowercase,
}: {
  preset: ChatMemberPreset;
  sessionId: string;
  sessionMembers: SessionMember[];
  defaultRunnerType: string | null | undefined;
  enabledRunnerTypes: string[];
  availableRunnerTypes: string[];
  takenNamesLowercase: Set<string>;
}): MemberPresetImportPlan | null {
  const runnerType = resolvePresetRunnerType({
    presetRunnerType: preset.runner_type,
    defaultRunnerType,
    enabledRunnerTypes,
    availableRunnerTypes,
  });
  if (!runnerType) {
    return null;
  }

  const presetName = preset.name.trim().length > 0
    ? preset.name.trim()
    : preset.id;
  const systemPrompt = preset.system_prompt?.trim() ?? '';
  const toolsEnabled = normalizePresetToolsEnabled(preset.tools_enabled);
  const hasSameNameInSession = sessionMembers.some(
    (member) => member.agent.name.toLowerCase() === presetName.toLowerCase()
  );
  if (hasSameNameInSession) {
    return {
      presetId: preset.id,
      presetName: preset.name,
      runnerType,
      finalName: presetName,
      systemPrompt,
      toolsEnabled,
      action: 'skip',
      reason: 'duplicate-name-in-session',
      agentId: null,
      workspacePath:
        preset.default_workspace_path?.trim() ||
        getSessionWorkspacePath(sessionId, presetName),
    };
  }

  const finalName = resolveUniqueAgentName(presetName, takenNamesLowercase);
  return {
    presetId: preset.id,
    presetName: preset.name,
    runnerType,
    finalName,
    systemPrompt,
    toolsEnabled,
    action: 'create',
    reason: 'create-new-agent',
    agentId: null,
    workspacePath:
      preset.default_workspace_path?.trim() ||
      getSessionWorkspacePath(sessionId, finalName),
  };
}
