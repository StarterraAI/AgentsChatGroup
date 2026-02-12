import {
  ArrowsOutSimpleIcon,
  ArrowsInSimpleIcon,
  XIcon,
} from '@phosphor-icons/react';
import { cn } from '@/lib/utils';
import { DiffViewBody } from '@/components/ui-new/primitives/conversation/PierreConversationDiff';
import { chatApi } from '@/lib/api';
import type { RunDiffState, UntrackedFileState } from '../types';

export interface DiffViewerModalProps {
  isOpen: boolean;
  runId: string | null;
  hasDiff: boolean;
  isFullscreen: boolean;
  runDiff: RunDiffState | null;
  untrackedFiles: string[];
  untrackedContent: Record<string, UntrackedFileState>;
  theme: 'light' | 'dark';
  onClose: () => void;
  onToggleFullscreen: () => void;
  onToggleUntracked: (runId: string, path: string) => void;
}

export function DiffViewerModal({
  isOpen,
  runId,
  hasDiff,
  isFullscreen,
  runDiff,
  untrackedFiles,
  untrackedContent,
  theme,
  onClose,
  onToggleFullscreen,
  onToggleUntracked,
}: DiffViewerModalProps) {
  if (!isOpen || !runId) return null;

  const DiffViewerIcon = isFullscreen
    ? ArrowsInSimpleIcon
    : ArrowsOutSimpleIcon;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
    >
      <div
        className={cn(
          'chat-session-modal-surface border border-border shadow-xl flex flex-col overflow-hidden',
          isFullscreen
            ? 'w-full h-full rounded-none'
            : 'w-[92vw] h-[85vh] max-w-[1200px] rounded-xl'
        )}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-border px-base py-half">
          <div>
            <div className="text-sm text-normal font-medium">Code changes</div>
            <div className="text-xs text-low">Run {runId.slice(0, 8)}</div>
          </div>
          <div className="flex items-center gap-half">
            <button
              type="button"
              className="text-low hover:text-normal"
              onClick={onToggleFullscreen}
              aria-label={
                isFullscreen ? 'Exit full screen' : 'Full screen'
              }
            >
              <DiffViewerIcon className="size-icon-sm" />
            </button>
            <button
              type="button"
              className="text-low hover:text-normal"
              onClick={onClose}
              aria-label="Close diff viewer"
            >
              <XIcon className="size-icon-sm" />
            </button>
          </div>
        </div>
        <div className="flex-1 overflow-y-auto p-base space-y-base">
          {hasDiff ? (
            <>
              {runDiff?.loading && (
                <div className="text-xs text-low">Loading diff...</div>
              )}
              {runDiff?.error && (
                <div className="text-xs text-error">{runDiff.error}</div>
              )}
              {!runDiff?.loading &&
                !runDiff?.error &&
                runDiff?.files?.length === 0 && (
                  <div className="text-xs text-low">
                    No tracked diff available.
                  </div>
                )}
              {runDiff?.files?.map((file) => (
                <div
                  key={`${runId}-${file.path}`}
                  className="border border-border rounded-sm bg-panel"
                >
                  <div className="flex items-center justify-between px-base py-half text-xs text-normal border-b border-border">
                    <span className="font-ibm-plex-mono break-all">
                      {file.path}
                    </span>
                    <span className="text-xs text-low">
                      {file.additions > 0 && (
                        <span className="text-success">
                          +{file.additions}
                        </span>
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
                    theme={theme}
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
                      chatApi.getRunDiffUrl(runId),
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
            <div className="text-xs text-low">
              No tracked diff available.
            </div>
          )}
          {untrackedFiles.length > 0 && (
            <div className="space-y-half">
              <div className="text-xs text-low">Untracked files</div>
              {untrackedFiles.map((path) => {
                const key = `${runId}:${path}`;
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
                        onClick={() => onToggleUntracked(runId, path)}
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
  );
}
