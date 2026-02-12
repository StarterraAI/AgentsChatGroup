import type { ChangeEvent, RefObject } from 'react';
import { XIcon } from '@phosphor-icons/react';
import { cn } from '@/lib/utils';
import { PrimaryButton } from '@/components/ui-new/primitives/PrimaryButton';

export interface PromptEditorModalProps {
  isOpen: boolean;
  value: string;
  onChange: (value: string) => void;
  onClose: () => void;
  promptFileInputRef: RefObject<HTMLInputElement>;
  onPromptFileChange: (event: ChangeEvent<HTMLInputElement>) => void;
  promptFileLoading: boolean;
  promptFileError: string | null;
}

export function PromptEditorModal({
  isOpen,
  value,
  onChange,
  onClose,
  promptFileInputRef,
  onPromptFileChange,
  promptFileLoading,
  promptFileError,
}: PromptEditorModalProps) {
  if (!isOpen) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
    >
      <div
        className="chat-session-modal-surface chat-session-prompt-modal border border-border shadow-xl flex flex-col overflow-hidden w-[92vw] h-[80vh] max-w-[1200px] rounded-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-border px-base py-half">
          <div>
            <div className="text-sm text-normal font-medium">System Prompt</div>
            <div className="text-xs text-low">
              Edit the AI member system prompt
            </div>
          </div>
          <button
            type="button"
            className="text-low hover:text-normal"
            onClick={onClose}
            aria-label="Close prompt editor"
          >
            <XIcon className="size-icon-sm" />
          </button>
        </div>
        <div className="flex-1 overflow-hidden p-base flex flex-col gap-base">
          <textarea
            value={value}
            onChange={(event) => onChange(event.target.value)}
            placeholder="Describe how this AI member should behave."
            className={cn(
              'chat-session-prompt-modal-input flex-1 w-full resize-none rounded-sm border border-border bg-panel',
              'px-base py-base text-sm text-normal leading-relaxed focus:outline-none focus:ring-1 focus:ring-brand'
            )}
          />
          <div className="flex items-center justify-between gap-base">
            <div className="flex items-center gap-half text-xs text-low">
              <button
                type="button"
                className="chat-session-member-expand-btn"
                onClick={() => promptFileInputRef.current?.click()}
                disabled={promptFileLoading}
              >
                Attach text file
              </button>
              <input
                ref={promptFileInputRef}
                type="file"
                accept=".txt,.md,.prompt,text/plain"
                className="hidden"
                onChange={onPromptFileChange}
              />
              {promptFileLoading && <span>Loading file...</span>}
              {promptFileError && (
                <span className="text-error">{promptFileError}</span>
              )}
            </div>
            <PrimaryButton value="Done" onClick={onClose} />
          </div>
        </div>
      </div>
    </div>
  );
}
