import React, { useRef, useState } from 'react';

const MIN_PANE_WIDTH_PX = 240;
const DIVIDER_WIDTH_PX = 6;

interface ResizableSplitPaneProps {
  left: React.ReactNode;
  right: React.ReactNode;
}

// Side-by-side panes with a draggable divider. The left share starts at 50%
// and only changes through divider drags. It is stored as a fraction of the
// space left after the fixed divider and applied via flex-grow, so the
// initial layout is strictly equal, both panes keep the 240px minimum while
// dragging, and window or container resizes never recompute an adjusted
// split.
export const ResizableSplitPane: React.FC<ResizableSplitPaneProps> = ({
  left,
  right,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const [leftShare, setLeftShare] = useState(0.5);

  const handleDividerPointerDown = (
    event: React.PointerEvent<HTMLDivElement>,
  ) => {
    // Prevent the drag from starting a text selection inside the editors.
    event.preventDefault();
    const container = containerRef.current;
    if (!container) return;
    const rect = container.getBoundingClientRect();
    const available = rect.width - DIVIDER_WIDTH_PX;
    if (available <= 0) return;
    const previousUserSelect = document.body.style.userSelect;
    document.body.style.userSelect = 'none';

    const handleMove = (moveEvent: PointerEvent) => {
      const minShare = Math.min(MIN_PANE_WIDTH_PX / available, 0.5);
      const share = (moveEvent.clientX - rect.left) / available;
      setLeftShare(Math.min(1 - minShare, Math.max(minShare, share)));
    };
    const handleUp = () => {
      document.body.style.userSelect = previousUserSelect;
      window.removeEventListener('pointermove', handleMove);
      window.removeEventListener('pointerup', handleUp);
    };
    window.addEventListener('pointermove', handleMove);
    window.addEventListener('pointerup', handleUp);
  };

  return (
    <div
      ref={containerRef}
      className="flex min-h-0 flex-col divide-y divide-[var(--hairline)] overflow-hidden lg:flex-row lg:divide-y-0"
    >
      <div
        className="min-h-0 min-w-0 flex-1 lg:grow-[var(--split-left)]"
        style={{ '--split-left': leftShare } as React.CSSProperties}
      >
        {left}
      </div>
      <div
        role="separator"
        aria-orientation="vertical"
        onPointerDown={handleDividerPointerDown}
        className="hidden w-1.5 shrink-0 cursor-col-resize touch-none select-none bg-[var(--hairline)] transition-colors hover:bg-[var(--ink-tertiary)] lg:block"
      />
      <div
        className="min-h-0 min-w-0 flex-1 lg:grow-[var(--split-right)]"
        style={{ '--split-right': 1 - leftShare } as React.CSSProperties}
      >
        {right}
      </div>
    </div>
  );
};
