import { useCallback, useEffect, useState } from 'react';
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
  clearCompressionWarning: () => void;
}

export function useChatWebSocket(
  activeSessionId: string | null,
  onMessageReceived: (message: ChatMessage) => void
): UseChatWebSocketResult {
  const [streamingRuns, setStreamingRuns] = useState<Record<string, StreamRun>>(
    {}
  );
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

  const clearCompressionWarning = useCallback(() => {
    setCompressionWarning(null);
  }, []);

  const handleMessageNew = useCallback(
    (message: ChatMessage) => {
      onMessageReceived(message);
      const runId = extractRunId(message.meta);
      if (runId) {
        setStreamingRuns((prev) => {
          if (!prev[runId]) return prev;
          const next = { ...prev };
          delete next[runId];
          return next;
        });
      }
    },
    [onMessageReceived]
  );

  const handleAgentDelta = useCallback(
    (payload: AgentDeltaPayload) => {
      setStreamingRuns((prev) => {
        const previous = prev[payload.run_id];
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
        return {
          ...prev,
          [payload.run_id]: {
            agentId: payload.agent_id,
            thinkingContent,
            assistantContent,
            content: assistantContent,
            isFinal: payload.is_final,
          },
        };
      });
      if (payload.is_final) {
        setTimeout(() => {
          setStreamingRuns((prev) => {
            if (!prev[payload.run_id]) return prev;
            const next = { ...prev };
            delete next[payload.run_id];
            return next;
          });
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
    setStreamingRuns({});
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
    clearCompressionWarning,
  };
}
