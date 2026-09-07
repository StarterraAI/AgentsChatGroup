import {
  chatSessionWorktreeApi,
  type ResolveSessionWorktreeConflictRequest,
} from '@/lib/api';
import type { ConflictFileContent, ConflictFileInfo } from '@/types';

export type BulkResolveSide = 'current' | 'session';

// When the chosen side has no version of the file (deleted on that side), the
// only consistent resolution is deleting the file; otherwise take that stage.
export const buildBulkResolveRequest = (
  detail: ConflictFileContent,
  side: BulkResolveSide,
): ResolveSessionWorktreeConflictRequest =>
  detail[side] == null
    ? { path: detail.path, delete_file: true }
    : { path: detail.path, use_stage: side };

export interface BulkResolveOutcome {
  total: number;
  completed: number;
  failedPath: string | null;
  error: string | null;
}

export interface BulkResolveDeps {
  getMergeConflictDetail: (
    sessionId: string,
    filePath: string,
  ) => Promise<ConflictFileContent>;
  resolveMergeConflict: (
    sessionId: string,
    request: ResolveSessionWorktreeConflictRequest,
  ) => Promise<void>;
}

// Resolves every conflict file to one side strictly serially: each resolve
// writes the Git index, and concurrent writes race on `index.lock`. Stops at
// the first failure so the user can inspect and retry the remaining files.
export const runBulkResolve = async (
  sessionId: string,
  files: ConflictFileInfo[],
  side: BulkResolveSide,
  deps: BulkResolveDeps = chatSessionWorktreeApi,
  onProgress?: (completed: number, total: number) => void,
): Promise<BulkResolveOutcome> => {
  const total = files.length;
  let completed = 0;
  for (const file of files) {
    try {
      const detail = await deps.getMergeConflictDetail(sessionId, file.path);
      await deps.resolveMergeConflict(
        sessionId,
        buildBulkResolveRequest(detail, side),
      );
      completed += 1;
      onProgress?.(completed, total);
    } catch (err) {
      return {
        total,
        completed,
        failedPath: file.path,
        error: err instanceof Error ? err.message : String(err),
      };
    }
  }
  return { total, completed, failedPath: null, error: null };
};
