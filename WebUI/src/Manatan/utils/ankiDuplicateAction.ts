import type { Settings } from '@/Manatan/types';

export type AnkiDuplicateAction = Settings['ankiDuplicateAction'];
export type AnkiDuplicateScope = Settings['ankiDuplicateScope'];
export type AnkiDuplicateStatus = 'unknown' | 'loading' | 'missing' | 'exists';
export type AnkiDuplicateButtonMode = 'checking' | 'add' | 'add-duplicate' | 'overwrite' | 'open-existing';

const VALID_DUPLICATE_ACTIONS: readonly AnkiDuplicateAction[] = ['prevent', 'add', 'overwrite'];

export const getAnkiDuplicateAction = (
    settings?: Partial<Pick<Settings, 'ankiDuplicateAction'>> | null,
): AnkiDuplicateAction => {
    const action = settings?.ankiDuplicateAction;
    if (typeof action === 'string' && (VALID_DUPLICATE_ACTIONS as readonly string[]).includes(action)) {
        return action as AnkiDuplicateAction;
    }
    return 'prevent';
};

export const getAnkiDuplicateButtonMode = (
    status: AnkiDuplicateStatus,
    action: AnkiDuplicateAction,
): AnkiDuplicateButtonMode => {
    if (status === 'unknown' || status === 'loading') {
        return 'checking';
    }
    if (status === 'missing') {
        return 'add';
    }
    if (action === 'add') {
        return 'add-duplicate';
    }
    if (action === 'overwrite') {
        return 'overwrite';
    }
    return 'open-existing';
};

export const getAnkiAddNoteOptions = ({
    isDuplicate,
    action,
    duplicateScope,
}: {
    isDuplicate: boolean;
    action: AnkiDuplicateAction;
    duplicateScope?: AnkiDuplicateScope | null;
}): { allowDuplicate: boolean; duplicateScope: AnkiDuplicateScope } => ({
    allowDuplicate: isDuplicate && action === 'add',
    duplicateScope: duplicateScope || 'deck',
});
