import {
  CheckCircleIcon,
  XCircleIcon,
  PaperPlaneTiltIcon,
  CheckSquareIcon,
  SquareIcon,
} from '@phosphor-icons/react';
import {
  type ChatMessage,
  ChatSenderType,
  ChatSessionAgentState,
} from 'shared/types';
import { cn } from '@/lib/utils';
import { Badge } from '@/components/ui/badge';
import { ChatEntryContainer } from '@/components/ui-new/primitives/conversation/ChatEntryContainer';
import { ChatMarkdown } from '@/components/ui-new/primitives/conversation/ChatMarkdown';
import { ChatSystemMessage } from '@/components/ui-new/primitives/conversation/ChatSystemMessage';
import { chatApi } from '@/lib/api';
import { formatDateShortWithTime } from '@/utils/date';
import type {
  ChatAttachment,
  DiffMeta,
  MentionStatus,
  MessageTone,
  RunDiffState,
} from '../types';
import {
  extractAttachments,
  detectApiError,
  formatBytes,
} from '../utils';

export interface ChatMessageItemProps {
  message: ChatMessage;
  senderLabel: string;
  tone: MessageTone;
  // Reference/reply
  referenceMessage: ChatMessage | null;
  referenceSenderLabel: string | null;
  referencePreview: string | null;
  // Mentions
  mentionList: string[];
  mentionStatusMap: Map<string, MentionStatus> | undefined;
  agentStates: Record<string, ChatSessionAgentState>;
  agentIdByName: Map<string, string>;
  // Attachments
  attachments: ChatAttachment[];
  activeSessionId: string | null;
  onAddAttachmentAsFile: (
    messageId: string,
    attachment: ChatAttachment
  ) => void;
  // Diff
  diffInfo: DiffMeta | null;
  runDiffs: Record<string, RunDiffState>;
  onOpenDiffViewer: (
    runId: string,
    untracked: string[],
    hasDiff: boolean
  ) => void;
  // Interaction
  isArchived: boolean;
  onReply: (message: ChatMessage) => void;
  // Cleanup mode
  isCleanupMode: boolean;
  isSelected: boolean;
  onToggleSelect: () => void;
}

export function ChatMessageItem({
  message,
  senderLabel,
  tone,
  referenceMessage,
  referenceSenderLabel,
  referencePreview,
  mentionList,
  mentionStatusMap,
  agentStates,
  agentIdByName,
  attachments,
  activeSessionId,
  onAddAttachmentAsFile,
  diffInfo,
  runDiffs,
  onOpenDiffViewer,
  isArchived,
  onReply,
  isCleanupMode,
  isSelected,
  onToggleSelect,
}: ChatMessageItemProps) {
  const isUser = message.sender_type === ChatSenderType.user;
  const isAgent = message.sender_type === ChatSenderType.agent;

  const diffRunId = diffInfo?.runId ?? '';
  const hasDiffInfo =
    !!diffInfo &&
    diffRunId.length > 0 &&
    (diffInfo.available || diffInfo.untrackedFiles.length > 0);
  const referenceId =
    referenceMessage?.id ??
    (message.meta &&
    typeof message.meta === 'object' &&
    !Array.isArray(message.meta) &&
    (message.meta as { reference?: { message_id?: string } }).reference
      ?.message_id) ??
    null;

  // System messages
  if (message.sender_type === ChatSenderType.system) {
    return (
      <div className="chat-session-message-row is-system flex items-start gap-base">
        {isCleanupMode && (
          <button
            type="button"
            className="flex-shrink-0 mt-1"
            onClick={onToggleSelect}
          >
            {isSelected ? (
              <CheckSquareIcon
                className="size-icon text-brand"
                weight="fill"
              />
            ) : (
              <SquareIcon className="size-icon text-low" />
            )}
          </button>
        )}
        <div className="flex-1">
          <ChatSystemMessage content={message.content} expanded />
        </div>
      </div>
    );
  }

  return (
    <div
      id={`chat-message-${message.id}`}
      className={cn(
        'chat-session-message-row flex items-start gap-base',
        isUser ? 'is-user justify-end' : 'is-agent justify-start'
      )}
    >
      {isCleanupMode && !isUser && (
        <button
          type="button"
          className="flex-shrink-0 mt-1"
          onClick={onToggleSelect}
        >
          {isSelected ? (
            <CheckSquareIcon
              className="size-icon text-brand"
              weight="fill"
            />
          ) : (
            <SquareIcon className="size-icon text-low" />
          )}
        </button>
      )}
      <ChatEntryContainer
        variant={isUser ? 'user' : 'system'}
        title={senderLabel}
        expanded
        iconContainerClassName={cn(
          'chat-session-message-avatar',
          isUser ? 'is-user' : 'is-agent'
        )}
        iconClassName="chat-session-message-avatar-icon"
        headerRight={
          <div className="chat-session-message-meta flex items-center gap-half text-xs text-low">
            <button
              type="button"
              className={cn(
                'chat-session-message-reply text-brand hover:text-brand-hover',
                isArchived && 'pointer-events-none opacity-50'
              )}
              onClick={() => onReply(message)}
              disabled={isArchived}
            >
              Quote
            </button>
            <span>{formatDateShortWithTime(message.created_at)}</span>
          </div>
        }
        className={cn(
          'chat-session-message-card max-w-[760px] w-full md:w-[84%] shadow-sm rounded-2xl',
          isUser && 'chat-session-message-card-self ml-auto',
          !isUser && 'chat-session-message-card-agent',
          isCleanupMode && isSelected && 'ring-2 ring-brand'
        )}
        headerClassName="chat-session-message-header"
        bodyClassName="chat-session-message-body"
        style={{
          backgroundColor: isUser
            ? `var(--chat-session-message-self-bg, ${tone.bg})`
            : `var(--chat-session-message-other-bg, ${tone.bg})`,
          borderColor: isUser
            ? `var(--chat-session-message-self-border, ${tone.border})`
            : `var(--chat-session-message-other-border, ${tone.border})`,
        }}
      >
        <div>
          {referenceId && (
            <div className="chat-session-reference-card mb-half border border-border rounded-sm bg-secondary/60 px-base py-half text-xs text-low">
              <div className="flex items-center justify-between gap-base">
                <span className="font-medium text-normal">
                  Replying to {referenceSenderLabel ?? 'message'}
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
                {referencePreview ?? 'Referenced message unavailable.'}
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
            const apiError = isAgent
              ? detectApiError(message.content)
              : null;
            return apiError ? (
              <div className="mb-half flex items-center gap-half rounded-sm bg-error/10 border border-error/30 px-base py-half text-xs text-error">
                <XCircleIcon
                  className="size-icon-sm flex-shrink-0"
                  weight="fill"
                />
                <span className="font-medium">{apiError.message}</span>
                <span className="text-error/70">
                  - Please check your API quota or try again later
                </span>
              </div>
            ) : null;
          })()}
          <ChatMarkdown content={message.content} />
          {mentionList.length > 0 && (
            <div className="chat-session-mentions mt-half flex flex-wrap items-center gap-half text-xs text-low">
              <span>Mentions:</span>
              {mentionList.map((mention) => {
                const agentId = agentIdByName.get(mention);
                const mentionStatus = mentionStatusMap?.get(mention);
                const isFallbackRunning =
                  !mentionStatusMap &&
                  !!agentId &&
                  agentStates[agentId] ===
                    ChatSessionAgentState.running;
                const isRunning =
                  mentionStatus === 'running' ||
                  mentionStatus === 'received' ||
                  isFallbackRunning;
                const isCompleted = mentionStatus === 'completed';
                const isFailed = mentionStatus === 'failed';
                const showCheck =
                  !isFailed && (isRunning || isCompleted);
                const pulse = mentionStatus === 'running';
                return (
                  <Badge
                    key={`${message.id}-mention-${mention}`}
                    variant="secondary"
                    className="chat-session-mention-tag flex items-center gap-1 px-2 py-0.5 text-xs"
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
            <div className="chat-session-attachments mt-half space-y-half">
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
                    className="chat-session-attachment-card border border-border rounded-sm bg-panel px-base py-half text-xs text-normal"
                  >
                    <div className="flex items-center justify-between gap-base">
                      <span className="font-ibm-plex-mono break-all">
                        {attachment.name}
                      </span>
                      <div className="flex gap-half">
                        <button
                          type="button"
                          className="text-brand hover:text-brand-hover px-1 py-0.5"
                          onClick={(e) => {
                            e.stopPropagation();
                            onAddAttachmentAsFile(
                              message.id,
                              attachment
                            );
                          }}
                          title="Send to agent"
                        >
                          <PaperPlaneTiltIcon className="size-icon-sm" />
                        </button>
                        <a
                          className="text-brand hover:text-brand-hover"
                          href={attachmentUrl}
                          target="_blank"
                          rel="noreferrer"
                        >
                          Open
                        </a>
                      </div>
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
            <div className="chat-session-code-card mt-half border border-border rounded-sm bg-secondary/70 px-base py-half text-xs text-normal">
              <div className="chat-session-code-card-header flex items-center justify-between gap-base">
                <span className="chat-session-code-card-title">Code changes</span>
                <button
                  type="button"
                  className="chat-session-code-card-link text-brand hover:text-brand-hover"
                  onClick={() =>
                    onOpenDiffViewer(
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
                <div className="chat-session-code-card-meta mt-half text-xs text-low">
                  Loading diff...
                </div>
              )}
              {diffInfo.available && runDiffs[diffRunId]?.error && (
                <div className="chat-session-code-card-meta mt-half text-xs text-error">
                  {runDiffs[diffRunId]?.error}
                </div>
              )}
              {diffInfo.untrackedFiles.length > 0 && (
                <div className="chat-session-code-card-meta mt-half text-xs text-low">
                  {diffInfo.untrackedFiles.length} untracked file
                  {diffInfo.untrackedFiles.length === 1 ? '' : 's'}
                </div>
              )}
            </div>
          )}
        </div>
      </ChatEntryContainer>
      {isCleanupMode && isUser && (
        <button
          type="button"
          className="flex-shrink-0 mt-1"
          onClick={onToggleSelect}
        >
          {isSelected ? (
            <CheckSquareIcon
              className="size-icon text-brand"
              weight="fill"
            />
          ) : (
            <SquareIcon className="size-icon text-low" />
          )}
        </button>
      )}
    </div>
  );
}
