import { assert, describe, test } from 'vitest';
import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';

import {
	CordSpaceTabs,
	to_cord_space_tabs_storage_key,
	type CordSpaceTabsStorage
} from '#lib/ui/cord_space_tabs.svelte.ts';

const ADA = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' as Uuid;
const BEA = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb' as Uuid;
const PORCH = '11111111-1111-4111-8111-111111111111' as Uuid;
const YARD = '22222222-2222-4222-8222-222222222222' as Uuid;
const HEARTH = '33333333-3333-4333-8333-333333333333' as Uuid;
const LOFT = '44444444-4444-4444-8444-444444444444' as Uuid;
const DECK = '55555555-5555-4555-8555-555555555555' as Uuid;
const STOOP = '66666666-6666-4666-8666-666666666666' as Uuid;

/** A store over a plain map, which the test can read and break. */
const create_storage = (initial: Record<string, string> = {}) => {
	const items = new Map(Object.entries(initial));
	const storage: CordSpaceTabsStorage = {
		getItem: (key) => items.get(key) ?? null,
		setItem: (key, value) => {
			items.set(key, value);
		},
		removeItem: (key) => {
			items.delete(key);
		}
	};
	return { items, storage };
};

/** A store whose every call throws, as a blocked one does. */
const blocked_storage: CordSpaceTabsStorage = {
	getItem: () => {
		throw new Error('blocked');
	},
	setItem: () => {
		throw new Error('blocked');
	},
	removeItem: () => {
		throw new Error('blocked');
	}
};

const PORCH_KEY = to_cord_space_tabs_storage_key(ADA, PORCH);

/** Tabs keyed to Ada in the porch, over the given store. */
const create_tabs = (storage: CordSpaceTabsStorage | null = create_storage().storage) => {
	const tabs = new CordSpaceTabs({ get_storage: () => storage });
	tabs.load(ADA, PORCH);
	return tabs;
};

describe('CordSpaceTabs', () => {
	test('a preview replaces the preview tab', () => {
		const tabs = create_tabs();
		tabs.open(PORCH, HEARTH, 'preview');
		assert.strictEqual(tabs.preview_id, HEARTH);
		tabs.open(PORCH, LOFT, 'preview');
		assert.strictEqual(tabs.preview_id, LOFT);
		assert.deepEqual(tabs.tab_ids, [LOFT]);
		assert.deepEqual(tabs.permanent_ids, []);
	});

	test('opening a permanent tab’s space by URL, as a preview, leaves the tabs as they are', () => {
		const tabs = create_tabs();
		tabs.open(PORCH, HEARTH, 'permanent');
		tabs.open(PORCH, LOFT, 'preview');
		tabs.open(PORCH, HEARTH, 'preview');
		assert.deepEqual(tabs.permanent_ids, [HEARTH]);
		assert.strictEqual(tabs.preview_id, LOFT, 'the preview tab keeps what it showed');
	});

	test('the strip is the permanent tabs in the order kept, then the preview', () => {
		const tabs = create_tabs();
		tabs.open(PORCH, LOFT, 'permanent');
		tabs.open(PORCH, HEARTH, 'permanent');
		tabs.open(PORCH, DECK, 'preview');
		assert.deepEqual(tabs.tab_ids, [LOFT, HEARTH, DECK]);
	});

	test('promote keeps the preview tab as the last permanent one', () => {
		const { items, storage } = create_storage();
		const tabs = create_tabs(storage);
		tabs.open(PORCH, HEARTH, 'permanent');
		tabs.open(PORCH, LOFT, 'preview');
		assert.isTrue(tabs.promote());
		assert.isNull(tabs.preview_id);
		assert.deepEqual(tabs.permanent_ids, [HEARTH, LOFT]);
		assert.strictEqual(items.get(PORCH_KEY), JSON.stringify([HEARTH, LOFT]));
		assert.isFalse(tabs.promote(), 'nothing left to keep');
	});

	test('keeping another space reuses the preview tab', () => {
		const tabs = create_tabs();
		tabs.open(PORCH, HEARTH, 'preview');
		tabs.open(PORCH, LOFT, 'permanent');
		assert.deepEqual(tabs.tab_ids, [LOFT]);
		assert.isNull(tabs.preview_id);
	});

	test('keeping a permanent tab again changes nothing', () => {
		const tabs = create_tabs();
		tabs.open(PORCH, HEARTH, 'permanent');
		tabs.open(PORCH, LOFT, 'preview');
		tabs.open(PORCH, HEARTH, 'permanent');
		assert.deepEqual(tabs.tab_ids, [HEARTH, LOFT]);
	});

	test('opening a space of another cord does nothing', () => {
		const tabs = create_tabs();
		tabs.open(YARD, HEARTH, 'preview');
		tabs.open(YARD, LOFT, 'permanent');
		assert.deepEqual(tabs.tab_ids, []);
	});

	const close_cases: Array<
		[label: string, closed: Uuid, neighbour: Uuid | null, left: Array<Uuid>]
	> = [
		['a middle tab goes to the next', HEARTH, DECK, [LOFT, DECK, STOOP]],
		['the first tab goes to the next', LOFT, HEARTH, [HEARTH, DECK, STOOP]],
		['the preview tab, last, goes to the previous', STOOP, DECK, [LOFT, HEARTH, DECK]]
	];
	test.each(close_cases)('close: %s', (_label, closed, neighbour, left) => {
		const tabs = create_tabs();
		tabs.open(PORCH, LOFT, 'permanent');
		tabs.open(PORCH, HEARTH, 'permanent');
		tabs.open(PORCH, DECK, 'permanent');
		tabs.open(PORCH, STOOP, 'preview');
		assert.strictEqual(tabs.close(PORCH, closed), neighbour);
		assert.deepEqual(tabs.tab_ids, left);
	});

	test('closing the last tab leaves no neighbour, so the caller goes to the overview', () => {
		const { items, storage } = create_storage();
		const tabs = create_tabs(storage);
		tabs.open(PORCH, HEARTH, 'permanent');
		assert.isNull(tabs.close(PORCH, HEARTH));
		assert.deepEqual(tabs.tab_ids, []);
		assert.isFalse(items.has(PORCH_KEY), 'an empty strip leaves nothing stored');
	});

	test('closing a tab that isn’t there, or another cord’s, does nothing', () => {
		const tabs = create_tabs();
		tabs.open(PORCH, HEARTH, 'permanent');
		assert.isNull(tabs.close(PORCH, LOFT));
		assert.isNull(tabs.close(YARD, HEARTH));
		assert.deepEqual(tabs.tab_ids, [HEARTH]);
	});

	test('the permanent tabs come back for the same account and cord, without the preview', () => {
		const { storage } = create_storage();
		const tabs = create_tabs(storage);
		tabs.open(PORCH, HEARTH, 'permanent');
		tabs.open(PORCH, LOFT, 'preview');

		const reloaded = create_tabs(storage);
		assert.deepEqual(reloaded.permanent_ids, [HEARTH]);
		assert.isNull(reloaded.preview_id);

		reloaded.load(BEA, PORCH);
		assert.deepEqual(reloaded.permanent_ids, [], 'another account’s tabs are its own');
		reloaded.load(ADA, YARD);
		assert.deepEqual(reloaded.permanent_ids, [], 'another cord’s tabs are its own');
		reloaded.load(ADA, PORCH);
		assert.deepEqual(reloaded.permanent_ids, [HEARTH]);
	});

	test('loading the pair already held keeps the preview', () => {
		const tabs = create_tabs();
		tabs.open(PORCH, HEARTH, 'preview');
		tabs.load(ADA, PORCH);
		assert.strictEqual(tabs.preview_id, HEARTH);
	});

	test('moving to another cord drops the preview, so it can’t be kept there', () => {
		const tabs = create_tabs();
		tabs.open(PORCH, HEARTH, 'preview');
		tabs.load(ADA, YARD);
		assert.isNull(tabs.preview_id);
		assert.isFalse(tabs.promote());
		assert.deepEqual(tabs.permanent_ids, []);
	});

	test('retain drops the tabs of spaces the list no longer has, stored ones too', () => {
		const { items, storage } = create_storage({
			[PORCH_KEY]: JSON.stringify([HEARTH, LOFT, DECK])
		});
		const tabs = create_tabs(storage);
		tabs.open(PORCH, STOOP, 'preview');
		tabs.retain(PORCH, [LOFT, HEARTH]);
		assert.deepEqual(tabs.tab_ids, [HEARTH, LOFT]);
		assert.strictEqual(items.get(PORCH_KEY), JSON.stringify([HEARTH, LOFT]));
	});

	test('retain for another cord does nothing', () => {
		const tabs = create_tabs();
		tabs.open(PORCH, HEARTH, 'permanent');
		tabs.retain(YARD, []);
		assert.deepEqual(tabs.tab_ids, [HEARTH]);
	});

	const unreadable_cases: Array<[label: string, stored: string]> = [
		['not JSON', '{'],
		['not a list', JSON.stringify({ ids: [HEARTH] })],
		['not ids', JSON.stringify(['hearth'])]
	];
	test.each(unreadable_cases)('a stored record that’s %s reads as no tabs', (_label, stored) => {
		const { storage } = create_storage({ [PORCH_KEY]: stored });
		const tabs = create_tabs(storage);
		assert.deepEqual(tabs.permanent_ids, []);
		tabs.open(PORCH, HEARTH, 'permanent');
		assert.deepEqual(tabs.permanent_ids, [HEARTH], 'and the next change overwrites it');
	});

	test('a stored record with repeats reads each space once', () => {
		const { storage } = create_storage({
			[PORCH_KEY]: JSON.stringify([HEARTH, LOFT, HEARTH])
		});
		const tabs = create_tabs(storage);
		assert.deepEqual(tabs.permanent_ids, [HEARTH, LOFT]);
	});

	test('a blocked store, or none, still keeps the tabs for the page’s life', () => {
		for (const storage of [blocked_storage, null]) {
			const tabs = create_tabs(storage);
			assert.deepEqual(tabs.permanent_ids, []);
			tabs.open(PORCH, HEARTH, 'permanent');
			tabs.open(PORCH, LOFT, 'preview');
			assert.isTrue(tabs.promote());
			assert.deepEqual(tabs.permanent_ids, [HEARTH, LOFT]);
			assert.strictEqual(tabs.close(PORCH, HEARTH), LOFT);
			tabs.retain(PORCH, []);
			assert.deepEqual(tabs.tab_ids, []);
		}
	});

	test('a store that throws on its getter reads as none', () => {
		const tabs = new CordSpaceTabs({
			get_storage: () => {
				throw new Error('denied');
			}
		});
		tabs.load(ADA, PORCH);
		tabs.open(PORCH, HEARTH, 'permanent');
		assert.deepEqual(tabs.permanent_ids, [HEARTH]);
	});

	test('reset forgets the tabs in memory and the pair they were for', () => {
		const { items, storage } = create_storage();
		const tabs = create_tabs(storage);
		tabs.open(PORCH, HEARTH, 'permanent');
		tabs.reset();
		assert.deepEqual(tabs.tab_ids, []);
		assert.isNull(tabs.cord_id);
		tabs.open(PORCH, LOFT, 'preview');
		assert.deepEqual(tabs.tab_ids, [], 'unkeyed tabs open nothing');
		assert.isTrue(items.has(PORCH_KEY), 'the store keeps its own');
	});
});
