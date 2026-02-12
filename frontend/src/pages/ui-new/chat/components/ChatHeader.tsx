import { useEffect, useRef } from 'react';
import {
  ArrowCounterClockwiseIcon,
  BoxArrowDownIcon,
  BroomIcon,
  GearSixIcon,
  PencilSimpleIcon,
  TrashIcon,
  UsersIcon,
  XIcon,
} from '@phosphor-icons/react';
import type { ChatSession } from 'shared/types';
import { cn } from '@/lib/utils';
import { Badge } from '@/components/ui/badge';
import { PrimaryButton } from '@/components/ui-new/primitives/PrimaryButton';
import { Tooltip } from '@/components/ui-new/primitives/Tooltip';
import { formatDateShortWithTime } from '@/utils/date';

export interface ChatHeaderProps {
  activeSession: ChatSession | null;
  messageCount: number;
  memberCount: number;
  isSearchOpen: boolean;
  searchQuery: string;
  onCloseSearch: () => void;
  onSearchQueryChange: (value: string) => void;
  isArchived: boolean;
  // Title editing
  isEditingTitle: boolean;
  titleDraft: string;
  titleError: string | null;
  isSavingTitle: boolean;
  onStartEditTitle: () => void;
  onTitleDraftChange: (value: string) => void;
  onSaveTitle: () => void;
  onCancelTitleEdit: () => void;
  // Session actions
  onDeleteSession: () => void;
  onOpenSettings: () => void;
  onArchive: () => void;
  onRestore: () => void;
  isArchiving: boolean;
  // Cleanup mode
  isCleanupMode: boolean;
  onToggleCleanupMode: () => void;
  isDeletingMessages: boolean;
}

export function ChatHeader({
  activeSession,
  messageCount,
  memberCount,
  isSearchOpen,
  searchQuery,
  onCloseSearch,
  onSearchQueryChange,
  isArchived,
  isEditingTitle,
  titleDraft,
  titleError,
  isSavingTitle,
  onStartEditTitle,
  onTitleDraftChange,
  onSaveTitle,
  onCancelTitleEdit,
  onDeleteSession,
  onOpenSettings,
  onArchive,
  onRestore,
  isArchiving,
  isCleanupMode,
  onToggleCleanupMode,
  isDeletingMessages,
}: ChatHeaderProps) {
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const searchInputContainerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!isSearchOpen) return;
    const timer = window.setTimeout(() => {
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [isSearchOpen]);

  useEffect(() => {
    if (!isSearchOpen) return;

    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (!target) return;

      if (
        searchInputContainerRef.current?.contains(target)
      ) {
        return;
      }

      onCloseSearch();
    };

    document.addEventListener('mousedown', handlePointerDown);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
    };
  }, [isSearchOpen, onCloseSearch]);

  return (
    <header className="chat-session-header px-base py-half border-b border-border flex items-center justify-between">
      <div className="chat-session-header-main min-w-0">
        {!isEditingTitle && (
          <div className="chat-session-header-title-row flex items-center gap-half">
            <div className="chat-session-header-title text-sm text-normal font-medium truncate">
              {activeSession?.title || 'Untitled session'}
            </div>
            {activeSession && (
              <>
                <button
                  type="button"
                  className="chat-session-header-icon-btn"
                  onClick={onStartEditTitle}
                  aria-label="Edit session name"
                >
                  <PencilSimpleIcon className="size-icon-xs" />
                </button>
                <button
                  type="button"
                  className="chat-session-header-icon-btn danger"
                  onClick={onDeleteSession}
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
              onChange={(event) => onTitleDraftChange(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  onSaveTitle();
                }
                if (event.key === 'Escape') {
                  event.preventDefault();
                  onCancelTitleEdit();
                }
              }}
              placeholder="Session name"
              disabled={isSavingTitle}
              className={cn(
                'w-[240px] max-w-full rounded-sm border border-border bg-panel px-base py-half',
                'chat-session-header-input',
                'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
              )}
            />
            <PrimaryButton
              value="Save"
              onClick={onSaveTitle}
              disabled={isSavingTitle}
              className="chat-session-header-btn"
            />
            <PrimaryButton
              variant="tertiary"
              value="Cancel"
              onClick={onCancelTitleEdit}
              disabled={isSavingTitle}
              className="chat-session-header-btn"
            />
          </div>
        )}
        {titleError && (
          <div className="text-xs text-error">{titleError}</div>
        )}
        {activeSession && (
          <div className="chat-session-header-meta text-xs text-low">
            Created {formatDateShortWithTime(activeSession.created_at)} /
            Total messages: {messageCount}
          </div>
        )}
      </div>
      <div className="chat-session-header-actions flex items-center gap-base">
        <div className="chat-session-header-members flex items-center gap-base text-xs text-low">
          <UsersIcon className="size-icon-xs" />
          <span>{memberCount} AI members</span>
        </div>
        {activeSession && isSearchOpen && (
          <div
            ref={searchInputContainerRef}
            className="chat-session-header-search flex items-center gap-half"
          >
            <div className="chat-session-header-search-input-wrap">
              <input
                ref={searchInputRef}
                value={searchQuery}
                onChange={(event) =>
                  onSearchQueryChange(event.target.value)
                }
                onKeyDown={(event) => {
                  if (event.key === 'Escape') {
                    event.preventDefault();
                    onCloseSearch();
                  }
                }}
                placeholder="Search messages..."
                className={cn(
                  'chat-session-header-search-input',
                  'px-base py-half text-sm text-normal focus:outline-none'
                )}
                aria-label="Search messages"
              />
              {searchQuery.length > 0 && (
                <button
                  type="button"
                  className="chat-session-header-search-clear-btn"
                  onClick={() => {
                    onSearchQueryChange('');
                    searchInputRef.current?.focus();
                  }}
                  aria-label="Clear search"
                  title="Clear search"
                >
                  <XIcon className="size-icon-2xs" weight="bold" />
                </button>
              )}
            </div>
          </div>
        )}
        {activeSession && (
          <div className="chat-session-header-ops flex items-center gap-half">
            {isArchived && (
              <Badge variant="secondary" className="chat-session-archived-badge text-xs">
                Archived
              </Badge>
            )}
            <Tooltip
              content={
                isArchived ? 'Restore session' : 'Archive session'
              }
              side="bottom"
            >
              <button
                type="button"
                className={cn(
                  'chat-session-header-icon-btn chat-session-header-op-btn',
                  isArchiving && 'opacity-50 cursor-not-allowed'
                )}
                onClick={isArchived ? onRestore : onArchive}
                disabled={isArchiving}
                aria-label={
                  isArchived ? 'Restore session' : 'Archive session'
                }
              >
                {isArchived ? (
                  <ArrowCounterClockwiseIcon className="size-icon-xs" />
                ) : (
                  <BoxArrowDownIcon className="size-icon-xs" />
                )}
              </button>
            </Tooltip>
            {!isArchived && (
              <Tooltip
                content={
                  isCleanupMode
                    ? 'Exit cleanup mode'
                    : 'Cleanup messages'
                }
                side="bottom"
              >
                <button
                  type="button"
                  className={cn(
                    'chat-session-header-icon-btn chat-session-header-op-btn',
                    isCleanupMode && 'active',
                    isDeletingMessages &&
                      'opacity-50 cursor-not-allowed'
                  )}
                  onClick={onToggleCleanupMode}
                  disabled={isDeletingMessages}
                  aria-label={
                    isCleanupMode
                      ? 'Exit cleanup mode'
                      : 'Cleanup messages'
                  }
                >
                  <BroomIcon
                    className="size-icon-xs"
                    weight={isCleanupMode ? 'fill' : 'regular'}
                  />
                </button>
              </Tooltip>
            )}
            <Tooltip content="Settings" side="bottom">
              <button
                type="button"
                className="chat-session-header-icon-btn chat-session-header-op-btn"
                onClick={onOpenSettings}
                aria-label="Settings"
                title="Settings"
              >
                <GearSixIcon className="size-icon-xs" />
              </button>
            </Tooltip>
          </div>
        )}
      </div>
    </header>
  );
}
