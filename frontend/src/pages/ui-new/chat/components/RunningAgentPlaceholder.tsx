import { useState } from 'react';
import { CaretDownIcon } from '@phosphor-icons/react';
import { useTranslation } from 'react-i18next';
import { cn } from '@/lib/utils';
import { ChatEntryContainer } from '@/components/ui-new/primitives/conversation/ChatEntryContainer';
import { ChatMarkdown } from '@/components/ui-new/primitives/conversation/ChatMarkdown';
import {
  AgentBrandIcon,
  getAgentAvatarSeed,
  getAgentAvatarStyle,
} from '../AgentAvatar';
import type {
  SessionMember,
  AgentStateInfo,
  MessageTone,
  StreamRun,
} from '../types';

export interface RunningAgentPlaceholderProps {
  member: SessionMember;
  run?: StreamRun;
  tone: MessageTone;
  stateInfo: AgentStateInfo | undefined;
  clock: number;
  isStopping: boolean;
  onStop: (sessionAgentId: string, agentId: string) => void;
}

export function RunningAgentPlaceholder({
  member,
  run,
  tone,
  stateInfo,
  clock,
  isStopping,
  onStop,
}: RunningAgentPlaceholderProps) {
  const { t } = useTranslation('chat');
  const [thinkingExpanded, setThinkingExpanded] = useState(true);
  const startedAtStr = stateInfo?.startedAt;
  const startedAtMs = startedAtStr
    ? new Date(startedAtStr).getTime()
    : clock;
  const elapsedSeconds = Math.max(
    0,
    Math.floor((clock - startedAtMs) / 1000)
  );
  const avatarSeed = getAgentAvatarSeed(
    member.agent.id,
    member.agent.runner_type,
    member.agent.name
  );
  const thinkingContent = run?.thinkingContent ?? '';
  const assistantContent = run?.assistantContent || run?.content || '';
  const hasThinking = thinkingContent.trim().length > 0;
  const hasAssistant = assistantContent.trim().length > 0;

  return (
    <div className="chat-session-message-row is-agent flex justify-start">
      <ChatEntryContainer
        variant="system"
        title={member.agent.name}
        icon={
          <AgentBrandIcon
            runnerType={member.agent.runner_type}
            className="chat-session-agent-avatar-logo"
          />
        }
        expanded
        iconContainerClassName="chat-session-message-avatar chat-session-agent-avatar"
        iconContainerStyle={getAgentAvatarStyle(avatarSeed)}
        className="chat-session-message-card chat-session-message-card-agent max-w-[760px] w-full md:w-[84%] opacity-80 shadow-sm rounded-2xl"
        headerClassName="chat-session-message-header"
        bodyClassName="chat-session-message-body"
        style={{
          backgroundColor: `var(--chat-session-message-other-bg, ${tone.bg})`,
          borderColor: `var(--chat-session-message-other-border, ${tone.border})`,
        }}
        headerRight={
          <button
            type="button"
            className={cn(
              'text-xs text-error hover:text-error/80',
              isStopping && 'opacity-50 cursor-not-allowed'
            )}
            onClick={() =>
              onStop(member.sessionAgent.id, member.agent.id)
            }
            disabled={isStopping}
          >
            {isStopping ? t('agent.stopping') : t('agent.stop')}
          </button>
        }
      >
        <div className="space-y-base">
          <div className="text-sm text-low">
            {t('agent.runningElapsed', { seconds: elapsedSeconds })}
          </div>

          {hasThinking && (
            <div className="rounded-lg border border-border bg-secondary/40 px-base py-half">
              <button
                type="button"
                className="flex w-full items-center justify-between gap-base"
                onClick={() =>
                  setThinkingExpanded((prevExpanded) => !prevExpanded)
                }
                aria-expanded={thinkingExpanded}
                aria-label={t('agent.thinking')}
              >
                <span className="text-xs text-low">{t('agent.thinking')}</span>
                <CaretDownIcon
                  className={cn(
                    'size-icon-xs text-low transition-transform',
                    !thinkingExpanded && '-rotate-90'
                  )}
                />
              </button>
              {thinkingExpanded && (
                <pre className="mt-half max-h-40 overflow-y-auto whitespace-pre-wrap break-words rounded border border-border/60 bg-primary/70 px-base py-half text-xs leading-5 text-low">
                  {thinkingContent}
                </pre>
              )}
            </div>
          )}

          {hasAssistant ? (
            <ChatMarkdown
              content={assistantContent}
              textClassName="text-sm"
            />
          ) : (
            <div className="text-sm text-low">{t('agent.processing')}</div>
          )}
        </div>
      </ChatEntryContainer>
    </div>
  );
}
