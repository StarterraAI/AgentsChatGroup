import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogHeader,
  DialogTitle,
  DialogContent,
  DialogFooter,
} from '@/components/ui/dialog';
import { PrimaryButton } from '@/components/ui-new/primitives/PrimaryButton';

export interface ConfirmModalProps {
  isOpen: boolean;
  title: string;
  message: string;
  isLoading: boolean;
  mode?: 'confirm' | 'alert';
  confirmText?: string;
  cancelText?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmModal({
  isOpen,
  title,
  message,
  isLoading,
  mode = 'confirm',
  confirmText,
  cancelText,
  onConfirm,
  onCancel,
}: ConfirmModalProps) {
  const { t } = useTranslation(['chat', 'common']);
  const isAlert = mode === 'alert';
  const resolvedConfirmText = isLoading
    ? t('modals.confirm.processing')
    : confirmText ??
      (isAlert ? t('common:ok') : t('modals.confirm.confirm'));
  const resolvedCancelText = cancelText ?? t('modals.confirm.cancel');

  return (
    <Dialog
      className="chat-session-modal-surface"
      open={isOpen}
      onOpenChange={(open) => {
        if (!open && !isLoading) {
          onCancel();
        }
      }}
    >
      <DialogHeader>
        <DialogTitle>{title}</DialogTitle>
      </DialogHeader>
      <DialogContent>
        <p className="text-sm text-normal">{message}</p>
      </DialogContent>
      <DialogFooter>
        {!isAlert && (
          <PrimaryButton
            variant="tertiary"
            value={resolvedCancelText}
            onClick={onCancel}
            disabled={isLoading}
          />
        )}
        <PrimaryButton
          variant="default"
          value={resolvedConfirmText}
          onClick={onConfirm}
          disabled={isLoading}
        />
      </DialogFooter>
    </Dialog>
  );
}
