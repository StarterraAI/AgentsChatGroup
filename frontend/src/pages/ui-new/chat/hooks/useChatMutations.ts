import { useMutation, useQueryClient } from '@tanstack/react-query';
import type { ChatMessage, ChatSession, JsonValue } from 'shared/types';
import { chatApi } from '@/lib/api';

export interface UseChatMutationsResult {
  createSession: ReturnType<typeof useMutation<ChatSession, Error, void>>;
  updateSession: ReturnType<
    typeof useMutation<ChatSession, Error, { sessionId: string; title: string | null }>
  >;
  archiveSession: ReturnType<typeof useMutation<ChatSession, Error, string>>;
  restoreSession: ReturnType<typeof useMutation<ChatSession, Error, string>>;
  deleteSession: ReturnType<typeof useMutation<void, Error, string>>;
  sendMessage: ReturnType<
    typeof useMutation<
      ChatMessage,
      Error,
      { sessionId: string; content: string; meta?: JsonValue }
    >
  >;
  deleteMessages: ReturnType<
    typeof useMutation<number, Error, { sessionId: string; messageIds: string[] }>
  >;
}

export function useChatMutations(
  onSessionCreated?: (session: ChatSession) => void,
  onSessionUpdated?: (session: ChatSession) => void,
  onMessageSent?: (message: ChatMessage) => void,
  onMessagesDeleted?: (count: number) => void,
  onSessionDeleted?: () => void
): UseChatMutationsResult {
  const queryClient = useQueryClient();

  const createSession = useMutation({
    mutationFn: () => chatApi.createSession({ title: null }),
    onSuccess: (session) => {
      // Add new session to cache immediately to prevent race condition
      // where useEffect navigates back before invalidateQueries completes
      queryClient.setQueryData<ChatSession[]>(['chatSessions'], (old) =>
        old ? [session, ...old] : [session]
      );
      // Navigate to the new session
      onSessionCreated?.(session);
      // Then refresh the full list in background
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
    },
  });

  const updateSession = useMutation({
    mutationFn: (params: { sessionId: string; title: string | null }) =>
      chatApi.updateSession(params.sessionId, {
        title: params.title,
        status: null,
        summary_text: null,
        archive_ref: null,
      }),
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
      onSessionUpdated?.(session);
    },
  });

  const archiveSession = useMutation({
    mutationFn: (id: string) => chatApi.archiveSession(id),
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
      onSessionUpdated?.(session);
    },
  });

  const restoreSession = useMutation({
    mutationFn: (id: string) => chatApi.restoreSession(id),
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
      onSessionUpdated?.(session);
    },
  });

  const deleteSession = useMutation({
    mutationFn: (id: string) => chatApi.deleteSession(id),
    onSuccess: (_data, deletedSessionId) => {
      queryClient.setQueryData<ChatSession[]>(['chatSessions'], (old) =>
        old ? old.filter((session) => session.id !== deletedSessionId) : []
      );
      queryClient.removeQueries({
        queryKey: ['chatSessionAgents', deletedSessionId],
        exact: true,
      });
      queryClient.removeQueries({
        queryKey: ['chatMessages', deletedSessionId],
        exact: true,
      });
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
      onSessionDeleted?.();
    },
  });

  const sendMessage = useMutation({
    mutationFn: async (params: {
      sessionId: string;
      content: string;
      meta?: JsonValue;
    }) =>
      chatApi.createMessage(
        params.sessionId,
        chatApi.buildCreateMessageRequest(params.content, params.meta ?? null)
      ),
    onSuccess: (message) => {
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
      onMessageSent?.(message);
    },
  });

  const deleteMessages = useMutation({
    mutationFn: async (params: { sessionId: string; messageIds: string[] }) =>
      chatApi.deleteMessagesBatch(params.sessionId, params.messageIds),
    onSuccess: (count, variables) => {
      queryClient.invalidateQueries({ queryKey: ['chatMessages', variables.sessionId] });
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
      onMessagesDeleted?.(count);
    },
  });

  return {
    createSession,
    updateSession,
    archiveSession,
    restoreSession,
    deleteSession,
    sendMessage,
    deleteMessages,
  };
}
