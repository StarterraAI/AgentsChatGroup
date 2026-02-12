import { XIcon } from '@phosphor-icons/react';
import { cn } from '@/lib/utils';
import type { ChatAgent } from 'shared/types';
import RawLogText from '@/components/common/RawLogText';
import { formatDateShortWithTime } from '@/utils/date';
import type { RunHistoryItem } from '../types';

export interface WorkspaceDrawerProps {
  isOpen: boolean;
  onClose: () => void;
  agent: ChatAgent | null;
  workspacePath: string | null;
  runs: RunHistoryItem[];
  logRunId: string | null;
  logContent: string;
  logLoading: boolean;
  logError: string | null;
  onLoadLog: (runId: string) => void;
}

export function WorkspaceDrawer({
  isOpen,
  onClose,
  agent,
  workspacePath,
  runs,
  logRunId,
  logContent,
  logLoading,
  logError,
  onLoadLog,
}: WorkspaceDrawerProps) {
  return (
    <div
      className={cn(
        'chat-session-workspace-drawer absolute top-0 right-0 h-full w-[360px] border-l border-border shadow-lg transition-transform z-50 flex flex-col',
        isOpen ? 'translate-x-0' : 'translate-x-full'
      )}
    >
      <div className="px-base py-base border-b border-border shrink-0 flex items-center justify-between">
        <div className="text-sm text-normal font-medium">
          {agent?.name ?? 'Agent workspace'}
        </div>
        <button
          type="button"
          onClick={onClose}
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
          {runs.length === 0 && (
            <div className="text-xs text-low">
              No runs yet for this agent.
            </div>
          )}
          {runs.map((run: RunHistoryItem) => (
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
                  onClick={() => onLoadLog(run.runId)}
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
                  onClick={() => onLoadLog(logRunId)}
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
  );
}
