import {
  type ChangeEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { useNavigate, useParams } from 'react-router-dom';
import {
  CaretRightIcon,
  ChatsTeardropIcon,
  CheckCircleIcon,
  PencilSimpleIcon,
  PaperclipIcon,
  PaperPlaneRightIcon,
  PlusIcon,
  UsersIcon,
  ArrowsOutSimpleIcon,
  ArrowsInSimpleIcon,
  XCircleIcon,
  XIcon,
  TrashIcon,
  CheckSquareIcon,
  SquareIcon,
} from '@phosphor-icons/react';
import {
  ChatMessage,
  ChatSenderType,
  ChatSessionStatus,
  ChatSessionAgentState,
  BaseCodingAgent,
  type AvailabilityInfo,
  type JsonValue,
} from 'shared/types';
import { ApiError, chatApi, configApi } from '@/lib/api';
import { cn } from '@/lib/utils';
import { Badge } from '@/components/ui/badge';
import {
  Dialog,
  DialogHeader,
  DialogTitle,
  DialogContent,
  DialogFooter,
} from '@/components/ui/dialog';
import { PrimaryButton } from '@/components/ui-new/primitives/PrimaryButton';
import { MultiSelectDropdown } from '@/components/ui-new/primitives/MultiSelectDropdown';
import { ChatEntryContainer } from '@/components/ui-new/primitives/conversation/ChatEntryContainer';
import { ChatMarkdown } from '@/components/ui-new/primitives/conversation/ChatMarkdown';
import { ChatSystemMessage } from '@/components/ui-new/primitives/conversation/ChatSystemMessage';
import { DiffViewBody } from '@/components/ui-new/primitives/conversation/PierreConversationDiff';
import { Tooltip } from '@/components/ui-new/primitives/Tooltip';
import RawLogText from '@/components/common/RawLogText';
import { useUserSystem } from '@/components/ConfigProvider';
import { useTheme } from '@/components/ThemeProvider';
import { formatDateShortWithTime } from '@/utils/date';
import { toPrettyCase } from '@/utils/string';
import { getActualTheme } from '@/utils/theme';

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
  agentStateLabels,
  agentStateDotClass,
  memberNameRegex,
  getMessageTone,
  extractDiffMeta,
  extractMentions,
  extractReferenceId,
  extractAttachments,
  formatBytes,
  truncateText,
  sanitizeHandle,
  detectApiError,
} from './chat';

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

export function ChatSessions() {
  const { sessionId } = useParams<{ sessionId?: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const promptFileInputRef = useRef<HTMLInputElement | null>(null);
  const { profiles, loginStatus } = useUserSystem();
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
    isLoading,
  } = useChatData(sessionId ?? null);

  const activeSessionId = sessionId ?? sortedSessions[0]?.id ?? null;

  // Messages state
  const [messages, setMessages] = useState<ChatMessage[]>([]);

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

  // WebSocket connection
  const {
    streamingRuns,
    agentStates,
    agentStateInfos,
    mentionStatuses,
    setAgentStates,
    setAgentStateInfos,
    setMentionStatuses,
  } =
    useChatWebSocket(
      activeSessionId,
      upsertMessage
    );

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
    (_count) => {
      // Refresh messages after deletion
      if (activeSessionId) {
        queryClient.invalidateQueries({ queryKey: ['chatMessages', activeSessionId] });
      }
    },
    () => {
      // Navigate to chat root after session deletion
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
  const [agentAvailability, setAgentAvailability] = useState<
    Record<string, AvailabilityInfo | null>
  >({});
  const [isCheckingAvailability, setIsCheckingAvailability] = useState(false);
  const [workspaceDrawerOpen, setWorkspaceDrawerOpen] = useState(false);
  const [workspaceAgentId, setWorkspaceAgentId] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);
  const [isAddMemberOpen, setIsAddMemberOpen] = useState(false);
  const [editingMember, setEditingMember] = useState<SessionMember | null>(null);
  const [newMemberName, setNewMemberName] = useState('');
  const [newMemberRunnerType, setNewMemberRunnerType] = useState('');
  const [newMemberVariant, setNewMemberVariant] = useState('DEFAULT');
  const [newMemberPrompt, setNewMemberPrompt] = useState('');
  const [newMemberWorkspace, setNewMemberWorkspace] = useState('');
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
  // Cleanup mode states
  const [isCleanupMode, setIsCleanupMode] = useState(false);
  const [selectedMessageIds, setSelectedMessageIds] = useState<Set<string>>(new Set());
  // Confirm modal state
  const [confirmModal, setConfirmModal] = useState<{
    title: string;
    message: string;
    onConfirm: () => void | Promise<void>;
  } | null>(null);
  const [isConfirmLoading, setIsConfirmLoading] = useState(false);
  const [isDeletingMessages, setIsDeletingMessages] = useState(false);
  // Resizable panel states
  const [leftSidebarWidth, setLeftSidebarWidth] = useState(288); // 18rem = 288px
  const [rightSidebarWidth, setRightSidebarWidth] = useState(320); // 20rem = 320px
  const [inputAreaHeight, setInputAreaHeight] = useState(240); // min-h-[240px]
  const [isResizing, setIsResizing] = useState<'left' | 'right' | 'input' | null>(null);
  const resizeStartRef = useRef<{ startX: number; startY: number; startWidth: number; startHeight: number } | null>(null);

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
          const existingPriority =
            existing ? mentionStatusPriority[existing] : -1;
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
    setIsPromptEditorOpen(false);
    setPromptFileError(null);
    setPromptFileLoading(false);
  }, [activeSessionId, resetInput, resetDiffViewer, setReplyToMessage]);

  // Navigate to first session if needed
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
      return info?.type === 'LOGIN_DETECTED' || info?.type === 'INSTALLATION_FOUND';
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
      if (!info) return isCheckingAvailability ? ' (checking)' : ' (unavailable)';
      if (info.type === 'LOGIN_DETECTED' || info.type === 'INSTALLATION_FOUND') {
        return '';
      }
      return ' (not installed)';
    },
    [agentAvailability, isCheckingAvailability]
  );

  // Get model name from profiles for a given runner type and variant
  const getModelName = useCallback(
    (runnerType: string, variant?: string): string | null => {
      if (!profiles) return null;
      const executorConfig = profiles[runnerType];
      if (!executorConfig) return null;
      // Get the specified variant, or DEFAULT, or first available variant
      const variantKey = variant && variant in executorConfig
        ? variant
        : 'DEFAULT' in executorConfig
          ? 'DEFAULT'
          : Object.keys(executorConfig)[0];
      if (!variantKey) return null;
      const variantConfig = executorConfig[variantKey];
      if (!variantConfig) return null;
      // The config is wrapped like { "CLAUDE_CODE": { model: "..." } }
      const innerConfig = Object.values(variantConfig)[0] as { model?: string | null } | undefined;
      return innerConfig?.model ?? null;
    },
    [profiles]
  );

  // Get available variants for a given runner type
  const getVariantOptions = useCallback(
    (runnerType: string): string[] => {
      if (!profiles) return [];
      const executorConfig = profiles[runnerType];
      if (!executorConfig) return [];
      const variants = Object.keys(executorConfig);
      // Sort: DEFAULT first, then alphabetically
      return variants.sort((a, b) => {
        if (a === 'DEFAULT') return -1;
        if (b === 'DEFAULT') return 1;
        return a.localeCompare(b);
      });
    },
    [profiles]
  );

  // Get variant options for current member runner type
  const memberVariantOptions = useMemo(
    () => getVariantOptions(newMemberRunnerType),
    [getVariantOptions, newMemberRunnerType]
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

  const messageById = useMemo(
    () => new Map(messageList.map((message) => [message.id, message])),
    [messageList]
  );

  const runHistory = useRunHistory(messages);

  const activeSession = useMemo(
    () => sortedSessions.find((session) => session.id === activeSessionId),
    [sortedSessions, activeSessionId]
  );

  const agentIdByName = useMemo(() => {
    const map = new Map<string, string>();
    sessionMembers.forEach((member) => {
      map.set(member.agent.name, member.agent.id);
    });
    return map;
  }, [sessionMembers]);

  const isArchived = activeSession?.status === ChatSessionStatus.archived;

  const placeholderAgents = useMemo(
    () =>
      sessionMembers.filter(
        (member) => agentStates[member.agent.id] === ChatSessionAgentState.running
      ),
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
    (draft.trim().length > 0 || selectedMentions.length > 0) &&
    !sendMessage.isPending &&
    !isUploadingAttachments;

  const diffViewerRun = diffViewerRunId ? runDiffs[diffViewerRunId] : null;
  const DiffViewerIcon = diffViewerFullscreen
    ? ArrowsInSimpleIcon
    : ArrowsOutSimpleIcon;

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
  }, [editingMember, enabledRunnerTypes, isRunnerAvailable, newMemberRunnerType]);

  // Set default variant when runner type changes
  useEffect(() => {
    if (memberVariantOptions.length > 0) {
      // Set to DEFAULT if available, otherwise first option
      const defaultVariant = memberVariantOptions.includes('DEFAULT')
        ? 'DEFAULT'
        : memberVariantOptions[0];
      setNewMemberVariant(defaultVariant);
    } else {
      setNewMemberVariant('DEFAULT');
    }
  }, [newMemberRunnerType, memberVariantOptions]);

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
    // Also sync agentStateInfos with updated_at as startedAt for running agents
    // This ensures the timer works correctly after page refresh
    setAgentStateInfos((prev) => {
      const next = { ...prev };
      for (const sessionAgent of sessionAgents) {
        // Only set if not already present (WebSocket events take precedence)
        if (!next[sessionAgent.agent_id]) {
          next[sessionAgent.agent_id] = {
            state: sessionAgent.state,
            // Use updated_at as startedAt for running agents
            startedAt:
              sessionAgent.state === ChatSessionAgentState.running
                ? sessionAgent.updated_at
                : null,
          };
        }
      }
      return next;
    });
  }, [sessionAgents, setAgentStates, setAgentStateInfos]);

  // Running timer - tick clock when any agent is running
  useEffect(() => {
    const hasRunning = sessionMembers.some(
      (member) => agentStates[member.agent.id] === ChatSessionAgentState.running
    );
    if (!hasRunning) return;
    const timer = setInterval(() => setClock(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [agentStates, sessionMembers]);

  // Title editing
  useEffect(() => {
    setTitleDraft(activeSession?.title ?? '');
    setIsEditingTitle(false);
    setTitleError(null);
  }, [activeSession?.id]);

  useEffect(() => {
    if (activeSession?.status === ChatSessionStatus.archived) {
      setShowArchived(true);
    }
  }, [activeSession?.status]);

  // Auto-scroll
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
    if (!content) return;

    // Check if any mentioned agent is currently running
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
        title: 'Agent Running',
        message: `The following agent(s) are currently running: @${runningMentionedAgents.join(', @')}. They will not process new messages until the current task is stopped. Do you still want to send this message?`,
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
      await sendMessage.mutateAsync({
        sessionId: activeSessionId,
        content,
        meta,
      });
      resetInput();
      inputRef.current?.focus();
    } catch (error) {
      console.warn('Failed to send chat message', error);
    }
  };

  const handleAttachmentUpload = async (files: FileList | File[]) => {
    if (!activeSessionId || isArchived) return;
    const list = Array.from(files);
    if (list.length === 0) return;
    setIsUploadingAttachments(true);
    setAttachmentError(null);
    try {
      const message = await chatApi.uploadChatAttachments(
        activeSessionId,
        list,
        senderHandle
      );
      upsertMessage(message);
      queryClient.invalidateQueries({ queryKey: ['chatSessions'] });
    } catch (error) {
      console.warn('Failed to upload attachments', error);
      setAttachmentError('Unable to upload attachments.');
    } finally {
      setIsUploadingAttachments(false);
    }
  };

  const handleAttachmentInputChange = (event: ChangeEvent<HTMLInputElement>) => {
    if (event.target.files) {
      handleAttachmentUpload(event.target.files);
    }
    event.target.value = '';
  };

  const handlePromptFileChange = async (event: ChangeEvent<HTMLInputElement>) => {
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

    if (!isRunnerAvailable(runnerType)) {
      setMemberError('Selected coding agent is not available locally.');
      return;
    }

    if (!workspacePathVal) {
      setMemberError('Workspace path is required.');
      return;
    }

    if (!prompt) {
      setMemberError('System prompt is required.');
      return;
    }

    let nameChanged = false;
    let runnerChanged = false;
    let promptChanged = false;
    let workspaceChanged = false;
    if (editingMember) {
      nameChanged = editingMember.agent.name !== name;
      runnerChanged = editingMember.agent.runner_type !== runnerType;
      promptChanged = (editingMember.agent.system_prompt ?? '') !== prompt;
      workspaceChanged =
        (editingMember.sessionAgent.workspace_path ?? '') !== workspacePathVal;

      if (nameChanged) {
        const conflict = agents.find(
          (agent) =>
            agent.id !== editingMember.agent.id &&
            agent.name.toLowerCase() === name.toLowerCase()
        );
        if (conflict) {
          setMemberError('An AI member with this name already exists.');
          return;
        }
      }

      if (
        !nameChanged &&
        !runnerChanged &&
        !promptChanged &&
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
        const updatePayload = {
          name: nameChanged ? name : null,
          runner_type: runnerChanged ? runnerType : null,
          system_prompt: promptChanged ? prompt : null,
          tools_enabled: null,
        };

        if (
          updatePayload.name ||
          updatePayload.runner_type ||
          updatePayload.system_prompt
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
        const existing = agents.find((agent) => agent.name === name);
        let agentId = existing?.id ?? null;
        if (existing) {
          const updatePayload = {
            name: null,
            runner_type: existing.runner_type !== runnerType ? runnerType : null,
            system_prompt:
              (existing.system_prompt ?? '') !== prompt ? prompt : null,
            tools_enabled: null,
          };

          if (updatePayload.runner_type || updatePayload.system_prompt) {
            const updated = await chatApi.updateAgent(existing.id, updatePayload);
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
      setNewMemberPrompt('');
      setNewMemberWorkspace('');
      setMemberError(null);
      setEditingMember(null);
      setIsAddMemberOpen(false);
    } catch (error) {
      console.warn('Failed to add AI member', error);
      if (error instanceof ApiError && error.message) {
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
      title: 'Remove AI Member',
      message: `Are you sure you want to remove @${member.agent.name} from this session?`,
      onConfirm: async () => {
        try {
          console.log('Deleting session agent:', { activeSessionId, sessionAgentId });
          await chatApi.deleteSessionAgent(activeSessionId, sessionAgentId);
          console.log('Delete successful, invalidating queries...');
          // Invalidate and refetch to ensure UI updates
          await queryClient.invalidateQueries({
            queryKey: ['chatSessionAgents', activeSessionId],
          });
          await queryClient.refetchQueries({
            queryKey: ['chatSessionAgents', activeSessionId],
          });
          console.log('Queries refetched');
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

  const handleSaveTitle = async () => {
    if (!activeSessionId) return;
    const trimmed = titleDraft.trim();
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
      setIsResizing(type);
      resizeStartRef.current = {
        startX: e.clientX,
        startY: e.clientY,
        startWidth: type === 'left' ? leftSidebarWidth : rightSidebarWidth,
        startHeight: inputAreaHeight,
      };
    },
    [leftSidebarWidth, rightSidebarWidth, inputAreaHeight]
  );

  useEffect(() => {
    if (!isResizing) return;

    const handleMouseMove = (e: MouseEvent) => {
      if (!resizeStartRef.current) return;
      const { startX, startY, startWidth, startHeight } = resizeStartRef.current;

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
    <div className="relative flex h-full min-h-0 bg-primary overflow-hidden select-none">
      {/* Session List Sidebar */}
      <aside
        className="border-r border-border flex flex-col min-h-0 shrink-0"
        style={{ width: leftSidebarWidth }}
      >
        <div className="px-base py-base border-b border-border flex items-center justify-between">
          <div className="flex items-center gap-half text-normal font-medium">
            <ChatsTeardropIcon className="size-icon-sm" />
            <span>Group Chat</span>
          </div>
          <PrimaryButton
            variant="secondary"
            value="New"
            onClick={() => createSession.mutate()}
            disabled={createSession.isPending}
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

      {/* Left Sidebar Resize Handle */}
      <div
        className="w-1 cursor-col-resize hover:bg-brand/50 active:bg-brand transition-colors shrink-0"
        onMouseDown={(e) => handleResizeStart('left', e)}
      />

      {/* Main Chat Section */}
      <section className="flex-1 min-w-0 min-h-0 flex flex-col">
        <header className="px-base py-half border-b border-border flex items-center justify-between">
          <div className="min-w-0">
            {!isEditingTitle && (
              <div className="flex items-center gap-half">
                <div className="text-sm text-normal font-medium truncate">
                  {activeSession?.title || 'Untitled session'}
                </div>
                {activeSession && (
                  <>
                    <button
                      type="button"
                      className="text-low hover:text-normal"
                      onClick={() => {
                        setIsEditingTitle(true);
                        setTitleError(null);
                      }}
                      aria-label="Edit session name"
                    >
                      <PencilSimpleIcon className="size-icon-xs" />
                    </button>
                    <button
                      type="button"
                      className="text-low hover:text-error"
                      onClick={() => {
                        setConfirmModal({
                          title: 'Delete Session',
                          message: `Are you sure you want to delete "${activeSession.title || 'Untitled session'}"? This action cannot be undone and all messages will be permanently deleted.`,
                          onConfirm: async () => {
                            await deleteSession.mutateAsync(activeSession.id);
                          },
                        });
                      }}
                      aria-label="Delete session"
                      title="Delete session"
                    >
                      <TrashIcon className="size-icon-xs" />
                    </button>
                  </>
                )}
              </div>
            )}
            {isEditingTitle && (
              <div className="flex items-center gap-half flex-wrap">
                <input
                  value={titleDraft}
                  onChange={(event) => setTitleDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      handleSaveTitle();
                    }
                    if (event.key === 'Escape') {
                      event.preventDefault();
                      handleCancelTitleEdit();
                    }
                  }}
                  placeholder="Session name"
                  disabled={updateSession.isPending}
                  className={cn(
                    'w-[240px] max-w-full rounded-sm border border-border bg-panel px-base py-half',
                    'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                  )}
                />
                <PrimaryButton
                  value="Save"
                  onClick={handleSaveTitle}
                  disabled={updateSession.isPending}
                />
                <PrimaryButton
                  variant="tertiary"
                  value="Cancel"
                  onClick={handleCancelTitleEdit}
                  disabled={updateSession.isPending}
                />
              </div>
            )}
            {titleError && <div className="text-xs text-error">{titleError}</div>}
            {activeSession && (
              <div className="text-xs text-low">
                Created {formatDateShortWithTime(activeSession.created_at)} / Total messages: {messageList.length}
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
                  variant="tertiary"
                  value={isArchived ? 'Restore' : 'Archive'}
                  onClick={() => {
                    if (!activeSessionId) return;
                    if (isArchived) {
                      restoreSession.mutate(activeSessionId);
                    } else {
                      archiveSession.mutate(activeSessionId);
                    }
                  }}
                  disabled={archiveSession.isPending || restoreSession.isPending}
                />
                {!isArchived && (
                  <PrimaryButton
                    variant={isCleanupMode ? 'default' : 'secondary'}
                    value={isCleanupMode ? 'Exit Cleanup' : 'Cleanup'}
                    onClick={() => {
                      if (isCleanupMode) {
                        setIsCleanupMode(false);
                        setSelectedMessageIds(new Set());
                      } else {
                        setIsCleanupMode(true);
                      }
                    }}
                    disabled={isDeletingMessages}
                  />
                )}
              </div>
            )}
          </div>
        </header>

        {/* Cleanup mode controls */}
        {activeSession && isCleanupMode && (
          <div className="px-base py-half border-b border-border text-xs text-low flex items-center justify-end gap-base">
            <span>Selected: {selectedMessageIds.size}</span>
            <button
              type="button"
              className="text-brand hover:text-brand-hover"
              onClick={() => {
                if (selectedMessageIds.size === messageList.length) {
                  setSelectedMessageIds(new Set());
                } else {
                  setSelectedMessageIds(new Set(messageList.map((m) => m.id)));
                }
              }}
            >
              {selectedMessageIds.size === messageList.length ? 'Deselect All' : 'Select All'}
            </button>
            {selectedMessageIds.size > 0 && (
              <button
                type="button"
                className="text-error hover:text-error/80 flex items-center gap-half"
                onClick={() => {
                  if (!activeSessionId) return;
                  const count = selectedMessageIds.size;
                  setConfirmModal({
                    title: 'Delete Messages',
                    message: `Are you sure you want to delete ${count} message(s)? This action cannot be undone.`,
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
                disabled={isDeletingMessages}
              >
                <TrashIcon className="size-icon-xs" />
                Delete Selected
              </button>
            )}
          </div>
        )}

        {/* Messages */}
        <div className="flex-1 min-h-0 overflow-y-auto p-base space-y-base">
          {isLoading && <div className="text-sm text-low">Loading chat...</div>}
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
            const diffInfo = diffMeta && diffMeta.runId ? diffMeta : null;
            const diffRunId = diffInfo?.runId ?? '';
            const hasDiffInfo =
              !!diffInfo &&
              diffRunId.length > 0 &&
              (diffInfo.available || diffInfo.untrackedFiles.length > 0);
            const attachments = extractAttachments(message.meta);
            const mentionList = Array.from(
              new Set(message.mentions.filter((mention) => mention.length > 0))
            );
            const mentionStatusMap = mentionStatuses.get(message.id);
            const referenceId = extractReferenceId(message.meta);
            const referenceMessage = referenceId
              ? messageById.get(referenceId)
              : null;
            const toneKey = isUser
              ? 'user'
              : message.sender_id ?? agentName ?? 'agent';
            const tone = getMessageTone(String(toneKey), isUser);

            const isSelected = selectedMessageIds.has(message.id);
            const toggleSelect = () => {
              setSelectedMessageIds((prev) => {
                const next = new Set(prev);
                if (next.has(message.id)) {
                  next.delete(message.id);
                } else {
                  next.add(message.id);
                }
                return next;
              });
            };

            if (message.sender_type === ChatSenderType.system) {
              return (
                <div key={message.id} className="flex items-start gap-base">
                  {isCleanupMode && (
                    <button
                      type="button"
                      className="flex-shrink-0 mt-1"
                      onClick={toggleSelect}
                    >
                      {isSelected ? (
                        <CheckSquareIcon className="size-icon text-brand" weight="fill" />
                      ) : (
                        <SquareIcon className="size-icon text-low" />
                      )}
                    </button>
                  )}
                  <div className="flex-1">
                    <ChatSystemMessage
                      content={message.content}
                      expanded
                    />
                  </div>
                </div>
              );
            }

            return (
              <div
                key={message.id}
                id={`chat-message-${message.id}`}
                className={cn('flex items-start gap-base', isUser ? 'justify-end' : 'justify-start')}
              >
                {isCleanupMode && !isUser && (
                  <button
                    type="button"
                    className="flex-shrink-0 mt-1"
                    onClick={toggleSelect}
                  >
                    {isSelected ? (
                      <CheckSquareIcon className="size-icon text-brand" weight="fill" />
                    ) : (
                      <SquareIcon className="size-icon text-low" />
                    )}
                  </button>
                )}
                <ChatEntryContainer
                  variant={isUser ? 'user' : 'system'}
                  title={isUser ? 'You' : agentName ?? 'Agent'}
                  expanded
                  headerRight={
                    <div className="flex items-center gap-half text-xs text-low">
                      <button
                        type="button"
                        className={cn(
                          'text-brand hover:text-brand-hover',
                          isArchived && 'pointer-events-none opacity-50'
                        )}
                        onClick={() => handleLocalReplySelect(message)}
                        disabled={isArchived}
                      >
                        引用
                      </button>
                      <span>{formatDateShortWithTime(message.created_at)}</span>
                    </div>
                  }
                  className={cn(
                    'max-w-[720px] w-full md:w-[80%] shadow-sm rounded-2xl',
                    isUser && 'ml-auto',
                    isCleanupMode && isSelected && 'ring-2 ring-brand'
                  )}
                  headerClassName="bg-transparent"
                  style={{
                    backgroundColor: tone.bg,
                    borderColor: tone.border,
                  }}
                >
                  {referenceId && (
                    <div className="mb-half border border-border rounded-sm bg-secondary/60 px-base py-half text-xs text-low">
                      <div className="flex items-center justify-between gap-base">
                        <span className="font-medium text-normal">
                          Replying to{' '}
                          {referenceMessage
                            ? getMessageSenderLabel(referenceMessage)
                            : 'message'}
                        </span>
                        <button
                          type="button"
                          className="text-brand hover:text-brand-hover"
                          onClick={() => {
                            if (referenceMessage) {
                              const element = document.getElementById(
                                `chat-message-${referenceMessage.id}`
                              );
                              element?.scrollIntoView({ behavior: 'smooth' });
                            }
                          }}
                        >
                          View
                        </button>
                      </div>
                      <div className="mt-half">
                        {referenceMessage
                          ? getReferencePreview(referenceMessage)
                          : 'Referenced message unavailable.'}
                      </div>
                      {referenceMessage &&
                        extractAttachments(referenceMessage.meta).length > 0 && (
                          <div className="mt-half text-xs text-low">
                            Attachments:{' '}
                            {extractAttachments(referenceMessage.meta)
                              .map((item) => item.name)
                              .filter(Boolean)
                              .slice(0, 3)
                              .join(', ')}
                          </div>
                        )}
                    </div>
                  )}
                  {(() => {
                    const apiError = isAgent ? detectApiError(message.content) : null;
                    return apiError ? (
                      <div className="mb-half flex items-center gap-half rounded-sm bg-error/10 border border-error/30 px-base py-half text-xs text-error">
                        <XCircleIcon className="size-icon-sm flex-shrink-0" weight="fill" />
                        <span className="font-medium">{apiError.message}</span>
                        <span className="text-error/70">- Please check your API quota or try again later</span>
                      </div>
                    ) : null;
                  })()}
                  <ChatMarkdown content={message.content} />
                  {mentionList.length > 0 && (
                    <div className="mt-half flex flex-wrap items-center gap-half text-xs text-low">
                      <span>Mentions:</span>
                      {mentionList.map((mention) => {
                        const agentId = agentIdByName.get(mention);
                        const mentionStatus = mentionStatusMap?.get(mention);
                        const isFallbackRunning =
                          !mentionStatusMap &&
                          !!agentId &&
                          agentStates[agentId] === ChatSessionAgentState.running;
                        const isRunning =
                          mentionStatus === 'running' ||
                          mentionStatus === 'received' ||
                          isFallbackRunning;
                        const isCompleted = mentionStatus === 'completed';
                        const isFailed = mentionStatus === 'failed';
                        const showCheck = !isFailed && (isRunning || isCompleted);
                        const pulse = mentionStatus === 'running';
                        return (
                          <Badge
                            key={`${message.id}-mention-${mention}`}
                            variant="secondary"
                            className="flex items-center gap-1 px-2 py-0.5 text-xs"
                          >
                            @{mention}
                            {showCheck && (
                              <CheckCircleIcon
                                className={cn(
                                  'size-icon-2xs text-success',
                                  pulse && 'animate-pulse'
                                )}
                                weight="fill"
                              />
                            )}
                            {isFailed && (
                              <XCircleIcon
                                className="size-icon-2xs text-error"
                                weight="fill"
                              />
                            )}
                          </Badge>
                        );
                      })}
                    </div>
                  )}
                  {attachments.length > 0 && (
                    <div className="mt-half space-y-half">
                      {attachments.map((attachment) => {
                        const attachmentUrl =
                          activeSessionId && attachment.id
                            ? chatApi.getChatAttachmentUrl(
                                activeSessionId,
                                message.id,
                                attachment.id
                              )
                            : '#';
                        const isImage =
                          attachment.kind === 'image' ||
                          (attachment.mime_type ?? '').startsWith('image/');
                        return (
                          <div
                            key={attachment.id}
                            className="border border-border rounded-sm bg-panel px-base py-half text-xs text-normal"
                          >
                            <div className="flex items-center justify-between gap-base">
                              <span className="font-ibm-plex-mono break-all">
                                {attachment.name}
                              </span>
                              <a
                                className="text-brand hover:text-brand-hover"
                                href={attachmentUrl}
                                target="_blank"
                                rel="noreferrer"
                              >
                                Open
                              </a>
                            </div>
                            {attachment.size_bytes && (
                              <div className="text-xs text-low">
                                {formatBytes(attachment.size_bytes)}
                              </div>
                            )}
                            {isImage && attachmentUrl !== '#' && (
                              <img
                                src={attachmentUrl}
                                alt={attachment.name}
                                loading="lazy"
                                className="mt-half max-h-56 w-auto rounded-sm border border-border"
                              />
                            )}
                          </div>
                        );
                      })}
                    </div>
                  )}
                  {hasDiffInfo && diffInfo && (
                    <div className="mt-half border border-border rounded-sm bg-secondary/70 px-base py-half text-xs text-normal">
                      <div className="flex items-center justify-between gap-base">
                        <span>Code changes</span>
                        <button
                          type="button"
                          className="text-brand hover:text-brand-hover"
                          onClick={() =>
                            handleOpenDiffViewer(
                              diffRunId,
                              diffInfo.untrackedFiles,
                              diffInfo.available
                            )
                          }
                        >
                          View changes
                        </button>
                      </div>
                      {diffInfo.available && runDiffs[diffRunId]?.loading && (
                        <div className="mt-half text-xs text-low">
                          Loading diff...
                        </div>
                      )}
                      {diffInfo.available && runDiffs[diffRunId]?.error && (
                        <div className="mt-half text-xs text-error">
                          {runDiffs[diffRunId]?.error}
                        </div>
                      )}
                      {diffInfo.untrackedFiles.length > 0 && (
                        <div className="mt-half text-xs text-low">
                          {diffInfo.untrackedFiles.length} untracked file
                          {diffInfo.untrackedFiles.length === 1 ? '' : 's'}
                        </div>
                      )}
                    </div>
                  )}
                </ChatEntryContainer>
                {isCleanupMode && isUser && (
                  <button
                    type="button"
                    className="flex-shrink-0 mt-1"
                    onClick={toggleSelect}
                  >
                    {isSelected ? (
                      <CheckSquareIcon className="size-icon text-brand" weight="fill" />
                    ) : (
                      <SquareIcon className="size-icon text-low" />
                    )}
                  </button>
                )}
              </div>
            );
          })}

          {/* Running placeholders */}
          {placeholderAgents.map((member) => {
            const stateInfo = agentStateInfos[member.agent.id];
            const startedAtStr = stateInfo?.startedAt;
            const startedAtMs = startedAtStr
              ? new Date(startedAtStr).getTime()
              : clock;
            const elapsedSeconds = Math.max(
              0,
              Math.floor((clock - startedAtMs) / 1000)
            );
            const tone = getMessageTone(member.agent.id, false);
            const isStopping = stoppingAgents.has(member.agent.id);
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
                  headerRight={
                    <button
                      type="button"
                      className={cn(
                        'text-xs text-error hover:text-error/80',
                        isStopping && 'opacity-50 cursor-not-allowed'
                      )}
                      onClick={() =>
                        handleStopAgent(member.sessionAgent.id, member.agent.id)
                      }
                      disabled={isStopping}
                    >
                      {isStopping ? '停止中...' : '停止'}
                    </button>
                  }
                >
                  <div className="text-sm text-low">
                    工作执行中，请稍等... 已用{elapsedSeconds}秒
                  </div>
                </ChatEntryContainer>
              </div>
            );
          })}

          {/* Streaming runs */}
          {Object.entries(streamingRuns).map(([runId, run]) => {
            const agentName = agentById.get(run.agentId)?.name ?? 'Agent';
            const tone = getMessageTone(run.agentId, false);
            const sessionAgent = sessionAgents.find(
              (sa) => sa.agent_id === run.agentId
            );
            const isStopping = stoppingAgents.has(run.agentId);
            return (
              <div key={`stream-${runId}`} className="flex justify-start">
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
                    <div className="flex items-center gap-base text-xs text-low">
                      <span className="flex items-center gap-half">
                        <span>工作执行中，请稍等</span>
                        <span className="flex items-center gap-[2px]">
                          <span className="size-dot rounded-full bg-brand animate-running-dot-1" />
                          <span className="size-dot rounded-full bg-brand animate-running-dot-2" />
                          <span className="size-dot rounded-full bg-brand animate-running-dot-3" />
                        </span>
                      </span>
                      {sessionAgent && (
                        <button
                          type="button"
                          className={cn(
                            'text-error hover:text-error/80',
                            isStopping && 'opacity-50 cursor-not-allowed'
                          )}
                          onClick={() =>
                            handleStopAgent(sessionAgent.id, run.agentId)
                          }
                          disabled={isStopping}
                        >
                          {isStopping ? '停止中...' : '停止'}
                        </button>
                      )}
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

        {/* Input Area Resize Handle */}
        <div
          className="h-1 cursor-row-resize hover:bg-brand/50 active:bg-brand transition-colors shrink-0 border-t border-border"
          onMouseDown={(e) => handleResizeStart('input', e)}
        />

        {/* Message Input */}
        <div className="p-base space-y-base shrink-0">
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

          {replyToMessage && (
            <div className="border border-border rounded-sm bg-secondary/60 px-base py-half text-xs text-low">
              <div className="flex items-center justify-between gap-base">
                <span className="font-medium text-normal">
                  Replying to {getMessageSenderLabel(replyToMessage)}
                </span>
                <button
                  type="button"
                  className="text-brand hover:text-brand-hover"
                  onClick={() => setReplyToMessage(null)}
                >
                  Cancel
                </button>
              </div>
              <div className="mt-half">{getReferencePreview(replyToMessage)}</div>
            </div>
          )}

          {attachmentError && (
            <div className="text-xs text-error">{attachmentError}</div>
          )}

          <div className="relative flex-1">
            <textarea
              ref={inputRef}
              value={draft}
              onChange={(event) => handleDraftChange(event.target.value)}
              onKeyDown={(event) => {
                // First check if mention suggestions should handle the key
                if (handleMentionKeyDown(event)) {
                  return;
                }
                // Otherwise handle send on Enter
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
              disabled={isArchived || !activeSessionId}
              style={{ height: inputAreaHeight }}
              className={cn(
                'w-full resize-none rounded-sm border border-border bg-panel',
                'px-base py-base text-sm text-normal leading-relaxed focus:outline-none focus:ring-1 focus:ring-brand',
                isArchived && 'opacity-60 cursor-not-allowed'
              )}
            />
            {mentionQuery !== null && visibleMentionSuggestions.length > 0 && (
              <div className="absolute z-20 left-0 right-0 bottom-full mb-half bg-panel border border-border rounded-sm shadow">
                {visibleMentionSuggestions.map((agent, index) => (
                  <button
                    key={agent.id}
                    type="button"
                    onClick={() => handleMentionSelect(agent.name)}
                    className={cn(
                      'w-full px-base py-half text-left text-sm text-normal',
                      'flex items-center justify-between',
                      index === highlightedMentionIndex
                        ? 'bg-secondary'
                        : 'hover:bg-secondary'
                    )}
                  >
                    <span>@{agent.name}</span>
                    <CaretRightIcon className="size-icon-xs text-low" />
                  </button>
                ))}
              </div>
            )}
          </div>

          <div className="flex items-center justify-between gap-base">
            <div className="flex items-center gap-half text-xs text-low">
              <button
                type="button"
                className={cn(
                  'flex items-center justify-center rounded-sm border border-border bg-panel px-2 py-1',
                  'text-low hover:text-normal hover:border-border/80',
                  (isArchived || !activeSessionId || isUploadingAttachments) &&
                    'pointer-events-none opacity-50'
                )}
                onClick={() => fileInputRef.current?.click()}
                disabled={isArchived || !activeSessionId || isUploadingAttachments}
              >
                <PaperclipIcon className="size-icon-xs" />
              </button>
              <input
                ref={fileInputRef}
                type="file"
                multiple
                className="hidden"
                onChange={handleAttachmentInputChange}
              />
              <span>
                {isUploadingAttachments
                  ? 'Uploading attachments...'
                  : 'Press Enter to send, Shift+Enter for new line.'}
              </span>
            </div>
            <PrimaryButton
              value="Send"
              actionIcon={sendMessage.isPending ? 'spinner' : PaperPlaneRightIcon}
              onClick={handleSend}
              disabled={!canSend}
            />
          </div>
        </div>
      </section>

      {/* Right Sidebar Resize Handle */}
      <div
        className="w-1 cursor-col-resize hover:bg-brand/50 active:bg-brand transition-colors shrink-0"
        onMouseDown={(e) => handleResizeStart('right', e)}
      />

      {/* AI Members Sidebar */}
      <aside
        className="border-l border-border flex flex-col min-h-0 shrink-0"
        style={{ width: rightSidebarWidth }}
      >
        <div className="px-base py-base border-b border-border flex items-center justify-between">
          <div className="text-sm text-normal font-medium">AI Members</div>
          <div className="text-xs text-low">{sessionMembers.length} in session</div>
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto p-base space-y-base">
          {!activeSessionId && (
            <div className="text-xs text-low mt-base">
              Select a session to manage AI members.
            </div>
          )}
          {activeSessionId && sessionMembers.length === 0 && (
            <div className="text-xs text-low mt-base">
              No AI members yet. Add one below to enable @mentions.
            </div>
          )}
          {sessionMembers.map(({ agent, sessionAgent }) => {
            const state = agentStates[agent.id] ?? ChatSessionAgentState.idle;
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
                        state === ChatSessionAgentState.running && 'animate-pulse'
                      )}
                    />
                    <div className="min-w-0 flex-1 overflow-hidden">
                      <div className="text-sm text-normal truncate">
                        @{agent.name}
                      </div>
                      {(() => {
                        const modelName = getModelName(agent.runner_type);
                        const fullText = `${toPrettyCase(agent.runner_type)} · ${agentStateLabels[state]}${modelName ? ` · ${modelName}` : ''}`;
                        return (
                          <Tooltip content={fullText} side="bottom">
                            <div className="text-xs text-low truncate cursor-default">
                              {fullText}
                            </div>
                          </Tooltip>
                        );
                      })()}
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

          {/* Add Member Form */}
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
                  setIsPromptEditorOpen(false);
                  setPromptFileError(null);
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
                  <label className="text-xs text-low">Base coding agent</label>
                  <select
                    value={newMemberRunnerType}
                    onChange={(event) => setNewMemberRunnerType(event.target.value)}
                    disabled={
                      isCheckingAvailability || enabledRunnerTypes.length === 0
                    }
                    className={cn(
                      'w-full rounded-sm border border-border bg-panel px-base py-half',
                      'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                    )}
                  >
                    {enabledRunnerTypes.length === 0 && (
                      <option value="">
                        {isCheckingAvailability
                          ? 'Checking agents...'
                          : 'No local agents detected'}
                      </option>
                    )}
                    {availableRunnerTypes.map((runner) => (
                      <option
                        key={runner}
                        value={runner}
                        disabled={!isRunnerAvailable(runner)}
                      >
                        {toPrettyCase(runner)}
                        {availabilityLabel(runner)}
                      </option>
                    ))}
                  </select>
                  {enabledRunnerTypes.length === 0 && !isCheckingAvailability && (
                    <div className="text-xs text-error">
                      No installed code agents detected on this machine.
                    </div>
                  )}
                </div>
                {memberVariantOptions.length > 0 && (
                  <div className="space-y-half">
                    <label className="text-xs text-low">Model variant</label>
                    <select
                      value={newMemberVariant}
                      onChange={(event) => setNewMemberVariant(event.target.value)}
                      className={cn(
                        'w-full rounded-sm border border-border bg-panel px-base py-half',
                        'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                      )}
                    >
                      {memberVariantOptions.map((variant) => {
                        const modelName = getModelName(newMemberRunnerType, variant);
                        return (
                          <option key={variant} value={variant}>
                            {variant}{modelName ? ` (${modelName})` : ''}
                          </option>
                        );
                      })}
                    </select>
                    {getModelName(newMemberRunnerType, newMemberVariant) && (
                      <div className="text-xs text-low">
                        Model: {getModelName(newMemberRunnerType, newMemberVariant)}
                      </div>
                    )}
                  </div>
                )}
                <div className="space-y-half">
                  <div className="flex items-center justify-between gap-base">
                    <label className="text-xs text-low">System prompt</label>
                    <button
                      type="button"
                      className="text-xs text-brand hover:text-brand-hover"
                      onClick={() => {
                        setIsPromptEditorOpen(true);
                        setPromptFileError(null);
                      }}
                    >
                      Expand
                    </button>
                  </div>
                  <textarea
                    value={newMemberPrompt}
                    onChange={(event) => setNewMemberPrompt(event.target.value)}
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
                    onChange={(event) => setNewMemberWorkspace(event.target.value)}
                    placeholder="Absolute path on the server"
                    disabled={!!editingMember}
                    title={editingMember ? 'Workspace path cannot be changed after creation' : undefined}
                    className={cn(
                      'w-full rounded-sm border border-border bg-panel px-base py-half',
                      'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand',
                      editingMember && 'opacity-50 cursor-not-allowed'
                    )}
                  />
                  {editingMember && (
                    <p className="text-xs text-low">Workspace path cannot be modified</p>
                  )}
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
                      setIsPromptEditorOpen(false);
                      setPromptFileError(null);
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

      {/* Workspace Drawer */}
      <div
        className={cn(
          'absolute top-0 right-0 h-full w-[360px] bg-primary border-l border-border shadow-lg transition-transform z-50 flex flex-col',
          workspaceDrawerOpen ? 'translate-x-0' : 'translate-x-full'
        )}
      >
        <div className="px-base py-base border-b border-border shrink-0 flex items-center justify-between">
          <div className="text-sm text-normal font-medium">
            {activeWorkspaceAgent?.name ?? 'Agent workspace'}
          </div>
          <button
            type="button"
            onClick={() => setWorkspaceDrawerOpen(false)}
            className="text-low hover:text-normal transition-colors"
            aria-label="Close workspace drawer"
            title="Close"
          >
            <XIcon className="size-icon-sm" />
          </button>
        </div>
        <div className="p-base space-y-base overflow-y-auto flex-1 min-h-0">
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
            <div className="text-sm text-normal font-medium">Run history</div>
            {activeWorkspaceRuns.length === 0 && (
              <div className="text-xs text-low">No runs yet for this agent.</div>
            )}
            {activeWorkspaceRuns.map((run: RunHistoryItem) => (
              <div
                key={run.runId}
                className="border border-border rounded-sm p-base space-y-half"
              >
                <div className="flex items-center justify-between text-xs text-low">
                  <span>Run {run.runId.slice(0, 8)}</span>
                  <span>{formatDateShortWithTime(run.createdAt)}</span>
                </div>
                <div className="text-xs text-normal">{run.content}</div>
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
                {logError && <div className="text-xs text-error">{logError}</div>}
                {!logLoading && !logError && (
                  <div className="max-h-64 overflow-y-auto border-t border-border pt-base">
                    {logContent ? (
                      <RawLogText content={logContent} />
                    ) : (
                      <div className="text-xs text-low">Log is empty.</div>
                    )}
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Diff Viewer Modal */}
      {diffViewerOpen && diffViewerRunId && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
          onClick={handleCloseDiffViewer}
        >
          <div
            className={cn(
              'bg-primary border border-border shadow-xl flex flex-col overflow-hidden',
              diffViewerFullscreen
                ? 'w-full h-full rounded-none'
                : 'w-[92vw] h-[85vh] max-w-[1200px] rounded-xl'
            )}
            onClick={(event) => event.stopPropagation()}
          >
            <div className="flex items-center justify-between border-b border-border px-base py-half">
              <div>
                <div className="text-sm text-normal font-medium">Code changes</div>
                <div className="text-xs text-low">
                  Run {diffViewerRunId.slice(0, 8)}
                </div>
              </div>
              <div className="flex items-center gap-half">
                <button
                  type="button"
                  className="text-low hover:text-normal"
                  onClick={handleToggleFullscreen}
                  aria-label={
                    diffViewerFullscreen ? 'Exit full screen' : 'Full screen'
                  }
                >
                  <DiffViewerIcon className="size-icon-sm" />
                </button>
                <button
                  type="button"
                  className="text-low hover:text-normal"
                  onClick={handleCloseDiffViewer}
                  aria-label="Close diff viewer"
                >
                  <XIcon className="size-icon-sm" />
                </button>
              </div>
            </div>
            <div className="flex-1 overflow-y-auto p-base space-y-base">
              {diffViewerHasDiff ? (
                <>
                  {diffViewerRun?.loading && (
                    <div className="text-xs text-low">Loading diff...</div>
                  )}
                  {diffViewerRun?.error && (
                    <div className="text-xs text-error">{diffViewerRun.error}</div>
                  )}
                  {!diffViewerRun?.loading &&
                    !diffViewerRun?.error &&
                    diffViewerRun?.files?.length === 0 && (
                      <div className="text-xs text-low">
                        No tracked diff available.
                      </div>
                    )}
                  {diffViewerRun?.files?.map((file) => (
                    <div
                      key={`${diffViewerRunId}-${file.path}`}
                      className="border border-border rounded-sm bg-panel"
                    >
                      <div className="flex items-center justify-between px-base py-half text-xs text-normal border-b border-border">
                        <span className="font-ibm-plex-mono break-all">
                          {file.path}
                        </span>
                        <span className="text-xs text-low">
                          {file.additions > 0 && (
                            <span className="text-success">+{file.additions}</span>
                          )}
                          {file.additions > 0 && file.deletions > 0 && ' '}
                          {file.deletions > 0 && (
                            <span className="text-error">-{file.deletions}</span>
                          )}
                        </span>
                      </div>
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
                  ))}
                  <div>
                    <button
                      type="button"
                      className="text-brand hover:text-brand-hover text-xs"
                      onClick={() =>
                        window.open(
                          chatApi.getRunDiffUrl(diffViewerRunId),
                          '_blank',
                          'noopener,noreferrer'
                        )
                      }
                    >
                      Open raw diff
                    </button>
                  </div>
                </>
              ) : (
                <div className="text-xs text-low">No tracked diff available.</div>
              )}
              {diffViewerUntracked.length > 0 && (
                <div className="space-y-half">
                  <div className="text-xs text-low">Untracked files</div>
                  {diffViewerUntracked.map((path) => {
                    const key = `${diffViewerRunId}:${path}`;
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
                              handleToggleUntracked(diffViewerRunId, path)
                            }
                          >
                            {entry?.open ? 'Hide' : 'View'}
                          </button>
                        </div>
                        {entry?.open && (
                          <div className="mt-half">
                            {entry.loading && (
                              <div className="text-xs text-low">Loading file...</div>
                            )}
                            {entry.error && (
                              <div className="text-xs text-error">{entry.error}</div>
                            )}
                            {!entry.loading && !entry.error && entry.content && (
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
            </div>
          </div>
        </div>
      )}

      {/* Prompt Editor Modal */}
      {isPromptEditorOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
          onClick={() => setIsPromptEditorOpen(false)}
        >
          <div
            className="bg-primary border border-border shadow-xl flex flex-col overflow-hidden w-[92vw] h-[80vh] max-w-[1200px] rounded-xl"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="flex items-center justify-between border-b border-border px-base py-half">
              <div>
                <div className="text-sm text-normal font-medium">System Prompt</div>
                <div className="text-xs text-low">Edit the AI member system prompt</div>
              </div>
              <button
                type="button"
                className="text-low hover:text-normal"
                onClick={() => setIsPromptEditorOpen(false)}
                aria-label="Close prompt editor"
              >
                <XIcon className="size-icon-sm" />
              </button>
            </div>
            <div className="flex-1 overflow-hidden p-base flex flex-col gap-base">
              <textarea
                value={newMemberPrompt}
                onChange={(event) => setNewMemberPrompt(event.target.value)}
                placeholder="Describe how this AI member should behave."
                className={cn(
                  'flex-1 w-full resize-none rounded-sm border border-border bg-panel',
                  'px-base py-base text-sm text-normal leading-relaxed focus:outline-none focus:ring-1 focus:ring-brand'
                )}
              />
              <div className="flex items-center justify-between gap-base">
                <div className="flex items-center gap-half text-xs text-low">
                  <button
                    type="button"
                    className="text-brand hover:text-brand-hover"
                    onClick={() => promptFileInputRef.current?.click()}
                    disabled={promptFileLoading}
                  >
                    Attach text file
                  </button>
                  <input
                    ref={promptFileInputRef}
                    type="file"
                    accept=".txt,.md,.prompt,text/plain"
                    className="hidden"
                    onChange={handlePromptFileChange}
                  />
                  {promptFileLoading && <span>Loading file...</span>}
                  {promptFileError && (
                    <span className="text-error">{promptFileError}</span>
                  )}
                </div>
                <PrimaryButton
                  value="Done"
                  onClick={() => setIsPromptEditorOpen(false)}
                />
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Confirm Modal */}
      <Dialog
        open={!!confirmModal}
        onOpenChange={(open) => {
          if (!open && !isConfirmLoading) {
            setConfirmModal(null);
          }
        }}
      >
        <DialogHeader>
          <DialogTitle>{confirmModal?.title ?? 'Confirm'}</DialogTitle>
        </DialogHeader>
        <DialogContent>
          <p className="text-sm text-normal">{confirmModal?.message}</p>
        </DialogContent>
        <DialogFooter>
          <PrimaryButton
            variant="tertiary"
            value="Cancel"
            onClick={() => setConfirmModal(null)}
            disabled={isConfirmLoading}
          />
          <PrimaryButton
            variant="default"
            value={isConfirmLoading ? 'Processing...' : 'Confirm'}
            onClick={async () => {
              if (!confirmModal) return;
              setIsConfirmLoading(true);
              try {
                await confirmModal.onConfirm();
              } finally {
                setIsConfirmLoading(false);
                setConfirmModal(null);
              }
            }}
            disabled={isConfirmLoading}
          />
        </DialogFooter>
      </Dialog>
    </div>
  );
}
