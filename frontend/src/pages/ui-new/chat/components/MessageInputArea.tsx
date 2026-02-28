import type { ChangeEvent, RefObject } from 'react';
import {
  CaretRightIcon,
  ChatsTeardropIcon,
  PaperclipIcon,
  PaperPlaneRightIcon,
  XIcon,
  EyeIcon,
} from '@phosphor-icons/react';
import { useTranslation } from 'react-i18next';
import type { ChatAgent, ChatMessage } from 'shared/types';
import { cn } from '@/lib/utils';
import { Badge } from '@/components/ui/badge';
import { PrimaryButton } from '@/components/ui-new/primitives/PrimaryButton';
import { MultiSelectDropdown } from '@/components/ui-new/primitives/MultiSelectDropdown';
import { Tooltip } from '@/components/ui-new/primitives/Tooltip';
import {
  AgentBrandIcon,
  getAgentAvatarSeed,
  getAgentAvatarStyle,
} from '../AgentAvatar';
import { mentionAllKeyword } from '../constants';

const allowedAttachmentExtensions = [
  '.txt', '.csv', '.md', '.json', '.xml', '.yaml', '.yml',
  '.html', '.htm', '.css', '.js', '.ts', '.jsx', '.tsx',
  '.py', '.java', '.c', '.cpp', '.h', '.hpp', '.rb', '.php',
  '.go', '.rs', '.sql', '.sh', '.bash', '.svg',
];

const isTextAttachment = (file: File) =>
  file.type.startsWith('text/') ||
  allowedAttachmentExtensions.some((ext) =>
    file.name.toLowerCase().endsWith(ext)
  );

const isImageAttachment = (file: File) => file.type.startsWith('image/');

export const isAllowedAttachment = (file: File) =>
  isTextAttachment(file) || isImageAttachment(file);

export interface MessageInputAreaProps {
  // Input state
  draft: string;
  onDraftChange: (value: string) => void;
  inputRef: RefObject<HTMLTextAreaElement>;
  // Mentions
  selectedMentions: string[];
  onSelectedMentionsChange: React.Dispatch<
    React.SetStateAction<string[]>
  >;
  agentOptions: { value: string; label: string }[];
  mentionAgentsCount: number;
  mentionQuery: string | null;
  showMentionAllSuggestion: boolean;
  visibleMentionSuggestions: ChatAgent[];
  highlightedMentionIndex: number;
  onMentionSelect: (name: string) => void;
  onMentionKeyDown: (event: React.KeyboardEvent) => boolean;
  // Reply
  replyToMessage: ChatMessage | null;
  replyToSenderLabel: string | null;
  replyToPreview: string | null;
  onCancelReply: () => void;
  // Attachments
  attachedFiles: File[];
  attachmentError: string | null;
  isUploadingAttachments: boolean;
  onAttachmentInputChange: (
    event: ChangeEvent<HTMLInputElement>
  ) => void;
  onRemoveAttachedFile: (name: string, size: number) => void;
  onClearAttachedFiles: () => void;
  onPreviewFile: (file: File) => void;
  fileInputRef: RefObject<HTMLInputElement>;
  // Send
  canSend: boolean;
  isSending: boolean;
  onSend: () => void;
  // Layout
  inputAreaHeight: number;
  // State
  isArchived: boolean;
  activeSessionId: string | null;
}

export function MessageInputArea({
  draft,
  onDraftChange,
  inputRef,
  selectedMentions,
  onSelectedMentionsChange,
  agentOptions,
  mentionAgentsCount,
  mentionQuery,
  showMentionAllSuggestion,
  visibleMentionSuggestions,
  highlightedMentionIndex,
  onMentionSelect,
  onMentionKeyDown,
  replyToMessage,
  replyToSenderLabel,
  replyToPreview,
  onCancelReply,
  attachedFiles,
  attachmentError,
  isUploadingAttachments,
  onAttachmentInputChange,
  onRemoveAttachedFile,
  onClearAttachedFiles,
  onPreviewFile,
  fileInputRef,
  canSend,
  isSending,
  onSend,
  inputAreaHeight,
  isArchived,
  activeSessionId,
}: MessageInputAreaProps) {
  const { t } = useTranslation('chat');
  const { t: tCommon } = useTranslation('common');
  const mentionSuggestionEntries = [
    ...(showMentionAllSuggestion ? [{ type: 'all' as const }] : []),
    ...visibleMentionSuggestions.map((agent) => ({
      type: 'agent' as const,
      agent,
    })),
  ];

  return (
    <div className="chat-session-input-area p-base space-y-base shrink-0">
      <div className="chat-session-mention-bar flex items-center gap-base flex-wrap">
        <MultiSelectDropdown
          icon={ChatsTeardropIcon}
          label={t('input.mentionAgents')}
          menuLabel={t('input.routeToAgents')}
          values={selectedMentions}
          options={agentOptions}
          onChange={onSelectedMentionsChange}
          triggerClassName="chat-session-mention-trigger !bg-[#cbcbd1] hover:!bg-[#cbcbd1] data-[state=open]:!bg-[#cbcbd1]"
          menuContentClassName="chat-session-mention-menu !bg-[#ecedf1]"
          disabled={
            !activeSessionId ||
            mentionAgentsCount === 0 ||
            isArchived
          }
        />
        {selectedMentions.length > 0 && (
          <div className="chat-session-selected-mentions flex items-center gap-half flex-wrap">
            {selectedMentions.map((mention) => (
              <Badge
                key={mention}
                variant="secondary"
                className="chat-session-selected-mention flex items-center gap-half px-2 py-0.5"
              >
                {mention === mentionAllKeyword
                  ? t('input.mentionAllBadge')
                  : `@${mention}`}
                <button
                  type="button"
                  onClick={() =>
                    onSelectedMentionsChange((prev) =>
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
        <div className="chat-session-reply-card border border-border rounded-sm bg-secondary/60 px-base py-half text-xs text-low">
          <div className="flex items-center justify-between gap-base">
            <span className="font-medium text-normal">
              {t('input.replyingTo', { name: replyToSenderLabel ?? 'message' })}
            </span>
            <button
              type="button"
              className="chat-session-reply-cancel text-brand hover:text-brand-hover"
              onClick={onCancelReply}
            >
              {tCommon('buttons.cancel')}
            </button>
          </div>
          <div className="mt-half">
            {replyToPreview ?? t('input.referencedMessage')}
          </div>
        </div>
      )}

      {/* Display attached files */}
      {attachedFiles.length > 0 && (
        <div className="chat-session-attachments flex flex-wrap gap-2 p-2 border border-border rounded-sm bg-secondary/40">
          {attachedFiles.map((file, index) => (
            <div
              key={`${file.name}-${file.size}-${index}`}
              className="chat-session-attachment-item flex items-center gap-1 bg-panel border border-border rounded px-2 py-1 text-xs"
            >
              <PaperclipIcon className="size-icon-2xs text-low" />
              <span
                className="max-w-[120px] truncate"
                title={file.name}
              >
                {file.name}
              </span>
              {isAllowedAttachment(file) && (
                <button
                  type="button"
                  className="text-low hover:text-normal"
                  onClick={() => onPreviewFile(file)}
                  title={t('input.preview')}
                >
                  <EyeIcon className="size-icon-2xs" />
                </button>
              )}
              <button
                type="button"
                className="text-low hover:text-normal"
                onClick={() =>
                  onRemoveAttachedFile(file.name, file.size)
                }
              >
                <XIcon className="size-icon-2xs" />
              </button>
            </div>
          ))}
          <button
            type="button"
            className="chat-session-attachment-clear text-xs text-brand hover:text-brand-hover ml-1"
            onClick={onClearAttachedFiles}
          >
            {t('input.clearAll')}
          </button>
        </div>
      )}

      {attachmentError && (
        <div className="text-xs text-error">{attachmentError}</div>
      )}

      <div className="chat-session-input-editor relative flex-1">
        <textarea
          ref={inputRef}
          value={draft}
          onChange={(event) => onDraftChange(event.target.value)}
          onKeyDown={(event) => {
            if (onMentionKeyDown(event)) {
              return;
            }
            if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault();
              onSend();
            }
          }}
          placeholder={
            isArchived
              ? t('input.archivedPlaceholder')
              : t('input.inputPlaceholder')
          }
          disabled={isArchived || !activeSessionId}
          style={{ height: inputAreaHeight }}
          className={cn(
            'chat-session-textarea w-full resize-none',
            'px-base py-base text-sm text-normal leading-relaxed focus:outline-none',
            isArchived && 'opacity-60 cursor-not-allowed'
          )}
        />
        {mentionQuery !== null &&
          mentionSuggestionEntries.length > 0 && (
            <div className="chat-session-mention-suggestions absolute z-20 left-0 right-0 bottom-full mb-half border border-border rounded-sm shadow">
              {mentionSuggestionEntries.map((entry, index) => (
                <button
                  key={
                    entry.type === 'all'
                      ? '__mention_all__'
                      : entry.agent.id
                  }
                  type="button"
                  onClick={() =>
                    onMentionSelect(
                      entry.type === 'all'
                        ? mentionAllKeyword
                        : entry.agent.name
                    )
                  }
                  className={cn(
                    'chat-session-mention-option w-full px-base py-half text-left text-sm',
                    'flex items-center justify-between',
                    index === highlightedMentionIndex
                      ? 'bg-[#d5d5dc] text-normal'
                      : 'text-normal hover:bg-[#d5d5dc]'
                  )}
                >
                  <span className="flex items-center gap-half min-w-0">
                    {entry.type === 'all' ? (
                      <>
                        <span className="chat-session-mention-avatar bg-panel border border-border text-xs font-semibold flex items-center justify-center">
                          @
                        </span>
                        <span className="truncate">
                          {t('input.mentionAllSuggestion')}
                        </span>
                      </>
                    ) : (
                      <>
                        <span
                          className="chat-session-mention-avatar"
                          style={getAgentAvatarStyle(
                            getAgentAvatarSeed(
                              entry.agent.id,
                              entry.agent.runner_type,
                              entry.agent.name
                            )
                          )}
                        >
                          <AgentBrandIcon
                            runnerType={entry.agent.runner_type}
                            className="chat-session-mention-avatar-logo"
                          />
                        </span>
                        <span className="truncate">@{entry.agent.name}</span>
                      </>
                    )}
                  </span>
                  <CaretRightIcon
                    className={cn(
                      'size-icon-xs',
                      index === highlightedMentionIndex
                        ? 'text-on-brand'
                        : 'text-low'
                    )}
                  />
                </button>
              ))}
            </div>
          )}
      </div>

      <div className="flex items-center justify-between gap-base">
        <div className="chat-session-input-hint flex items-center gap-half text-xs text-low">
          <Tooltip content={t('input.addAttachment')} side="top">
            <span className="inline-flex">
              <button
                type="button"
                className={cn(
                  'chat-session-attach-btn flex items-center justify-center rounded-sm border border-border bg-panel px-2 py-1',
                  'hover:border-border/80',
                  (isArchived ||
                    !activeSessionId ||
                    isUploadingAttachments) &&
                    'pointer-events-none opacity-50'
                )}
                onClick={() => fileInputRef.current?.click()}
                disabled={
                  isArchived ||
                  !activeSessionId ||
                  isUploadingAttachments
                }
                aria-label={t('input.addAttachment')}
              >
                <PaperclipIcon className="size-icon-xs" />
              </button>
            </span>
          </Tooltip>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            className="hidden"
            onChange={onAttachmentInputChange}
          />
          <span>
            {isUploadingAttachments
              ? t('input.uploadingAttachments')
              : t('input.sendHint')}
          </span>
        </div>
        <PrimaryButton
          value={tCommon('buttons.send')}
          actionIcon={
            isSending ? 'spinner' : PaperPlaneRightIcon
          }
          onClick={onSend}
          disabled={!canSend}
          className="chat-session-send-btn"
        />
      </div>
    </div>
  );
}
