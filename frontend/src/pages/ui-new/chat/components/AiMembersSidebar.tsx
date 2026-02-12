import { PlusIcon } from '@phosphor-icons/react';
import { ChatSessionAgentState } from 'shared/types';
import { cn } from '@/lib/utils';
import { PrimaryButton } from '@/components/ui-new/primitives/PrimaryButton';
import { Tooltip } from '@/components/ui-new/primitives/Tooltip';
import { toPrettyCase } from '@/utils/string';
import type { SessionMember } from '../types';
import { agentStateLabels, agentStateDotClass } from '../constants';

const truncateByChars = (value: string, maxChars: number): string => {
  const chars = Array.from(value);
  if (chars.length <= maxChars) return value;
  return `${chars.slice(0, maxChars).join('')}...`;
};

export interface AiMembersSidebarProps {
  sessionMembers: SessionMember[];
  agentStates: Record<string, ChatSessionAgentState>;
  activeSessionId: string | null;
  isArchived: boolean;
  width: number;
  // Member form
  isAddMemberOpen: boolean;
  editingMember: SessionMember | null;
  newMemberName: string;
  newMemberRunnerType: string;
  newMemberVariant: string;
  newMemberPrompt: string;
  newMemberWorkspace: string;
  memberNameLengthError: string | null;
  onNameChange: (value: string) => void;
  onRunnerTypeChange: (value: string) => void;
  onVariantChange: (value: string) => void;
  onPromptChange: (value: string) => void;
  onWorkspaceChange: (value: string) => void;
  memberError: string | null;
  isSavingMember: boolean;
  // Runner availability
  availableRunnerTypes: string[];
  enabledRunnerTypes: string[];
  isCheckingAvailability: boolean;
  isRunnerAvailable: (runner: string) => boolean;
  availabilityLabel: (runner: string) => string;
  memberVariantOptions: string[];
  getModelName: (runnerType: string, variant?: string) => string | null;
  // Actions
  onOpenAddMember: () => void;
  onCancelMember: () => void;
  onSaveMember: () => void;
  onEditMember: (member: SessionMember) => void;
  onRemoveMember: (member: SessionMember) => void;
  onOpenWorkspace: (agentId: string) => void;
  onExpandPromptEditor: () => void;
}

export function AiMembersSidebar({
  sessionMembers,
  agentStates,
  activeSessionId,
  isArchived,
  width,
  isAddMemberOpen,
  editingMember,
  newMemberName,
  newMemberRunnerType,
  newMemberVariant,
  newMemberPrompt,
  newMemberWorkspace,
  memberNameLengthError,
  onNameChange,
  onRunnerTypeChange,
  onVariantChange,
  onPromptChange,
  onWorkspaceChange,
  memberError,
  isSavingMember,
  availableRunnerTypes,
  enabledRunnerTypes,
  isCheckingAvailability,
  isRunnerAvailable,
  availabilityLabel,
  memberVariantOptions,
  getModelName,
  onOpenAddMember,
  onCancelMember,
  onSaveMember,
  onEditMember,
  onRemoveMember,
  onOpenWorkspace,
  onExpandPromptEditor,
}: AiMembersSidebarProps) {
  return (
    <aside
      className="chat-session-members-panel border-l border-border flex flex-col min-h-0 shrink-0"
      style={{ width }}
    >
      <div className="chat-session-members-header px-base py-base border-b border-border flex items-center justify-between">
        <div className="chat-session-members-title text-sm text-normal font-medium">
          AI Members
        </div>
        <div className="chat-session-members-count text-xs text-low">
          {sessionMembers.length} in session
        </div>
      </div>
      <div className="chat-session-members-list flex-1 min-h-0 overflow-y-auto p-base space-y-base">
        {!activeSessionId && (
          <div className="chat-session-members-empty text-xs text-low mt-base">
            Select a session to manage AI members.
          </div>
        )}
        {activeSessionId && sessionMembers.length === 0 && (
          <div className="chat-session-members-empty text-xs text-low mt-base">
            No AI members yet. Add one below to enable @mentions.
          </div>
        )}
        {sessionMembers.map(({ agent, sessionAgent }) => {
          const state =
            agentStates[agent.id] ?? ChatSessionAgentState.idle;
          const modelName = getModelName(agent.runner_type);
          const fullText = `${toPrettyCase(agent.runner_type)} | ${agentStateLabels[state]}${modelName ? ` | ${modelName}` : ''}`;
          const modelStatusPreview = truncateByChars(fullText, 15);
          const workspacePath = sessionAgent.workspace_path ?? '';
          const shouldShowWorkspaceTooltip =
            workspacePath.length > 48;

          return (
            <div
              key={sessionAgent.id}
              className="chat-session-member-card border border-border rounded-sm px-base py-half space-y-half"
            >
              <div className="flex items-center justify-between gap-base">
                <div className="flex items-center gap-half min-w-0 flex-1">
                  <span
                    className={cn(
                      'size-2 rounded-full',
                      agentStateDotClass[state],
                      state === ChatSessionAgentState.running &&
                        'animate-pulse'
                    )}
                  />
                  <div className="chat-session-member-name text-sm text-normal min-w-0 flex-1">
                    @{agent.name}
                  </div>
                </div>
                <div className="chat-session-member-actions flex items-center gap-half text-xs">
                  <button
                    type="button"
                    className="chat-session-member-action workspace"
                    onClick={() => onOpenWorkspace(agent.id)}
                  >
                    Workspace
                  </button>
                  <button
                    type="button"
                    className={cn(
                      'chat-session-member-action edit',
                      isArchived && 'pointer-events-none opacity-50'
                    )}
                    onClick={() =>
                      onEditMember({ agent, sessionAgent })
                    }
                    disabled={isArchived}
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    className={cn(
                      'chat-session-member-action danger',
                      isArchived && 'pointer-events-none opacity-50'
                    )}
                    onClick={() =>
                      onRemoveMember({ agent, sessionAgent })
                    }
                    disabled={isArchived}
                  >
                    Remove
                  </button>
                </div>
              </div>
              <Tooltip content={fullText} side="bottom">
                <div className="chat-session-member-model text-xs text-low cursor-default">
                  <div className="chat-session-member-model-full">
                    {fullText}
                  </div>
                  <div className="chat-session-member-model-truncated">
                    {modelStatusPreview}
                  </div>
                </div>
              </Tooltip>
              {workspacePath && (
                <div className="chat-session-member-workspace-row">
                  {shouldShowWorkspaceTooltip ? (
                    <Tooltip content={workspacePath} side="bottom">
                      <div className="chat-session-member-workspace text-xs text-low truncate cursor-default">
                        {workspacePath}
                      </div>
                    </Tooltip>
                  ) : (
                    <div className="chat-session-member-workspace text-xs text-low truncate">
                      {workspacePath}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}

        {/* Add Member Form */}
        <div className="chat-session-member-form border-t border-border pt-base space-y-half">
          {!isAddMemberOpen ? (
            <button
              type="button"
              className="chat-session-add-member-btn"
              onClick={onOpenAddMember}
              disabled={!activeSessionId || isArchived}
            >
              Add AI member
              <PlusIcon className="size-icon-xs" weight="light" />
            </button>
          ) : (
            <div className="chat-session-member-form-panel border border-border rounded-sm p-base space-y-half">
              <div className="text-sm text-normal font-medium">
                {editingMember ? 'Edit AI member' : 'Add AI member'}
              </div>
              <div className="text-xs text-low">
                AI member name is the @mention handle.
              </div>
              <div className="space-y-half">
                <label className="text-xs text-low">AI member name</label>
                <input
                  value={newMemberName}
                  onChange={(event) => onNameChange(event.target.value)}
                  placeholder="e.g. coder"
                  className={cn(
                    'chat-session-member-field w-full rounded-sm border border-border bg-panel px-base py-half',
                    'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                  )}
                />
                {memberNameLengthError && (
                  <div className="text-xs text-error">
                    {memberNameLengthError}
                  </div>
                )}
              </div>
              <div className="space-y-half">
                <label className="text-xs text-low">
                  Base coding agent
                </label>
                <select
                  value={newMemberRunnerType}
                  onChange={(event) =>
                    onRunnerTypeChange(event.target.value)
                  }
                  disabled={
                    isCheckingAvailability ||
                    enabledRunnerTypes.length === 0
                  }
                  className={cn(
                    'chat-session-member-field w-full rounded-sm border border-border bg-panel px-base py-half',
                    'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                  )}
                >
                  {enabledRunnerTypes.length === 0 && (
                    <option value="">
                      {isCheckingAvailability
                        ? 'Checking agents...'
                        : 'No local agents detected'}
                    </option>
                  )}
                  {availableRunnerTypes.map((runner) => (
                    <option
                      key={runner}
                      value={runner}
                      disabled={!isRunnerAvailable(runner)}
                    >
                      {toPrettyCase(runner)}
                      {availabilityLabel(runner)}
                    </option>
                  ))}
                </select>
                {enabledRunnerTypes.length === 0 &&
                  !isCheckingAvailability && (
                    <div className="text-xs text-error">
                      No installed code agents detected on this machine.
                    </div>
                  )}
              </div>
              {memberVariantOptions.length > 0 && (
                <div className="space-y-half">
                  <label className="text-xs text-low">
                    Model variant
                  </label>
                  <select
                    value={newMemberVariant}
                    onChange={(event) =>
                      onVariantChange(event.target.value)
                    }
                    className={cn(
                      'chat-session-member-field w-full rounded-sm border border-border bg-panel px-base py-half',
                      'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                    )}
                  >
                    {memberVariantOptions.map((variant) => {
                      const name = getModelName(
                        newMemberRunnerType,
                        variant
                      );
                      return (
                        <option key={variant} value={variant}>
                          {variant}
                          {name ? ` (${name})` : ''}
                        </option>
                      );
                    })}
                  </select>
                  {getModelName(
                    newMemberRunnerType,
                    newMemberVariant
                  ) && (
                    <div className="text-xs text-low">
                      Model:{' '}
                      {getModelName(
                        newMemberRunnerType,
                        newMemberVariant
                      )}
                    </div>
                  )}
                </div>
              )}
              <div className="space-y-half">
                <div className="flex items-center justify-between gap-base">
                  <label className="text-xs text-low">
                    System prompt
                  </label>
                  <button
                    type="button"
                    className="chat-session-member-expand-btn text-xs"
                    onClick={onExpandPromptEditor}
                  >
                    Expand
                  </button>
                </div>
                <textarea
                  value={newMemberPrompt}
                  onChange={(event) =>
                    onPromptChange(event.target.value)
                  }
                  rows={3}
                  placeholder="Describe how this AI member should behave."
                  className={cn(
                    'chat-session-member-field w-full resize-none rounded-sm border border-border bg-panel',
                    'px-base py-half text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand'
                  )}
                />
              </div>
              <div className="space-y-half">
                <label className="text-xs text-low">
                  Workspace path
                </label>
                <input
                  value={newMemberWorkspace}
                  onChange={(event) =>
                    onWorkspaceChange(event.target.value)
                  }
                  placeholder="Absolute path on the server"
                  disabled={!!editingMember}
                  title={
                    editingMember
                      ? 'Workspace path cannot be changed after creation'
                      : undefined
                  }
                  className={cn(
                    'chat-session-member-field w-full rounded-sm border border-border bg-panel px-base py-half',
                    'text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand',
                    editingMember && 'opacity-50 cursor-not-allowed'
                  )}
                />
                {editingMember && (
                  <p className="text-xs text-low">
                    Workspace path cannot be modified
                  </p>
                )}
              </div>
              {memberError && (
                <div className="text-xs text-error">{memberError}</div>
              )}
              <div className="flex items-center justify-end gap-half pt-half">
                <PrimaryButton
                  variant="tertiary"
                  value="Cancel"
                  onClick={onCancelMember}
                  disabled={isSavingMember}
                  className="chat-session-member-btn cancel"
                />
                <PrimaryButton
                  value={editingMember ? 'Save' : 'Add'}
                  actionIcon={isSavingMember ? 'spinner' : PlusIcon}
                  onClick={onSaveMember}
                  disabled={
                    isSavingMember ||
                    isArchived ||
                    !!memberNameLengthError
                  }
                  className="chat-session-member-btn"
                />
              </div>
            </div>
          )}
        </div>
      </div>
    </aside>
  );
}
