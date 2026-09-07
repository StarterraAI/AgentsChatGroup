// Behavior tests for bulk conflict resolution across all files.
//
// Run with:
//     pnpm exec tsx src/components/source-control/mergeConflictBulkResolve.test.ts

import {
  buildBulkResolveRequest,
  runBulkResolve,
  type BulkResolveDeps,
} from './mergeConflictBulkResolve';
import type { ConflictFileContent, ConflictFileInfo } from '@/types';
import type { ResolveSessionWorktreeConflictRequest } from '@/lib/api';

let failures = 0;
const check = (label: string, cond: boolean, detail?: unknown) => {
  if (cond) {
    // eslint-disable-next-line no-console
    console.log(`  ok  ${label}`);
  } else {
    failures += 1;
    // eslint-disable-next-line no-console
    console.error(`  FAIL ${label}`, detail ?? '');
  }
};

const detailFor = (
  path: string,
  overrides: Partial<ConflictFileContent> = {},
): ConflictFileContent => ({
  path,
  base: 'base',
  current: 'current',
  session: 'session',
  working_tree: '<<<<<<<',
  is_binary: false,
  is_too_large: false,
  size_bytes: 8,
  ...overrides,
});

console.log('mergeConflictBulkResolve behavior');

// --- buildBulkResolveRequest: side selection and delete semantics ---

{
  const cases: Array<[string, Partial<ConflictFileContent>]> = [
    ['text conflict', {}],
    ['binary conflict', { is_binary: true, working_tree: '' }],
    ['too-large conflict', { is_too_large: true, size_bytes: 300_000 }],
    ['renamed conflict', { working_tree: '' }],
  ];
  for (const [label, overrides] of cases) {
    const detail = detailFor('src/a.txt', overrides);
    for (const side of ['current', 'session'] as const) {
      const req = buildBulkResolveRequest(detail, side);
      check(
        `${label} with both sides uses stage ${side}`,
        req.use_stage === side && req.delete_file !== true && !req.content,
        req,
      );
    }
  }
}

{
  // deleted_by_them: the current side has no version of the file.
  const detail = detailFor('src/gone.txt', { current: null });
  const req = buildBulkResolveRequest(detail, 'current');
  check(
    'missing current side resolves to delete_file',
    req.delete_file === true && req.use_stage == null,
    req,
  );
  const keep = buildBulkResolveRequest(detail, 'session');
  check(
    'opposite side still uses stage when it exists',
    keep.use_stage === 'session' && keep.delete_file !== true,
    keep,
  );
}

{
  // deleted_by_us: the session side has no version of the file.
  const detail = detailFor('src/gone.txt', { session: null });
  const req = buildBulkResolveRequest(detail, 'session');
  check(
    'missing session side resolves to delete_file',
    req.delete_file === true && req.use_stage == null,
    req,
  );
}

{
  // both_deleted: neither side has a version.
  const detail = detailFor('src/gone.txt', { current: null, session: null });
  for (const side of ['current', 'session'] as const) {
    const req = buildBulkResolveRequest(detail, side);
    check(
      `both_deleted resolves to delete_file for ${side}`,
      req.delete_file === true,
      req,
    );
  }
}

// --- runBulkResolve: serial execution, progress, partial failure ---

const files = (...paths: string[]): ConflictFileInfo[] =>
  paths.map((path) => ({ path, status: 'both_modified' }));

interface RecordedCall {
  kind: 'detail' | 'resolve';
  path: string;
}

const makeDeps = (
  calls: RecordedCall[],
  details: Record<string, ConflictFileContent>,
  failOnResolvePath?: string,
  failOnDetailPath?: string,
): BulkResolveDeps => ({
  getMergeConflictDetail: async (_sessionId, filePath) => {
    calls.push({ kind: 'detail', path: filePath });
    if (filePath === failOnDetailPath) throw new Error('detail boom');
    const detail = details[filePath];
    if (!detail) throw new Error(`no detail for ${filePath}`);
    return detail;
  },
  resolveMergeConflict: async (
    _sessionId,
    request: ResolveSessionWorktreeConflictRequest,
  ) => {
    calls.push({ kind: 'resolve', path: request.path });
    if (request.path === failOnResolvePath) throw new Error('resolve boom');
  },
});

{
  const calls: RecordedCall[] = [];
  const details = {
    'a.txt': detailFor('a.txt'),
    'b.txt': detailFor('b.txt', { session: null }),
    'c.txt': detailFor('c.txt'),
  };
  const progress: Array<[number, number]> = [];
  const outcome = await runBulkResolve(
    'session-1',
    files('a.txt', 'b.txt', 'c.txt'),
    'session',
    makeDeps(calls, details),
    (done, total) => progress.push([done, total]),
  );
  check(
    'success resolves every file exactly once, in list order',
    JSON.stringify(calls) ===
      JSON.stringify([
        { kind: 'detail', path: 'a.txt' },
        { kind: 'resolve', path: 'a.txt' },
        { kind: 'detail', path: 'b.txt' },
        { kind: 'resolve', path: 'b.txt' },
        { kind: 'detail', path: 'c.txt' },
        { kind: 'resolve', path: 'c.txt' },
      ]),
    calls,
  );
  check(
    'success reports full completion and no failure',
    outcome.completed === 3 &&
      outcome.total === 3 &&
      outcome.failedPath === null &&
      outcome.error === null,
    outcome,
  );
  check(
    'progress advances after each resolved file',
    JSON.stringify(progress) ===
      JSON.stringify([
        [1, 3],
        [2, 3],
        [3, 3],
      ]),
    progress,
  );
}

{
  const calls: RecordedCall[] = [];
  const details = {
    'a.txt': detailFor('a.txt'),
    'b.txt': detailFor('b.txt'),
    'c.txt': detailFor('c.txt'),
  };
  const outcome = await runBulkResolve(
    'session-1',
    files('a.txt', 'b.txt', 'c.txt'),
    'current',
    makeDeps(calls, details, 'b.txt'),
  );
  check(
    'partial failure stops after the failing file',
    JSON.stringify(calls) ===
      JSON.stringify([
        { kind: 'detail', path: 'a.txt' },
        { kind: 'resolve', path: 'a.txt' },
        { kind: 'detail', path: 'b.txt' },
        { kind: 'resolve', path: 'b.txt' },
      ]),
    calls,
  );
  check(
    'partial failure reports completed count, failed path and error',
    outcome.completed === 1 &&
      outcome.total === 3 &&
      outcome.failedPath === 'b.txt' &&
      outcome.error === 'resolve boom',
    outcome,
  );
}

{
  const calls: RecordedCall[] = [];
  const details = { 'a.txt': detailFor('a.txt') };
  const outcome = await runBulkResolve(
    'session-1',
    files('a.txt', 'b.txt'),
    'current',
    makeDeps(calls, details, undefined, 'a.txt'),
  );
  check(
    'detail fetch failure reports zero completed',
    outcome.completed === 0 && outcome.failedPath === 'a.txt',
    outcome,
  );
}

if (failures > 0) {
  // eslint-disable-next-line no-console
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
