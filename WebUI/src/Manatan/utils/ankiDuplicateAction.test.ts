import assert from 'node:assert/strict';
import test from 'node:test';

import {
    getAnkiAddNoteOptions,
    getAnkiDuplicateAction,
    getAnkiDuplicateButtonMode,
} from '@/Manatan/utils/ankiDuplicateAction';
import type { Settings } from '@/Manatan/types';

const settingsWithAction = (ankiDuplicateAction?: unknown): Settings =>
    ({
        ankiDuplicateAction,
    }) as Settings;

test('getAnkiDuplicateAction defaults to prevent for missing or invalid values', () => {
    assert.equal(getAnkiDuplicateAction(settingsWithAction()), 'prevent');
    assert.equal(getAnkiDuplicateAction(settingsWithAction('nonsense')), 'prevent');
});

test('getAnkiDuplicateAction keeps valid settings values', () => {
    assert.equal(getAnkiDuplicateAction(settingsWithAction('prevent')), 'prevent');
    assert.equal(getAnkiDuplicateAction(settingsWithAction('add')), 'add');
    assert.equal(getAnkiDuplicateAction(settingsWithAction('overwrite')), 'overwrite');
});

test('getAnkiDuplicateButtonMode honors duplicate action for existing notes', () => {
    assert.equal(getAnkiDuplicateButtonMode('exists', 'add'), 'add-duplicate');
    assert.equal(getAnkiDuplicateButtonMode('exists', 'overwrite'), 'overwrite');
    assert.equal(getAnkiDuplicateButtonMode('exists', 'prevent'), 'open-existing');
});

test('getAnkiAddNoteOptions only allows duplicate when adding an existing note', () => {
    assert.deepEqual(getAnkiAddNoteOptions({ isDuplicate: true, action: 'add' }), {
        allowDuplicate: true,
        duplicateScope: 'deck',
    });
    assert.deepEqual(getAnkiAddNoteOptions({ isDuplicate: true, action: 'overwrite' }), {
        allowDuplicate: false,
        duplicateScope: 'deck',
    });
    assert.deepEqual(getAnkiAddNoteOptions({ isDuplicate: false, action: 'add', duplicateScope: 'collection' }), {
        allowDuplicate: false,
        duplicateScope: 'collection',
    });
});

test('missing notes show add for all duplicate actions', () => {
    assert.equal(getAnkiDuplicateButtonMode('missing', 'prevent'), 'add');
    assert.equal(getAnkiDuplicateButtonMode('missing', 'add'), 'add');
    assert.equal(getAnkiDuplicateButtonMode('missing', 'overwrite'), 'add');
});
