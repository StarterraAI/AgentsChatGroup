import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import {
  AlertTriangle,
  Check,
  ChevronRight,
  FileWarning,
  Loader2,
  RefreshCw,
  X,
} from 'lucide-react';

import {
  chatSessionWorktreeApi,
  type ResolveSessionWorktreeConflictRequest,
} from '@/lib/api';
import type {
  ConflictFileContent,
  ConflictFileInfo,
} from '@/types';
import { ScrollArea } from '@/components/ScrollArea';
import {
  WorktreeConflictActionButton,
  WorktreeConflictChoiceCard,
  WorktreeMergeConflictFrame,
  WorktreeQuickActionButton,
} from '@/components/worktree/WorktreeMergeConflictSurface';
import { CodeMirrorConflictEditor } from './CodeMirrorConflictEditor';
import {
  runBulkResolve,
  type BulkResolveSide,
} from './mergeConflictBulkResolve';

export interface WorktreeMergeConflictsViewProps {
  sessionId: string;
  surface?: 'modal' | 'page';
  tr: (
    key: string,
    fallback: string,
    replacements?: Record<string, string | number>,
  ) => string;
  onCompleted: () => void;
  onAbort: () => void;
  onClose?: () => void;
}

export type FileResolution =
  | { kind: 'text'; content: string }
  | { kind: 'binary'; choice: 'current' | 'session' | 'deleted' };

interface ResolvedFile {
  path: string;
  resolution: FileResolution;
}

// `deleted_by_us` (session removed the file) and `deleted_by_them` (base
// removed it) are conflicts where the working tree has no mergeable text; we
// surface them as binary-style keep/use/delete choices so the user does not
// have to hand-edit a missing file.
const NON_TEXT_STATUSES = new Set([
  'deleted_by_us',
  'deleted_by_them',
  'both_deleted',
  'renamed',
]);

export const isNonTextConflict = (
  info: ConflictFileInfo,
  detail: ConflictFileContent | undefined,
): boolean =>
  NON_TEXT_STATUSES.has(info.status) ||
  Boolean(detail?.is_binary) ||
  Boolean(detail?.is_too_large);

export const buildResolveConflictRequest = (
  path: string,
  resolution: FileResolution,
): ResolveSessionWorktreeConflictRequest => {
  if (resolution.kind === 'text') {
    return { path, content: resolution.content };
  }
  if (resolution.choice === 'deleted') {
    return { path, delete_file: true };
  }
  return { path, use_stage: resolution.choice };
};

export const canContinueMerge = (
  files: ConflictFileInfo[],
  listLoaded: boolean,
  listLoading: boolean,
): boolean => listLoaded && !listLoading && files.length === 0;

export type ConflictHunkChoice = 'current' | 'session' | 'both';

export interface ConflictHunk {
  id: string;
  index: number;
  startLine: number;
  endLine: number;
  currentLabel: string;
  sessionLabel: string;
  baseLabel: string | null;
  current: string;
  session: string;
  base: string | null;
  original: string;
}

type ParsedConflictSegment =
  | { kind: 'text'; content: string }
  | { kind: 'conflict'; hunk: ConflictHunk };

export interface ParsedConflictText {
  segments: ParsedConflictSegment[];
  hunks: ConflictHunk[];
}

const splitLinesPreservingEndings = (content: string): string[] =>
  content.match(/[^\n]*\n|[^\n]+/g) ?? [];

const trimMarkerLine = (line: string, marker: string): string =>
  line
    .replace(/\r?\n$/, '')
    .slice(marker.length)
    .trim();

export const containsConflictMarkers = (content: string): boolean =>
  splitLinesPreservingEndings(content).some(
    (line) =>
      line.startsWith('<<<<<<<') || line.startsWith('>>>>>>>'),
  );

export const parseConflictText = (content: string): ParsedConflictText => {
  const lines = splitLinesPreservingEndings(content);
  const segments: ParsedConflictSegment[] = [];
  const hunks: ConflictHunk[] = [];
  let textBuffer: string[] = [];
  let index = 0;

  const flushText = () => {
    if (textBuffer.length === 0) return;
    segments.push({ kind: 'text', content: textBuffer.join('') });
    textBuffer = [];
  };

  while (index < lines.length) {
    const line = lines[index];
    if (!line.startsWith('<<<<<<<')) {
      textBuffer.push(line);
      index += 1;
      continue;
    }

    const startIndex = index;
    const startLine = startIndex + 1;
    const currentLabel = trimMarkerLine(line, '<<<<<<<') || 'current';
    index += 1;

    const currentLines: string[] = [];
    while (
      index < lines.length &&
      !lines[index].startsWith('|||||||') &&
      !lines[index].startsWith('=======') &&
      !lines[index].startsWith('>>>>>>>')
    ) {
      currentLines.push(lines[index]);
      index += 1;
    }

    let baseLabel: string | null = null;
    let baseLines: string[] | null = null;
    if (index < lines.length && lines[index].startsWith('|||||||')) {
      baseLabel = trimMarkerLine(lines[index], '|||||||') || 'base';
      baseLines = [];
      index += 1;
      while (
        index < lines.length &&
        !lines[index].startsWith('=======') &&
        !lines[index].startsWith('>>>>>>>')
      ) {
        baseLines.push(lines[index]);
        index += 1;
      }
    }

    if (index >= lines.length || !lines[index].startsWith('=======')) {
      textBuffer.push(...lines.slice(startIndex, index));
      continue;
    }
    index += 1;

    const sessionLines: string[] = [];
    while (index < lines.length && !lines[index].startsWith('>>>>>>>')) {
      sessionLines.push(lines[index]);
      index += 1;
    }

    if (index >= lines.length || !lines[index].startsWith('>>>>>>>')) {
      textBuffer.push(...lines.slice(startIndex, index));
      continue;
    }

    const sessionLabel = trimMarkerLine(lines[index], '>>>>>>>') || 'source';
    index += 1;
    const original = lines.slice(startIndex, index).join('');
    const hunk: ConflictHunk = {
      id: `hunk-${hunks.length + 1}`,
      index: hunks.length,
      startLine,
      endLine: startLine + index - startIndex - 1,
      currentLabel,
      sessionLabel,
      baseLabel,
      current: currentLines.join(''),
      session: sessionLines.join(''),
      base: baseLines ? baseLines.join('') : null,
      original,
    };
    flushText();
    hunks.push(hunk);
    segments.push({ kind: 'conflict', hunk });
  }

  flushText();
  return { segments, hunks };
};

const joinAcceptedBoth = (current: string, session: string): string => {
  if (!current) return session;
  if (!session) return current;
  return `${current}${current.endsWith('\n') ? '' : '\n'}${session}`;
};

const contentForHunkChoice = (
  hunk: ConflictHunk,
  choice: ConflictHunkChoice,
): string => {
  if (choice === 'current') return hunk.current;
  if (choice === 'session') return hunk.session;
  return joinAcceptedBoth(hunk.current, hunk.session);
};

export const buildConflictResolutionContent = (
  parsed: ParsedConflictText,
  choices: Record<string, ConflictHunkChoice>,
): string =>
  parsed.segments
    .map((segment) => {
      if (segment.kind === 'text') return segment.content;
      const choice = choices[segment.hunk.id];
      return choice
        ? contentForHunkChoice(segment.hunk, choice)
        : segment.hunk.original;
    })
    .join('');

export const WorktreeMergeConflictsView: React.FC<
  WorktreeMergeConflictsViewProps
> = ({ sessionId, surface = 'modal', tr, onCompleted, onAbort, onClose }) => {
  const [files, setFiles] = useState<ConflictFileInfo[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [detail, setDetail] = useState<ConflictFileContent | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [resolved, setResolved] = useState<Map<string, FileResolution>>(
    new Map(),
  );
  const [listLoading, setListLoading] = useState(false);
  const [listLoaded, setListLoaded] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [pendingAction, setPendingAction] = useState<string | null>(null);
  const [bulkProgress, setBulkProgress] = useState<{
    done: number;
    total: number;
  } | null>(null);
  // Ref guard so a second click landing before React re-renders cannot start
  // a parallel bulk run (state updates alone would be too late).
  const bulkRunningRef = useRef(false);
  const [commitMessage, setCommitMessage] = useState('');
  const [currentBranch, setCurrentBranch] = useState('HEAD');
  const [incomingBranch, setIncomingBranch] = useState('session');

  const refreshList = useCallback(async () => {
    setListLoading(true);
    setActionError(null);
    try {
      const list = await chatSessionWorktreeApi.listMergeConflicts(sessionId);
      setFiles(list);
      setListLoaded(true);
      if (list.length > 0 && !selectedPath) {
        setSelectedPath(list[0].path);
      }
      if (list.length === 0) {
        setSelectedPath(null);
      } else if (selectedPath && !list.some((f) => f.path === selectedPath)) {
        setSelectedPath(list[0].path);
      }
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setListLoading(false);
    }
  }, [sessionId, selectedPath]);

  useEffect(() => {
    void refreshList();
    // Only run on mount; subsequent refreshes are triggered after resolves.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  useEffect(() => {
    let cancelled = false;
    void chatSessionWorktreeApi
      .getStatus(sessionId)
      .then((worktree) => {
        if (cancelled || !worktree) return;
        setCurrentBranch(
          worktree.merge_target_branch ?? worktree.base_branch ?? 'HEAD',
        );
        setIncomingBranch(worktree.branch_name || 'session');
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [sessionId]);

  useEffect(() => {
    if (!selectedPath) {
      setDetail(null);
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    setDetail(null);
    void chatSessionWorktreeApi
      .getMergeConflictDetail(sessionId, selectedPath)
      .then((content) => {
        if (cancelled) return;
        setDetail(content);
        // Seed the result editor with the working tree content (markers
        // included) so the user has a starting point. They can use the
        // quick actions to replace it with one side or accept both.
        setResolved((prev) => {
          if (prev.has(selectedPath)) return prev;
          const next = new Map(prev);
          if (
            isNonTextConflict(
              files.find((f) => f.path === selectedPath) ?? {
                path: selectedPath,
                status: 'both_modified',
              },
              content,
            )
          ) {
            next.set(selectedPath, { kind: 'binary', choice: 'current' });
          } else {
            next.set(selectedPath, { kind: 'text', content: content.working_tree });
          }
          return next;
        });
      })
      .catch((err) => {
        if (!cancelled) {
          setActionError(err instanceof Error ? err.message : String(err));
        }
      })
      .finally(() => {
        if (!cancelled) setDetailLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [files, sessionId, selectedPath]);

  const selectedInfo = useMemo(
    () => files.find((f) => f.path === selectedPath) ?? null,
    [files, selectedPath],
  );

  const isNonText = useMemo(
    () =>
      Boolean(selectedInfo) &&
      isNonTextConflict(selectedInfo as ConflictFileInfo, detail ?? undefined),
    [selectedInfo, detail],
  );

  const canContinue = canContinueMerge(files, listLoaded, listLoading);

  const setResolution = useCallback((path: string, resolution: FileResolution) => {
    setResolved((prev) => {
      const next = new Map(prev);
      next.set(path, resolution);
      return next;
    });
  }, []);

  const handleUseCurrent = () => {
    if (!selectedPath || !detail) return;
    setResolution(selectedPath, {
      kind: 'text',
      content: detail.current ?? '',
    });
  };
  const handleUseSession = () => {
    if (!selectedPath || !detail) return;
    setResolution(selectedPath, {
      kind: 'text',
      content: detail.session ?? '',
    });
  };
  const handleAcceptBoth = () => {
    if (!selectedPath || !detail) return;
    // Concatenate both sides with a clear separator; this is the simplest
    // "accept both" that keeps both changes without requiring a manual merge.
    const parts = [detail.current ?? '', detail.session ?? ''].filter(
      (s) => s.length > 0,
    );
    setResolution(selectedPath, {
      kind: 'text',
      content: parts.join('\n\n'),
    });
  };

  const handleBinaryChoice = (choice: 'current' | 'session' | 'deleted') => {
    if (!selectedPath) return;
    setResolution(selectedPath, { kind: 'binary', choice });
  };

  const runResolve = async (file: ResolvedFile) => {
    await chatSessionWorktreeApi.resolveMergeConflict(sessionId, {
      ...buildResolveConflictRequest(file.path, file.resolution),
    });
  };

  const handleMarkResolved = async () => {
    if (!selectedPath) return;
    const resolution = resolved.get(selectedPath);
    if (!resolution) return;
    setPendingAction(`resolve:${selectedPath}`);
    setActionError(null);
    try {
      await runResolve({ path: selectedPath, resolution });
      // Re-read the conflict list: the backend marks the path resolved and
      // removes it from the unmerged set. If no conflicts remain, the user
      // can Continue.
      await refreshList();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setPendingAction(null);
    }
  };

  const handleBulkResolve = async (side: BulkResolveSide) => {
    if (bulkRunningRef.current || pendingAction || files.length === 0) return;
    bulkRunningRef.current = true;
    setPendingAction(`bulk:${side}`);
    setBulkProgress({ done: 0, total: files.length });
    setActionError(null);
    try {
      const outcome = await runBulkResolve(
        sessionId,
        files,
        side,
        undefined,
        (done, total) => setBulkProgress({ done, total }),
      );
      setResolved(new Map());
      // Refresh first: it clears the action error, so the failure summary
      // must be applied afterwards to stay visible.
      await refreshList();
      if (outcome.failedPath) {
        setActionError(
          tr(
            'worktree.merge.bulkFailed',
            'Resolved {done} of {total} files; failed on {path}: {error}',
            {
              done: outcome.completed,
              total: outcome.total,
              path: outcome.failedPath,
              error: outcome.error ?? '',
            },
          ),
        );
      }
    } finally {
      bulkRunningRef.current = false;
      setPendingAction(null);
      setBulkProgress(null);
    }
  };

  const handleContinue = async () => {
    if (!canContinue) return;
    setPendingAction('continue');
    setActionError(null);
    try {
      // Resolve any files the user marked but did not explicitly submit via
      // "Mark resolved" (defensive; the explicit button is the primary path).
      for (const file of files) {
        const r = resolved.get(file.path);
        if (!r) continue;
        try {
          await runResolve({ path: file.path, resolution: r });
        } catch {
          // Surface upstream; continue loop so the final `continue/merge`
          // error is the most informative.
        }
      }
      const worktree = await chatSessionWorktreeApi.continueMerge(sessionId, {
        commit_message: commitMessage.trim() || null,
      });
      if (worktree.status === 'needs_conflict_resolution') {
        // Keep the resolver open if a compatible backend reports another
        // conflict set after continuing the integration.
        setResolved(new Map());
        await refreshList();
      } else {
        onCompleted();
      }
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
      await refreshList().catch(() => undefined);
    } finally {
      setPendingAction(null);
    }
  };

  const handleAbort = async () => {
    setPendingAction('abort');
    setActionError(null);
    try {
      await chatSessionWorktreeApi.abortMerge(sessionId);
      onAbort();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setPendingAction(null);
    }
  };

  const currentResolution = selectedPath
    ? resolved.get(selectedPath)
    : undefined;
  const currentTextHasMarkers =
    currentResolution?.kind === 'text' &&
    containsConflictMarkers(currentResolution.content);

  return (
    <WorktreeMergeConflictFrame surface={surface}>
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-[var(--hairline)] px-3">
        <div className="flex min-w-0 items-center gap-2">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-amber-600" />
          <h2 className="truncate text-[14px] font-semibold text-[var(--ink)]">
            {tr('worktree.merge.title', 'Merge Conflicts')}
          </h2>
          <span className="rounded-full bg-[var(--surface-3)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--ink-tertiary)]">
            {tr('worktree.merge.fileCount', '{count} files', {
              count: files.length,
            })}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            onClick={() => void refreshList()}
            disabled={listLoading || Boolean(pendingAction)}
            className="inline-flex h-6 w-6 items-center justify-center rounded-md text-[var(--ink-tertiary)] transition hover:bg-[var(--surface-3)] hover:text-[var(--ink)] disabled:opacity-40"
            title={tr('sourceControl.refresh', 'Refresh')}
          >
            <RefreshCw
              aria-hidden
              className={`h-3.5 w-3.5 ${listLoading ? 'animate-spin' : ''}`}
            />
          </button>
          {onClose && (
            <button
              type="button"
              onClick={onClose}
              disabled={Boolean(pendingAction)}
              className="inline-flex h-6 w-6 items-center justify-center rounded-md text-[var(--ink-tertiary)] transition hover:bg-[var(--surface-3)] hover:text-[var(--ink)] disabled:opacity-40"
              title={tr('worktree.merge.closeWindow', 'Close resolver')}
            >
              <X className="h-3.5 w-3.5" aria-hidden />
            </button>
          )}
        </div>
      </div>

      {actionError && (
        <div className="mx-3 mt-2 rounded-md bg-rose-500/10 px-3 py-2 text-[12px] text-rose-600">
          {actionError}
        </div>
      )}

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden pb-20">
        <div className="max-h-40 shrink-0 border-b border-[var(--hairline)]">
          <div className="flex shrink-0 flex-wrap items-center gap-1 border-b border-[var(--hairline)] px-2 py-1">
            <span className="mr-auto px-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--ink-tertiary)]">
              {tr('worktree.merge.allFilesActions', 'All files')}
            </span>
            {(['current', 'session'] as const).map((side) => {
              const running =
                pendingAction === `bulk:${side}` && bulkProgress !== null;
              return (
                <button
                  key={side}
                  type="button"
                  onClick={() => void handleBulkResolve(side)}
                  disabled={
                    listLoading ||
                    files.length === 0 ||
                    Boolean(pendingAction)
                  }
                  title={
                    side === 'current'
                      ? tr(
                          'worktree.merge.useAllCurrentHint',
                          'Resolve every conflict file with the current workspace version; files missing there are deleted',
                        )
                      : tr(
                          'worktree.merge.useAllSessionHint',
                          'Resolve every conflict file with the source worktree version; files missing there are deleted',
                        )
                  }
                  className="inline-flex max-w-full items-center gap-1 truncate whitespace-nowrap rounded-sm px-1.5 py-0.5 text-[11px] font-medium text-[var(--ink-subtle)] transition hover:bg-[var(--surface-3)] hover:text-[var(--ink)] disabled:opacity-40"
                >
                  {running && (
                    <Loader2 className="h-3 w-3 animate-spin" aria-hidden />
                  )}
                  {running
                    ? tr(
                        'worktree.merge.bulkProgress',
                        'Resolving {done}/{total}...',
                        { done: bulkProgress.done, total: bulkProgress.total },
                      )
                    : side === 'current'
                      ? tr(
                          'worktree.merge.useAllCurrent',
                          'All files: keep current',
                        )
                      : tr(
                          'worktree.merge.useAllSession',
                          'All files: use source',
                        )}
                </button>
              );
            })}
          </div>
          <ScrollArea className="h-full">
            {files.length === 0 && !listLoading ? (
              <div className="px-3 py-4 text-[12px] text-[var(--ink-tertiary)]">
                {tr(
                  'worktree.merge.noConflicts',
                  'No conflicts remaining',
                )}
              </div>
            ) : (
              <ul className="py-1">
                {files.map((file) => {
                  const fileResolution = resolved.get(file.path);
                  const isResolved =
                    fileResolution?.kind === 'binary' ||
                    (fileResolution?.kind === 'text' &&
                      !containsConflictMarkers(fileResolution.content));
                  const isSelected = file.path === selectedPath;
                  return (
                    <li key={file.path}>
                      <button
                        type="button"
                        className={`flex w-full items-center gap-1.5 px-2 py-1.5 text-left text-[12px] transition hover:bg-[var(--surface-3)] ${
                          isSelected
                            ? 'bg-[var(--surface-3)] text-[var(--ink)]'
                            : 'text-[var(--ink-subtle)]'
                        }`}
                        onClick={() => setSelectedPath(file.path)}
                        title={file.path}
                      >
                        {isResolved ? (
                          <Check
                            aria-hidden
                            className="h-3 w-3 shrink-0 text-emerald-600"
                          />
                        ) : (
                          <FileWarning
                            aria-hidden
                            className="h-3 w-3 shrink-0 text-amber-600"
                          />
                        )}
                        <span className="min-w-0 flex-1 truncate">
                          {file.path}
                        </span>
                        <span className="max-w-[40%] shrink-0 truncate text-[10px] text-[var(--ink-tertiary)]">
                          {file.status.replaceAll('_', ' ')}
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </ScrollArea>
        </div>

        <div className="flex min-h-0 flex-1 flex-col">
          {!selectedPath ? (
            <div className="flex flex-1 items-center justify-center px-4 text-center text-[12px] text-[var(--ink-tertiary)]">
              {tr(
                'worktree.merge.selectPrompt',
                'Select a file to resolve',
              )}
            </div>
          ) : detailLoading ? (
            <div className="flex flex-1 items-center justify-center gap-2 text-[12px] text-[var(--ink-tertiary)]">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {tr('worktree.merge.loading', 'Loading conflict...')}
            </div>
          ) : !detail ? (
            <div className="flex flex-1 items-center justify-center px-4 text-[12px] text-rose-600">
              {tr(
                'worktree.merge.loadFailed',
                'Failed to load conflict content',
              )}
            </div>
          ) : isNonText ? (
            <BinaryConflictEditor
              path={selectedPath}
              status={selectedInfo?.status ?? ''}
              detail={detail}
              choice={
                currentResolution?.kind === 'binary'
                  ? currentResolution.choice
                  : undefined
              }
              tr={tr}
              onChoose={handleBinaryChoice}
            />
          ) : (
            <CodeMirrorConflictEditor
              key={selectedPath}
              path={selectedPath}
              resultContent={
                currentResolution?.kind === 'text'
                  ? currentResolution.content
                  : detail.working_tree
              }
              parsed={parseConflictText(detail.working_tree)}
              currentBranch={currentBranch}
              incomingBranch={incomingBranch}
              tr={tr}
              onUseCurrent={handleUseCurrent}
              onUseIncoming={handleUseSession}
              onAcceptBoth={handleAcceptBoth}
              onChange={(content) =>
                setResolution(selectedPath, { kind: 'text', content })
              }
            />
          )}
        </div>
      </div>

      <div className="pointer-events-none absolute inset-x-0 bottom-4 z-20 flex justify-center px-4">
        <div className="pointer-events-auto flex max-w-full flex-wrap items-center justify-center gap-1.5 rounded-2xl border border-[var(--hairline)] bg-[color-mix(in_srgb,var(--surface-1)_94%,transparent)] p-1.5 shadow-[0_12px_38px_rgba(0,0,0,0.2)] backdrop-blur-md">
          <WorktreeConflictActionButton
            onClick={() => void handleAbort()}
            disabled={Boolean(pendingAction)}
            borderless
            icon={<X className="h-3 w-3" aria-hidden />}
          >
            {tr('worktree.merge.abort', 'Abort merge')}
          </WorktreeConflictActionButton>
          <input
            type="text"
            value={commitMessage}
            onChange={(e) => setCommitMessage(e.target.value)}
            placeholder={tr(
              'worktree.merge.commitPlaceholder',
              'Merge commit message (optional)',
            )}
            className="h-8 w-[min(280px,45vw)] rounded-xl border border-[var(--hairline)] bg-transparent px-2.5 text-[12px] text-[var(--ink)] outline-none placeholder:text-[var(--ink-tertiary)] focus:border-[var(--ink-tertiary)]"
          />
          <WorktreeConflictActionButton
            onClick={() => void handleMarkResolved()}
            borderless
            disabled={
              !selectedPath ||
              !currentResolution ||
              currentTextHasMarkers ||
              Boolean(pendingAction)
            }
            title={
              currentTextHasMarkers
                ? tr(
                    'worktree.merge.unresolvedMarkersHint',
                    'Choose a resolution for every conflict point first',
                  )
                : tr(
                    'worktree.merge.markResolvedHint',
                    'Save this file and mark it resolved',
                  )
            }
            icon={
              pendingAction === `resolve:${selectedPath}` ? (
                <Loader2 className="h-3 w-3 animate-spin" aria-hidden />
              ) : (
                <Check className="h-3 w-3" aria-hidden />
              )
            }
          >
            {tr('worktree.merge.markResolved', 'Mark resolved')}
          </WorktreeConflictActionButton>
          <WorktreeConflictActionButton
            variant="primary"
            onClick={() => void handleContinue()}
            disabled={!canContinue || Boolean(pendingAction)}
            title={
              canContinue
                ? tr('worktree.merge.continueHint', 'Finish the merge')
                : tr(
                    'worktree.merge.continueDisabled',
                    'Resolve all files to continue',
                  )
            }
            icon={
              pendingAction === 'continue' ? (
                <Loader2 className="h-3 w-3 animate-spin" aria-hidden />
              ) : (
                <ChevronRight className="h-3 w-3" aria-hidden />
              )
            }
          >
            {tr('worktree.merge.continue', 'Continue merge')}
          </WorktreeConflictActionButton>
        </div>
      </div>
    </WorktreeMergeConflictFrame>
  );
};

interface TextConflictEditorProps {
  path: string;
  detail: ConflictFileContent;
  resolution: FileResolution | undefined;
  tr: WorktreeMergeConflictsViewProps['tr'];
  onUseCurrent: () => void;
  onUseSession: () => void;
  onAcceptBoth: () => void;
  onChange: (content: string) => void;
}

const TextConflictEditor: React.FC<TextConflictEditorProps> = ({
  path,
  detail,
  resolution,
  tr,
  onUseCurrent,
  onUseSession,
  onAcceptBoth,
  onChange,
}) => {
  const parsed = useMemo(
    () => parseConflictText(detail.working_tree),
    [detail.working_tree],
  );
  const [selectedHunkId, setSelectedHunkId] = useState<string | null>(
    () => parsed.hunks[0]?.id ?? null,
  );
  const [choices, setChoices] = useState<Record<string, ConflictHunkChoice>>(
    {},
  );
  const resultContent =
    resolution?.kind === 'text' ? resolution.content : detail.working_tree;
  const selectedHunk =
    parsed.hunks.find((hunk) => hunk.id === selectedHunkId) ??
    parsed.hunks[0] ??
    null;
  const unresolvedCount = parsed.hunks.filter(
    (hunk) => !choices[hunk.id],
  ).length;

  useEffect(() => {
    setSelectedHunkId(parsed.hunks[0]?.id ?? null);
    setChoices({});
  }, [path, parsed]);

  const applyChoices = (nextChoices: Record<string, ConflictHunkChoice>) => {
    setChoices(nextChoices);
    onChange(buildConflictResolutionContent(parsed, nextChoices));
  };

  const chooseHunk = (hunkId: string, choice: ConflictHunkChoice) => {
    applyChoices({ ...choices, [hunkId]: choice });
  };

  const chooseAll = (choice: ConflictHunkChoice) => {
    if (parsed.hunks.length === 0) return;
    applyChoices(
      Object.fromEntries(
        parsed.hunks.map((hunk) => [hunk.id, choice]),
      ) as Record<string, ConflictHunkChoice>,
    );
  };

  if (parsed.hunks.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex shrink-0 flex-wrap items-center gap-1 border-b border-[var(--hairline)] px-3 py-1.5">
          <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-[var(--ink-tertiary)]">
            {path}
          </span>
          <div className="ml-auto flex max-w-full flex-wrap items-center justify-end gap-1">
            <QuickAction
              label={tr('worktree.merge.useCurrent', 'Use current')}
              onClick={onUseCurrent}
            />
            <QuickAction
              label={tr('worktree.merge.useSession', 'Use source')}
              onClick={onUseSession}
            />
            <QuickAction
              label={tr('worktree.merge.acceptBoth', 'Accept both')}
              onClick={onAcceptBoth}
            />
          </div>
        </div>
        <div className="grid min-h-0 flex-1 grid-cols-1 divide-y divide-[var(--hairline)] overflow-auto xl:grid-cols-3 xl:divide-x xl:divide-y-0">
          <ConflictPane
            title={tr('worktree.merge.pane.current', 'Current')}
            content={detail.current ?? ''}
            emptyHint={tr(
              'worktree.merge.emptyCurrent',
              'No current-side content',
            )}
          />
          <ConflictPane
            title={tr('worktree.merge.pane.session', 'Source Worktree')}
            content={detail.session ?? ''}
            emptyHint={tr(
              'worktree.merge.emptySession',
              'No source-side content',
            )}
          />
          <ResultPane
            title={tr('worktree.merge.pane.result', 'Result')}
            content={resultContent}
            onChange={onChange}
            markerWarning={containsConflictMarkers(resultContent)}
            tr={tr}
          />
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-1 border-b border-[var(--hairline)] px-3 py-1.5">
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-[var(--ink-tertiary)]">
          {path}
        </span>
        <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-700">
          {tr('worktree.merge.hunkCount', '{count} conflict points', {
            count: parsed.hunks.length,
          })}
        </span>
        {unresolvedCount > 0 && (
          <span className="rounded bg-rose-500/10 px-1.5 py-0.5 text-[10px] font-medium text-rose-600">
            {tr('worktree.merge.unresolvedCount', '{count} unresolved', {
              count: unresolvedCount,
            })}
          </span>
        )}
        <div className="ml-auto flex max-w-full flex-wrap items-center justify-end gap-1">
          <QuickAction
            label={tr('worktree.merge.acceptAllCurrent', 'All current')}
            onClick={() => chooseAll('current')}
          />
          <QuickAction
            label={tr('worktree.merge.acceptAllSource', 'All source')}
            onClick={() => chooseAll('session')}
          />
          <QuickAction
            label={tr('worktree.merge.acceptAllBoth', 'All both')}
            onClick={() => chooseAll('both')}
          />
        </div>
      </div>
      <div className="grid min-h-0 flex-1 grid-cols-1 overflow-hidden lg:grid-cols-[220px_minmax(0,1fr)]">
        <div className="min-h-0 border-b border-[var(--hairline)] lg:border-b-0 lg:border-r">
          <ScrollArea className="h-full">
            <ul className="p-2">
              {parsed.hunks.map((hunk) => {
                const selected = hunk.id === selectedHunk?.id;
                const choice = choices[hunk.id];
                return (
                  <li key={hunk.id}>
                    <button
                      type="button"
                      onClick={() => setSelectedHunkId(hunk.id)}
                      className={`mb-1 flex w-full min-w-0 flex-col items-start rounded-md border px-2 py-1.5 text-left transition ${
                        selected
                          ? 'border-[var(--primary)] bg-[var(--primary-tint)]'
                          : 'border-transparent hover:bg-[var(--surface-3)]'
                      }`}
                    >
                      <span className="flex w-full min-w-0 items-center gap-1">
                        <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-[var(--ink)]">
                          {tr('worktree.merge.hunkTitle', 'Conflict {index}', {
                            index: hunk.index + 1,
                          })}
                        </span>
                        <span className="font-mono text-[10px] text-[var(--ink-tertiary)]">
                          {hunk.startLine}
                        </span>
                      </span>
                      <span
                        className={`mt-1 rounded px-1 py-0.5 text-[10px] ${
                          choice
                            ? 'bg-emerald-500/10 text-emerald-700'
                            : 'bg-amber-500/10 text-amber-700'
                        }`}
                      >
                        {choice
                          ? choiceLabel(choice, tr)
                          : tr('worktree.merge.pendingChoice', 'Unresolved')}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </ScrollArea>
        </div>
        <div className="grid min-h-0 grid-rows-[minmax(0,1fr)_minmax(120px,0.7fr)] overflow-hidden">
          {selectedHunk ? (
            <HunkDiffView
              hunk={selectedHunk}
              choice={choices[selectedHunk.id]}
              tr={tr}
              onChoose={(choice) => chooseHunk(selectedHunk.id, choice)}
            />
          ) : (
            <div className="flex min-h-0 items-center justify-center text-[12px] text-[var(--ink-tertiary)]">
              {tr('worktree.merge.selectHunk', 'Select a conflict point')}
            </div>
          )}
          <ResultPane
            title={tr('worktree.merge.pane.result', 'Merged Result')}
            content={resultContent}
            onChange={onChange}
            markerWarning={containsConflictMarkers(resultContent)}
            tr={tr}
          />
        </div>
      </div>
    </div>
  );
};

const choiceLabel = (
  choice: ConflictHunkChoice,
  tr: WorktreeMergeConflictsViewProps['tr'],
) => {
  if (choice === 'current') {
    return tr('worktree.merge.choice.current', 'Current');
  }
  if (choice === 'session') {
    return tr('worktree.merge.choice.source', 'Source');
  }
  return tr('worktree.merge.choice.both', 'Both');
};

const HunkDiffView: React.FC<{
  hunk: ConflictHunk;
  choice: ConflictHunkChoice | undefined;
  tr: WorktreeMergeConflictsViewProps['tr'];
  onChoose: (choice: ConflictHunkChoice) => void;
}> = ({ hunk, choice, tr, onChoose }) => (
  <div className="flex min-h-0 flex-col overflow-hidden">
    <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--hairline)] px-3 py-2">
      <span className="font-mono text-[11px] text-[var(--ink-tertiary)]">
        {tr('worktree.merge.lines', 'Lines {start}-{end}', {
          start: hunk.startLine,
          end: hunk.endLine,
        })}
      </span>
      <div className="ml-auto flex flex-wrap items-center justify-end gap-1">
        <ConflictChoiceButton
          selected={choice === 'current'}
          label={tr('worktree.merge.acceptCurrent', 'Accept current')}
          onClick={() => onChoose('current')}
        />
        <ConflictChoiceButton
          selected={choice === 'session'}
          label={tr('worktree.merge.acceptSource', 'Accept source')}
          onClick={() => onChoose('session')}
        />
        <ConflictChoiceButton
          selected={choice === 'both'}
          label={tr('worktree.merge.acceptBoth', 'Accept both')}
          onClick={() => onChoose('both')}
        />
      </div>
    </div>
    <div className="grid min-h-0 flex-1 grid-cols-1 divide-y divide-[var(--hairline)] overflow-hidden xl:grid-cols-2 xl:divide-x xl:divide-y-0">
      <ConflictPane
        title={tr('worktree.merge.pane.current', 'Current')}
        subtitle={hunk.currentLabel}
        content={hunk.current}
        emptyHint={tr('worktree.merge.emptyCurrent', 'No current-side content')}
        tone="current"
      />
      <ConflictPane
        title={tr('worktree.merge.pane.source', 'Source')}
        subtitle={hunk.sessionLabel}
        content={hunk.session}
        emptyHint={tr('worktree.merge.emptySession', 'No source-side content')}
        tone="source"
      />
    </div>
    {hunk.base !== null && (
      <div className="max-h-24 shrink-0 border-t border-[var(--hairline)]">
        <ConflictPane
          title={tr('worktree.merge.pane.base', 'Base')}
          subtitle={hunk.baseLabel ?? undefined}
          content={hunk.base}
          emptyHint={tr('worktree.merge.emptyBase', 'No base content')}
        />
      </div>
    )}
  </div>
);

const ConflictChoiceButton: React.FC<{
  selected: boolean;
  label: string;
  onClick: () => void;
}> = ({ selected, label, onClick }) => (
  <button
    type="button"
    onClick={onClick}
    className={`h-6 max-w-full rounded-md px-2 text-[11px] font-semibold transition ${
      selected
        ? 'bg-[var(--primary)] text-[var(--on-primary)]'
        : 'border border-[var(--hairline)] text-[var(--ink-subtle)] hover:bg-[var(--surface-3)] hover:text-[var(--ink)]'
    }`}
    title={label}
  >
    <span className="block truncate">{label}</span>
  </button>
);

const ConflictPane: React.FC<{
  title: string;
  subtitle?: string;
  content: string;
  emptyHint: string;
  tone?: 'current' | 'source';
}> = ({ title, subtitle, content, emptyHint, tone }) => (
  <div className="flex min-h-0 flex-col">
    <div
      className={`shrink-0 border-b border-[var(--hairline)] px-2 py-1 text-[10px] font-semibold uppercase tracking-wide ${
        tone === 'current'
          ? 'text-sky-700'
          : tone === 'source'
            ? 'text-emerald-700'
            : 'text-[var(--ink-tertiary)]'
      }`}
    >
      <span>{title}</span>
      {subtitle && (
        <span className="ml-2 normal-case tracking-normal text-[var(--ink-tertiary)]">
          {subtitle}
        </span>
      )}
    </div>
    {content.length === 0 ? (
      <div className="flex flex-1 items-center justify-center px-2 text-center text-[10px] text-[var(--ink-tertiary)]">
        {emptyHint}
      </div>
    ) : (
      <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words p-2 font-mono text-[11px] leading-snug text-[var(--ink-subtle)]">
        {content}
      </pre>
    )}
  </div>
);

const ResultPane: React.FC<{
  title: string;
  content: string;
  markerWarning: boolean;
  tr: WorktreeMergeConflictsViewProps['tr'];
  onChange: (content: string) => void;
}> = ({ title, content, markerWarning, tr, onChange }) => (
  <div className="flex min-h-0 flex-col border-t border-[var(--hairline)]">
    <div className="flex shrink-0 items-center gap-2 border-b border-[var(--hairline)] px-2 py-1">
      <span className="text-[10px] font-semibold uppercase tracking-wide text-[var(--ink-tertiary)]">
        {title}
      </span>
      {markerWarning && (
        <span className="rounded bg-rose-500/10 px-1.5 py-0.5 text-[10px] font-medium text-rose-600">
          {tr('worktree.merge.markersRemain', 'Conflict markers remain')}
        </span>
      )}
    </div>
    <textarea
      value={content}
      onChange={(e) => onChange(e.target.value)}
      spellCheck={false}
      className={`min-h-0 flex-1 resize-none border-0 bg-transparent p-2 font-mono text-[11px] leading-snug text-[var(--ink)] outline-none ${
        markerWarning ? 'shadow-[inset_3px_0_0_#e11d48]' : ''
      }`}
    />
  </div>
);

const QuickAction: React.FC<{
  label: string;
  onClick: () => void;
}> = ({ label, onClick }) => (
  <WorktreeQuickActionButton label={label} onClick={onClick} />
);

interface BinaryConflictEditorProps {
  path: string;
  status: string;
  detail: ConflictFileContent;
  choice: 'current' | 'session' | 'deleted' | undefined;
  tr: WorktreeMergeConflictsViewProps['tr'];
  onChoose: (choice: 'current' | 'session' | 'deleted') => void;
}

const BinaryConflictEditor: React.FC<BinaryConflictEditorProps> = ({
  path,
  status,
  detail,
  choice,
  tr,
  onChoose,
}) => (
  <div className="flex min-h-0 flex-1 flex-col px-4 py-3">
    <div className="mb-3 flex min-w-0 items-start gap-2">
      <FileWarning className="h-4 w-4 shrink-0 text-amber-600" />
      <div className="min-w-0 flex-1">
        <p className="truncate font-mono text-[12px] text-[var(--ink)]">
          {path}
        </p>
        <p className="text-[11px] text-[var(--ink-tertiary)]">
          {tr(
            'worktree.merge.binaryHint',
            'Binary, large, or deleted/renamed conflict - pick one version',
          )}
          {' - '}
          {status.replaceAll('_', ' ')}
        </p>
      </div>
    </div>
    <div className="grid grid-cols-1 gap-2">
      <BinaryChoiceCard
        title={tr('worktree.merge.keepCurrent', 'Keep current')}
        description={
          detail.current !== null
            ? tr(
                'worktree.merge.keepCurrentHint',
                'Use the base workspace version',
              )
            : tr(
                'worktree.merge.keepCurrentMissing',
                'File does not exist in base',
              )
        }
        disabled={detail.current === null}
        selected={choice === 'current'}
        onSelect={() => onChoose('current')}
      />
      <BinaryChoiceCard
        title={tr('worktree.merge.useSession', 'Use session version')}
        description={
          detail.session !== null
            ? tr(
                'worktree.merge.useSessionHint',
                'Use the session worktree version',
              )
            : tr(
                'worktree.merge.useSessionMissing',
                'File does not exist in session',
              )
        }
        disabled={detail.session === null}
        selected={choice === 'session'}
        onSelect={() => onChoose('session')}
      />
      <BinaryChoiceCard
        title={tr('worktree.merge.deleteFile', 'Delete file')}
        description={tr(
          'worktree.merge.deleteHint',
          'Remove the file from the result',
        )}
        selected={choice === 'deleted'}
        onSelect={() => onChoose('deleted')}
      />
    </div>
  </div>
);

const BinaryChoiceCard: React.FC<{
  title: string;
  description: string;
  disabled?: boolean;
  selected: boolean;
  onSelect: () => void;
}> = ({ title, description, disabled, selected, onSelect }) => (
  <WorktreeConflictChoiceCard
    title={title}
    description={description}
    disabled={disabled}
    selected={selected}
    selectedIcon={<Check className="h-3 w-3 text-[var(--primary)]" />}
    onSelect={onSelect}
  />
);
