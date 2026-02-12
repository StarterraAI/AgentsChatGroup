import { useTranslation } from 'react-i18next';
import { XIcon } from '@phosphor-icons/react';

import { GeneralSettingsSectionContent } from './GeneralSettingsSection';
import { ReposSettingsSectionContent } from './ReposSettingsSection';
import { AgentsSettingsSectionContent } from './AgentsSettingsSection';
import { McpSettingsSectionContent } from './McpSettingsSection';

export type SettingsSectionType =
  | 'general'
  | 'repos'
  | 'agents'
  | 'mcp';

// Section-specific initial state types
export type SettingsSectionInitialState = {
  general: undefined;
  repos: undefined;
  agents: { executor?: string; variant?: string } | undefined;
  mcp: undefined;
};

interface SettingsSectionProps {
  type: SettingsSectionType;
  onClose?: () => void;
}

export function SettingsSection({
  type,
  onClose,
}: SettingsSectionProps) {
  const { t } = useTranslation('settings');

  const renderContent = () => {
    switch (type) {
      case 'general':
        return <GeneralSettingsSectionContent />;
      case 'repos':
        return <ReposSettingsSectionContent />;
      case 'agents':
        return <AgentsSettingsSectionContent />;
      case 'mcp':
        return <McpSettingsSectionContent />;
      default:
        return <GeneralSettingsSectionContent />;
    }
  };

  return (
    <div className="settings-section flex flex-col h-full">
      {/* Header - sticky */}
      <div className="settings-section-header p-4 border-b border-border bg-panel/95 backdrop-blur-sm hidden sm:flex items-center justify-between">
        <h2 className="text-lg font-semibold text-high">
          {t(`settings.layout.nav.${type}`)}
        </h2>
        {onClose && (
          <button
            onClick={onClose}
            className="rounded-sm opacity-70 ring-offset-panel transition-opacity hover:opacity-100 focus:outline-none focus:ring-2 focus:ring-brand focus:ring-offset-2"
          >
            <XIcon className="h-4 w-4 text-normal" weight="bold" />
            <span className="sr-only">{t('close', { ns: 'common' })}</span>
          </button>
        )}
      </div>

      {/* Content */}
      <div className="settings-section-body space-y-6 px-6 pt-4 overflow-y-auto">
        {renderContent()}
      </div>
    </div>
  );
}
