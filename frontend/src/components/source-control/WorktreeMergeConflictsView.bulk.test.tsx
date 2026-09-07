// Component tests for the "all files" bulk-resolve actions in the merge
// conflict resolver. Covers serial resolution order, delete semantics for a
// missing side, partial-failure reporting, and double-click prevention.
//
// Run with:
//     pnpm exec tsx src/components/source-control/WorktreeMergeConflictsView.bulk.test.tsx

import { JSDOM } from 'jsdom';

const dom = new JSDOM('<!doctype html><html><body></body></html>', {
  url: 'http://localhost',
});
Object.defineProperties(globalThis, {
  window: { value: dom.window, configurable: true },
  document: { value: dom.window.document, configurable: true },
  navigator: { value: dom.window.navigator, configurable: true },
  HTMLElement: { value: dom.window.HTMLElement, configurable: true },
  Event: { value: dom.window.Event, configurable: true },
  MouseEvent: { value: dom.window.MouseEvent, configurable: true },
  Node: { value: dom.window.Node, configurable: true },
});
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
  .IS_REACT_ACT_ENVIRONMENT = true;

const React = await import('react');
const { act } = React;
const { createRoot } = await import('react-dom/client');
const { WorktreeMergeConflictsView } = await import(
  './WorktreeMergeConflictsView'
);

let failures = 0;
const check = (label: string, condition: boolean, detail?: unknown) => {
  if (condition) {
    // eslint-disable-next-line no-console
    console.log(`  ok  ${label}`);
    return;
  }
  failures += 1;
  // eslint-disable-next-line no-console
  console.error(`  FAIL ${label}`, detail ?? '');
};

const tr = (
  _key: string,
  fallback: string,
  replacements?: Record<string, string | number>,
) =>
  replacements
    ? Object.entries(replacements).reduce(
        (text, [name, value]) =>
          text.replace(`{${name}}`, String(value)),
        fallback,
      )
    : fallback;

interface ResolveCall {
  path: string;
  use_stage?: string | null;
  delete_file?: boolean;
}

interface MockState {
  lists: Array<Array<{ path: string; status: string }>>;
  details: Record<string, unknown>;
  failResolveOn?: string;
  resolveCalls: ResolveCall[];
}

const installFetchMock = (state: MockState) => {
  globalThis.fetch = (async (
    input: RequestInfo | URL,
    init?: RequestInit,
  ) => {
    const url = String(input);
    const json = (body: unknown, status = 200) =>
      new Response(JSON.stringify(body), { status });
    if (url.includes('/merge-conflicts/resolve')) {
      const body = JSON.parse(String(init?.body ?? '{}')) as ResolveCall;
      state.resolveCalls.push(body);
      if (body.path === state.failResolveOn) {
        return json({ success: false, message: 'resolve boom' }, 500);
      }
      return json({ success: true, data: null });
    }
    if (url.includes('/merge-conflicts/')) {
      const path = decodeURIComponent(url.split('/merge-conflicts/')[1]);
      return json({ success: true, data: state.details[path] });
    }
    if (url.endsWith('/merge-conflicts')) {
      const next =
        state.lists.length > 1 ? state.lists.shift() : state.lists[0];
      return json({ success: true, data: next });
    }
    if (url.endsWith('/worktree')) {
      return json({ success: true, data: null });
    }
    throw new Error(`unexpected fetch: ${url}`);
  }) as typeof fetch;
};

const flush = async (rounds = 8) => {
  for (let i = 0; i < rounds; i += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
};

const findButton = (container: HTMLElement, text: string) =>
  Array.from(container.querySelectorAll('button')).find((button) =>
    (button.textContent ?? '').includes(text),
  );

const click = (button: HTMLButtonElement) => {
  button.dispatchEvent(
    new dom.window.MouseEvent('click', { bubbles: true, cancelable: true }),
  );
};

const detailFor = (path: string, overrides: Record<string, unknown> = {}) => ({
  path,
  base: null,
  current: 'current-side',
  session: 'session-side',
  working_tree: '',
  is_binary: true,
  is_too_large: false,
  size_bytes: 8,
  ...overrides,
});

const render = (container: HTMLElement) => {
  const root = createRoot(container);
  act(() => {
    root.render(
      React.createElement(WorktreeMergeConflictsView, {
        sessionId: 's1',
        tr,
        onCompleted: () => undefined,
        onAbort: () => undefined,
      }),
    );
  });
  return root;
};

console.log('WorktreeMergeConflictsView bulk resolve');

// --- Happy path: serial order, delete semantics, double-click guard ---

{
  const state: MockState = {
    lists: [
      [
        { path: 'a.txt', status: 'deleted_by_us' },
        { path: 'b.bin', status: 'both_modified' },
      ],
      [],
    ],
    details: {
      'a.txt': detailFor('a.txt', { session: null }),
      'b.bin': detailFor('b.bin'),
    },
    resolveCalls: [],
  };
  installFetchMock(state);
  const container = document.createElement('div');
  document.body.appendChild(container);
  const root = render(container);
  await flush();

  const button = findButton(container, 'All files: use source') as
    | HTMLButtonElement
    | undefined;
  check('bulk "use source" button is rendered', Boolean(button));
  check(
    'bulk button enabled while conflicts remain',
    Boolean(button) && !button?.disabled,
  );

  // Two synchronous clicks: the ref guard must drop the second one.
  click(button as HTMLButtonElement);
  click(button as HTMLButtonElement);
  await flush();

  check(
    'double click resolves each file exactly once (no parallel run)',
    state.resolveCalls.length === 2,
    state.resolveCalls,
  );
  check(
    'resolution runs serially in list order',
    state.resolveCalls[0]?.path === 'a.txt' &&
      state.resolveCalls[1]?.path === 'b.bin',
    state.resolveCalls,
  );
  check(
    'missing session side sends delete_file',
    state.resolveCalls[0]?.delete_file === true &&
      state.resolveCalls[0]?.use_stage == null,
    state.resolveCalls[0],
  );
  check(
    'existing session side sends use_stage session',
    state.resolveCalls[1]?.use_stage === 'session' &&
      state.resolveCalls[1]?.delete_file !== true,
    state.resolveCalls[1],
  );
  check(
    'list refreshes to empty and no error is shown',
    container.textContent?.includes('No conflicts remaining') === true,
    container.textContent,
  );
  const continueButton = findButton(container, 'Continue merge') as
    | HTMLButtonElement
    | undefined;
  check(
    'Continue merge is enabled after bulk resolve',
    Boolean(continueButton) && !continueButton?.disabled,
  );

  await act(async () => {
    root.unmount();
  });
  container.remove();
}

// --- Partial failure: stop, report failed file and completed count ---

{
  const state: MockState = {
    lists: [
      [
        { path: 'a.txt', status: 'both_modified' },
        { path: 'b.bin', status: 'both_modified' },
      ],
      [{ path: 'b.bin', status: 'both_modified' }],
    ],
    details: {
      'a.txt': detailFor('a.txt'),
      'b.bin': detailFor('b.bin'),
    },
    failResolveOn: 'b.bin',
    resolveCalls: [],
  };
  installFetchMock(state);
  const container = document.createElement('div');
  document.body.appendChild(container);
  const root = render(container);
  await flush();

  const button = findButton(container, 'All files: keep current') as
    | HTMLButtonElement
    | undefined;
  check('bulk "keep current" button is rendered', Boolean(button));
  click(button as HTMLButtonElement);
  await flush();

  check(
    'failing file stops the batch (later files untouched)',
    state.resolveCalls.length === 2 &&
      state.resolveCalls[1]?.path === 'b.bin',
    state.resolveCalls,
  );
  check(
    'failure summary names failed file and completed count',
    container.textContent?.includes(
      'Resolved 1 of 2 files; failed on b.bin: resolve boom',
    ) === true,
    container.textContent,
  );
  check(
    'remaining conflict is still listed for retry',
    container.querySelector('ul')?.textContent?.includes('b.bin') === true,
    container.querySelector('ul')?.textContent,
  );
  const retryButton = findButton(container, 'All files: keep current') as
    | HTMLButtonElement
    | undefined;
  check(
    'bulk button is usable again after failure',
    Boolean(retryButton) && !retryButton?.disabled,
  );

  await act(async () => {
    root.unmount();
  });
  container.remove();
}

if (failures > 0) {
  // eslint-disable-next-line no-console
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
process.exit(0);
