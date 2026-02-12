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
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmModal({
  isOpen,
  title,
  message,
  isLoading,
  onConfirm,
  onCancel,
}: ConfirmModalProps) {
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
        <PrimaryButton
          variant="tertiary"
          value="Cancel"
          onClick={onCancel}
          disabled={isLoading}
        />
        <PrimaryButton
          variant="default"
          value={isLoading ? 'Processing...' : 'Confirm'}
          onClick={onConfirm}
          disabled={isLoading}
        />
      </DialogFooter>
    </Dialog>
  );
}
