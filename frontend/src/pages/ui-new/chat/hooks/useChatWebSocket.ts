import { useCallback, useEffect, useMemo, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import {
  type ChatMessage,
  type ChatSessionAgent,
  type ChatSessionAgentState,
  type ChatStreamEvent,
  type CompressionWarning,
} from 'shared/types';
import { chatApi } from '@/lib/api';
import type { AgentStateInfo, MentionStatus, StreamRun } from '../types';
import { extractRunId } from '../utils';

type MentionAcknowledgedEvent = {
  type: 'mention_acknowledged';
  session_id: string;
  message_id: string;
  mentioned_agent: string;
  agent_id: string;
  status: MentionStatus;
};

type ChatStreamPayload = ChatStreamEvent | MentionAcknowledgedEvent;
type AgentDeltaPayload = Extract<ChatStreamEvent, { type: 'agent_delta' }> & {
  type: 'agent_delta';
  stream_type?: 'assistant' | 'thinking';
};

type PersistedStreamRun = StreamRun & {
  updatedAtMs: number;
};

type StreamingRunsBySession = Record<
  string,
  Record<string, PersistedStreamRun>
>;

type StreamingRunCachePayload = {
  version: number;
  runs_by_session: StreamingRunsBySession;
};

const STREAMING_RUN_CACHE_KEY = 'chat_streaming_runs_cache_v1';
const STREAMING_RUN_CACHE_VERSION = 1;
const STREAMING_RUN_TTL_MS = 6 * 60 * 60 * 1000;
const INACTIVE_RUN_PRUNE_GRACE_MS = 15 * 1000;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === 'object' && !Array.isArray(value);

const extractCompressionWarningFromMeta = (
  meta: unknown
): CompressionWarning | null => {
  if (!isRecord(meta)) return null;
  const rawWarning = meta.compression_warning;
  if (!isRecord(rawWarning)) return null;

  const code = rawWarning.code;
  const message = rawWarning.message;
  const splitFilePath = rawWarning.split_file_path;

  if (
    typeof code !== 'string' ||
    typeof message !== 'string' ||
    typeof splitFilePath !== 'string'
  ) {
    return null;
  }

  return {
    code,
    message,
    split_file_path: splitFilePath,
  };
};

function pruneExpiredStreamingRuns(
  runsBySession: StreamingRunsBySession,
  nowMs: number = Date.now()
): StreamingRunsBySession {
  let changed = false;
  const nextRunsBySession: StreamingRunsBySession = {};

  for (const [sessionId, runs] of Object.entries(runsBySession)) {
    const nextRuns: Record<string, PersistedStreamRun> = {};
    const runEntries = Object.entries(runs);

    for (const [runId, run] of runEntries) {
      if (nowMs - run.updatedAtMs > STREAMING_RUN_TTL_MS) {
        changed = true;
        continue;
      }
      nextRuns[runId] = run;
    }

    if (Object.keys(nextRuns).length > 0) {
      nextRunsBySession[sessionId] = nextRuns;
    } else if (runEntries.length > 0) {
      changed = true;
    }

    if (runEntries.length !== Object.keys(nextRuns).length) {
      changed = true;
    }
  }

  if (
    !changed &&
    Object.keys(nextRunsBySession).length === Object.keys(runsBySession).length
  ) {
    return runsBySession;
  }

  return nextRunsBySession;
}

function readStreamingRunsCache(): StreamingRunsBySession {
  if (typeof window === 'undefined') return {};

  try {
    const raw = window.localStorage.getItem(STREAMING_RUN_CACHE_KEY);
    if (!raw) return {};

    const parsed = JSON.parse(raw) as unknown;
    if (!isRecord(parsed)) return {};
    if (parsed.version !== STREAMING_RUN_CACHE_VERSION) return {};
    if (!isRecord(parsed.runs_by_session)) return {};

    const nowMs = Date.now();
    const runsBySession: StreamingRunsBySession = {};

    for (const [sessionId, rawRuns] of Object.entries(parsed.runs_by_session)) {
      if (!isRecord(rawRuns)) continue;
      const runs: Record<string, PersistedStreamRun> = {};

      for (const [runId, rawRun] of Object.entries(rawRuns)) {
        if (!isRecord(rawRun)) continue;

        const agentId = rawRun.agentId;
        const thinkingContent = rawRun.thinkingContent;
        const assistantContent = rawRun.assistantContent;
        const content = rawRun.content;
        const isFinal = rawRun.isFinal;
        const updatedAtMs = rawRun.updatedAtMs;

        if (
          typeof agentId !== 'string' ||
          typeof thinkingContent !== 'string' ||
          typeof assistantContent !== 'string' ||
          typeof content !== 'string' ||
          typeof isFinal !== 'boolean' ||
          typeof updatedAtMs !== 'number' ||
          !Number.isFinite(updatedAtMs)
        ) {
          continue;
        }

        if (nowMs - updatedAtMs > STREAMING_RUN_TTL_MS) {
          continue;
        }

        runs[runId] = {
          agentId,
          thinkingContent,
          assistantContent,
          content,
          isFinal,
          updatedAtMs,
        };
      }

      if (Object.keys(runs).length > 0) {
        runsBySession[sessionId] = runs;
      }
    }

    return runsBySession;
  } catch (error) {
    console.warn('Failed to read chat streaming run cache', error);
    return {};
  }
}

function writeStreamingRunsCache(runsBySession: StreamingRunsBySession) {
  if (typeof window === 'undefined') return;

  try {
    if (Object.keys(runsBySession).length === 0) {
      window.localStorage.removeItem(STREAMING_RUN_CACHE_KEY);
      return;
    }

    const payload: StreamingRunCachePayload = {
      version: STREAMING_RUN_CACHE_VERSION,
      runs_by_session: runsBySession,
    };

    window.localStorage.setItem(
      STREAMING_RUN_CACHE_KEY,
      JSON.stringify(payload)
    );
  } catch (error) {
    console.warn('Failed to persist chat streaming run cache', error);
  }
}

function removeRunFromSession(
  runsBySession: StreamingRunsBySession,
  sessionId: string,
  runId: string
): StreamingRunsBySession {
  const sessionRuns = runsBySession[sessionId];
  if (!sessionRuns || !sessionRuns[runId]) return runsBySession;

  const nextSessionRuns = { ...sessionRuns };
  delete nextSessionRuns[runId];

  const nextRunsBySession = { ...runsBySession };
  if (Object.keys(nextSessionRuns).length === 0) {
    delete nextRunsBySession[sessionId];
  } else {
    nextRunsBySession[sessionId] = nextSessionRuns;
  }

  return nextRunsBySession;
}

export interface UseChatWebSocketResult {
  streamingRuns: Record<string, StreamRun>;
  agentStates: Record<string, ChatSessionAgentState>;
  agentStateInfos: Record<string, AgentStateInfo>;
  mentionStatuses: Map<string, Map<string, MentionStatus>>;
  compressionWarning: CompressionWarning | null;
  setAgentStates: React.Dispatch<
    React.SetStateAction<Record<string, ChatSessionAgentState>>
  >;
  setAgentStateInfos: React.Dispatch<
    React.SetStateAction<Record<string, AgentStateInfo>>
  >;
  setMentionStatuses: React.Dispatch<
    React.SetStateAction<Map<string, Map<string, MentionStatus>>>
  >;
  pruneStreamingRunsForSession: (
    sessionId: string,
    completedRunIds: Set<string>,
    runningAgentIds: Set<string>
  ) => void;
  clearCompressionWarning: () => void;
}

export function useChatWebSocket(
  activeSessionId: string | null,
  onMessageReceived: (message: ChatMessage) => void
): UseChatWebSocketResult {
  const [streamingRunsBySession, setStreamingRunsBySession] =
    useState<StreamingRunsBySession>(() => readStreamingRunsCache());
  const [agentStates, setAgentStates] = useState<
    Record<string, ChatSessionAgentState>
  >({});
  const [agentStateInfos, setAgentStateInfos] = useState<
    Record<string, AgentStateInfo>
  >({});
  const [mentionStatuses, setMentionStatuses] = useState<
    Map<string, Map<string, MentionStatus>>
  >(new Map());
  const [compressionWarning, setCompressionWarning] =
    useState<CompressionWarning | null>(null);
  const queryClient = useQueryClient();

  const streamingRuns = useMemo<Record<string, StreamRun>>(() => {
    if (!activeSessionId) return {};
    const sessionRuns = streamingRunsBySession[activeSessionId] ?? {};
    const next: Record<string, StreamRun> = {};

    for (const [runId, run] of Object.entries(sessionRuns)) {
      next[runId] = {
        agentId: run.agentId,
        thinkingContent: run.thinkingContent,
        assistantContent: run.assistantContent,
        content: run.content,
        isFinal: run.isFinal,
      };
    }

    return next;
  }, [activeSessionId, streamingRunsBySession]);

  const clearCompressionWarning = useCallback(() => {
    setCompressionWarning(null);
  }, []);

  const pruneStreamingRunsForSession = useCallback(
    (
      sessionId: string,
      completedRunIds: Set<string>,
      runningAgentIds: Set<string>
    ) => {
      if (!sessionId) return;

      setStreamingRunsBySession((prev) => {
        const sessionRuns = prev[sessionId];
        if (!sessionRuns) return prev;

        const nowMs = Date.now();
        let changed = false;
        const nextSessionRuns: Record<string, PersistedStreamRun> = {};

        for (const [runId, run] of Object.entries(sessionRuns)) {
          const isExpired = nowMs - run.updatedAtMs > STREAMING_RUN_TTL_MS;
          const isCompleted = completedRunIds.has(runId);
          const isInactiveTooLong =
            !runningAgentIds.has(run.agentId) &&
            nowMs - run.updatedAtMs > INACTIVE_RUN_PRUNE_GRACE_MS;

          if (isExpired || isCompleted || isInactiveTooLong) {
            changed = true;
            continue;
          }

          nextSessionRuns[runId] = run;
        }

        if (!changed) return prev;

        const next = { ...prev };
        if (Object.keys(nextSessionRuns).length === 0) {
          delete next[sessionId];
        } else {
          next[sessionId] = nextSessionRuns;
        }
        return next;
      });
    },
    []
  );

  const handleMessageNew = useCallback(
    (message: ChatMessage) => {
      const metaWarning = extractCompressionWarningFromMeta(message.meta);
      if (metaWarning) {
        setCompressionWarning(metaWarning);
      }
      onMessageReceived(message);
      const runId = extractRunId(message.meta);
      const sessionId = message.session_id;
      if (!runId || !sessionId) return;
      setStreamingRunsBySession((prev) =>
        removeRunFromSession(prev, sessionId, runId)
      );
    },
    [onMessageReceived]
  );

  const handleAgentDelta = useCallback(
    (payload: AgentDeltaPayload) => {
      setStreamingRunsBySession((prev) => {
        const sessionRuns = prev[payload.session_id] ?? {};
        const previous = sessionRuns[payload.run_id];
        const streamType = payload.stream_type ?? 'assistant';
        const previousAssistant =
          previous?.assistantContent ?? previous?.content ?? '';
        const previousThinking = previous?.thinkingContent ?? '';
        const applyDelta = (base: string) =>
          payload.delta ? `${base}${payload.content}` : payload.content;
        const assistantContent =
          streamType === 'thinking'
            ? previousAssistant
            : applyDelta(previousAssistant);
        const thinkingContent =
          streamType === 'thinking'
            ? applyDelta(previousThinking)
            : previousThinking;
        const nowMs = Date.now();

        return {
          ...prev,
          [payload.session_id]: {
            ...sessionRuns,
            [payload.run_id]: {
              agentId: payload.agent_id,
              thinkingContent,
              assistantContent,
              content: assistantContent,
              isFinal: payload.is_final,
              updatedAtMs: nowMs,
            },
          },
        };
      });
      if (payload.is_final) {
        setTimeout(() => {
          setStreamingRunsBySession((prev) =>
            removeRunFromSession(prev, payload.session_id, payload.run_id)
          );
        }, 1500);
      }
    },
    []
  );

  const handleAgentState = useCallback(
    (
      payload: ChatStreamEvent & {
        type: 'agent_state';
        started_at?: string | null;
      }
    ) => {
      setAgentStates((prev) => ({
        ...prev,
        [payload.agent_id]: payload.state,
      }));
      setAgentStateInfos((prev) => ({
        ...prev,
        [payload.agent_id]: {
          state: payload.state,
          startedAt: payload.started_at ?? null,
        },
      }));

      if (!activeSessionId) return;
      queryClient.setQueryData<ChatSessionAgent[]>(
        ['chatSessionAgents', activeSessionId],
        (prev) => {
          if (!prev) return prev;
          let changed = false;
          const next = prev.map((sessionAgent) => {
            if (sessionAgent.agent_id !== payload.agent_id) {
              return sessionAgent;
            }
            changed = true;
            return {
              ...sessionAgent,
              state: payload.state,
              updated_at: payload.started_at ?? sessionAgent.updated_at,
            };
          });
          return changed ? next : prev;
        }
      );
    },
    [activeSessionId, queryClient]
  );

  const handleMentionAcknowledged = useCallback(
    (payload: MentionAcknowledgedEvent) => {
      setMentionStatuses((prev) => {
        const next = new Map(prev);
        const perMessage = new Map(next.get(payload.message_id) ?? []);
        perMessage.set(payload.mentioned_agent, payload.status);
        next.set(payload.message_id, perMessage);
        return next;
      });
    },
    []
  );

  useEffect(() => {
    setStreamingRunsBySession((prev) => pruneExpiredStreamingRuns(prev));
  }, [activeSessionId]);

  useEffect(() => {
    const pruned = pruneExpiredStreamingRuns(streamingRunsBySession);
    if (pruned !== streamingRunsBySession) {
      setStreamingRunsBySession(pruned);
      return;
    }

    writeStreamingRunsCache(streamingRunsBySession);
  }, [streamingRunsBySession]);

  useEffect(() => {
    if (!activeSessionId) return;
    let ws: WebSocket | null = null;
    let shouldReconnect = true;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const connect = () => {
      const streamUrl = chatApi.getStreamUrl(activeSessionId);
      const protocol = window.location.protocol === 'https:' ? 'wss' : 'ws';
      const wsUrl = `${protocol}://${window.location.host}${streamUrl}`;
      ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        queryClient.invalidateQueries({
          queryKey: ['chatMessages', activeSessionId],
        });
      };

      ws.onmessage = (event) => {
        try {
          const payload = JSON.parse(event.data) as ChatStreamPayload;
          if (payload.type === 'mention_acknowledged') {
            handleMentionAcknowledged(payload);
            return;
          }
          if (payload.type === 'message_new') {
            handleMessageNew(payload.message);
            return;
          }

          if (payload.type === 'agent_delta') {
            handleAgentDelta(payload);
            return;
          }

          if (payload.type === 'agent_state') {
            handleAgentState(payload);
            return;
          }

          if (payload.type === 'compression_warning') {
            setCompressionWarning(payload.warning);
          }
        } catch (error) {
          console.warn('Failed to parse chat stream payload', error);
        }
      };

      ws.onclose = () => {
        if (!shouldReconnect) return;
        reconnectTimer = setTimeout(connect, 1500);
      };

      ws.onerror = () => {
        ws?.close();
      };
    };

    connect();

    return () => {
      shouldReconnect = false;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      ws?.close();
    };
  }, [
    activeSessionId,
    queryClient,
    handleMessageNew,
    handleAgentDelta,
    handleAgentState,
    handleMentionAcknowledged,
  ]);

  // Reset state when session changes
  useEffect(() => {
    setAgentStates({});
    setAgentStateInfos({});
    setMentionStatuses(new Map());
    setCompressionWarning(null);
  }, [activeSessionId]);

  return {
    streamingRuns,
    agentStates,
    agentStateInfos,
    mentionStatuses,
    compressionWarning,
    setAgentStates,
    setAgentStateInfos,
    setMentionStatuses,
    pruneStreamingRunsForSession,
    clearCompressionWarning,
  };
}
