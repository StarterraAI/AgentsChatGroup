import { WarningCircleIcon, XCircleIcon } from '@phosphor-icons/react';
import { cn } from '@/lib/utils';

interface ChatErrorMessageProps {
  content: string;
  className?: string;
  expanded?: boolean;
  onToggle?: () => void;
  tone?: 'error' | 'warning';
}

export function ChatErrorMessage({
  content,
  className,
  expanded,
  onToggle,
  tone = 'error',
}: ChatErrorMessageProps) {
  const isWarning = tone === 'warning';
  const Icon = isWarning ? WarningCircleIcon : XCircleIcon;

  return (
    <div
      className={cn(
        'flex items-start gap-base text-sm cursor-pointer',
        isWarning ? 'text-warning' : 'text-error',
        className
      )}
      onClick={onToggle}
      role="button"
    >
      <Icon className="shrink-0 size-icon-base mt-0.5" />
      <span
        className={cn(
          !expanded && 'truncate',
          expanded && 'whitespace-pre-wrap break-all'
        )}
      >
        {content}
      </span>
    </div>
  );
}
