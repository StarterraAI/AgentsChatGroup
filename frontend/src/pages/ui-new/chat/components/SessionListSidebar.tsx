import {
  ChatCircleDotsIcon,
  PlusIcon,
} from '@phosphor-icons/react';
import { useTranslation } from 'react-i18next';
import type { ChatSession } from 'shared/types';
import { cn } from '@/lib/utils';
import { formatDateShortWithTime } from '@/utils/date';

const MAX_SESSION_TITLE_LENGTH = 20;

export interface SessionListSidebarProps {
  activeSessions: ChatSession[];
  archivedSessions: ChatSession[];
  activeSessionId: string | null;
  showArchived: boolean;
  onToggleArchived: () => void;
  onSelectSession: (sessionId: string) => void;
  onCreateSession: () => void;
  isCreating: boolean;
  width: number;
  isCollapsed: boolean;
  onToggleCollapsed: () => void;
}

export function SessionListSidebar({
  activeSessions,
  archivedSessions,
  activeSessionId,
  showArchived,
  onToggleArchived,
  onSelectSession,
  onCreateSession,
  isCreating,
  width,
  isCollapsed,
  onToggleCollapsed,
}: SessionListSidebarProps) {
  const { t } = useTranslation('chat');
  const isCompactSessionRow = !isCollapsed && width < 320;
  const appVersion = __APP_VERSION__.trim();
  const appVersionLabel = appVersion.startsWith('v')
    ? appVersion
    : `v${appVersion}`;
  const collapseActionLabel = isCollapsed
    ? t('sidebar.expandSidebar')
    : t('sidebar.collapseSidebar');

  const getDisplaySessionTitle = (title: string | null) => {
    const fullTitle = title?.trim() || t('sidebar.untitledSession');
    if (fullTitle.length <= MAX_SESSION_TITLE_LENGTH) {
      return { fullTitle, displayTitle: fullTitle };
    }
    return {
      fullTitle,
      displayTitle: `${fullTitle.slice(0, MAX_SESSION_TITLE_LENGTH)}...`,
    };
  };

  const renderSessionItem = (session: ChatSession) => {
    const isActive = session.id === activeSessionId;
    const { fullTitle, displayTitle } = getDisplaySessionTitle(
      session.title
    );

    return (
      <button
        key={session.id}
        type="button"
        onClick={() => onSelectSession(session.id)}
        className={cn(
          'chat-session-item w-full text-left',
          isActive && 'active',
          isCollapsed && 'collapsed'
        )}
        title={fullTitle}
      >
        <div className="chat-session-item-inner flex items-center gap-half">
          {isActive ? (
            <span
              className="chat-session-item-active-dot"
              aria-hidden="true"
            />
          ) : (
            <ChatCircleDotsIcon className="chat-session-item-icon size-icon-xs" />
          )}
          {!isCollapsed && (
            <div className="chat-session-item-content min-w-0 flex-1">
              <div
                className={cn(
                  'chat-session-item-row',
                  isCompactSessionRow && 'compact'
                )}
              >
                <div
                  className="chat-session-item-title truncate"
                  title={fullTitle}
                >
                  {displayTitle}
                </div>
                <div className="chat-session-item-meta">
                  {formatDateShortWithTime(session.updated_at)}
                </div>
              </div>
            </div>
          )}
        </div>
      </button>
    );
  };

  return (
    <aside
      className={cn(
        'chat-session-left border-r flex flex-col min-h-0 shrink-0',
        isCollapsed && 'collapsed'
      )}
      style={{ width }}
    >
      {isCollapsed ? (
        <>
          <div className="chat-session-left-collapsed-shell">
            <button
              type="button"
              onClick={onToggleCollapsed}
              className="chat-session-left-control-btn"
              aria-label={collapseActionLabel}
              title={collapseActionLabel}
            >
              <img
                src="/icon_menu.svg"
                alt=""
                aria-hidden="true"
                className="chat-session-left-control-icon"
              />
            </button>
          </div>
          <div className="chat-session-left-divider" />
          <div className="chat-session-list chat-session-list-collapsed flex-1 min-h-0 overflow-y-auto p-base space-y-half">
            {activeSessions.length === 0 ? (
              <div className="chat-session-list-empty text-sm text-low">
                {t('sidebar.noActiveSessions')}
              </div>
            ) : (
              activeSessions.map((session) =>
                renderSessionItem(session)
              )
            )}
          </div>
          <div
            className="chat-session-left-version collapsed"
            title={appVersionLabel}
          >
            {appVersionLabel}
          </div>
        </>
      ) : (
        <>
          <div className="chat-session-left-header px-base py-base">
            <div className="chat-session-left-brand">
              <img
                src="/agent-chatgroup-logo-white.png"
                alt="Agents ChatGroup"
                className="chat-session-left-brand-logo"
              />
              <button
                type="button"
                onClick={onToggleCollapsed}
                className="chat-session-left-control-btn"
                aria-label={collapseActionLabel}
                title={collapseActionLabel}
              >
                <img
                  src="/icon_menu.svg"
                  alt=""
                  aria-hidden="true"
                  className="chat-session-left-control-icon"
                />
              </button>
            </div>
            <div className="chat-session-left-divider" />
            <div className="chat-session-left-header-row flex items-center justify-between">
              <div className="chat-session-left-title flex items-center gap-base font-medium min-w-0">
                <img
                  src="/icon_vectorized_all_white.svg"
                  alt="Agents ChatGroup"
                  className="chat-session-left-logo"
                />
                <span className="truncate">{t('sidebar.title')}</span>
              </div>
              <button
                type="button"
                onClick={onCreateSession}
                disabled={isCreating}
                className="chat-session-left-new-btn"
                aria-label={t('sidebar.createNewSession')}
                title={t('sidebar.createNewSession')}
              >
                <span className="chat-session-left-new-label">
                  {t('sidebar.newSession')}
                </span>
                <PlusIcon className="chat-session-left-new-icon size-icon-xs" />
              </button>
            </div>
          </div>
          <div className="chat-session-left-divider" />
          <div className="chat-session-lists flex-1 min-h-0">
            <div className="chat-session-active-panel min-h-0">
              <div className="chat-session-active-list min-h-0 overflow-y-auto p-base space-y-half">
                {activeSessions.length === 0 ? (
                  <div className="chat-session-list-empty text-sm text-low">
                    {t('sidebar.noActiveSessions')}
                  </div>
                ) : (
                  activeSessions.map((session) =>
                    renderSessionItem(session)
                  )
                )}
              </div>
            </div>

            <div className="chat-session-left-divider" />
            <div className="chat-session-archived-panel min-h-0">
              <div className="chat-session-archived-header px-base py-half">
                <button
                  type="button"
                  onClick={onToggleArchived}
                  disabled={archivedSessions.length === 0}
                  className={cn(
                    'chat-session-archived-toggle',
                    archivedSessions.length === 0 &&
                      'opacity-60 cursor-not-allowed'
                  )}
                >
                  {showArchived ? t('sidebar.hideArchived') : t('sidebar.showArchived')} (
                  {archivedSessions.length})
                </button>
              </div>
              <div className="chat-session-archived-list min-h-0 overflow-y-auto p-base space-y-half">
                {archivedSessions.length === 0 && (
                  <div className="chat-session-list-empty text-sm text-low">
                    {t('sidebar.noArchivedSessions')}
                  </div>
                )}
                {archivedSessions.length > 0 &&
                  showArchived &&
                  archivedSessions.map((session) =>
                    renderSessionItem(session)
                  )}
              </div>
            </div>
          </div>
          <div className="chat-session-left-version" title={appVersionLabel}>
            {appVersionLabel}
          </div>
        </>
      )}
    </aside>
  );
}
