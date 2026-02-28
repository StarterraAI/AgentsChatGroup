import {
  type ChangeEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import { useQueryClient } from '@tanstack/react-query';
import { useNavigate, useParams } from 'react-router-dom';
import {
  ChatMessage,
  ChatSenderType,
  ChatSessionStatus,
  ChatSessionAgentState,
  BaseCodingAgent,
  type AvailabilityInfo,
  type JsonValue,
  type ChatMemberPreset,
  type ChatTeamPreset,
} from 'shared/types';
import { ApiError, chatApi, configApi } from '@/lib/api';
import { useUserSystem } from '@/components/ConfigProvider';
import { useTheme } from '@/components/ThemeProvider';
import { getActualTheme } from '@/utils/theme';
import {
  extractExecutorProfileVariant,
  formatExecutorModelLabel,
  getVariantDisplayLabel,
  getVariantModelName,
  getVariantOptions as getExecutorVariantOptions,
  withExecutorProfileVariant,
} from '@/utils/executor';
import { SettingsDialog } from '@/components/ui-new/dialogs/SettingsDialog';

import {
  type SessionMember,
  type RunHistoryItem,
  type MentionStatus,
  useChatData,
  useRunHistory,
  useChatMutations,
  useChatWebSocket,
  useMessageInput,
  useDiffViewer,
  fallbackRunnerTypes,
  memberNameRegex,
  MAX_MEMBER_NAME_LENGTH,
  getMemberNameLength,
  getMessageTone,
  extractDiffMeta,
  extractMentions,
  extractReferenceId,
  extractAttachments,
  truncateText,
  sanitizeHandle,
} from './chat';
import {
  buildMemberPresetImportPlan,
  getLocalizedMemberPresetName,
  getLocalizedTeamPresetName,
  validateWorkspacePath,
  type MemberPresetImportPlan,
} from './chat/utils';

import { isAllowedAttachment } from './chat/components/MessageInputArea';
import { SessionListSidebar } from './chat/components/SessionListSidebar';
import { ChatHeader } from './chat/components/ChatHeader';
import { CleanupModeBar } from './chat/components/CleanupModeBar';
import { ChatMessageItem } from './chat/components/ChatMessageItem';
import { RunningAgentPlaceholder } from './chat/components/RunningAgentPlaceholder';
import { StreamingRunEntry } from './chat/components/StreamingRunEntry';
import { MessageInputArea } from './chat/components/MessageInputArea';
import { AiMembersSidebar } from './chat/components/AiMembersSidebar';
import { WorkspaceDrawer } from './chat/components/WorkspaceDrawer';
import { DiffViewerModal } from './chat/components/DiffViewerModal';
import { PromptEditorModal } from './chat/components/PromptEditorModal';
import { ConfirmModal } from './chat/components/ConfirmModal';
import { FilePreviewModal } from './chat/components/FilePreviewModal';

const mentionStatusPriority: Record<MentionStatus, number> = {
  received: 0,
  running: 1,
  completed: 2,
  failed: 2,
};

const coerceMentionStatus = (value: unknown): MentionStatus | null => {
  if (
    value === 'received' ||
    value === 'running' ||
    value === 'completed' ||
    value === 'failed'
  ) {
    return value;
  }
  return null;
};

const isImageAttachment = (file: File) => file.type.startsWith('image/');

const isTextAttachment = (file: File) =>
  file.type.startsWith('text/') ||
  [
    '.txt',
    '.csv',
    '.md',
    '.json',
    '.xml',
    '.yaml',
    '.yml',
    '.html',
    '.htm',
    '.css',
    '.js',
    '.ts',
    '.jsx',
    '.tsx',
    '.py',
    '.java',
    '.c',
    '.cpp',
    '.h',
    '.hpp',
    '.rb',
    '.php',
    '.go',
    '.rs',
    '.sql',
    '.sh',
    '.bash',
    '.svg',
  ].some((ext) => file.name.toLowerCase().endsWith(ext));

const MAX_SESSION_TITLE_LENGTH = 20;
const COLLAPSED_LEFT_SIDEBAR_WIDTH = 52;
const MESSAGE_SEARCH_HIGHLIGHT_NAME = 'chat-session-search-highlight';
const MAX_MESSAGE_SEARCH_HIGHLIGHT_RANGES = 4000;
const MESSAGE_SEARCH_DEBOUNCE_MS = 120;

type CSSHighlightRegistry = {
  set: (name: string, highlight: unknown) => void;
  delete: (name: string) => void;
};

const escapeSearchRegExp = (value: string) =>
  value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const getSessionTitleLength = (value: string) =>
  Array.from(value.trim()).length;

export function ChatSessions() {
  const { t } = useTranslation('chat');
  const { t: tCommon } = useTranslation('common');
  const { sessionId } = useParams<{ sessionId?: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const promptFileInputRef = useRef<HTMLInputElement | null>(null);
  const { config, profiles, loginStatus } = useUserSystem();
  const { theme } = useTheme();
  const actualTheme = getActualTheme(theme);

  // Data queries
  const {
    sortedSessions,
    activeSessions,
    archivedSessions,
    agents,
    sessionAgents,
    messagesData,
    agentById,
    sessionMembers,
    mentionAgents,
    isSessionsLoading,
    isLoading,
  } = useChatData(sessionId ?? null);

  const activeSessionExists = useMemo(
    () =>
      !!sessionId &&
      sortedSessions.some((session) => session.id === sessionId),
    [sessionId, sortedSessions]
  );
  const activeSessionId = sessionId
    ? isSessionsLoading || activeSessionExists
      ? sessionId
      : null
    : sortedSessions[0]?.id ?? null;
  const notificationsRef = useRef(config?.notifications ?? null);
  const sessionTitleByIdRef = useRef<Map<string, string>>(new Map());
  const agentByIdRef = useRef(agentById);
  const notifiedMessageIdsRef = useRef<Set<string>>(new Set());
  const notificationPermissionRequestedRef = useRef(false);

  useEffect(() => {
    notificationsRef.current = config?.notifications ?? null;
  }, [config?.notifications]);

  useEffect(() => {
    sessionTitleByIdRef.current = new Map(
      sortedSessions.map((session) => [
        session.id,
        session.title?.trim() || 'Group Chat',
      ])
    );
  }, [sortedSessions]);

  useEffect(() => {
    agentByIdRef.current = agentById;
  }, [agentById]);

  useEffect(() => {
    notifiedMessageIdsRef.current.clear();
  }, [activeSessionId]);

  // Messages state
  const [messages, setMessages] = useState<ChatMessage[]>([]);

  const upsertMessage = useCallback(
    (message: ChatMessage) => {
      setMessages((prev) => {
        const existingIndex = prev.findIndex((item) => item.id === message.id);
        if (existingIndex >= 0) {
          const next = [...prev];
          next[existingIndex] = message;
          return next;
        }
        return [...prev, message];
      });

      if (!message.session_id) return;
      queryClient.setQueryData<ChatMessage[]>(
        ['chatMessages', message.session_id],
        (prev) => {
          if (!prev) return [message];
          const existingIndex = prev.findIndex(
            (item) => item.id === message.id
          );
          if (existingIndex >= 0) {
            const next = [...prev];
            next[existingIndex] = message;
            return next;
          }
          return [...prev, message];
        }
      );
    },
    [queryClient]
  );

  const handleIncomingMessage = useCallback(
    (message: ChatMessage) => {
      upsertMessage(message);

      const notifications = notificationsRef.current;
      if (!notifications || message.sender_type === ChatSenderType.user) return;
      if (!notifications.sound_enabled && !notifications.push_enabled) return;
      if (notifiedMessageIdsRef.current.has(message.id)) return;
      notifiedMessageIdsRef.current.add(message.id);

      if (notifications.sound_enabled) {
        const audio = new Audio(`/api/sounds/${notifications.sound_file}`);
        void audio.play().catch((error) => {
          console.warn(
            'Failed to play incoming chat notification sound',
            error
          );
        });
      }

      const canShowPush =
        notifications.push_enabled &&
        typeof window !== 'undefined' &&
        'Notification' in window &&
        (document.visibilityState === 'hidden' || !document.hasFocus());

      if (!canShowPush) return;

      const senderLabel =
        message.sender_type === ChatSenderType.agent
          ? message.sender_id
            ? (agentByIdRef.current.get(message.sender_id)?.name ?? 'Agent')
            : 'Agent'
          : 'System';

      const attachmentCount = extractAttachments(message.meta).length;
      const content = message.content.trim();
      const preview =
        content.length > 0
          ? truncateText(content, 120)
          : attachmentCount > 0
            ? `Shared ${attachmentCount} attachment${attachmentCount > 1 ? 's' : ''}`
            : 'Sent a new message';

      const sessionTitle =
        (message.session_id &&
          sessionTitleByIdRef.current.get(message.session_id)) ||
        'Group Chat';

      const showNotification = () => {
        try {
          const notification = new Notification(sessionTitle, {
            body: `${senderLabel}: ${preview}`,
            tag: `chat-session-${message.session_id ?? 'unknown'}`,
          });
          notification.onclick = () => {
            window.focus();
          };
        } catch (error) {
          console.warn('Failed to show chat notification', error);
        }
      };

      if (Notification.permission === 'granted') {
        showNotification();
        return;
      }

      if (
        Notification.permission === 'default' &&
        !notificationPermissionRequestedRef.current
      ) {
        notificationPermissionRequestedRef.current = true;
        void Notification.requestPermission()
          .then((permission) => {
            if (permission === 'granted') {
              showNotification();
            }
          })
          .catch((error) => {
            console.warn('Failed to request notification permission', error);
          });
      }
    },
    [upsertMessage]
  );

  // WebSocket connection
  const {
    streamingRuns,
    agentStates,
    agentStateInfos,
    mentionStatuses,
    compressionWarning,
    setAgentStates,
    setAgentStateInfos,
    setMentionStatuses,
    clearCompressionWarning,
  } = useChatWebSocket(activeSessionId, handleIncomingMessage);

  // Mutations
  const {
    createSession,
    updateSession,
    archiveSession,
    restoreSession,
    deleteSession,
    sendMessage,
    deleteMessages,
  } = useChatMutations(
    (session) => navigate(`/chat/${session.id}`),
    (session) => navigate(`/chat/${session.id}`),
    upsertMessage,
    () => {
      if (activeSessionId) {
        queryClient.invalidateQueries({
          queryKey: ['chatMessages', activeSessionId],
        });
      }
    },
    () => {
      navigate('/chat');
    }
  );

  // Message input
  const getMessageMentionHandle = useCallback(
    (message: ChatMessage) => {
      if (message.sender_type !== ChatSenderType.agent) return null;
      if (!message.sender_id) return null;
      const name = agentById.get(message.sender_id)?.name ?? null;
      if (!name || !memberNameRegex.test(name)) return null;
      return name;
    },
    [agentById]
  );

  const {
    draft,
    selectedMentions,
    setSelectedMentions,
    mentionQuery,
    replyToMessage,
    setReplyToMessage,
    inputRef,
    handleDraftChange,
    handleMentionSelect,
    handleReplySelect,
    visibleMentionSuggestions,
    agentOptions,
    resetInput,
    highlightedMentionIndex,
    handleMentionKeyDown,
  } = useMessageInput(mentionAgents);

  // Diff viewer
  const {
    diffViewerRunId,
    diffViewerUntracked,
    diffViewerHasDiff,
    diffViewerOpen,
    diffViewerFullscreen,
    runDiffs,
    untrackedContent,
    handleOpenDiffViewer,
    handleCloseDiffViewer,
    handleToggleFullscreen,
    handleToggleUntracked,
    resetDiffViewer,
  } = useDiffViewer();

  // Local state
  const [isUploadingAttachments, setIsUploadingAttachments] = useState(false);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const [attachedFiles, setAttachedFiles] = useState<File[]>([]);
  const [previewFile, setPreviewFile] = useState<{
    file: File | null;
    content: string | null;
  }>({ file: null, content: null });
  const [agentAvailability, setAgentAvailability] = useState<
    Record<string, AvailabilityInfo | null>
  >({});
  const [isCheckingAvailability, setIsCheckingAvailability] = useState(false);
  const [workspaceDrawerOpen, setWorkspaceDrawerOpen] = useState(false);
  const [workspaceAgentId, setWorkspaceAgentId] = useState<string | null>(null);
  const messagesContainerRef = useRef<HTMLDivElement | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);
  const previousSessionIdRef = useRef<string | null>(null);
  const [isAddMemberOpen, setIsAddMemberOpen] = useState(false);
  const [editingMember, setEditingMember] = useState<SessionMember | null>(
    null
  );
  const [newMemberName, setNewMemberName] = useState('');
  const [newMemberRunnerType, setNewMemberRunnerType] = useState('');
  const [newMemberVariant, setNewMemberVariant] = useState('DEFAULT');
  const [newMemberPrompt, setNewMemberPrompt] = useState('');
  const [newMemberWorkspace, setNewMemberWorkspace] = useState('');
  const memberNameLengthError =
    newMemberName.trim().length > 0 &&
    getMemberNameLength(newMemberName) > MAX_MEMBER_NAME_LENGTH
      ? `AI member name cannot exceed ${MAX_MEMBER_NAME_LENGTH} characters.`
      : null;
  const [memberError, setMemberError] = useState<string | null>(null);
  const [isSavingMember, setIsSavingMember] = useState(false);
  const [isPromptEditorOpen, setIsPromptEditorOpen] = useState(false);
  const [promptFileError, setPromptFileError] = useState<string | null>(null);
  const [promptFileLoading, setPromptFileLoading] = useState(false);
  const [logRunId, setLogRunId] = useState<string | null>(null);
  const [logContent, setLogContent] = useState('');
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);
  const [clock, setClock] = useState(() => Date.now());
  const [stoppingAgents, setStoppingAgents] = useState<Set<string>>(new Set());
  const [showArchived, setShowArchived] = useState(false);
  const [isEditingTitle, setIsEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState('');
  const [titleError, setTitleError] = useState<string | null>(null);
  const [isCleanupMode, setIsCleanupMode] = useState(false);
  const [selectedMessageIds, setSelectedMessageIds] = useState<Set<string>>(
    new Set()
  );
  const [isMessageSearchOpen, setIsMessageSearchOpen] = useState(false);
  const [messageSearchQuery, setMessageSearchQuery] = useState('');
  const [debouncedMessageSearchQuery, setDebouncedMessageSearchQuery] =
    useState('');
  const [confirmModal, setConfirmModal] = useState<{
    title: string;
    message: string;
    onConfirm: () => void | Promise<void>;
    mode?: 'confirm' | 'alert';
    confirmText?: string;
    cancelText?: string;
  } | null>(null);
  const [teamImportPlan, setTeamImportPlan] = useState<
    MemberPresetImportPlan[] | null
  >(null);
  const [teamImportName, setTeamImportName] = useState<string | null>(null);
  const [isImportingTeam, setIsImportingTeam] = useState(false);
  const [isConfirmLoading, setIsConfirmLoading] = useState(false);
  const [isDeletingMessages, setIsDeletingMessages] = useState(false);
  const [leftSidebarWidth, setLeftSidebarWidth] = useState(340);
  const [isLeftSidebarCollapsed, setIsLeftSidebarCollapsed] = useState(false);
  const [rightSidebarWidth, setRightSidebarWidth] = useState(320);
  const [inputAreaHeight, setInputAreaHeight] = useState(160);
  const [isResizing, setIsResizing] = useState<
    'left' | 'right' | 'input' | null
  >(null);
  const resizeStartRef = useRef<{
    startX: number;
    startY: number;
    startWidth: number;
    startHeight: number;
  } | null>(null);
  const lastExpandedLeftWidthRef = useRef(340);

  const showDuplicateMemberNameWarning = useCallback(
    (name: string) => {
      setMemberError(null);
      setConfirmModal({
        title: t('modals.confirm.titles.duplicateMemberName'),
        message: t('modals.confirm.messages.duplicateMemberName', {
          name: `@${name}`,
        }),
        mode: 'alert',
        confirmText: tCommon('ok'),
        onConfirm: () => {},
      });
    },
    [t, tCommon]
  );

  // Sync messages from query
  useEffect(() => {
    if (activeSessionId) {
      setMessages(messagesData);
    } else {
      setMessages([]);
    }
  }, [messagesData, activeSessionId]);

  useEffect(() => {
    if (messagesData.length === 0) return;
    setMentionStatuses((prev) => {
      let changed = false;
      const next = new Map(prev);
      for (const message of messagesData) {
        const meta = message.meta;
        if (!meta || typeof meta !== 'object' || Array.isArray(meta)) continue;
        const rawStatuses = (meta as { mention_statuses?: unknown })
          .mention_statuses;
        if (
          !rawStatuses ||
          typeof rawStatuses !== 'object' ||
          Array.isArray(rawStatuses)
        ) {
          continue;
        }
        const perMessage = new Map(next.get(message.id) ?? []);
        let perMessageChanged = false;
        for (const [agentName, statusValue] of Object.entries(
          rawStatuses as Record<string, unknown>
        )) {
          const status = coerceMentionStatus(statusValue);
          if (!status) continue;
          const existing = perMessage.get(agentName);
          const existingPriority = existing
            ? mentionStatusPriority[existing]
            : -1;
          if (mentionStatusPriority[status] > existingPriority) {
            perMessage.set(agentName, status);
            perMessageChanged = true;
          }
        }
        if (perMessageChanged) {
          next.set(message.id, perMessage);
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [messagesData, setMentionStatuses]);

  // Reset state on session change
  useEffect(() => {
    resetInput();
    resetDiffViewer();
    setReplyToMessage(null);
    setIsUploadingAttachments(false);
    setAttachmentError(null);
    setWorkspaceDrawerOpen(false);
    setWorkspaceAgentId(null);
    setIsAddMemberOpen(false);
    setNewMemberName('');
    setNewMemberVariant('DEFAULT');
    setNewMemberPrompt('');
    setNewMemberWorkspace('');
    setMemberError(null);
    setEditingMember(null);
    setLogRunId(null);
    setLogContent('');
    setLogError(null);
    setClock(Date.now());
    setStoppingAgents(new Set());
    setIsEditingTitle(false);
    setTitleError(null);
    setIsMessageSearchOpen(false);
    setMessageSearchQuery('');
    setDebouncedMessageSearchQuery('');
    setIsPromptEditorOpen(false);
    setPromptFileError(null);
    setPromptFileLoading(false);
    setTeamImportPlan(null);
    setTeamImportName(null);
  }, [activeSessionId, resetInput, resetDiffViewer, setReplyToMessage]);

  // Navigate to first session if needed
  useEffect(() => {
    if (isSessionsLoading) return;

    if (!sessionId && sortedSessions.length > 0) {
      navigate(`/chat/${sortedSessions[0].id}`, { replace: true });
      return;
    }

    if (sessionId && sortedSessions.length === 0) {
      navigate('/chat', { replace: true });
      return;
    }

    if (
      sessionId &&
      sortedSessions.length > 0 &&
      !sortedSessions.some((session) => session.id === sessionId)
    ) {
      navigate(`/chat/${sortedSessions[0].id}`, { replace: true });
    }
  }, [isSessionsLoading, navigate, sessionId, sortedSessions]);

  // Derived state
  const availableRunnerTypes = useMemo(() => {
    const keys = Object.keys(profiles ?? {});
    const baseList = keys.length > 0 ? keys : fallbackRunnerTypes;
    if (editingMember && !baseList.includes(editingMember.agent.runner_type)) {
      return [...baseList, editingMember.agent.runner_type];
    }
    return baseList;
  }, [editingMember, profiles]);

  const isRunnerAvailable = useCallback(
    (runner: string) => {
      const info = agentAvailability[runner];
      return (
        info?.type === 'LOGIN_DETECTED' || info?.type === 'INSTALLATION_FOUND'
      );
    },
    [agentAvailability]
  );

  const enabledRunnerTypes = useMemo(
    () => availableRunnerTypes.filter((runner) => isRunnerAvailable(runner)),
    [availableRunnerTypes, isRunnerAvailable]
  );

  const availabilityLabel = useCallback(
    (runner: string) => {
      const info = agentAvailability[runner];
      if (!info)
        return isCheckingAvailability ? ' (checking)' : ' (unavailable)';
      if (
        info.type === 'LOGIN_DETECTED' ||
        info.type === 'INSTALLATION_FOUND'
      ) {
        return '';
      }
      return ' (not installed)';
    },
    [agentAvailability, isCheckingAvailability]
  );

  const getModelName = useCallback(
    (runnerType: string, variant?: string): string | null => {
      return getVariantModelName(
        runnerType as BaseCodingAgent,
        variant ?? null,
        profiles
      );
    },
    [profiles]
  );

  const getModelDisplayName = useCallback(
    (runnerType: string, modelName: string | null): string | null =>
      formatExecutorModelLabel(runnerType as BaseCodingAgent, modelName),
    []
  );

  const getVariantLabel = useCallback(
    (runnerType: string, variant: string): string =>
      getVariantDisplayLabel(
        runnerType as BaseCodingAgent,
        variant,
        profiles
      ),
    [profiles]
  );

  const getVariantOptions = useCallback(
    (runnerType: string): string[] => {
      return getExecutorVariantOptions(
        runnerType as BaseCodingAgent,
        profiles
      );
    },
    [profiles]
  );

  const memberVariantOptions = useMemo(
    () => getVariantOptions(newMemberRunnerType),
    [getVariantOptions, newMemberRunnerType]
  );

  // Preset-derived state
  const enabledMemberPresets = useMemo(
    () => (config?.chat_presets?.members ?? []).filter((m) => m.enabled),
    [config?.chat_presets?.members]
  );
  const enabledTeamPresets = useMemo(
    () => (config?.chat_presets?.teams ?? []).filter((t) => t.enabled),
    [config?.chat_presets?.teams]
  );

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

  const messageList = useMemo(
    () =>
      [...messages].sort(
        (a, b) =>
          new Date(a.created_at).getTime() - new Date(b.created_at).getTime()
      ),
    [messages]
  );
  const lastMessageId =
    messageList.length > 0 ? messageList[messageList.length - 1].id : null;

  const messageById = useMemo(
    () => new Map(messageList.map((message) => [message.id, message])),
    [messageList]
  );

  const totalTokens = useMemo(() => {
    let sum = 0;
    for (const message of messageList) {
      const meta = message.meta;
      if (
        meta &&
        typeof meta === 'object' &&
        !Array.isArray(meta) &&
        'token_usage' in meta
      ) {
        const tokenUsage = (meta as { token_usage?: { total_tokens?: number } })
          .token_usage;
        if (typeof tokenUsage?.total_tokens === 'number') {
          sum += tokenUsage.total_tokens;
        }
      }
    }
    return sum;
  }, [messageList]);

  const runHistory = useRunHistory(messages);

  const activeSession = useMemo(
    () => sortedSessions.find((session) => session.id === activeSessionId),
    [sortedSessions, activeSessionId]
  );

  const memberPresetById = useMemo(() => {
    const map = new Map<string, ChatMemberPreset>();
    for (const preset of config?.chat_presets?.members ?? []) {
      map.set(preset.id, preset);
    }
    return map;
  }, [config?.chat_presets?.members]);

  const agentIdByName = useMemo(() => {
    const map = new Map<string, string>();
    sessionMembers.forEach((member) => {
      map.set(member.agent.name, member.agent.id);
    });
    return map;
  }, [sessionMembers]);

  const isArchived = activeSession?.status === ChatSessionStatus.archived;
  const activeSessionTitle = activeSession?.title ?? '';
  const streamingRunCount = useMemo(
    () => Object.keys(streamingRuns).length,
    [streamingRuns]
  );

  const placeholderAgents = useMemo(
    () =>
      sessionMembers.filter((member) => {
        const state = agentStates[member.agent.id] ?? member.sessionAgent.state;
        return state === ChatSessionAgentState.running;
      }),
    [agentStates, sessionMembers]
  );

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

  const activeWorkspaceRuns = useMemo<RunHistoryItem[]>(
    () =>
      runHistory
        .filter((run: RunHistoryItem) => run.agentId === workspaceAgentId)
        .sort(
          (a: RunHistoryItem, b: RunHistoryItem) =>
            new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime()
        ),
    [runHistory, workspaceAgentId]
  );

  const canSend =
    !!activeSessionId &&
    !isArchived &&
    (draft.trim().length > 0 ||
      selectedMentions.length > 0 ||
      attachedFiles.length > 0) &&
    !sendMessage.isPending &&
    !isUploadingAttachments;

  const diffViewerRun = diffViewerRunId ? runDiffs[diffViewerRunId] : null;

  // Check agent availability
  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      setIsCheckingAvailability(true);
      const knownAgents = new Set(Object.values(BaseCodingAgent));
      const results = await Promise.all(
        availableRunnerTypes.map(async (runner) => {
          if (!knownAgents.has(runner as BaseCodingAgent)) {
            return [runner, null] as const;
          }
          try {
            const info = await configApi.checkAgentAvailability(
              runner as BaseCodingAgent
            );
            return [runner, info] as const;
          } catch (error) {
            console.warn('Failed to check agent availability', error);
            return [runner, null] as const;
          }
        })
      );
      if (cancelled) return;
      const next: Record<string, AvailabilityInfo | null> = {};
      results.forEach(([runner, info]) => {
        next[runner] = info;
      });
      setAgentAvailability(next);
      setIsCheckingAvailability(false);
    };

    if (availableRunnerTypes.length > 0) {
      run();
    } else {
      setAgentAvailability({});
      setIsCheckingAvailability(false);
    }

    return () => {
      cancelled = true;
    };
  }, [availableRunnerTypes]);

  // Set default runner type
  useEffect(() => {
    if (editingMember) return;
    if (enabledRunnerTypes.length === 0) {
      setNewMemberRunnerType('');
      return;
    }
    if (!newMemberRunnerType || !isRunnerAvailable(newMemberRunnerType)) {
      setNewMemberRunnerType(enabledRunnerTypes[0]);
    }
  }, [
    editingMember,
    enabledRunnerTypes,
    isRunnerAvailable,
    newMemberRunnerType,
  ]);

  // Set default variant when runner type changes
  useEffect(() => {
    if (memberVariantOptions.length === 0) {
      if (newMemberVariant !== 'DEFAULT') {
        setNewMemberVariant('DEFAULT');
      }
      return;
    }

    if (memberVariantOptions.includes(newMemberVariant)) return;
    const defaultVariant = memberVariantOptions.includes('DEFAULT')
      ? 'DEFAULT'
      : memberVariantOptions[0];
    if (defaultVariant) {
      setNewMemberVariant(defaultVariant);
    }
  }, [memberVariantOptions, newMemberVariant]);

  // Sync agent states from session agents
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
  }, [agents, setAgentStates]);

  useEffect(() => {
    setAgentStates((prev) => {
      const next = { ...prev };
      for (const sessionAgent of sessionAgents) {
        next[sessionAgent.agent_id] = sessionAgent.state;
      }
      return next;
    });
    setAgentStateInfos((prev) => {
      const next = { ...prev };
      for (const sessionAgent of sessionAgents) {
        const existing = next[sessionAgent.agent_id];
        const shouldSetStartedAt =
          sessionAgent.state === ChatSessionAgentState.running &&
          !existing?.startedAt;
        const shouldUpdateState =
          !existing || existing.state !== sessionAgent.state;
        if (shouldUpdateState || shouldSetStartedAt) {
          next[sessionAgent.agent_id] = {
            state: sessionAgent.state,
            startedAt:
              sessionAgent.state === ChatSessionAgentState.running
                ? (existing?.startedAt ?? sessionAgent.updated_at)
                : null,
          };
        }
      }
      return next;
    });
  }, [sessionAgents, setAgentStates, setAgentStateInfos]);

  // Running timer
  useEffect(() => {
    if (placeholderAgents.length === 0) return;
    const timer = setInterval(() => setClock(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [placeholderAgents]);

  // Title editing
  useEffect(() => {
    setTitleDraft(activeSessionTitle);
    setIsEditingTitle(false);
    setTitleError(null);
  }, [activeSession?.id, activeSessionTitle]);

  useEffect(() => {
    if (activeSession?.status === ChatSessionStatus.archived) {
      setShowArchived(true);
    }
  }, [activeSession?.status]);

  // Auto-scroll
  useEffect(() => {
    const isSessionChanged = previousSessionIdRef.current !== activeSessionId;
    previousSessionIdRef.current = activeSessionId;
    const animationFrame = requestAnimationFrame(() => {
      bottomRef.current?.scrollIntoView({
        behavior: isSessionChanged ? 'auto' : 'smooth',
        block: 'end',
      });
    });
    return () => cancelAnimationFrame(animationFrame);
  }, [
    activeSessionId,
    lastMessageId,
    streamingRunCount,
    placeholderAgents.length,
  ]);

  useEffect(() => {
    setLogRunId(null);
    setLogContent('');
    setLogError(null);
  }, [workspaceAgentId]);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setDebouncedMessageSearchQuery(messageSearchQuery);
    }, MESSAGE_SEARCH_DEBOUNCE_MS);

    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [messageSearchQuery]);

  // Handlers
  const getMessageSenderLabel = useCallback(
    (message: ChatMessage) => {
      if (message.sender_type === ChatSenderType.user) return 'You';
      if (message.sender_type === ChatSenderType.agent) {
        if (message.sender_id) {
          return agentById.get(message.sender_id)?.name ?? 'Agent';
        }
        return 'Agent';
      }
      return 'System';
    },
    [agentById]
  );

  const trimmedMessageSearchQuery = isMessageSearchOpen
    ? debouncedMessageSearchQuery.trim()
    : '';

  const escapedMessageSearchQuery = useMemo(
    () =>
      trimmedMessageSearchQuery.length > 0
        ? escapeSearchRegExp(trimmedMessageSearchQuery)
        : '',
    [trimmedMessageSearchQuery]
  );

  const messageSearchRegExp = useMemo(() => {
    if (!escapedMessageSearchQuery) return null;
    return new RegExp(escapedMessageSearchQuery, 'iu');
  }, [escapedMessageSearchQuery]);

  const messageSearchHighlightRegExp = useMemo(() => {
    if (!escapedMessageSearchQuery) return null;
    return new RegExp(escapedMessageSearchQuery, 'giu');
  }, [escapedMessageSearchQuery]);

  const filteredMessageList = useMemo(() => {
    if (!messageSearchRegExp) return messageList;

    return messageList.filter((message) => {
      if (messageSearchRegExp.test(message.content)) {
        return true;
      }

      if (messageSearchRegExp.test(getMessageSenderLabel(message))) {
        return true;
      }

      const attachments = extractAttachments(message.meta);
      return attachments.some((attachment) =>
        messageSearchRegExp.test(attachment.name ?? '')
      );
    });
  }, [getMessageSenderLabel, messageList, messageSearchRegExp]);

  const handleCloseMessageSearch = useCallback(() => {
    setIsMessageSearchOpen(false);
  }, []);

  useEffect(() => {
    const handleGlobalSearchShortcut = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) return;
      if (event.key.toLowerCase() !== 'f') return;
      if (!activeSession) return;

      event.preventDefault();
      setIsMessageSearchOpen(true);
    };

    document.addEventListener('keydown', handleGlobalSearchShortcut);
    return () => {
      document.removeEventListener('keydown', handleGlobalSearchShortcut);
    };
  }, [activeSession]);

  useEffect(() => {
    if (
      typeof window === 'undefined' ||
      typeof document === 'undefined' ||
      typeof CSS === 'undefined'
    ) {
      return;
    }

    const cssHighlights = (
      CSS as unknown as { highlights?: CSSHighlightRegistry }
    ).highlights;
    const HighlightCtor = (
      window as unknown as {
        Highlight?: new (...ranges: Range[]) => unknown;
      }
    ).Highlight;

    if (!cssHighlights || typeof HighlightCtor !== 'function') {
      return;
    }

    cssHighlights.delete(MESSAGE_SEARCH_HIGHLIGHT_NAME);

    if (!messageSearchHighlightRegExp) {
      return;
    }

    const container = messagesContainerRef.current;
    if (!container) {
      return;
    }

    const roots = container.querySelectorAll<HTMLElement>(
      '.chat-session-message-body, .chat-session-message-row.is-system'
    );
    const ranges: Range[] = [];

    roots.forEach((root) => {
      if (ranges.length >= MAX_MESSAGE_SEARCH_HIGHLIGHT_RANGES) {
        return;
      }

      const rangeRegExp = new RegExp(
        messageSearchHighlightRegExp.source,
        messageSearchHighlightRegExp.flags
      );
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
        acceptNode: (node) => {
          const text = node.textContent;
          const parent = node.parentElement;
          if (!parent || !text || text.trim().length === 0) {
            return NodeFilter.FILTER_REJECT;
          }
          if (parent.closest('button, a')) {
            return NodeFilter.FILTER_REJECT;
          }
          return NodeFilter.FILTER_ACCEPT;
        },
      });

      let current = walker.nextNode();
      while (current && ranges.length < MAX_MESSAGE_SEARCH_HIGHLIGHT_RANGES) {
        const textNode = current as Text;
        rangeRegExp.lastIndex = 0;
        let match = rangeRegExp.exec(textNode.data);

        while (match && ranges.length < MAX_MESSAGE_SEARCH_HIGHLIGHT_RANGES) {
          const matchedText = match[0];
          if (matchedText.length === 0) {
            rangeRegExp.lastIndex += 1;
            match = rangeRegExp.exec(textNode.data);
            continue;
          }

          const range = document.createRange();
          range.setStart(textNode, match.index);
          range.setEnd(textNode, match.index + matchedText.length);
          ranges.push(range);

          match = rangeRegExp.exec(textNode.data);
        }

        current = walker.nextNode();
      }
    });

    if (ranges.length > 0) {
      cssHighlights.set(
        MESSAGE_SEARCH_HIGHLIGHT_NAME,
        new HighlightCtor(...ranges)
      );
    }

    return () => {
      cssHighlights.delete(MESSAGE_SEARCH_HIGHLIGHT_NAME);
    };
  }, [filteredMessageList, messageSearchHighlightRegExp]);

  const getReferencePreview = useCallback((message: ChatMessage) => {
    const attachments = extractAttachments(message.meta);
    const trimmed = message.content.trim();
    if (trimmed) return truncateText(trimmed, 140);
    if (attachments.length > 0) {
      const names = attachments
        .map((item) => item.name)
        .filter(Boolean)
        .slice(0, 3);
      const suffix =
        attachments.length > 3 ? ` and ${attachments.length - 3} more` : '';
      return `Attachment: ${names.join(', ')}${suffix}`;
    }
    return 'Referenced message';
  }, []);

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

    if (!content && attachedFiles.length === 0) return;

    const allMentions = new Set([...contentMentions, ...selectedMentions]);
    const runningMentionedAgents: string[] = [];
    allMentions.forEach((name) => {
      const agentId = agentIdByName.get(name);
      if (agentId && agentStates[agentId] === ChatSessionAgentState.running) {
        runningMentionedAgents.push(name);
      }
    });

    if (runningMentionedAgents.length > 0) {
      setConfirmModal({
        title: t('modals.confirm.titles.agentRunning'),
        message: t('modals.confirm.messages.agentRunning', {
          agents: runningMentionedAgents.join(', @'),
        }),
        onConfirm: async () => {
          await doSendMessage(content);
        },
      });
      return;
    }

    await doSendMessage(content);
  };

  const doSendMessage = async (content: string) => {
    if (!activeSessionId) return;
    const meta: JsonValue = {
      sender_handle: senderHandle,
      ...(replyToMessage
        ? { reference: { message_id: replyToMessage.id } }
        : {}),
    };

    try {
      if (attachedFiles.length > 0) {
        await handleAttachmentUpload(attachedFiles, {
          content: content || undefined,
          referenceMessageId: replyToMessage?.id,
        });
      } else {
        await sendMessage.mutateAsync({
          sessionId: activeSessionId,
          content,
          meta,
        });
      }

      resetInput();
      inputRef.current?.focus();
      setAttachedFiles([]);
    } catch (error) {
      console.warn('Failed to send chat message', error);
    }
  };

  const handleAttachmentUpload = async (
    files: FileList | File[],
    options?: { content?: string; referenceMessageId?: string }
  ) => {
    if (!activeSessionId || isArchived) return;
    const list = Array.from(files);
    if (list.length === 0) return;

    const allowedFiles = list.filter((file) => isAllowedAttachment(file));

    if (allowedFiles.length === 0) {
      setAttachmentError('Only text files and images are allowed.');
      return;
    }

    setIsUploadingAttachments(true);
    setAttachmentError(null);
    try {
      const message = await chatApi.uploadChatAttachments(
        activeSessionId,
        allowedFiles,
        {
          senderHandle,
          content: options?.content,
          referenceMessageId: options?.referenceMessageId,
        }
      );
      upsertMessage(message);
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
      setAttachedFiles([]);
    } catch (error) {
      console.warn('Failed to upload attachments', error);
      setAttachmentError('Unable to upload attachments.');
    } finally {
      setIsUploadingAttachments(false);
    }
  };

  const handleAttachmentInputChange = (
    event: ChangeEvent<HTMLInputElement>
  ) => {
    if (event.target.files) {
      const files = Array.from(event.target.files);
      const allowedFiles = files.filter((file) => isAllowedAttachment(file));

      if (allowedFiles.length !== files.length) {
        const rejectedCount = files.length - allowedFiles.length;
        setAttachmentError(
          `Some files were rejected (${rejectedCount}). Only text files and images are allowed.`
        );
      }

      setAttachedFiles((prev) => [...prev, ...allowedFiles]);
    }
    event.target.value = '';
  };

  const removeAttachedFile = (fileName: string, fileSize: number) => {
    setAttachedFiles((prev) =>
      prev.filter((file) => !(file.name === fileName && file.size === fileSize))
    );
  };

  const clearAttachedFiles = () => {
    setAttachedFiles([]);
  };

  const addAttachmentAsFile = async (
    messageId: string,
    attachment: { id: string; name: string; mime_type?: string | null }
  ) => {
    if (!activeSessionId || !attachment.id) return;

    try {
      const attachmentUrl = chatApi.getChatAttachmentUrl(
        activeSessionId,
        messageId,
        attachment.id
      );

      const response = await fetch(attachmentUrl);
      if (!response.ok) {
        throw new Error(
          `Failed to download attachment: ${response.statusText}`
        );
      }

      const blob = await response.blob();
      const file = new File([blob], attachment.name, { type: blob.type });
      setAttachedFiles((prev) => [...prev, file]);
    } catch (error) {
      console.error('Error downloading attachment:', error);
      setAttachmentError('Could not download attachment.');
    }
  };

  const previewAttachedFile = async (file: File) => {
    try {
      if (isTextAttachment(file)) {
        const content = await file.text();
        setPreviewFile({ file, content });
      } else if (isImageAttachment(file)) {
        const reader = new FileReader();
        reader.onload = (e) => {
          setPreviewFile({ file, content: e.target?.result as string });
        };
        reader.readAsDataURL(file);
      } else {
        setPreviewFile({ file, content: null });
      }
    } catch (error) {
      console.error('Error previewing file:', error);
      setAttachmentError('Could not preview file.');
    }
  };

  const closePreview = () => {
    setPreviewFile({ file: null, content: null });
  };

  const handlePromptFileChange = async (
    event: ChangeEvent<HTMLInputElement>
  ) => {
    const file = event.target.files?.[0];
    if (!file) return;
    setPromptFileLoading(true);
    setPromptFileError(null);
    try {
      const text = await file.text();
      setNewMemberPrompt(text);
    } catch (error) {
      console.warn('Failed to read prompt file', error);
      setPromptFileError('Unable to read the file.');
    } finally {
      setPromptFileLoading(false);
      event.target.value = '';
    }
  };

  const buildTeamImportPlan = useCallback(
    (teamPreset: ChatTeamPreset): MemberPresetImportPlan[] => {
      if (!activeSessionId) return [];

      const takenNamesLowercase = new Set(
        sessionMembers.map((member) => member.agent.name.toLowerCase())
      );
      const plans: MemberPresetImportPlan[] = [];

      for (const memberPresetId of teamPreset.member_ids) {
        const preset = memberPresetById.get(memberPresetId);
        if (!preset) {
          plans.push({
            presetId: memberPresetId,
            presetName: memberPresetId,
            runnerType: '',
            finalName: memberPresetId,
            systemPrompt: '',
            toolsEnabled: {},
            action: 'skip',
            reason: 'member-preset-missing',
            agentId: null,
            workspacePath: '',
          });
          continue;
        }

        if (!preset.enabled) {
          plans.push({
            presetId: preset.id,
            presetName: preset.name,
            runnerType: '',
            finalName: preset.name,
            systemPrompt: '',
            toolsEnabled: {},
            action: 'skip',
            reason: 'member-preset-disabled',
            agentId: null,
            workspacePath: '',
          });
          continue;
        }

        const plan = buildMemberPresetImportPlan({
          preset,
          sessionId: activeSessionId,
          sessionMembers,
          defaultRunnerType: config?.executor_profile?.executor ?? null,
          enabledRunnerTypes,
          availableRunnerTypes,
          takenNamesLowercase,
        });

        if (!plan) {
          plans.push({
            presetId: preset.id,
            presetName: preset.name,
            runnerType: '',
            finalName: preset.name,
            systemPrompt: '',
            toolsEnabled: {},
            action: 'skip',
            reason: 'runner-not-available',
            agentId: null,
            workspacePath: '',
          });
          continue;
        }

        plans.push(plan);
      }

      return plans;
    },
    [
      activeSessionId,
      availableRunnerTypes,
      config?.executor_profile?.executor,
      enabledRunnerTypes,
      memberPresetById,
      sessionMembers,
    ]
  );

  const importMembersFromPlan = useCallback(
    async (plan: MemberPresetImportPlan[]) => {
      if (!activeSessionId) return;

      const attachedAgentIds = new Set(
        sessionMembers.map((member) => member.agent.id)
      );

      for (const entry of plan) {
        if (entry.action === 'skip') continue;

        let agentId = entry.agentId;
        if (entry.action === 'create') {
          const created = await chatApi.createAgent({
            name: entry.finalName,
            runner_type: entry.runnerType,
            system_prompt: entry.systemPrompt,
            tools_enabled: entry.toolsEnabled as JsonValue,
          });
          agentId = created.id;
        }

        if (!agentId || attachedAgentIds.has(agentId)) {
          continue;
        }

        await chatApi.createSessionAgent(activeSessionId, {
          agent_id: agentId,
          workspace_path: entry.workspacePath,
        });
        attachedAgentIds.add(agentId);
      }

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['chatAgents'] }),
        queryClient.invalidateQueries({
          queryKey: ['chatSessionAgents', activeSessionId],
        }),
      ]);
    },
    [activeSessionId, queryClient, sessionMembers]
  );

  const validateAndPrepareImportPlan = useCallback(
    (plan: MemberPresetImportPlan[]): MemberPresetImportPlan[] | null => {
      const existingSessionMemberNamesLower = new Set(
        sessionMembers.map((member) => member.agent.name.toLowerCase())
      );
      const enabledRunnerTypesSet = new Set(enabledRunnerTypes);
      const createNamesLower = new Set<string>();
      const projectNameLower = activeSessionTitle.trim().toLowerCase();
      const preparedPlan: MemberPresetImportPlan[] = [];

      for (const entry of plan) {
        if (entry.action === 'skip') {
          preparedPlan.push(entry);
          continue;
        }

        const finalName = entry.finalName.trim();
        const workspacePath = entry.workspacePath.trim();
        const runnerType = entry.runnerType.trim();

        if (!runnerType) {
          setMemberError('Base coding agent is required.');
          return null;
        }

        if (!enabledRunnerTypesSet.has(runnerType)) {
          setMemberError('Selected coding agent is unavailable.');
          return null;
        }

        if (!finalName) {
          setMemberError('AI member name is required.');
          return null;
        }

        if (getMemberNameLength(finalName) > MAX_MEMBER_NAME_LENGTH) {
          setMemberError(
            `AI member name cannot exceed ${MAX_MEMBER_NAME_LENGTH} characters.`
          );
          return null;
        }

        if (!memberNameRegex.test(finalName)) {
          setMemberError('Name can only include letters, numbers, "_" or "-".');
          return null;
        }

        const workspacePathError = validateWorkspacePath(workspacePath);
        if (workspacePathError) {
          setMemberError(workspacePathError);
          return null;
        }

        const nextEntry: MemberPresetImportPlan = {
          ...entry,
          finalName,
          workspacePath,
          runnerType,
        };

        if (nextEntry.action === 'create') {
          const finalNameLower = finalName.toLowerCase();
          if (
            projectNameLower.length > 0 &&
            finalNameLower === projectNameLower
          ) {
            setMemberError('AI member name cannot match the project name.');
            return null;
          }
          if (existingSessionMemberNamesLower.has(finalNameLower)) {
            showDuplicateMemberNameWarning(finalName);
            return null;
          }
          if (createNamesLower.has(finalNameLower)) {
            setMemberError('Duplicate AI member names in import plan.');
            return null;
          }
          createNamesLower.add(finalNameLower);
        }

        preparedPlan.push(nextEntry);
      }

      return preparedPlan;
    },
    [
      activeSessionTitle,
      enabledRunnerTypes,
      sessionMembers,
      showDuplicateMemberNameWarning,
    ]
  );

  const handleAddMemberPreset = useCallback(
    (preset: ChatMemberPreset) => {
      if (!activeSessionId) {
        setMemberError('Select a chat session first.');
        return;
      }
      if (isArchived) {
        setMemberError('This session is archived and read-only.');
        return;
      }

      const takenNamesLowercase = new Set(
        sessionMembers.map((member) => member.agent.name.toLowerCase())
      );
      const plan = buildMemberPresetImportPlan({
        preset,
        sessionId: activeSessionId,
        sessionMembers,
        defaultRunnerType: config?.executor_profile?.executor ?? null,
        enabledRunnerTypes,
        availableRunnerTypes,
        takenNamesLowercase,
      });

      if (!plan) {
        setMemberError('No available coding agent to import this preset.');
        return;
      }

      if (plan.action === 'skip') {
        showDuplicateMemberNameWarning(plan.finalName);
        return;
      }

      setTeamImportName(getLocalizedMemberPresetName(preset, t));
      setTeamImportPlan([plan]);
      setMemberError(null);
    },
    [
      activeSessionId,
      availableRunnerTypes,
      config?.executor_profile?.executor,
      enabledRunnerTypes,
      isArchived,
      sessionMembers,
      showDuplicateMemberNameWarning,
      t,
    ]
  );

  const handleImportTeamPreset = useCallback(
    (teamPreset: ChatTeamPreset) => {
      if (!activeSessionId) {
        setMemberError('Select a chat session first.');
        return;
      }
      if (isArchived) {
        setMemberError('This session is archived and read-only.');
        return;
      }

      const plan = buildTeamImportPlan(teamPreset);
      if (plan.length === 0) {
        setMemberError('Selected team has no member presets.');
        return;
      }

      const duplicateEntry = plan.find(
        (entry) => entry.reason === 'duplicate-name-in-session'
      );
      if (duplicateEntry) {
        showDuplicateMemberNameWarning(duplicateEntry.finalName);
        return;
      }

      setTeamImportName(getLocalizedTeamPresetName(teamPreset, t));
      setTeamImportPlan(plan);
      setMemberError(null);
    },
    [
      activeSessionId,
      buildTeamImportPlan,
      isArchived,
      showDuplicateMemberNameWarning,
      t,
    ]
  );

  const handleUpdateTeamImportPlanEntry = useCallback(
    (
      index: number,
      updates: {
        finalName?: string;
        workspacePath?: string;
        runnerType?: string;
        systemPrompt?: string;
        toolsEnabled?: JsonValue;
      }
    ) => {
      setTeamImportPlan((prev) => {
        if (!prev || index < 0 || index >= prev.length) return prev;
        const next = [...prev];
        const patch: Partial<MemberPresetImportPlan> = {};
        if (updates.finalName !== undefined) patch.finalName = updates.finalName;
        if (updates.workspacePath !== undefined) patch.workspacePath = updates.workspacePath;
        if (updates.runnerType !== undefined) patch.runnerType = updates.runnerType;
        if (updates.systemPrompt !== undefined) patch.systemPrompt = updates.systemPrompt;
        if (updates.toolsEnabled !== undefined) patch.toolsEnabled = updates.toolsEnabled;
        next[index] = { ...next[index], ...patch };
        return next;
      });
    },
    []
  );

  const handleConfirmTeamImport = useCallback(async () => {
    if (!teamImportPlan || teamImportPlan.length === 0) return;

    const preparedPlan = validateAndPrepareImportPlan(teamImportPlan);
    if (!preparedPlan) return;

    const actionablePlan = preparedPlan.filter(
      (entry) => entry.action !== 'skip'
    );
    if (actionablePlan.length === 0) {
      setMemberError('Nothing to import from this team preset.');
      setTeamImportPlan(null);
      setTeamImportName(null);
      return;
    }

    setIsImportingTeam(true);
    setMemberError(null);
    setTeamImportPlan(preparedPlan);
    try {
      await importMembersFromPlan(preparedPlan);
      setTeamImportPlan(null);
      setTeamImportName(null);
    } catch (error) {
      console.error('Failed to import team preset', error);
      setMemberError('Failed to import team preset.');
    } finally {
      setIsImportingTeam(false);
    }
  }, [importMembersFromPlan, teamImportPlan, validateAndPrepareImportPlan]);

  const handleCancelTeamImport = useCallback(() => {
    if (isImportingTeam) return;
    setTeamImportPlan(null);
    setTeamImportName(null);
  }, [isImportingTeam]);

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
    const workspacePathVal = newMemberWorkspace.trim();
    const selectedVariant = newMemberVariant.trim() || 'DEFAULT';

    if (!name) {
      setMemberError('AI member name is required.');
      return;
    }

    if (getMemberNameLength(name) > MAX_MEMBER_NAME_LENGTH) {
      setMemberError(
        `AI member name cannot exceed ${MAX_MEMBER_NAME_LENGTH} characters.`
      );
      return;
    }

    if (!memberNameRegex.test(name)) {
      setMemberError('Name can only include letters, numbers, "_" or "-".');
      return;
    }

    const projectName = activeSessionTitle.trim();
    const isNameChange =
      !editingMember ||
      editingMember.agent.name.trim().toLowerCase() !== name.toLowerCase();
    if (
      projectName.length > 0 &&
      isNameChange &&
      projectName.toLowerCase() === name.toLowerCase()
    ) {
      setMemberError('AI member name cannot match the project name.');
      return;
    }

    if (!runnerType) {
      setMemberError('Choose a base coding agent.');
      return;
    }

    if (!isRunnerAvailable(runnerType)) {
      setMemberError('Selected coding agent is not available locally.');
      return;
    }

    const workspacePathError = validateWorkspacePath(workspacePathVal);
    if (workspacePathError) {
      setMemberError(workspacePathError);
      return;
    }

    if (!prompt) {
      setMemberError(t('members.roleSettingsRequired'));
      return;
    }

    let nameChanged = false;
    let runnerChanged = false;
    let promptChanged = false;
    let variantChanged = false;
    let workspaceChanged = false;
    if (editingMember) {
      nameChanged = editingMember.agent.name !== name;
      runnerChanged = editingMember.agent.runner_type !== runnerType;
      promptChanged = (editingMember.agent.system_prompt ?? '') !== prompt;
      const existingVariant =
        extractExecutorProfileVariant(editingMember.agent.tools_enabled) ??
        'DEFAULT';
      variantChanged = existingVariant !== selectedVariant;
      workspaceChanged =
        (editingMember.sessionAgent.workspace_path ?? '') !== workspacePathVal;

      if (nameChanged) {
        const conflict = sessionMembers.find(
          (member) =>
            member.sessionAgent.id !== editingMember.sessionAgent.id &&
            member.agent.name.toLowerCase() === name.toLowerCase()
        );
        if (conflict) {
          showDuplicateMemberNameWarning(name);
          return;
        }
      }

      if (
        !nameChanged &&
        !runnerChanged &&
        !promptChanged &&
        !variantChanged &&
        !workspaceChanged
      ) {
        setEditingMember(null);
        setIsAddMemberOpen(false);
        setMemberError(null);
        return;
      }
    }

    setIsSavingMember(true);
    setMemberError(null);

    try {
      if (editingMember) {
        const agentId = editingMember.agent.id;
        const toolsEnabledPayload = withExecutorProfileVariant(
          editingMember.agent.tools_enabled,
          selectedVariant
        );
        const updatePayload = {
          name: nameChanged ? name : null,
          runner_type: runnerChanged ? runnerType : null,
          system_prompt: promptChanged ? prompt : null,
          tools_enabled: variantChanged ? toolsEnabledPayload : null,
        };

        if (
          updatePayload.name ||
          updatePayload.runner_type ||
          updatePayload.system_prompt ||
          updatePayload.tools_enabled
        ) {
          await chatApi.updateAgent(agentId, updatePayload);
        }

        if (workspaceChanged) {
          const sessionIdForUpdate =
            editingMember.sessionAgent.session_id ?? activeSessionId;
          if (!sessionIdForUpdate) {
            throw new ApiError('Missing session context for AI member update.');
          }
          await chatApi.updateSessionAgent(
            sessionIdForUpdate,
            editingMember.sessionAgent.id,
            { workspace_path: workspacePathVal }
          );
        }
      } else {
        const conflict = sessionMembers.find(
          (member) => member.agent.name.toLowerCase() === name.toLowerCase()
        );
        if (conflict) {
          showDuplicateMemberNameWarning(name);
          return;
        }

        const created = await chatApi.createAgent({
          name,
          runner_type: runnerType,
          system_prompt: prompt,
          tools_enabled: withExecutorProfileVariant({}, selectedVariant),
        });
        const agentId = created.id;

        if (!agentId) {
          setMemberError('Unable to create AI member.');
          return;
        }

        await chatApi.createSessionAgent(activeSessionId, {
          agent_id: agentId,
          workspace_path: workspacePathVal,
        });
      }

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['chatAgents'] }),
        queryClient.invalidateQueries({
          queryKey: ['chatSessionAgents', activeSessionId],
        }),
      ]);

      setNewMemberName('');
      setNewMemberVariant('DEFAULT');
      setNewMemberPrompt('');
      setNewMemberWorkspace('');
      setMemberError(null);
      setEditingMember(null);
      setIsAddMemberOpen(false);
    } catch (error) {
      console.warn('Failed to add AI member', error);
      if (error instanceof ApiError && error.message) {
        if (
          error.message.includes('already exists in this session') ||
          error.message.includes('An AI member with this name already exists.')
        ) {
          showDuplicateMemberNameWarning(name);
          return;
        }
        setMemberError(error.message);
      } else if (error instanceof Error && error.message) {
        setMemberError(error.message);
      } else {
        setMemberError(
          editingMember
            ? 'Failed to update AI member. Check server logs.'
            : 'Failed to add AI member. Check server logs.'
        );
      }
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
    setNewMemberVariant(
      extractExecutorProfileVariant(member.agent.tools_enabled) ?? 'DEFAULT'
    );
    setNewMemberPrompt(member.agent.system_prompt ?? '');
    setNewMemberWorkspace(member.sessionAgent.workspace_path ?? '');
    setMemberError(null);
    setIsPromptEditorOpen(false);
    setPromptFileError(null);
    setPromptFileLoading(false);
    setIsAddMemberOpen(true);
  };

  const handleRemoveMember = async (member: SessionMember) => {
    if (!activeSessionId) return;
    if (isArchived) {
      setMemberError('This session is archived and read-only.');
      return;
    }
    const sessionAgentId = member.sessionAgent.id;
    setConfirmModal({
      title: t('modals.confirm.titles.removeMember'),
      message: t('modals.confirm.messages.removeMember', {
        name: member.agent.name,
      }),
      onConfirm: async () => {
        try {
          await chatApi.deleteSessionAgent(activeSessionId, sessionAgentId);
          await queryClient.invalidateQueries({
            queryKey: ['chatSessionAgents', activeSessionId],
          });
          await queryClient.refetchQueries({
            queryKey: ['chatSessionAgents', activeSessionId],
          });
          if (workspaceAgentId === member.agent.id) {
            setWorkspaceDrawerOpen(false);
            setWorkspaceAgentId(null);
          }
        } catch (error) {
          console.error('Failed to remove AI member:', error);
          setMemberError('Failed to remove AI member.');
        }
      },
    });
  };

  // 鈹€鈹€ Preset import handlers 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€

  const handleSaveTitle = async () => {
    if (!activeSessionId) return;
    const trimmed = titleDraft.trim();
    if (getSessionTitleLength(trimmed) > MAX_SESSION_TITLE_LENGTH) {
      setTitleError(
        `Session name cannot exceed ${MAX_SESSION_TITLE_LENGTH} characters.`
      );
      return;
    }
    try {
      await updateSession.mutateAsync({
        sessionId: activeSessionId,
        title: trimmed.length > 0 ? trimmed : null,
      });
      setIsEditingTitle(false);
      setTitleError(null);
    } catch (error) {
      console.warn('Failed to update session title', error);
      setTitleError('Unable to update session name.');
    }
  };

  const handleCancelTitleEdit = () => {
    setTitleDraft(activeSession?.title ?? '');
    setIsEditingTitle(false);
    setTitleError(null);
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

  const handleLocalReplySelect = useCallback(
    (message: ChatMessage) => {
      const handle = getMessageMentionHandle(message);
      handleReplySelect(message, handle);
    },
    [getMessageMentionHandle, handleReplySelect]
  );

  const handleStopAgent = useCallback(
    async (sessionAgentId: string, agentId: string) => {
      if (!activeSessionId) return;
      setStoppingAgents((prev) => new Set(prev).add(agentId));
      try {
        await chatApi.stopSessionAgent(activeSessionId, sessionAgentId);
      } catch (error) {
        console.warn('Failed to stop agent', error);
      } finally {
        setStoppingAgents((prev) => {
          const next = new Set(prev);
          next.delete(agentId);
          return next;
        });
      }
    },
    [activeSessionId]
  );

  // Resize handlers
  const handleResizeStart = useCallback(
    (type: 'left' | 'right' | 'input', e: React.MouseEvent) => {
      e.preventDefault();
      if (type === 'left' && isLeftSidebarCollapsed) {
        return;
      }
      setIsResizing(type);
      resizeStartRef.current = {
        startX: e.clientX,
        startY: e.clientY,
        startWidth: type === 'left' ? leftSidebarWidth : rightSidebarWidth,
        startHeight: inputAreaHeight,
      };
    },
    [
      inputAreaHeight,
      isLeftSidebarCollapsed,
      leftSidebarWidth,
      rightSidebarWidth,
    ]
  );

  const handleToggleLeftSidebar = useCallback(() => {
    setIsLeftSidebarCollapsed((prev) => {
      if (!prev) {
        lastExpandedLeftWidthRef.current = leftSidebarWidth;
        setLeftSidebarWidth(COLLAPSED_LEFT_SIDEBAR_WIDTH);
        return true;
      }
      setLeftSidebarWidth(Math.max(220, lastExpandedLeftWidthRef.current));
      return false;
    });
  }, [leftSidebarWidth]);

  useEffect(() => {
    if (!isResizing) return;

    const handleMouseMove = (e: MouseEvent) => {
      if (!resizeStartRef.current) return;
      const { startX, startY, startWidth, startHeight } =
        resizeStartRef.current;

      if (isResizing === 'left') {
        const delta = e.clientX - startX;
        const newWidth = Math.max(200, Math.min(500, startWidth + delta));
        setLeftSidebarWidth(newWidth);
      } else if (isResizing === 'right') {
        const delta = startX - e.clientX;
        const newWidth = Math.max(240, Math.min(600, startWidth + delta));
        setRightSidebarWidth(newWidth);
      } else if (isResizing === 'input') {
        const delta = startY - e.clientY;
        const newHeight = Math.max(120, Math.min(500, startHeight + delta));
        setInputAreaHeight(newHeight);
      }
    };

    const handleMouseUp = () => {
      setIsResizing(null);
      resizeStartRef.current = null;
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isResizing]);

  return (
    <div className="chat-session-page relative flex h-full min-h-0 overflow-hidden select-none">
      {/* Session List Sidebar */}
      <SessionListSidebar
        activeSessions={activeSessions}
        archivedSessions={archivedSessions}
        activeSessionId={activeSessionId}
        showArchived={showArchived}
        onToggleArchived={() => setShowArchived((prev) => !prev)}
        onSelectSession={(id) => navigate(`/chat/${id}`)}
        onCreateSession={() => createSession.mutate()}
        isCreating={createSession.isPending}
        width={leftSidebarWidth}
        isCollapsed={isLeftSidebarCollapsed}
        onToggleCollapsed={handleToggleLeftSidebar}
      />

      {/* Left Sidebar Resize Handle */}
      {!isLeftSidebarCollapsed && (
        <div
          className="chat-session-resize-handle w-1 cursor-col-resize transition-colors shrink-0"
          onMouseDown={(e) => handleResizeStart('left', e)}
        />
      )}

      {/* Main Chat Section */}
      <section className="chat-session-main flex-1 min-w-0 min-h-0 flex flex-col">
        <ChatHeader
          activeSession={activeSession ?? null}
          messageCount={messageList.length}
          totalTokens={totalTokens}
          memberCount={sessionMembers.length}
          isSearchOpen={isMessageSearchOpen}
          searchQuery={messageSearchQuery}
          onCloseSearch={handleCloseMessageSearch}
          onSearchQueryChange={setMessageSearchQuery}
          isArchived={isArchived}
          isEditingTitle={isEditingTitle}
          titleDraft={titleDraft}
          titleError={titleError}
          isSavingTitle={updateSession.isPending}
          onStartEditTitle={() => {
            setIsEditingTitle(true);
            setTitleError(null);
          }}
          onTitleDraftChange={(value) => {
            setTitleDraft(value);
            if (getSessionTitleLength(value) > MAX_SESSION_TITLE_LENGTH) {
              setTitleError(
                `Session name cannot exceed ${MAX_SESSION_TITLE_LENGTH} characters.`
              );
            } else {
              setTitleError(null);
            }
          }}
          onSaveTitle={handleSaveTitle}
          onCancelTitleEdit={handleCancelTitleEdit}
          onDeleteSession={() => {
            if (!activeSession) return;
            setConfirmModal({
              title: t('modals.confirm.titles.deleteSession'),
              message: t('modals.confirm.messages.deleteSession', {
                title: activeSession.title || t('sidebar.untitledSession'),
              }),
              onConfirm: async () => {
                await deleteSession.mutateAsync(activeSession.id);
              },
            });
          }}
          onOpenSettings={() => {
            SettingsDialog.show();
          }}
          onArchive={() => {
            if (activeSessionId) archiveSession.mutate(activeSessionId);
          }}
          onRestore={() => {
            if (activeSessionId) restoreSession.mutate(activeSessionId);
          }}
          isArchiving={archiveSession.isPending || restoreSession.isPending}
          isCleanupMode={isCleanupMode}
          onToggleCleanupMode={() => {
            if (isCleanupMode) {
              setIsCleanupMode(false);
              setSelectedMessageIds(new Set());
            } else {
              setIsCleanupMode(true);
            }
          }}
          isDeletingMessages={isDeletingMessages}
        />

        {/* Cleanup mode controls */}
        {activeSession && isCleanupMode && (
          <CleanupModeBar
            selectedCount={selectedMessageIds.size}
            totalCount={messageList.length}
            onToggleSelectAll={() => {
              if (selectedMessageIds.size === messageList.length) {
                setSelectedMessageIds(new Set());
              } else {
                setSelectedMessageIds(new Set(messageList.map((m) => m.id)));
              }
            }}
            onDeleteSelected={() => {
              if (!activeSessionId) return;
              const count = selectedMessageIds.size;
              setConfirmModal({
                title: t('modals.confirm.titles.deleteMessages'),
                message: t('modals.confirm.messages.deleteMessages', {
                  count,
                }),
                onConfirm: async () => {
                  setIsDeletingMessages(true);
                  try {
                    await deleteMessages.mutateAsync({
                      sessionId: activeSessionId,
                      messageIds: Array.from(selectedMessageIds),
                    });
                    setSelectedMessageIds(new Set());
                    setIsCleanupMode(false);
                  } finally {
                    setIsDeletingMessages(false);
                  }
                },
              });
            }}
            isDeletingMessages={isDeletingMessages}
          />
        )}

        {/* Messages */}
        <div
          ref={messagesContainerRef}
          className="chat-session-messages flex-1 min-h-0 overflow-y-auto p-base space-y-base"
        >
          {isLoading && <div className="text-sm text-low">Loading chat...</div>}
          {isArchived && !isLoading && (
            <div className="text-xs text-low border border-border rounded-sm bg-secondary/60 px-base py-half">
              This session is archived. Messages and members are read-only.
            </div>
          )}
          {compressionWarning && (
            <div className="chat-session-compression-warning text-xs border border-yellow-500/50 rounded-sm bg-yellow-500/10 px-base py-half flex items-center justify-between">
              <div className="flex items-center gap-half">
                <span className="text-yellow-600 dark:text-yellow-400">!</span>
                <span className="text-yellow-700 dark:text-yellow-300">
                  {compressionWarning.message}
                </span>
                <span className="text-yellow-600/80 dark:text-yellow-400/80 ml-1">
                  ({compressionWarning.split_file_path})
                </span>
              </div>
              <button
                type="button"
                className="text-yellow-600 dark:text-yellow-400 hover:text-yellow-800 dark:hover:text-yellow-200 text-xs"
                onClick={clearCompressionWarning}
              >
                x
              </button>
            </div>
          )}
          {!isLoading && messageList.length === 0 && (
            <div className="text-sm text-low">
              No messages yet. Start the conversation below.
            </div>
          )}
          {!isLoading &&
            messageList.length > 0 &&
            trimmedMessageSearchQuery &&
            filteredMessageList.length === 0 && (
              <div className="text-sm text-low">
                No messages match "{messageSearchQuery.trim()}".
              </div>
            )}

          {filteredMessageList.map((message) => {
            const isAgent = message.sender_type === ChatSenderType.agent;
            const agentName =
              isAgent && message.sender_id
                ? (agentById.get(message.sender_id)?.name ?? 'Agent')
                : null;
            const diffMeta = isAgent ? extractDiffMeta(message.meta) : null;
            const diffInfo = diffMeta && diffMeta.runId ? diffMeta : null;
            const attachments = extractAttachments(message.meta);
            const mentionList = Array.from(
              new Set(message.mentions.filter((mention) => mention.length > 0))
            );
            const mentionStatusMap = mentionStatuses.get(message.id);
            const referenceId = extractReferenceId(message.meta);
            const referenceMessage = referenceId
              ? messageById.get(referenceId)
              : null;
            const isUser = message.sender_type === ChatSenderType.user;
            const toneKey = isUser
              ? 'user'
              : (message.sender_id ?? agentName ?? 'agent');
            const tone = getMessageTone(String(toneKey), isUser);

            const isSelected = selectedMessageIds.has(message.id);

            return (
              <ChatMessageItem
                key={message.id}
                message={message}
                senderLabel={getMessageSenderLabel(message)}
                senderRunnerType={
                  isAgent && message.sender_id
                    ? (agentById.get(message.sender_id)?.runner_type ?? null)
                    : null
                }
                tone={tone}
                referenceMessage={referenceMessage ?? null}
                referenceSenderLabel={
                  referenceMessage
                    ? getMessageSenderLabel(referenceMessage)
                    : null
                }
                referencePreview={
                  referenceMessage
                    ? getReferencePreview(referenceMessage)
                    : null
                }
                mentionList={mentionList}
                mentionStatusMap={mentionStatusMap}
                agentStates={agentStates}
                agentIdByName={agentIdByName}
                attachments={attachments}
                activeSessionId={activeSessionId}
                onAddAttachmentAsFile={addAttachmentAsFile}
                diffInfo={diffInfo}
                runDiffs={runDiffs}
                onOpenDiffViewer={handleOpenDiffViewer}
                isArchived={isArchived}
                onReply={handleLocalReplySelect}
                isCleanupMode={isCleanupMode}
                isSelected={isSelected}
                onToggleSelect={() => {
                  setSelectedMessageIds((prev) => {
                    const next = new Set(prev);
                    if (next.has(message.id)) {
                      next.delete(message.id);
                    } else {
                      next.add(message.id);
                    }
                    return next;
                  });
                }}
              />
            );
          })}

          {/* Running placeholders */}
          {placeholderAgents.map((member) => (
            <RunningAgentPlaceholder
              key={`placeholder-${member.agent.id}`}
              member={member}
              tone={getMessageTone(member.agent.id, false)}
              stateInfo={agentStateInfos[member.agent.id]}
              clock={clock}
              isStopping={stoppingAgents.has(member.agent.id)}
              onStop={handleStopAgent}
            />
          ))}

          {/* Streaming runs */}
          {Object.entries(streamingRuns).map(([runId, run]) => (
            <StreamingRunEntry
              key={`stream-${runId}`}
              runId={runId}
              run={run}
              agentName={agentById.get(run.agentId)?.name ?? 'Agent'}
              runnerType={agentById.get(run.agentId)?.runner_type ?? null}
              tone={getMessageTone(run.agentId, false)}
              sessionAgent={sessionAgents.find(
                (sa) => sa.agent_id === run.agentId
              )}
              isStopping={stoppingAgents.has(run.agentId)}
              onStop={handleStopAgent}
            />
          ))}

          <div ref={bottomRef} />
        </div>

        {/* Input Area Resize Handle */}
        <div
          className="chat-session-resize-handle h-1 cursor-row-resize transition-colors shrink-0 border-t border-border"
          onMouseDown={(e) => handleResizeStart('input', e)}
        />

        {/* Message Input */}
        <MessageInputArea
          draft={draft}
          onDraftChange={handleDraftChange}
          inputRef={inputRef}
          selectedMentions={selectedMentions}
          onSelectedMentionsChange={setSelectedMentions}
          agentOptions={agentOptions}
          mentionAgentsCount={mentionAgents.length}
          mentionQuery={mentionQuery}
          visibleMentionSuggestions={visibleMentionSuggestions}
          highlightedMentionIndex={highlightedMentionIndex}
          onMentionSelect={handleMentionSelect}
          onMentionKeyDown={handleMentionKeyDown}
          replyToMessage={replyToMessage}
          replyToSenderLabel={
            replyToMessage ? getMessageSenderLabel(replyToMessage) : null
          }
          replyToPreview={
            replyToMessage ? getReferencePreview(replyToMessage) : null
          }
          onCancelReply={() => setReplyToMessage(null)}
          attachedFiles={attachedFiles}
          attachmentError={attachmentError}
          isUploadingAttachments={isUploadingAttachments}
          onAttachmentInputChange={handleAttachmentInputChange}
          onRemoveAttachedFile={removeAttachedFile}
          onClearAttachedFiles={clearAttachedFiles}
          onPreviewFile={previewAttachedFile}
          fileInputRef={fileInputRef}
          canSend={canSend}
          isSending={sendMessage.isPending}
          onSend={handleSend}
          inputAreaHeight={inputAreaHeight}
          isArchived={isArchived}
          activeSessionId={activeSessionId}
        />
      </section>

      {/* Right Sidebar Resize Handle */}
      <div
        className="chat-session-resize-handle w-1 cursor-col-resize transition-colors shrink-0"
        onMouseDown={(e) => handleResizeStart('right', e)}
      />

      {/* AI Members Sidebar */}
      <AiMembersSidebar
        sessionMembers={sessionMembers}
        agentStates={agentStates}
        activeSessionId={activeSessionId}
        isArchived={isArchived}
        width={rightSidebarWidth}
        isAddMemberOpen={isAddMemberOpen}
        editingMember={editingMember}
        newMemberName={newMemberName}
        newMemberRunnerType={newMemberRunnerType}
        newMemberVariant={newMemberVariant}
        newMemberPrompt={newMemberPrompt}
        newMemberWorkspace={newMemberWorkspace}
        memberNameLengthError={memberNameLengthError}
        onNameChange={setNewMemberName}
        onRunnerTypeChange={setNewMemberRunnerType}
        onVariantChange={setNewMemberVariant}
        onPromptChange={setNewMemberPrompt}
        onWorkspaceChange={setNewMemberWorkspace}
        memberError={memberError}
        isSavingMember={isSavingMember}
        availableRunnerTypes={availableRunnerTypes}
        enabledRunnerTypes={enabledRunnerTypes}
        isCheckingAvailability={isCheckingAvailability}
        isRunnerAvailable={isRunnerAvailable}
        availabilityLabel={availabilityLabel}
        memberVariantOptions={memberVariantOptions}
        getModelName={getModelName}
        getModelDisplayName={getModelDisplayName}
        getVariantLabel={getVariantLabel}
        getVariantOptions={getVariantOptions}
        onOpenAddMember={() => {
          setIsAddMemberOpen(true);
          setMemberError(null);
          setEditingMember(null);
          setNewMemberName('');
          setNewMemberVariant('DEFAULT');
          setNewMemberPrompt('');
          setNewMemberWorkspace('');
          setIsPromptEditorOpen(false);
          setPromptFileError(null);
        }}
        onCancelMember={() => {
          setIsAddMemberOpen(false);
          setMemberError(null);
          setEditingMember(null);
          setNewMemberVariant('DEFAULT');
          setIsPromptEditorOpen(false);
          setPromptFileError(null);
        }}
        onSaveMember={handleAddMember}
        onEditMember={handleEditMember}
        onRemoveMember={handleRemoveMember}
        onOpenWorkspace={(agentId) => {
          setWorkspaceAgentId(agentId);
          setWorkspaceDrawerOpen(true);
        }}
        onExpandPromptEditor={() => {
          setIsPromptEditorOpen(true);
          setPromptFileError(null);
        }}
        enabledMemberPresets={enabledMemberPresets}
        enabledTeamPresets={enabledTeamPresets}
        onAddMemberPreset={handleAddMemberPreset}
        onImportTeamPreset={handleImportTeamPreset}
        teamImportPlan={teamImportPlan}
        teamImportName={teamImportName}
        isImportingTeam={isImportingTeam}
        onUpdateTeamImportPlanEntry={handleUpdateTeamImportPlanEntry}
        onConfirmTeamImport={handleConfirmTeamImport}
        onCancelTeamImport={handleCancelTeamImport}
      />

      {/* Workspace Drawer */}
      <WorkspaceDrawer
        isOpen={workspaceDrawerOpen}
        onClose={() => setWorkspaceDrawerOpen(false)}
        agent={activeWorkspaceAgent ?? null}
        workspacePath={workspacePath}
        runs={activeWorkspaceRuns}
        logRunId={logRunId}
        logContent={logContent}
        logLoading={logLoading}
        logError={logError}
        onLoadLog={handleLoadLog}
      />

      {/* Diff Viewer Modal */}
      <DiffViewerModal
        isOpen={diffViewerOpen}
        runId={diffViewerRunId}
        hasDiff={diffViewerHasDiff}
        isFullscreen={diffViewerFullscreen}
        runDiff={diffViewerRun}
        untrackedFiles={diffViewerUntracked}
        untrackedContent={untrackedContent}
        theme={actualTheme}
        onClose={handleCloseDiffViewer}
        onToggleFullscreen={handleToggleFullscreen}
        onToggleUntracked={handleToggleUntracked}
      />

      {/* Prompt Editor Modal */}
      <PromptEditorModal
        isOpen={isPromptEditorOpen}
        value={newMemberPrompt}
        onChange={setNewMemberPrompt}
        onClose={() => setIsPromptEditorOpen(false)}
        promptFileInputRef={promptFileInputRef}
        onPromptFileChange={handlePromptFileChange}
        promptFileLoading={promptFileLoading}
        promptFileError={promptFileError}
      />

      {/* Confirm Modal */}
      <ConfirmModal
        isOpen={!!confirmModal}
        title={confirmModal?.title ?? t('modals.confirm.defaultTitle')}
        message={confirmModal?.message ?? ''}
        isLoading={isConfirmLoading}
        mode={confirmModal?.mode}
        confirmText={confirmModal?.confirmText}
        cancelText={confirmModal?.cancelText}
        onConfirm={async () => {
          if (!confirmModal) return;
          setIsConfirmLoading(true);
          try {
            await confirmModal.onConfirm();
          } finally {
            setIsConfirmLoading(false);
            setConfirmModal(null);
          }
        }}
        onCancel={() => setConfirmModal(null)}
      />

      {/* File Preview Modal */}
      <FilePreviewModal
        file={previewFile.file}
        content={previewFile.content}
        onClose={closePreview}
      />
    </div>
  );
}

