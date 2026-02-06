import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate, useParams } from 'react-router-dom';
import {
  CaretRightIcon,
  ChatsTeardropIcon,
  PaperPlaneRightIcon,
  PlusIcon,
  UsersIcon,
  XIcon,
} from '@phosphor-icons/react';
import {
  ChatAgent,
  ChatMessage,
  ChatSenderType,
  ChatSessionStatus,
  ChatSessionAgent,
  ChatSessionAgentState,
  type ChatStreamEvent,
} from 'shared/types';
import { chatApi } from '@/lib/api';
import { cn } from '@/lib/utils';
import { Badge } from '@/components/ui/badge';
import { PrimaryButton } from '@/components/ui-new/primitives/PrimaryButton';
import { MultiSelectDropdown } from '@/components/ui-new/primitives/MultiSelectDropdown';
import { ChatEntryContainer } from '@/components/ui-new/primitives/conversation/ChatEntryContainer';
import { ChatMarkdown } from '@/components/ui-new/primitives/conversation/ChatMarkdown';
import { ChatSystemMessage } from '@/components/ui-new/primitives/conversation/ChatSystemMessage';
import { DiffViewBody } from '@/components/ui-new/primitives/conversation/PierreConversationDiff';
import { useUserSystem } from '@/components/ConfigProvider';
import { useTheme } from '@/components/ThemeProvider';
import { formatDateShortWithTime } from '@/utils/date';
import { parseDiffStats } from '@/utils/diffStatsParser';
import { toPrettyCase } from '@/utils/string';
import { getActualTheme } from '@/utils/theme';

type StreamRun = {
  agentId: string;
  content: string;
  isFinal: boolean;
};

type RunHistoryItem = {
  runId: string;
  agentId: string;
  createdAt: string;
  content: string;
};

type SessionMember = {
  agent: ChatAgent;
  sessionAgent: ChatSessionAgent;
};

type DiffMeta = {
  runId: string | null;
  preview: string | null;
  truncated: boolean;
  available: boolean;
  untrackedFiles: string[];
};

const mentionRegex = /(^|\s)@([a-zA-Z0-9_-]*)$/;
const mentionTokenRegex = /(^|\s)@([a-zA-Z0-9_-]+)/g;
const memberNameRegex = /^[a-zA-Z0-9_-]+$/;

const fallbackRunnerTypes = [
  'CLAUDE_CODE',
  'CODEX',
  'AMP',
  'GEMINI',
  'OPENCODE',
  'CURSOR_AGENT',
  'QWEN_CODE',
  'COPILOT',
  'DROID',
];

const agentStateLabels: Record<ChatSessionAgentState, string> = {
  idle: 'Idle',
  running: 'Running',
  waitingapproval: 'Waiting approval',
  dead: 'Dead',
};

const agentStateDotClass: Record<ChatSessionAgentState, string> = {
  idle: 'bg-low',
  running: 'bg-brand',
  waitingapproval: 'bg-brand-secondary',
  dead: 'bg-error',
};

const messagePalette = [
  { bg: 'rgba(226, 239, 255, 0.8)', border: 'rgba(176, 205, 242, 0.7)' },
  { bg: 'rgba(231, 249, 239, 0.85)', border: 'rgba(176, 223, 198, 0.7)' },
  { bg: 'rgba(255, 243, 227, 0.85)', border: 'rgba(231, 204, 173, 0.7)' },
  { bg: 'rgba(245, 236, 255, 0.8)', border: 'rgba(209, 187, 234, 0.7)' },
  { bg: 'rgba(255, 236, 242, 0.8)', border: 'rgba(232, 184, 198, 0.7)' },
  { bg: 'rgba(238, 244, 248, 0.85)', border: 'rgba(189, 205, 217, 0.7)' },
];

function hashKey(value: string): number {
  let hash = 0;
  for (let i = 0; i < value.length; i += 1) {
    hash = (hash << 5) - hash + value.charCodeAt(i);
    hash |= 0;
  }
  return Math.abs(hash);
}

function getMessageTone(key: string, isUser: boolean) {
  if (isUser) {
    return { bg: 'rgba(219, 238, 255, 0.9)', border: 'rgba(156, 197, 237, 0.8)' };
  }
  const index = hashKey(key) % messagePalette.length;
  return messagePalette[index];
}

function extractMentions(content: string): Set<string> {
  const mentions = new Set<string>();
  for (const match of content.matchAll(mentionTokenRegex)) {
    const name = match[2];
    if (name) mentions.add(name);
  }
  return mentions;
}

function extractRunId(meta: unknown): string | null {
  if (!meta || typeof meta !== 'object') return null;
  const runId = (meta as { run_id?: unknown }).run_id;
  return typeof runId === 'string' ? runId : null;
}

function sanitizeHandle(value: string | null | undefined): string {
  if (!value) return 'you';
  const sanitized = value
    .split('@')[0]
    .split(' ')[0]
    .trim()
    .replace(/[^a-zA-Z0-9_-]/g, '');
  return sanitized.length > 0 ? sanitized : 'you';
}

function extractDiffMeta(meta: unknown): DiffMeta {
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

type DiffFileEntry = {
  path: string;
  patch: string;
  additions: number;
  deletions: number;
};

function splitUnifiedDiff(patch: string): DiffFileEntry[] {
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

export function ChatSessions() {
  const { sessionId } = useParams<{ sessionId?: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const { profiles, loginStatus } = useUserSystem();
  const { theme } = useTheme();
  const actualTheme = getActualTheme(theme);

  const { data: sessions = [], isLoading: isSessionsLoading } = useQuery({
    queryKey: ['chatSessions'],
    queryFn: () => chatApi.listSessions(),
  });

  const { data: agents = [], isLoading: isAgentsLoading } = useQuery({
    queryKey: ['chatAgents'],
    queryFn: () => chatApi.listAgents(),
  });

  const sortedSessions = useMemo(
    () =>
      [...sessions].sort(
        (a, b) =>
          new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime()
      ),
    [sessions]
  );

  const activeSessions = useMemo(
    () =>
      sortedSessions.filter(
        (session) => session.status === ChatSessionStatus.active
      ),
    [sortedSessions]
  );

  const archivedSessions = useMemo(
    () =>
      sortedSessions.filter(
        (session) => session.status === ChatSessionStatus.archived
      ),
    [sortedSessions]
  );

  const activeSessionId = sessionId ?? sortedSessions[0]?.id ?? null;

  useEffect(() => {
    if (!sessionId && sortedSessions.length > 0) {
      navigate(`/chat/${sortedSessions[0].id}`, { replace: true });
    }
    if (
      sessionId &&
      sortedSessions.length > 0 &&
      !sortedSessions.some((session) => session.id === sessionId)
    ) {
      navigate(`/chat/${sortedSessions[0].id}`, { replace: true });
    }
  }, [navigate, sessionId, sortedSessions]);

  const { data: sessionAgents = [], isLoading: isSessionAgentsLoading } =
    useQuery({
      queryKey: ['chatSessionAgents', activeSessionId],
      queryFn: () => chatApi.listSessionAgents(activeSessionId!),
      enabled: !!activeSessionId,
    });

  const {
    data: messagesData = [],
    isLoading: isMessagesLoading,
  } = useQuery({
    queryKey: ['chatMessages', activeSessionId],
    queryFn: () => chatApi.listMessages(activeSessionId!),
    enabled: !!activeSessionId,
  });

  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [streamingRuns, setStreamingRuns] = useState<Record<string, StreamRun>>(
    {}
  );
  const [agentStates, setAgentStates] = useState<
    Record<string, ChatSessionAgentState>
  >({});
  const [selectedMentions, setSelectedMentions] = useState<string[]>([]);
  const [draft, setDraft] = useState('');
  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [workspaceDrawerOpen, setWorkspaceDrawerOpen] = useState(false);
  const [workspaceAgentId, setWorkspaceAgentId] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);
  const [isAddMemberOpen, setIsAddMemberOpen] = useState(false);
  const [editingMember, setEditingMember] = useState<SessionMember | null>(
    null
  );
  const [newMemberName, setNewMemberName] = useState('');
  const [newMemberRunnerType, setNewMemberRunnerType] = useState('');
  const [newMemberPrompt, setNewMemberPrompt] = useState('');
  const [newMemberWorkspace, setNewMemberWorkspace] = useState('');
  const [memberError, setMemberError] = useState<string | null>(null);
  const [isSavingMember, setIsSavingMember] = useState(false);
  const [logRunId, setLogRunId] = useState<string | null>(null);
  const [logContent, setLogContent] = useState('');
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);
  const [runningSince, setRunningSince] = useState<Record<string, number>>({});
  const [clock, setClock] = useState(() => Date.now());
  const [showArchived, setShowArchived] = useState(false);
  const [runDiffs, setRunDiffs] = useState<
    Record<
      string,
      { loading: boolean; error: string | null; files: DiffFileEntry[] }
    >
  >({});
  const [untrackedContent, setUntrackedContent] = useState<
    Record<
      string,
      { loading: boolean; error: string | null; content: string | null; open: boolean }
    >
  >({});

  useEffect(() => {
    if (activeSessionId) {
      setMessages(messagesData);
    } else {
      setMessages([]);
    }
  }, [messagesData, activeSessionId]);

  useEffect(() => {
    setStreamingRuns({});
    setAgentStates({});
    setSelectedMentions([]);
    setDraft('');
    setMentionQuery(null);
    setWorkspaceDrawerOpen(false);
    setWorkspaceAgentId(null);
    setIsAddMemberOpen(false);
    setNewMemberName('');
    setNewMemberPrompt('');
    setNewMemberWorkspace('');
    setMemberError(null);
    setEditingMember(null);
    setLogRunId(null);
    setLogContent('');
    setLogError(null);
    setRunningSince({});
    setClock(Date.now());
    setRunDiffs({});
    setUntrackedContent({});
  }, [activeSessionId]);

  const availableRunnerTypes = useMemo(() => {
    const keys = Object.keys(profiles ?? {});
    const baseList = keys.length > 0 ? keys : fallbackRunnerTypes;
    if (editingMember && !baseList.includes(editingMember.agent.runner_type)) {
      return [...baseList, editingMember.agent.runner_type];
    }
    return baseList;
  }, [editingMember, profiles]);

  const senderHandle = useMemo(() => {
    if (loginStatus?.status === 'loggedin') {
      return sanitizeHandle(
        loginStatus.profile.username ??
          loginStatus.profile.email ??
          loginStatus.profile.user_id
      );
    }
    return 'you';
  }, [loginStatus]);

  useEffect(() => {
    if (!newMemberRunnerType && availableRunnerTypes.length > 0) {
      setNewMemberRunnerType(availableRunnerTypes[0]);
    }
  }, [availableRunnerTypes, newMemberRunnerType]);

  useEffect(() => {
    setAgentStates((prev) => {
      let changed = false;
      const next = { ...prev };
      for (const agent of agents) {
        if (!next[agent.id]) {
          next[agent.id] = ChatSessionAgentState.idle;
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [agents]);

  useEffect(() => {
    setAgentStates((prev) => {
      const next = { ...prev };
      for (const sessionAgent of sessionAgents) {
        next[sessionAgent.agent_id] = sessionAgent.state;
      }
      return next;
    });
  }, [sessionAgents]);

  const upsertMessage = useCallback((message: ChatMessage) => {
    setMessages((prev) => {
      const existingIndex = prev.findIndex((item) => item.id === message.id);
      if (existingIndex >= 0) {
        const next = [...prev];
        next[existingIndex] = message;
        return next;
      }
      return [...prev, message];
    });
  }, []);

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

      ws.onmessage = (event) => {
        try {
          const payload = JSON.parse(event.data) as ChatStreamEvent;
          if (payload.type === 'message_new') {
            const message = payload.message;
            upsertMessage(message);
            const runId = extractRunId(message.meta);
            if (runId) {
              setStreamingRuns((prev) => {
                if (!prev[runId]) return prev;
                const next = { ...prev };
                delete next[runId];
                return next;
              });
            }
            return;
          }

          if (payload.type === 'agent_delta') {
            setStreamingRuns((prev) => {
              const previous = prev[payload.run_id];
              const content =
                payload.delta && previous
                  ? `${previous.content}${payload.content}`
                  : payload.content;
              return {
                ...prev,
                [payload.run_id]: {
                  agentId: payload.agent_id,
                  content,
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
            return;
          }

          if (payload.type === 'agent_state') {
            setAgentStates((prev) => ({
              ...prev,
              [payload.agent_id]: payload.state,
            }));
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
  }, [activeSessionId, upsertMessage]);

  const messageList = useMemo(
    () =>
      [...messages].sort(
        (a, b) =>
          new Date(a.created_at).getTime() -
          new Date(b.created_at).getTime()
      ),
    [messages]
  );

  const createSessionMutation = useMutation({
    mutationFn: () => chatApi.createSession({ title: null }),
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
      navigate(`/chat/${session.id}`);
    },
  });

  const archiveSessionMutation = useMutation({
    mutationFn: (id: string) => chatApi.archiveSession(id),
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
      navigate(`/chat/${session.id}`);
    },
  });

  const restoreSessionMutation = useMutation({
    mutationFn: (id: string) => chatApi.restoreSession(id),
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
      navigate(`/chat/${session.id}`);
    },
  });

  const sendMessageMutation = useMutation({
    mutationFn: async (params: { sessionId: string; content: string }) =>
      chatApi.createMessage(
        params.sessionId,
        chatApi.buildCreateMessageRequest(params.content, {
          sender_handle: senderHandle,
        })
      ),
    onSuccess: (message) => {
      upsertMessage(message);
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
    },
  });

  const handleSend = async () => {
    if (!activeSessionId || isArchived) return;
    const trimmed = draft.trim();
    const contentMentions = extractMentions(draft);
    const mentionsToInject = selectedMentions.filter(
      (name) => !contentMentions.has(name)
    );
    const mentionPrefix =
      mentionsToInject.length > 0
        ? mentionsToInject.map((name) => `@${name}`).join(' ')
        : '';
    const content = [mentionPrefix, trimmed].filter(Boolean).join(' ').trim();
    if (!content) return;

    try {
      await sendMessageMutation.mutateAsync({
        sessionId: activeSessionId,
        content,
      });

      setDraft('');
      setSelectedMentions([]);
      setMentionQuery(null);
      inputRef.current?.focus();
    } catch (error) {
      console.warn('Failed to send chat message', error);
    }
  };

  const handleDraftChange = (value: string) => {
    setDraft(value);
    const match = mentionRegex.exec(value);
    if (match) {
      setMentionQuery(match[2] ?? '');
    } else {
      setMentionQuery(null);
    }
  };

  const handleMentionSelect = (name: string) => {
    setDraft((prev) => {
      const match = mentionRegex.exec(prev);
      if (!match) {
        return `${prev}${
          prev.endsWith(' ') || prev.length === 0 ? '' : ' '
        }@${name} `;
      }
      const matchIndex = match.index ?? prev.length;
      const prefix = prev.slice(0, matchIndex);
      const spacer = match[1] ?? '';
      return `${prefix}${spacer}@${name} `;
    });
    setMentionQuery(null);
    inputRef.current?.focus();
  };

  const agentById = useMemo(() => {
    const map = new Map<string, ChatAgent>();
    for (const agent of agents) map.set(agent.id, agent);
    return map;
  }, [agents]);

  const sessionMembers = useMemo<SessionMember[]>(() => {
    return sessionAgents
      .map((sessionAgent) => {
        const agent = agentById.get(sessionAgent.agent_id);
        if (!agent) return null;
        return { agent, sessionAgent };
      })
      .filter((item): item is SessionMember => item !== null);
  }, [agentById, sessionAgents]);

  const mentionAgents = useMemo(
    () => sessionMembers.map((member) => member.agent),
    [sessionMembers]
  );

  useEffect(() => {
    if (mentionAgents.length === 0) {
      setSelectedMentions([]);
      return;
    }
    setSelectedMentions((prev) =>
      prev.filter((mention) =>
        mentionAgents.some((agent) => agent.name === mention)
      )
    );
  }, [mentionAgents]);

  const visibleMentionSuggestions = useMemo(() => {
    if (mentionQuery === null) return [];
    const query = mentionQuery.toLowerCase();
    return mentionAgents.filter((agent) =>
      agent.name.toLowerCase().includes(query)
    );
  }, [mentionAgents, mentionQuery]);

  const agentOptions = useMemo(
    () =>
      mentionAgents.map((agent) => ({
        value: agent.name,
        label: agent.name,
      })),
    [mentionAgents]
  );

  const placeholderAgents = useMemo(
    () =>
      sessionMembers.filter(
        (member) => agentStates[member.agent.id] === ChatSessionAgentState.running
      ),
    [agentStates, sessionMembers]
  );

  useEffect(() => {
    setRunningSince((prev) => {
      const runningIds = new Set(
        sessionMembers
          .filter(
            (member) =>
              agentStates[member.agent.id] === ChatSessionAgentState.running
          )
          .map((member) => member.agent.id)
      );

      let changed = false;
      const next = { ...prev };
      for (const id of runningIds) {
        if (!next[id]) {
          next[id] = Date.now();
          changed = true;
        }
      }
      for (const id of Object.keys(next)) {
        if (!runningIds.has(id)) {
          delete next[id];
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [agentStates, sessionMembers]);

  useEffect(() => {
    if (Object.keys(runningSince).length === 0) return;
    const timer = setInterval(() => setClock(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [runningSince]);

  const runHistory = useMemo(() => {
    const runs: RunHistoryItem[] = [];
    for (const message of messages) {
      if (message.sender_type !== ChatSenderType.agent || !message.sender_id) {
        continue;
      }
      const runId = extractRunId(message.meta);
      if (!runId) continue;
      runs.push({
        runId,
        agentId: message.sender_id,
        createdAt: message.created_at,
        content: message.content,
      });
    }
    return runs;
  }, [messages]);

  const activeWorkspaceAgent = workspaceAgentId
    ? agentById.get(workspaceAgentId)
    : null;
  const workspacePath = useMemo(() => {
    if (!workspaceAgentId) return null;
    const sessionAgent = sessionAgents.find(
      (item) => item.agent_id === workspaceAgentId
    );
    if (sessionAgent?.workspace_path) return sessionAgent.workspace_path;
    if (!activeSessionId) return null;
    return `chat/session_${activeSessionId}/agents/${workspaceAgentId}`;
  }, [activeSessionId, sessionAgents, workspaceAgentId]);

  const activeWorkspaceRuns = useMemo(
    () =>
      runHistory
        .filter((run) => run.agentId === workspaceAgentId)
        .sort(
          (a, b) =>
            new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime()
        ),
    [runHistory, workspaceAgentId]
  );

  const activeSession = useMemo(
    () => sessions.find((session) => session.id === activeSessionId),
    [sessions, activeSessionId]
  );
  const isArchived =
    activeSession?.status === ChatSessionStatus.archived;

  useEffect(() => {
    if (activeSession?.status === ChatSessionStatus.archived) {
      setShowArchived(true);
    }
  }, [activeSession?.status]);

  const handleAddMember = async () => {
    if (!activeSessionId) {
      setMemberError('Select a chat session first.');
      return;
    }
    if (isArchived) {
      setMemberError('This session is archived and read-only.');
      return;
    }

    const name = newMemberName.trim();
    const runnerType = newMemberRunnerType.trim();
    const prompt = newMemberPrompt.trim();
    const workspacePath = newMemberWorkspace.trim();

    if (!name) {
      setMemberError('AI member name is required.');
      return;
    }

    if (!memberNameRegex.test(name)) {
      setMemberError('Name can only include letters, numbers, "_" or "-".');
      return;
    }

    if (!runnerType) {
      setMemberError('Choose a base coding agent.');
      return;
    }

    if (!workspacePath) {
      setMemberError('Workspace path is required.');
      return;
    }

    if (!prompt) {
      setMemberError('System prompt is required.');
      return;
    }

    setIsSavingMember(true);
    setMemberError(null);

    try {
      if (editingMember) {
        const agentId = editingMember.agent.id;
        const updatePayload = {
          name: editingMember.agent.name !== name ? name : null,
          runner_type:
            editingMember.agent.runner_type !== runnerType
              ? runnerType
              : null,
          system_prompt:
            (editingMember.agent.system_prompt ?? '') !== prompt ? prompt : null,
          tools_enabled: null,
        };

        if (
          updatePayload.name ||
          updatePayload.runner_type ||
          updatePayload.system_prompt
        ) {
          await chatApi.updateAgent(agentId, updatePayload);
        }

        await chatApi.updateSessionAgent(
          activeSessionId,
          editingMember.sessionAgent.id,
          { workspace_path: workspacePath }
        );
      } else {
        const existing = agents.find((agent) => agent.name === name);
        let agentId = existing?.id ?? null;
        if (existing) {
          const updatePayload = {
            name: null,
            runner_type:
              existing.runner_type !== runnerType ? runnerType : null,
            system_prompt:
              (existing.system_prompt ?? '') !== prompt ? prompt : null,
            tools_enabled: null,
          };

          if (updatePayload.runner_type || updatePayload.system_prompt) {
            const updated = await chatApi.updateAgent(
              existing.id,
              updatePayload
            );
            agentId = updated.id;
          }
        } else {
          const created = await chatApi.createAgent({
            name,
            runner_type: runnerType,
            system_prompt: prompt,
            tools_enabled: null,
          });
          agentId = created.id;
        }

        if (!agentId) {
          setMemberError('Unable to create AI member.');
          return;
        }

        await chatApi.createSessionAgent(activeSessionId, {
          agent_id: agentId,
          workspace_path: workspacePath,
        });
      }

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['chatAgents'] }),
        queryClient.invalidateQueries({
          queryKey: ['chatSessionAgents', activeSessionId],
        }),
      ]);

      setNewMemberName('');
      setNewMemberPrompt('');
      setNewMemberWorkspace('');
      setMemberError(null);
      setEditingMember(null);
      setIsAddMemberOpen(false);
    } catch (error) {
      console.warn('Failed to add AI member', error);
      setMemberError('Failed to add AI member. Check server logs.');
    } finally {
      setIsSavingMember(false);
    }
  };

  const handleEditMember = (member: SessionMember) => {
    if (isArchived) {
      setMemberError('This session is archived and read-only.');
      return;
    }
    setEditingMember(member);
    setNewMemberName(member.agent.name);
    setNewMemberRunnerType(member.agent.runner_type);
    setNewMemberPrompt(member.agent.system_prompt ?? '');
    setNewMemberWorkspace(member.sessionAgent.workspace_path ?? '');
    setMemberError(null);
    setIsAddMemberOpen(true);
  };

  const handleRemoveMember = async (member: SessionMember) => {
    if (!activeSessionId) return;
    if (isArchived) {
      setMemberError('This session is archived and read-only.');
      return;
    }
    const confirmed = window.confirm(
      `Remove @${member.agent.name} from this session?`
    );
    if (!confirmed) return;

    try {
      await chatApi.deleteSessionAgent(
        activeSessionId,
        member.sessionAgent.id
      );
      await queryClient.invalidateQueries({
        queryKey: ['chatSessionAgents', activeSessionId],
      });
      if (workspaceAgentId === member.agent.id) {
        setWorkspaceDrawerOpen(false);
        setWorkspaceAgentId(null);
      }
    } catch (error) {
      console.warn('Failed to remove AI member', error);
      setMemberError('Failed to remove AI member.');
    }
  };

  const handleLoadLog = async (runId: string) => {
    setLogRunId(runId);
    setLogLoading(true);
    setLogError(null);
    try {
      const content = await chatApi.getRunLog(runId);
      setLogContent(content);
    } catch (error) {
      console.warn('Failed to load run log', error);
      setLogError('Unable to load run log.');
      setLogContent('');
    } finally {
      setLogLoading(false);
    }
  };

  const handleLoadDiff = async (runId: string) => {
    setRunDiffs((prev) => {
      const existing = prev[runId];
      if (existing?.loading || existing?.files.length) return prev;
      return {
        ...prev,
        [runId]: { loading: true, error: null, files: [] },
      };
    });

    try {
      const patch = await chatApi.getRunDiff(runId);
      const files = splitUnifiedDiff(patch);
      setRunDiffs((prev) => ({
        ...prev,
        [runId]: { loading: false, error: null, files },
      }));
    } catch (error) {
      console.warn('Failed to load run diff', error);
      setRunDiffs((prev) => ({
        ...prev,
        [runId]: {
          loading: false,
          error: 'Unable to load diff.',
          files: [],
        },
      }));
    }
  };

  const handleToggleUntracked = async (runId: string, path: string) => {
    const key = `${runId}:${path}`;
    const existing = untrackedContent[key];
    if (existing?.open) {
      setUntrackedContent((prev) => ({
        ...prev,
        [key]: { ...existing, open: false },
      }));
      return;
    }

    setUntrackedContent((prev) => ({
      ...prev,
      [key]: {
        loading: !existing?.content && !existing?.error,
        error: existing?.error ?? null,
        content: existing?.content ?? null,
        open: true,
      },
    }));

    if (existing?.content || existing?.error) {
      return;
    }

    try {
      const content = await chatApi.getRunUntrackedFile(runId, path);
      setUntrackedContent((prev) => ({
        ...prev,
        [key]: { loading: false, error: null, content, open: true },
      }));
    } catch (error) {
      console.warn('Failed to load untracked file content', error);
      setUntrackedContent((prev) => ({
        ...prev,
        [key]: {
          loading: false,
          error: 'Unable to load file.',
          content: null,
          open: true,
        },
      }));
    }
  };

  const isLoading =
    isSessionsLoading ||
    isMessagesLoading ||
    isAgentsLoading ||
    isSessionAgentsLoading;
  const canSend =
    !!activeSessionId &&
    !isArchived &&
    (draft.trim().length > 0 || selectedMentions.length > 0) &&
    !sendMessageMutation.isPending;

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [
    messageList.length,
    Object.keys(streamingRuns).length,
    placeholderAgents.length,
  ]);

  useEffect(() => {
    setLogRunId(null);
    setLogContent('');
    setLogError(null);
  }, [workspaceAgentId]);

  return (
    <div className="relative flex h-full min-h-0 bg-primary">
      <aside className="w-72 border-r border-border flex flex-col min-h-0">
        <div className="px-base py-base border-b border-border flex items-center justify-between">
          <div className="flex items-center gap-half text-normal font-medium">
            <ChatsTeardropIcon className="size-icon-sm" />
            <span>Group Chat</span>
          </div>
          <PrimaryButton
            variant="secondary"
            value="New"
            onClick={() => createSessionMutation.mutate()}
            disabled={createSessionMutation.isPending}
          >
            <PlusIcon className="size-icon-xs" />
          </PrimaryButton>
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto p-base space-y-half">
          {activeSessions.length === 0 && archivedSessions.length === 0 && (
            <div className="text-sm text-low">
              No sessions yet. Create your first group chat.
            </div>
          )}
          {activeSessions.map((session) => {
            const isActive = session.id === activeSessionId;
            return (
              <button
                key={session.id}
                type="button"
                onClick={() => navigate(`/chat/${session.id}`)}
                className={cn(
                  'w-full text-left rounded-sm border px-base py-half',
                  isActive
                    ? 'border-brand bg-brand/10 text-normal'
                    : 'border-transparent hover:border-border hover:bg-secondary text-low'
                )}
              >
                <div className="flex items-center justify-between gap-base">
                  <span className="text-sm font-medium truncate text-normal">
                    {session.title || 'Untitled session'}
                  </span>
                  <span className="text-xs text-low">
                    {formatDateShortWithTime(session.updated_at)}
                  </span>
                </div>
              </button>
            );
          })}
          {archivedSessions.length > 0 && (
            <div className="pt-base mt-base border-t border-border space-y-half">
              <button
                type="button"
                onClick={() => setShowArchived((prev) => !prev)}
                className="text-xs text-low uppercase tracking-wide hover:text-normal"
              >
                {showArchived ? 'Hide Archived' : 'Show Archived'} (
                {archivedSessions.length})
              </button>
              {showArchived &&
                archivedSessions.map((session) => {
                  const isActive = session.id === activeSessionId;
                  return (
                    <button
                      key={session.id}
                      type="button"
                      onClick={() => navigate(`/chat/${session.id}`)}
                      className={cn(
                        'w-full text-left rounded-sm border px-base py-half',
                        isActive
                          ? 'border-brand bg-brand/10 text-normal'
                          : 'border-transparent hover:border-border hover:bg-secondary text-low'
                      )}
                    >
                      <div className="flex items-center justify-between gap-base">
                        <span className="text-sm font-medium truncate text-normal">
                          {session.title || 'Untitled session'}
                        </span>
                        <span className="text-xs text-low">
                          {formatDateShortWithTime(session.updated_at)}
                        </span>
                      </div>
                    </button>
                  );
                })}
            </div>
          )}
        </div>
      </aside>

      <section className="flex-1 min-w-0 flex flex-col">
        <header className="px-base py-half border-b border-border flex items-center justify-between">
          <div>
            <div className="text-sm text-normal font-medium truncate">
              {activeSession?.title || 'Untitled session'}
            </div>
            {activeSession && (
              <div className="text-xs text-low">
                Created {formatDateShortWithTime(activeSession.created_at)}
              </div>
            )}
          </div>
          <div className="flex items-center gap-base">
            <div className="flex items-center gap-base text-xs text-low">
              <UsersIcon className="size-icon-xs" />
              <span>{sessionMembers.length} AI members</span>
            </div>
            {activeSession && (
              <div className="flex items-center gap-half">
                {isArchived && (
                  <Badge variant="secondary" className="text-xs">
                    Archived
                  </Badge>
                )}
                <PrimaryButton
                  variant="secondary"
                  value={isArchived ? 'Restore' : 'Archive'}
                  onClick={() => {
                    if (!activeSessionId) return;
                    if (isArchived) {
                      restoreSessionMutation.mutate(activeSessionId);
                    } else {
                      archiveSessionMutation.mutate(activeSessionId);
                    }
                  }}
                  disabled={
                    archiveSessionMutation.isPending ||
                    restoreSessionMutation.isPending
                  }
                />
              </div>
            )}
          </div>
        </header>

        <div className="flex-1 min-h-0 overflow-y-auto p-base space-y-base">
          {isLoading && (
            <div className="text-sm text-low">Loading chat...</div>
          )}
          {isArchived && !isLoading && (
            <div className="text-xs text-low border border-border rounded-sm bg-secondary/60 px-base py-half">
              This session is archived. Messages and members are read-only.
            </div>
          )}
          {!isLoading && messageList.length === 0 && (
            <div className="text-sm text-low">
              No messages yet. Start the conversation below.
            </div>
          )}

          {messageList.map((message) => {
            const isUser = message.sender_type === ChatSenderType.user;
            const isAgent = message.sender_type === ChatSenderType.agent;
            const agentName =
              isAgent && message.sender_id
                ? agentById.get(message.sender_id)?.name ?? 'Agent'
                : null;
            const diffMeta = isAgent ? extractDiffMeta(message.meta) : null;
            const toneKey = isUser
              ? 'user'
              : message.sender_id ?? agentName ?? 'agent';
            const tone = getMessageTone(String(toneKey), isUser);

            if (message.sender_type === ChatSenderType.system) {
              return (
                <ChatSystemMessage
                  key={message.id}
                  content={message.content}
                  expanded
                />
              );
            }

            return (
              <div
                key={message.id}
                className={cn('flex', isUser ? 'justify-end' : 'justify-start')}
              >
                <ChatEntryContainer
                  variant={isUser ? 'user' : 'system'}
                  title={isUser ? 'You' : agentName ?? 'Agent'}
                  expanded
                  headerRight={
                    <span className="text-xs text-low">
                      {formatDateShortWithTime(message.created_at)}
                    </span>
                  }
                  className={cn(
                    'max-w-[720px] w-full md:w-[80%] shadow-sm rounded-2xl',
                    isUser && 'ml-auto'
                  )}
                  headerClassName="bg-transparent"
                  style={{
                    backgroundColor: tone.bg,
                    borderColor: tone.border,
                  }}
                >
                  <ChatMarkdown content={message.content} />
                  {(diffMeta?.available ||
                    (diffMeta?.untrackedFiles ?? []).length > 0) &&
                    diffMeta.runId && (
                      <div className="mt-half border border-border rounded-sm bg-secondary/70 p-base text-xs text-normal">
                        <details
                          onToggle={(event) => {
                            const target = event.currentTarget as HTMLDetailsElement;
                            if (
                              target.open &&
                              diffMeta.available &&
                              diffMeta.runId
                            ) {
                              handleLoadDiff(diffMeta.runId);
                            }
                          }}
                        >
                          <summary className="cursor-pointer select-none">
                            Code changes
                          </summary>
                          {diffMeta.available && (
                            <div className="mt-half space-y-half">
                              {runDiffs[diffMeta.runId]?.loading && (
                                <div className="text-xs text-low">
                                  Loading diff...
                                </div>
                              )}
                              {runDiffs[diffMeta.runId]?.error && (
                                <div className="text-xs text-error">
                                  {runDiffs[diffMeta.runId]?.error}
                                </div>
                              )}
                              {!runDiffs[diffMeta.runId]?.loading &&
                                runDiffs[diffMeta.runId]?.files.length === 0 &&
                                !runDiffs[diffMeta.runId]?.error && (
                                  <div className="text-xs text-low">
                                    No tracked diff available.
                                  </div>
                                )}
                              {runDiffs[diffMeta.runId]?.files?.map((file) => (
                                <details
                                  key={`${diffMeta.runId}-${file.path}`}
                                  className="border border-border rounded-sm bg-panel"
                                >
                                  <summary className="flex items-center justify-between px-base py-half cursor-pointer text-xs text-normal">
                                    <span className="font-ibm-plex-mono">
                                      {file.path}
                                    </span>
                                    <span className="text-xs text-low">
                                      {file.additions > 0 && (
                                        <span className="text-success">
                                          +{file.additions}
                                        </span>
                                      )}
                                      {file.additions > 0 &&
                                        file.deletions > 0 &&
                                        ' '}
                                      {file.deletions > 0 && (
                                        <span className="text-error">
                                          -{file.deletions}
                                        </span>
                                      )}
                                    </span>
                                  </summary>
                                  <div className="border-t border-border">
                                    <DiffViewBody
                                      fileDiffMetadata={null}
                                      unifiedDiff={file.patch}
                                      isValid={file.patch.trim().length > 0}
                                      hideLineNumbers={false}
                                      theme={actualTheme}
                                      wrapText={false}
                                      modeOverride="split"
                                    />
                                  </div>
                                </details>
                              ))}
                              <div className="mt-half">
                                <button
                                  type="button"
                                  className="text-brand hover:text-brand-hover"
                                  onClick={() =>
                                    window.open(
                                      chatApi.getRunDiffUrl(diffMeta.runId!),
                                      '_blank',
                                      'noopener,noreferrer'
                                    )
                                  }
                                >
                                  Open raw diff
                                </button>
                              </div>
                            </div>
                          )}
                          {diffMeta.untrackedFiles.length > 0 && (
                            <div className="mt-base space-y-half">
                              <div className="text-xs text-low">
                                Untracked files
                              </div>
                              {diffMeta.untrackedFiles.map((path) => {
                                const key = `${diffMeta.runId}:${path}`;
                                const entry = untrackedContent[key];
                                return (
                                  <div
                                    key={key}
                                    className="border border-border rounded-sm bg-panel px-base py-half"
                                  >
                                    <div className="flex items-center justify-between text-xs">
                                      <span className="font-ibm-plex-mono break-all">
                                        {path}
                                      </span>
                                      <button
                                        type="button"
                                        className="text-brand hover:text-brand-hover"
                                        onClick={() =>
                                          handleToggleUntracked(
                                            diffMeta.runId!,
                                            path
                                          )
                                        }
                                      >
                                        {entry?.open ? 'Hide' : 'View'}
                                      </button>
                                    </div>
                                    {entry?.open && (
                                      <div className="mt-half">
                                        {entry.loading && (
                                          <div className="text-xs text-low">
                                            Loading file...
                                          </div>
                                        )}
                                        {entry.error && (
                                          <div className="text-xs text-error">
                                            {entry.error}
                                          </div>
                                        )}
                                        {!entry.loading &&
                                          !entry.error &&
                                          entry.content && (
                                            <pre className="max-h-64 overflow-y-auto whitespace-pre-wrap break-words text-xs font-ibm-plex-mono text-normal">
                                              {entry.content}
                                            </pre>
                                          )}
                                      </div>
                                    )}
                                  </div>
                                );
                              })}
                            </div>
                          )}
                        </details>
                      </div>
                    )}
                </ChatEntryContainer>
              </div>
            );
          })}

          {placeholderAgents.map((member) => {
            const startedAt = runningSince[member.agent.id] ?? clock;
            const elapsedSeconds = Math.max(
              0,
              Math.floor((clock - startedAt) / 1000)
            );
            const tone = getMessageTone(member.agent.id, false);
            return (
              <div
                key={`placeholder-${member.agent.id}`}
                className="flex justify-start"
              >
                <ChatEntryContainer
                  variant="system"
                  title={member.agent.name}
                  expanded
                  className="max-w-[720px] w-full md:w-[80%] opacity-80 shadow-sm rounded-2xl"
                  headerClassName="bg-transparent"
                  style={{
                    backgroundColor: tone.bg,
                    borderColor: tone.border,
                  }}
                >
                  <ChatMarkdown
                    content={`回复中。。。已用${elapsedSeconds}秒`}
                  />
                </ChatEntryContainer>
              </div>
            );
          })}

          {Object.entries(streamingRuns).map(([runId, run]) => {
            const agentName =
              agentById.get(run.agentId)?.name ?? 'Agent';
            const tone = getMessageTone(run.agentId, false);
            return (
              <div
                key={`stream-${runId}`}
                className="flex justify-start"
              >
                <ChatEntryContainer
                  variant="system"
                  title={agentName}
                  expanded
                  className="max-w-[720px] w-full md:w-[80%] opacity-90 shadow-sm rounded-2xl"
                  headerClassName="bg-transparent"
                  style={{
                    backgroundColor: tone.bg,
                    borderColor: tone.border,
                  }}
                  headerRight={
                    <div className="flex items-center gap-half text-xs text-low">
                      <span>Streaming</span>
                      <span className="flex items-center gap-[2px]">
                        <span className="size-dot rounded-full bg-brand animate-running-dot-1" />
                        <span className="size-dot rounded-full bg-brand animate-running-dot-2" />
                        <span className="size-dot rounded-full bg-brand animate-running-dot-3" />
                      </span>
                    </div>
                  }
                >
                  <ChatMarkdown content={run.content} />
                </ChatEntryContainer>
              </div>
            );
          })}
          <div ref={bottomRef} />
        </div>

        <div className="border-t border-border p-base space-y-base">
          <div className="flex items-center gap-base flex-wrap">
            <MultiSelectDropdown
              icon={ChatsTeardropIcon}
              label="Mention agents"
              menuLabel="Route to agents"
              values={selectedMentions}
              options={agentOptions}
              onChange={setSelectedMentions}
              disabled={
                !activeSessionId || mentionAgents.length === 0 || isArchived
              }
            />
            {selectedMentions.length > 0 && (
              <div className="flex items-center gap-half flex-wrap">
                {selectedMentions.map((mention) => (
                  <Badge
                    key={mention}
                    variant="secondary"
                    className="flex items-center gap-half px-2 py-0.5"
                  >
                    @{mention}
                    <button
                      type="button"
                      onClick={() =>
                        setSelectedMentions((prev) =>
                          prev.filter((item) => item !== mention)
                        )
                      }
                      className="text-xs text-low hover:text-normal"
                    >
                      <XIcon className="size-icon-2xs" />
                    </button>
                  </Badge>
                ))}
              </div>
            )}
          </div>

          <div className="relative">
            <textarea
              ref={inputRef}
              value={draft}
              onChange={(event) => handleDraftChange(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !event.shiftKey) {
                  event.preventDefault();
                  handleSend();
                }
              }}
              placeholder={
                isArchived
                  ? 'This session is archived and read-only.'
                  : 'Type your message and @mention agents...'
              }
              rows={3}
              disabled={isArchived || !activeSessionId}
              className={cn(
                'w-full resize-none rounded-sm border border-border bg-panel',
                'px-base py-half text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand',
                isArchived && 'opacity-60 cursor-not-allowed'
              )}
            />
            {mentionQuery !== null && visibleMentionSuggestions.length > 0 && (
              <div className="absolute z-20 left-0 right-0 mt-half bg-panel border border-border rounded-sm shadow">
                {visibleMentionSuggestions.map((agent) => (
                  <button
                    key={agent.id}
                    type="button"
                    onClick={() => handleMentionSelect(agent.name)}
                    className={cn(
                      'w-full px-base py-half text-left text-sm text-normal',
                      'hover:bg-secondary flex items-center justify-between'
                    )}
                  >
                    <span>@{agent.name}</span>
                    <CaretRightIcon className="size-icon-xs text-low" />
                  </button>
                ))}
              </div>
            )}
          </div>

          <div className="flex items-center justify-between">
            <span className="text-xs text-low">
              Press Enter to send, Shift+Enter for new line.
            </span>
            <PrimaryButton
              value="Send"
              actionIcon={
                sendMessageMutation.isPending ? 'spinner' : PaperPlaneRightIcon
              }
              onClick={handleSend}
              disabled={!canSend}
            />
          </div>
        </div>
      </section>

      <aside className="w-80 border-l border-border flex flex-col min-h-0">
        <div className="px-base py-base border-b border-border flex items-center justify-between">
          <div className="text-sm text-normal font-medium">AI Members</div>
          <div className="text-xs text-low">
            {sessionMembers.length} in session
          </div>
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto p-base space-y-base">
          {!activeSessionId && (
            <div className="text-xs text-low">
              Select a session to manage AI members.
            </div>
          )}
          {activeSessionId && sessionMembers.length === 0 && (
            <div className="text-xs text-low">
              No AI members yet. Add one below to enable @mentions.
            </div>
          )}
          {sessionMembers.map(({ agent, sessionAgent }) => {
            const state =
              agentStates[agent.id] ?? ChatSessionAgentState.idle;
            return (
              <div
                key={sessionAgent.id}
                className="border border-border rounded-sm px-base py-half space-y-half"
              >
                <div className="flex items-center justify-between gap-base">
                  <div className="flex items-center gap-half min-w-0">
                    <span
                      className={cn(
                        'size-2 rounded-full',
                        agentStateDotClass[state],
                        state === ChatSessionAgentState.running &&
                          'animate-pulse'
                      )}
                    />
                    <div className="min-w-0">
                      <div className="text-sm text-normal truncate">
                        @{agent.name}
                      </div>
                      <div className="text-xs text-low">
                        {toPrettyCase(agent.runner_type)} ·{' '}
                        {agentStateLabels[state]}
                      </div>
                    </div>
                  </div>
                  <div className="flex items-center gap-half text-xs">
                    <button
                      type="button"
                      className="text-brand hover:text-brand-hover"
                      onClick={() => {
                        setWorkspaceAgentId(agent.id);
                        setWorkspaceDrawerOpen(true);
                      }}
                    >
                      Workspace
                    </button>
                    <button
                      type="button"
                      className={cn(
                        'text-low hover:text-normal',
                        isArchived && 'pointer-events-none opacity-50'
                      )}
                      onClick={() => handleEditMember({ agent, sessionAgent })}
                      disabled={isArchived}
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      className={cn(
                        'text-error hover:text-error/80',
                        isArchived && 'pointer-events-none opacity-50'
                      )}
                      onClick={() => handleRemoveMember({ agent, sessionAgent })}
                      disabled={isArchived}
                    >
                      Remove
                    </button>
                  </div>
                </div>
                {sessionAgent.workspace_path && (
                  <div className="text-xs text-low break-all">
                    {sessionAgent.workspace_path}
                  </div>
                )}
              </div>
            );
          })}

          <div className="border-t border-border pt-base space-y-half">
            {!isAddMemberOpen ? (
              <PrimaryButton
                variant="secondary"
                value="Add AI member"
                onClick={() => {
                  setIsAddMemberOpen(true);
                  setMemberError(null);
                  setEditingMember(null);
                  setNewMemberName('');
                  setNewMemberPrompt('');
                  setNewMemberWorkspace('');
                }}
                disabled={!activeSessionId || isArchived}
              >
                <PlusIcon className="size-icon-xs" />
              </PrimaryButton>
            ) : (
              <div className="border border-border rounded-sm p-base space-y-half">
                <div className="text-sm text-normal font-medium">
                  {editingMember ? 'Edit AI member' : 'Add AI member'}
                </div>
                <div className="text-xs text-low">
                  AI member name is the @mention handle.
                </div>
                <div className="space-y-half">
                  <label className="text-xs text-low">AI member name</label>
                  <input
                    value={newMemberName}
                    onChange={(event) => setNewMemberName(event.target.value)}
                    placeholder="e.g. coder"
                    className={cn(
                      'w-full rounded-sm border border-border bg-panel px-base py-half',
                      'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                    )}
                  />
                </div>
                <div className="space-y-half">
                  <label className="text-xs text-low">
                    Base coding agent
                  </label>
                  <select
                    value={newMemberRunnerType}
                    onChange={(event) =>
                      setNewMemberRunnerType(event.target.value)
                    }
                    className={cn(
                      'w-full rounded-sm border border-border bg-panel px-base py-half',
                      'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                    )}
                  >
                    {availableRunnerTypes.map((runner) => (
                      <option key={runner} value={runner}>
                        {toPrettyCase(runner)}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="space-y-half">
                  <label className="text-xs text-low">System prompt</label>
                  <textarea
                    value={newMemberPrompt}
                    onChange={(event) =>
                      setNewMemberPrompt(event.target.value)
                    }
                    rows={3}
                    placeholder="Describe how this AI member should behave."
                    className={cn(
                      'w-full resize-none rounded-sm border border-border bg-panel',
                      'px-base py-half text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                    )}
                  />
                </div>
                <div className="space-y-half">
                  <label className="text-xs text-low">Workspace path</label>
                  <input
                    value={newMemberWorkspace}
                    onChange={(event) =>
                      setNewMemberWorkspace(event.target.value)
                    }
                    placeholder="Absolute path on the server"
                    className={cn(
                      'w-full rounded-sm border border-border bg-panel px-base py-half',
                      'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                    )}
                  />
                </div>
                {memberError && (
                  <div className="text-xs text-error">{memberError}</div>
                )}
                <div className="flex items-center justify-end gap-half pt-half">
                  <PrimaryButton
                    variant="tertiary"
                    value="Cancel"
                    onClick={() => {
                      setIsAddMemberOpen(false);
                      setMemberError(null);
                      setEditingMember(null);
                    }}
                    disabled={isSavingMember}
                  />
                  <PrimaryButton
                    value={editingMember ? 'Save' : 'Add'}
                    actionIcon={isSavingMember ? 'spinner' : PlusIcon}
                    onClick={handleAddMember}
                    disabled={isSavingMember || isArchived}
                  />
                </div>
              </div>
            )}
          </div>
        </div>
      </aside>

      <div
        className={cn(
          'absolute top-0 right-0 h-full w-[360px] bg-primary border-l border-border shadow-lg transition-transform',
          workspaceDrawerOpen ? 'translate-x-0' : 'translate-x-full'
        )}
      >
        <div className="px-base py-base border-b border-border flex items-center justify-between">
          <div>
            <div className="text-sm text-normal font-medium">
              {activeWorkspaceAgent?.name ?? 'Agent workspace'}
            </div>
            <div className="text-xs text-low">
              Workspace & run history
            </div>
          </div>
          <button
            type="button"
            onClick={() => setWorkspaceDrawerOpen(false)}
            className="text-low hover:text-normal"
          >
            <XIcon className="size-icon-sm" />
          </button>
        </div>
        <div className="p-base space-y-base overflow-y-auto h-full">
          <div className="space-y-half">
            <div className="text-xs text-low">
              Workspace path is created on first run.
            </div>
            {workspacePath && (
              <div className="border border-border rounded-sm px-base py-half text-xs font-mono text-normal break-all">
                {workspacePath}
              </div>
            )}
          </div>

          <div className="space-y-half">
            <div className="text-sm text-normal font-medium">
              Run history
            </div>
            {activeWorkspaceRuns.length === 0 && (
              <div className="text-xs text-low">
                No runs yet for this agent.
              </div>
            )}
            {activeWorkspaceRuns.map((run) => (
              <div
                key={run.runId}
                className="border border-border rounded-sm p-base space-y-half"
              >
                <div className="flex items-center justify-between text-xs text-low">
                  <span>Run {run.runId.slice(0, 8)}</span>
                  <span>{formatDateShortWithTime(run.createdAt)}</span>
                </div>
                <div className="text-xs text-normal">
                  {run.content}
                </div>
                <div className="flex items-center justify-between text-xs">
                  <button
                    type="button"
                    className="text-brand hover:text-brand-hover"
                    onClick={() => handleLoadLog(run.runId)}
                  >
                    View log
                  </button>
                  {logRunId === run.runId && (
                    <span className="text-low">Selected</span>
                  )}
                </div>
              </div>
            ))}
          </div>

          <div className="space-y-half">
            <div className="text-sm text-normal font-medium">Run log</div>
            {!logRunId && (
              <div className="text-xs text-low">
                Select a run to view its log output.
              </div>
            )}
            {logRunId && (
              <div className="border border-border rounded-sm bg-secondary p-base">
                <div className="flex items-center justify-between text-xs text-low pb-half">
                  <span>Run {logRunId.slice(0, 8)}</span>
                  <button
                    type="button"
                    className="text-brand hover:text-brand-hover"
                    onClick={() => handleLoadLog(logRunId)}
                    disabled={logLoading}
                  >
                    Refresh
                  </button>
                </div>
                {logLoading && (
                  <div className="text-xs text-low">Loading log...</div>
                )}
                {logError && (
                  <div className="text-xs text-error">{logError}</div>
                )}
                {!logLoading && !logError && (
                  <pre
                    className={cn(
                      'text-xs text-normal whitespace-pre-wrap break-words',
                      'max-h-64 overflow-y-auto font-ibm-plex-mono'
                    )}
                  >
                    {logContent || 'Log is empty.'}
                  </pre>
                )}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
