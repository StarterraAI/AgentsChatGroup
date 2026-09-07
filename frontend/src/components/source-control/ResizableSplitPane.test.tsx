import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { ResizableSplitPane } from './ResizableSplitPane';

const dom = new JSDOM(
  '<!doctype html><html><body><div id="root"></div></body></html>',
  { url: 'http://localhost' },
);
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  Element: dom.window.Element,
  HTMLElement: dom.window.HTMLElement,
  Node: dom.window.Node,
  Event: dom.window.Event,
});
Object.defineProperty(globalThis, 'navigator', {
  configurable: true,
  value: dom.window.navigator,
});
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
  .IS_REACT_ACT_ENVIRONMENT = true;

const root = createRoot(document.getElementById('root')!);
await act(async () =>
  root.render(
    <ResizableSplitPane left={<div>left</div>} right={<div>right</div>} />,
  ),
);

const separator = document.querySelector<HTMLElement>('[role="separator"]')!;
assert.ok(separator, 'divider renders with role=separator');
assert.equal(separator.getAttribute('aria-orientation'), 'vertical');
assert.ok(separator.classList.contains('cursor-col-resize'));
assert.ok(separator.classList.contains('select-none'));
assert.ok(separator.classList.contains('touch-none'));

const container = separator.parentElement!;
const leftPane = container.firstElementChild as HTMLElement;
const rightPane = container.lastElementChild as HTMLElement;

// jsdom reports zero-sized boxes; pin a 1006px-wide container so the
// draggable area is exactly 1000px after the 6px divider.
container.getBoundingClientRect = () =>
  ({
    left: 0,
    right: 1006,
    width: 1006,
    top: 0,
    bottom: 100,
    height: 100,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  }) as DOMRect;

// Initial layout: strictly equal shares of the space left after the divider.
assert.equal(leftPane.style.getPropertyValue('--split-left'), '0.5');
assert.equal(rightPane.style.getPropertyValue('--split-right'), '0.5');

const dispatchMove = async (clientX: number) => {
  const event = new dom.window.Event('pointermove');
  Object.assign(event, { clientX });
  await act(async () => {
    dom.window.dispatchEvent(event);
  });
};

// Drag: disables text selection for the duration and updates the shares.
await act(async () => {
  separator.dispatchEvent(new dom.window.Event('pointerdown', { bubbles: true }));
});
assert.equal(document.body.style.userSelect, 'none');

await dispatchMove(750);
assert.equal(leftPane.style.getPropertyValue('--split-left'), '0.75');
assert.equal(rightPane.style.getPropertyValue('--split-right'), '0.25');

// Right edge clamp: the right pane never drops below 240px of the 1000px
// draggable area, i.e. the left share caps at 0.76.
await dispatchMove(10000);
assert.equal(leftPane.style.getPropertyValue('--split-left'), '0.76');
assert.equal(rightPane.style.getPropertyValue('--split-right'), '0.24');

// Left edge clamp: symmetric minimum for the left pane.
await dispatchMove(-500);
assert.equal(leftPane.style.getPropertyValue('--split-left'), '0.24');
assert.equal(rightPane.style.getPropertyValue('--split-right'), '0.76');

// Releasing restores text selection and freezes the share until the next drag.
await act(async () => {
  dom.window.dispatchEvent(new dom.window.Event('pointerup'));
});
assert.equal(document.body.style.userSelect, '');
assert.equal(leftPane.style.getPropertyValue('--split-left'), '0.24');

console.log('ResizableSplitPane tests passed');
