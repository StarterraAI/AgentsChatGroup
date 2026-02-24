import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import type { Icon } from '@phosphor-icons/react';
import {
  PlusIcon,
  UsersThreeIcon,
  UserPlusIcon,
  UserIcon,
  CodeIcon,
  BugBeetleIcon,
  MagnifyingGlassIcon,
  ShieldCheckIcon,
  PencilSimpleLineIcon,
  ChartBarIcon,
  LightbulbIcon,
  GearIcon,
  RocketIcon,
  PaintBrushIcon,
  MegaphoneIcon,
  FilmStripIcon,
  BookOpenIcon,
  TreeStructureIcon,
  TerminalIcon,
  TrendUpIcon,
} from '@phosphor-icons/react';
import { useTranslation } from 'react-i18next';
import {
  ChatSessionAgentState,
  type ChatMemberPreset,
  type ChatTeamPreset,
  type JsonValue,
} from 'shared/types';
import { cn } from '@/lib/utils';
import { getWorkspacePathExample } from '@/utils/platform';
import {
  extractExecutorProfileVariant,
  withExecutorProfileVariant,
} from '@/utils/executor';
import { PrimaryButton } from '@/components/ui-new/primitives/PrimaryButton';
import { Tooltip } from '@/components/ui-new/primitives/Tooltip';
import { toPrettyCase } from '@/utils/string';
import type { SessionMember } from '../types';
import { agentStateLabels, agentStateDotClass } from '../constants';
import { PromptEditorModal } from './PromptEditorModal';
import {
  AgentBrandIcon,
  getAgentAvatarSeed,
  getAgentAvatarStyle,
} from '../AgentAvatar';
import {
  getLocalizedMemberPresetName,
  getLocalizedMemberPresetNameById,
  getLocalizedTeamPresetName,
  type MemberPresetImportPlan,
} from '../utils';

const truncateByChars = (value: string, maxChars: number): string => {
  const chars = Array.from(value);
  if (chars.length <= maxChars) return value;
  return `${chars.slice(0, maxChars).join('')}...`;
};

/* Map preset IDs to role-appropriate icons */
const presetRoleIcons: Record<string, Icon> = {
  coordinator_pmo: GearIcon,
  product_manager: LightbulbIcon,
  system_architect: TreeStructureIcon,
  prompt_engineer: PencilSimpleLineIcon,
  frontend_engineer: CodeIcon,
  backend_engineer: TerminalIcon,
  fullstack_engineer: CodeIcon,
  qa_tester: BugBeetleIcon,
  ux_ui_designer: PaintBrushIcon,
  safety_policy_officer: ShieldCheckIcon,
  solution_manager: RocketIcon,
  code_reviewer: MagnifyingGlassIcon,
  devops_engineer: GearIcon,
  product_analyst: ChartBarIcon,
  data_analyst: ChartBarIcon,
  technical_writer: BookOpenIcon,
  content_researcher: MagnifyingGlassIcon,
  content_editor: PencilSimpleLineIcon,
  frontier_researcher: LightbulbIcon,
  marketing_specialist: MegaphoneIcon,
  video_editor: FilmStripIcon,
  market_analyst: TrendUpIcon,
};

const teamRoleIcons: Record<string, Icon> = {
  fullstack_delivery_team: RocketIcon,
  ai_prompt_quality_team: PencilSimpleLineIcon,
  architecture_governance_team: TreeStructureIcon,
  product_discovery_team: LightbulbIcon,
  content_studio_team: FilmStripIcon,
  growth_marketing_team: MegaphoneIcon,
  research_innovation_team: MagnifyingGlassIcon,
  rapid_bugfix_team: BugBeetleIcon,
};

function getPresetIcon(presetId: string) {
  return presetRoleIcons[presetId] ?? UserIcon;
}

function getTeamIcon(teamId: string) {
  return teamRoleIcons[teamId] ?? UsersThreeIcon;
}

function MemberNameWithTooltip({ name }: { name: string }) {
  const textRef = useRef<HTMLDivElement | null>(null);
  const [isTruncated, setIsTruncated] = useState(false);

  const updateTruncation = useCallback(() => {
    const el = textRef.current;
    if (!el) return;
    setIsTruncated(el.scrollWidth > el.clientWidth + 1);
  }, []);

  useLayoutEffect(() => {
    updateTruncation();
  }, [name, updateTruncation]);

  useEffect(() => {
    const el = textRef.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      updateTruncation();
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [updateTruncation]);

  const nameNode = (
    <div
      ref={textRef}
      className="chat-session-member-name text-sm text-normal min-w-0 flex-1"
    >
      @{name}
    </div>
  );

  if (!isTruncated) return nameNode;
  return (
    <Tooltip content={`@${name}`} side="bottom">
      {nameNode}
    </Tooltip>
  );
}

type AddMemberTab = 'preset' | 'custom';

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
  getModelDisplayName: (
    runnerType: string,
    modelName: string | null
  ) => string | null;
  getVariantLabel: (runnerType: string, variant: string) => string;
  getVariantOptions: (runnerType: string) => string[];
  // Actions
  onOpenAddMember: () => void;
  onCancelMember: () => void;
  onSaveMember: () => void;
  onEditMember: (member: SessionMember) => void;
  onRemoveMember: (member: SessionMember) => void;
  onOpenWorkspace: (agentId: string) => void;
  onExpandPromptEditor: () => void;
  // Preset quick-add
  enabledMemberPresets: ChatMemberPreset[];
  enabledTeamPresets: ChatTeamPreset[];
  onAddMemberPreset: (preset: ChatMemberPreset) => void;
  onImportTeamPreset: (team: ChatTeamPreset) => void;
  teamImportPlan: MemberPresetImportPlan[] | null;
  teamImportName: string | null;
  isImportingTeam: boolean;
  onUpdateTeamImportPlanEntry: (
    index: number,
    updates: {
      finalName?: string;
      workspacePath?: string;
      runnerType?: string;
      systemPrompt?: string;
      toolsEnabled?: JsonValue;
    }
  ) => void;
  onConfirmTeamImport: () => void;
  onCancelTeamImport: () => void;
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
  getModelDisplayName,
  getVariantLabel,
  getVariantOptions,
  onOpenAddMember,
  onCancelMember,
  onSaveMember,
  onEditMember,
  onRemoveMember,
  onOpenWorkspace,
  onExpandPromptEditor,
  enabledMemberPresets,
  enabledTeamPresets,
  onAddMemberPreset,
  onImportTeamPreset,
  teamImportPlan,
  isImportingTeam,
  onUpdateTeamImportPlanEntry,
  onConfirmTeamImport,
  onCancelTeamImport,
}: AiMembersSidebarProps) {
  const { t } = useTranslation('chat');
  const { t: tCommon } = useTranslation('common');
  const [activeTab, setActiveTab] = useState<AddMemberTab>('preset');
  const [importPromptEditorIndex, setImportPromptEditorIndex] = useState<
    number | null
  >(null);
  const workspacePathPlaceholder = getWorkspacePathExample();

  const hasPresets =
    enabledMemberPresets.length > 0 || enabledTeamPresets.length > 0;

  // When entering edit mode, switch to custom tab
  useEffect(() => {
    if (editingMember) {
      setActiveTab('custom');
    }
  }, [editingMember]);

  useEffect(() => {
    if (importPromptEditorIndex === null) return;
    if (
      !teamImportPlan ||
      importPromptEditorIndex < 0 ||
      importPromptEditorIndex >= teamImportPlan.length
    ) {
      setImportPromptEditorIndex(null);
    }
  }, [teamImportPlan, importPromptEditorIndex]);

  const handleImportPlanVariantChange = useCallback(
    (index: number, variant: string, currentToolsEnabled: JsonValue) => {
      const newToolsEnabled = withExecutorProfileVariant(
        currentToolsEnabled,
        variant === 'DEFAULT' ? null : variant
      );
      onUpdateTeamImportPlanEntry(index, { toolsEnabled: newToolsEnabled });
    },
    [onUpdateTeamImportPlanEntry]
  );

  const renderPresetTab = () => (
    <div className="space-y-half">
      {enabledMemberPresets.length > 0 && (
        <div>
          <div className="flex items-center gap-1 text-xs text-low mb-1">
            <UserPlusIcon className="size-3" />
            <span>{t('members.presetMemberSection')}</span>
          </div>
          <div className="space-y-0.5 max-h-40 overflow-y-auto">
            {enabledMemberPresets.map((preset) => {
              const RoleIcon = getPresetIcon(preset.id);
              return (
                <button
                  key={preset.id}
                  type="button"
                  className="flex w-full items-center gap-2 rounded-sm border border-border px-2 py-1 text-left text-xs hover:bg-secondary/50"
                  onClick={() => onAddMemberPreset(preset)}
                >
                  <RoleIcon className="size-3.5 shrink-0 text-low" />
                  <span className="font-medium text-normal truncate">
                    {getLocalizedMemberPresetName(preset, t)}
                  </span>
                  <PlusIcon className="size-3 shrink-0 text-low ml-auto" />
                </button>
              );
            })}
          </div>
        </div>
      )}
      {enabledTeamPresets.length > 0 && (
        <div>
          <div className="flex items-center gap-1 text-xs text-low mb-1 mt-half">
            <UsersThreeIcon className="size-3" />
            <span>{t('members.presetTeamSection')}</span>
          </div>
          <div className="space-y-0.5 max-h-40 overflow-y-auto">
            {enabledTeamPresets.map((team) => {
              const TeamIcon = getTeamIcon(team.id);
              return (
                <button
                  key={team.id}
                  type="button"
                  className="flex w-full items-center gap-2 rounded-sm border border-border px-2 py-1 text-left text-xs hover:bg-secondary/50"
                  onClick={() => onImportTeamPreset(team)}
                  disabled={!!teamImportPlan}
                >
                  <TeamIcon className="size-3.5 shrink-0 text-low" />
                  <span className="font-medium text-normal truncate">
                    {getLocalizedTeamPresetName(team, t)}
                  </span>
                  <UsersThreeIcon className="size-3 shrink-0 text-low ml-auto" />
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* Team Import Preview (inline within preset tab) */}
      {teamImportPlan && (
        <div className="chat-session-member-import-preview border border-border rounded-sm p-base space-y-half mt-half">
          <p className="text-xs text-low">
            {t('members.importPreview.description')}
          </p>
          <div className="space-y-2 max-h-[400px] overflow-y-auto">
            {teamImportPlan.map((plan, index) => {
              const planVariant =
                extractExecutorProfileVariant(plan.toolsEnabled) ?? 'DEFAULT';
              const planVariantOptions = getVariantOptions(plan.runnerType);

              return (
                <div
                  key={`${plan.presetId}-${index}`}
                  className="rounded-sm border border-border px-2 py-1.5 space-y-1"
                >
                  <div className="flex items-center gap-2 text-xs">
                    <span className="font-medium truncate max-w-[180px]">
                      {getLocalizedMemberPresetNameById(
                        plan.presetId,
                        plan.presetName || plan.presetId,
                        t
                      )}
                    </span>
                    <span
                      className={cn(
                        'size-1.5 rounded-full shrink-0',
                        plan.action === 'create' && 'bg-success',
                        plan.action === 'reuse' && 'bg-brand',
                        plan.action === 'skip' && 'bg-low'
                      )}
                    />
                    <span className="text-low truncate">
                      {plan.action === 'create' &&
                        t('members.importPreview.actionCreate')}
                      {plan.action === 'reuse' &&
                        t('members.importPreview.actionReuse')}
                      {plan.action === 'skip' &&
                        t('members.importPreview.actionSkip')}
                    </span>
                  </div>
                  {plan.action !== 'skip' ? (
                    <div className="space-y-1">
                      <div className="space-y-0.5">
                        <label className="text-[11px] text-low">
                          {t('members.memberNameLabel')}
                        </label>
                        <input
                          value={plan.finalName}
                          onChange={(event) =>
                            onUpdateTeamImportPlanEntry(index, {
                              finalName: event.target.value,
                            })
                          }
                          placeholder={t('members.memberNamePlaceholder')}
                          disabled={isImportingTeam}
                          className={cn(
                            'chat-session-member-field w-full rounded-sm border bg-panel px-2 py-1',
                            'text-xs text-normal focus:outline-none disabled:opacity-50'
                          )}
                        />
                      </div>
                      <div className="space-y-0.5">
                        <label className="text-[11px] text-low">
                          {t('members.baseCodingAgent')}
                        </label>
                        <select
                          value={plan.runnerType}
                          onChange={(event) =>
                            onUpdateTeamImportPlanEntry(index, {
                              runnerType: event.target.value,
                            })
                          }
                          disabled={
                            isImportingTeam ||
                            isCheckingAvailability ||
                            enabledRunnerTypes.length === 0
                          }
                          className={cn(
                            'chat-session-member-field w-full rounded-sm border bg-panel px-2 py-1',
                            'text-xs text-normal focus:outline-none disabled:opacity-50'
                          )}
                        >
                          {enabledRunnerTypes.length === 0 && (
                            <option value="">
                              {isCheckingAvailability
                                ? t('members.checkingAgents')
                                : t('members.noLocalAgentsDetected')}
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
                      </div>
                      {planVariantOptions.length > 0 && (
                        <div className="space-y-0.5">
                          <label className="text-[11px] text-low">
                            {t('members.modelVariant')}
                          </label>
                          <select
                            value={planVariant}
                            onChange={(event) =>
                              handleImportPlanVariantChange(
                                index,
                                event.target.value,
                                plan.toolsEnabled
                              )
                            }
                            disabled={isImportingTeam}
                            className={cn(
                              'chat-session-member-field w-full rounded-sm border bg-panel px-2 py-1',
                              'text-xs text-normal focus:outline-none disabled:opacity-50'
                            )}
                          >
                            {planVariantOptions.map((variant) => (
                              <option key={variant} value={variant}>
                                {getVariantLabel(plan.runnerType, variant)}
                              </option>
                            ))}
                          </select>
                        </div>
                      )}
                      <div className="space-y-0.5">
                        <div className="flex items-center justify-between gap-1">
                          <label className="text-[11px] text-low">
                            {t('members.systemPrompt')}
                          </label>
                          <button
                            type="button"
                            className="chat-session-member-expand-btn text-[11px]"
                            onClick={() => setImportPromptEditorIndex(index)}
                            disabled={isImportingTeam}
                          >
                            {t('members.expand')}
                          </button>
                        </div>
                        <textarea
                          value={plan.systemPrompt}
                          onChange={(event) =>
                            onUpdateTeamImportPlanEntry(index, {
                              systemPrompt: event.target.value,
                            })
                          }
                          rows={2}
                          placeholder={t('members.systemPromptPlaceholder')}
                          disabled={isImportingTeam}
                          className={cn(
                            'chat-session-member-field w-full resize-none rounded-sm border bg-panel px-2 py-1',
                            'text-xs text-normal focus:outline-none disabled:opacity-50'
                          )}
                        />
                      </div>
                      <div className="space-y-0.5">
                        <label className="text-[11px] text-low">
                          {t('members.workspacePath')}
                        </label>
                        <input
                          value={plan.workspacePath}
                          onChange={(event) =>
                            onUpdateTeamImportPlanEntry(index, {
                              workspacePath: event.target.value,
                            })
                          }
                          placeholder={workspacePathPlaceholder}
                          disabled={isImportingTeam}
                          className={cn(
                            'chat-session-member-field w-full rounded-sm border bg-panel px-2 py-1',
                            'text-xs text-normal focus:outline-none disabled:opacity-50'
                          )}
                        />
                      </div>
                    </div>
                  ) : (
                    <div className="text-xs text-low truncate">
                      @{plan.finalName}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
          <div className="flex items-center justify-end gap-half pt-half">
            <PrimaryButton
              variant="tertiary"
              value={t('members.importPreview.cancel')}
              onClick={onCancelTeamImport}
              disabled={isImportingTeam}
              className="chat-session-member-btn cancel h-7 min-w-[56px] px-2 text-xs"
            />
            <PrimaryButton
              value={
                isImportingTeam
                  ? t('members.importPreview.importing')
                  : t('members.importPreview.confirmImport')
              }
              actionIcon={isImportingTeam ? 'spinner' : UsersThreeIcon}
              onClick={onConfirmTeamImport}
              disabled={isImportingTeam || isArchived}
              className="chat-session-member-btn h-7 min-w-[56px] px-2 text-xs"
            />
          </div>
        </div>
      )}

      {!hasPresets && (
        <div className="text-xs text-low py-base text-center">
          {t('members.noEnabledPresets')}
        </div>
      )}

      {/* Close button at bottom-right */}
      <div className="flex justify-end pt-half">
        <PrimaryButton
          variant="tertiary"
          value={t('members.closePanel')}
          onClick={onCancelMember}
          className="chat-session-member-btn cancel"
        />
      </div>
    </div>
  );

  const renderCustomTab = () => (
    <div className="space-y-half">
      <div className="text-xs text-low">
        {t('members.memberNameHint')}
      </div>
      <div className="space-y-half">
        <label className="text-xs text-low">
          {t('members.memberNameLabel')}
        </label>
        <input
          value={newMemberName}
          onChange={(event) => onNameChange(event.target.value)}
          placeholder={t('members.memberNamePlaceholder')}
          className={cn(
            'chat-session-member-field w-full rounded-sm border bg-panel px-base py-half',
            'text-sm text-normal focus:outline-none'
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
          {t('members.baseCodingAgent')}
        </label>
        <select
          value={newMemberRunnerType}
          onChange={(event) => onRunnerTypeChange(event.target.value)}
          disabled={
            isCheckingAvailability || enabledRunnerTypes.length === 0
          }
          className={cn(
            'chat-session-member-field w-full rounded-sm border bg-panel px-base py-half',
            'text-sm text-normal focus:outline-none'
          )}
        >
          {enabledRunnerTypes.length === 0 && (
            <option value="">
              {isCheckingAvailability
                ? t('members.checkingAgents')
                : t('members.noLocalAgentsDetected')}
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
        {enabledRunnerTypes.length === 0 && !isCheckingAvailability && (
          <div className="text-xs text-error">
            {t('members.noInstalledAgents')}
          </div>
        )}
      </div>
      {memberVariantOptions.length > 0 && (
        <div className="space-y-half">
          <label className="text-xs text-low">
            {t('members.modelVariant')}
          </label>
          <select
            value={newMemberVariant}
            onChange={(event) => onVariantChange(event.target.value)}
            className={cn(
              'chat-session-member-field w-full rounded-sm border bg-panel px-base py-half',
              'text-sm text-normal focus:outline-none'
            )}
          >
            {memberVariantOptions.map((variant) => {
              return (
                <option key={variant} value={variant}>
                  {getVariantLabel(newMemberRunnerType, variant)}
                </option>
              );
            })}
          </select>
          {getModelDisplayName(
            newMemberRunnerType,
            getModelName(newMemberRunnerType, newMemberVariant)
          ) && (
            <div className="text-xs text-low">
              {t('members.model')}:{' '}
              {getModelDisplayName(
                newMemberRunnerType,
                getModelName(newMemberRunnerType, newMemberVariant)
              )}
            </div>
          )}
        </div>
      )}
      <div className="space-y-half">
        <div className="flex items-center justify-between gap-base">
          <label className="text-xs text-low">
            {t('members.systemPrompt')}
          </label>
          <button
            type="button"
            className="chat-session-member-expand-btn text-xs"
            onClick={onExpandPromptEditor}
          >
            {t('members.expand')}
          </button>
        </div>
        <textarea
          value={newMemberPrompt}
          onChange={(event) => onPromptChange(event.target.value)}
          rows={3}
          placeholder={t('members.systemPromptPlaceholder')}
          className={cn(
            'chat-session-member-field w-full resize-none rounded-sm border bg-panel',
            'px-base py-half text-sm text-normal focus:outline-none'
          )}
        />
      </div>
      <div className="space-y-half">
        <label className="text-xs text-low">
          {t('members.workspacePath')}
        </label>
        <input
          value={newMemberWorkspace}
          onChange={(event) => onWorkspaceChange(event.target.value)}
          placeholder={workspacePathPlaceholder}
          disabled={!!editingMember}
          title={
            editingMember
              ? t('members.workspacePathCannotBeModified')
              : undefined
          }
          className={cn(
            'chat-session-member-field w-full rounded-sm border bg-panel px-base py-half',
            'text-sm text-normal focus:outline-none',
            editingMember && 'opacity-50 cursor-not-allowed'
          )}
        />
        {editingMember && (
          <p className="text-xs text-low">
            {t('members.workspacePathCannotBeModified')}
          </p>
        )}
      </div>
      {memberError && (
        <div className="text-xs text-error">{memberError}</div>
      )}
      <div className="flex items-center justify-end gap-half pt-half">
        <PrimaryButton
          variant="tertiary"
          value={tCommon('buttons.cancel')}
          onClick={onCancelMember}
          disabled={isSavingMember}
          className="chat-session-member-btn cancel"
        />
        <PrimaryButton
          value={editingMember ? t('members.save') : t('members.add')}
          actionIcon={isSavingMember ? 'spinner' : PlusIcon}
          onClick={onSaveMember}
          disabled={
            isSavingMember || isArchived || !!memberNameLengthError
          }
          className="chat-session-member-btn"
        />
      </div>
    </div>
  );

  const importPromptEditorValue =
    importPromptEditorIndex !== null && teamImportPlan?.[importPromptEditorIndex]
      ? teamImportPlan[importPromptEditorIndex].systemPrompt
      : '';

  return (
    <>
      <aside
        className="chat-session-members-panel border-l border-border flex flex-col min-h-0 shrink-0"
        style={{ width }}
      >
      <div className="chat-session-members-header px-base py-base border-b border-border flex items-center justify-between">
        <div className="chat-session-members-title text-sm text-normal font-medium">
          {t('members.title')}
        </div>
        <div className="chat-session-members-count text-xs text-low">
          {t('members.countInSession', { count: sessionMembers.length })}
        </div>
      </div>
      <div className="chat-session-members-list flex-1 min-h-0 overflow-y-auto p-base space-y-base">
        {!activeSessionId && (
          <div className="chat-session-members-empty text-xs text-low mt-base">
            {t('members.selectSessionToManage')}
          </div>
        )}
        {activeSessionId && sessionMembers.length === 0 && (
          <div className="chat-session-members-empty text-xs text-low mt-base">
            {t('members.noMembersYet')}
          </div>
        )}

        {sessionMembers.map(({ agent, sessionAgent }) => {
          const state = agentStates[agent.id] ?? ChatSessionAgentState.idle;
          const memberVariant =
            extractExecutorProfileVariant(agent.tools_enabled) ?? undefined;
          const modelName = getModelName(agent.runner_type, memberVariant);
          const modelDisplayName = getModelDisplayName(
            agent.runner_type,
            modelName
          );
          const fullText = `${toPrettyCase(agent.runner_type)} | ${agentStateLabels[state]}${modelDisplayName ? ` | ${modelDisplayName}` : ''}`;
          const modelStatusPreview = truncateByChars(fullText, 15);
          const avatarSeed = getAgentAvatarSeed(
            agent.id,
            agent.runner_type,
            agent.name
          );
          const workspacePath = sessionAgent.workspace_path ?? '';
          const shouldShowWorkspaceTooltip = workspacePath.length > 48;

          return (
            <div
              key={sessionAgent.id}
              className="chat-session-member-card border border-border rounded-sm px-base py-half space-y-half"
            >
              <div className="chat-session-member-header">
                <div className="chat-session-member-primary flex items-center gap-half min-w-0">
                  <span
                    className={cn(
                      'size-2 rounded-full',
                      agentStateDotClass[state],
                      state === ChatSessionAgentState.running && 'animate-pulse'
                    )}
                  />
                  <span
                    className="chat-session-member-avatar"
                    style={getAgentAvatarStyle(avatarSeed)}
                  >
                    <AgentBrandIcon
                      runnerType={agent.runner_type}
                      className="chat-session-member-avatar-logo"
                    />
                  </span>
                  <MemberNameWithTooltip name={agent.name} />
                </div>
                <div className="chat-session-member-actions flex items-center gap-half text-xs">
                  <button
                    type="button"
                    className="chat-session-member-action workspace"
                    onClick={() => onOpenWorkspace(agent.id)}
                  >
                    {t('members.workspace')}
                  </button>
                  <button
                    type="button"
                    className={cn(
                      'chat-session-member-action edit',
                      isArchived && 'pointer-events-none opacity-50'
                    )}
                    onClick={() => onEditMember({ agent, sessionAgent })}
                    disabled={isArchived}
                  >
                    {t('members.edit')}
                  </button>
                  <button
                    type="button"
                    className={cn(
                      'chat-session-member-action danger',
                      isArchived && 'pointer-events-none opacity-50'
                    )}
                    onClick={() => onRemoveMember({ agent, sessionAgent })}
                    disabled={isArchived}
                  >
                    {t('members.remove')}
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

        {/* Add Member Section */}
        <div className="chat-session-member-form border-t border-border pt-base space-y-half">
          {!isAddMemberOpen ? (
            <button
              type="button"
              className="chat-session-add-member-btn"
              onClick={onOpenAddMember}
              disabled={!activeSessionId || isArchived}
            >
              {t('members.addAiMember')}
              <PlusIcon className="size-icon-xs" weight="light" />
            </button>
          ) : (
            <div className="chat-session-member-form-panel border border-border rounded-sm p-base space-y-half">
              {/* Tab bar - only show when not editing */}
              {!editingMember && (
                <div className="flex border-b border-border">
                  <button
                    type="button"
                    className={cn(
                      'flex-1 text-xs py-1 text-center transition-colors',
                      activeTab === 'preset'
                        ? 'text-normal border-b-2 border-brand font-medium'
                        : 'text-low hover:text-normal'
                    )}
                    onClick={() => setActiveTab('preset')}
                  >
                    {t('members.tabPreset')}
                  </button>
                  <button
                    type="button"
                    className={cn(
                      'flex-1 text-xs py-1 text-center transition-colors',
                      activeTab === 'custom'
                        ? 'text-normal border-b-2 border-brand font-medium'
                        : 'text-low hover:text-normal'
                    )}
                    onClick={() => setActiveTab('custom')}
                  >
                    {t('members.tabCustom')}
                  </button>
                </div>
              )}

              {/* Edit mode header */}
              {editingMember && (
                <div className="text-sm text-normal font-medium">
                  {t('members.editAiMember')}
                </div>
              )}

              {/* Tab content */}
              <div className="pt-half">
                {activeTab === 'preset' && !editingMember
                  ? renderPresetTab()
                  : renderCustomTab()}
              </div>
            </div>
          )}
        </div>
      </div>
      </aside>
      <PromptEditorModal
        isOpen={importPromptEditorIndex !== null}
        value={importPromptEditorValue}
        onChange={(value) => {
          if (importPromptEditorIndex === null) return;
          onUpdateTeamImportPlanEntry(importPromptEditorIndex, {
            systemPrompt: value,
          });
        }}
        onClose={() => setImportPromptEditorIndex(null)}
        showFileImport={false}
      />
    </>
  );
}
