/**
 * The space tabs on a cord's page, as an editor's file tabs work: one
 * `preview` tab that opening a space replaces, and `permanent` tabs that stay
 * until closed. The `selected` tab, the one showing, is the route's: a tab
 * is a link, and the page the URL names is the one selected.
 *
 * Opening a space as `'preview'` (a single click in the rail, or opening it
 * by URL) puts it in the preview tab, unless it's already permanent;
 * opening it as `'permanent'` (a double click in the rail, on the preview
 * tab, or the space page's keep control) keeps it. The strip shows the
 * permanent tabs in the order they were kept, then the preview tab.
 *
 * The tabs are one cord's for one account at a time (`load`), and the
 * permanent ones are remembered per browser in `localStorage`, keyed by
 * both; a blocked or unreadable store is fine, and the tabs then last the
 * page's life. Each page reads the store when its tabs load and writes its
 * own list on every change, so with one cord open in two browser tabs, the
 * last change in either is what the store keeps. Tabs for spaces the cord
 * no longer has drop when its list loads (`retain`).
 *
 * The root layout provides the one `CordSpaceTabs`, so the rail and the
 * cord's pages share it; the cord layout keys it to the route's cord.
 * Every method reads its own state untracked, so an `$effect` may call it.
 *
 * @module
 */

import { untrack } from 'svelte';
import { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';
import { array, parse } from '@fuzdev/fuz_schema/schema.ts';
import { create_context } from '@fuzdev/fuz_ui/context_helpers.ts';

import { CORD_SPACE_COUNT_MAX } from '../cord_action_specs.ts';

/** How a space opens in the strip. */
export type CordSpaceTabOpen = 'preview' | 'permanent';

/** The slice of `Storage` the tabs need. */
export type CordSpaceTabsStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

/** This browser's `localStorage`, or `null` where there's none or it's blocked. */
export const cord_space_tabs_local_storage = (): CordSpaceTabsStorage | null => {
	try {
		return globalThis.localStorage ?? null;
	} catch {
		return null;
	}
};

/** The `localStorage` key for one account's permanent tabs in one cord. */
export const to_cord_space_tabs_storage_key = (account_id: Uuid, cord_id: Uuid): string =>
	`cord_space_tabs:${account_id}:${cord_id}`;

/** What the store keeps: the permanent tabs' space ids, in strip order. */
const CordSpaceTabsStored = array(Uuid);

/** Options for `CordSpaceTabs`. */
export interface CordSpaceTabsOptions {
	/**
	 * Reads the store for the permanent tabs, called only when the tabs load
	 * or change, so a page that prerenders never touches it.
	 */
	get_storage: () => CordSpaceTabsStorage | null;
}

/** One cord's space tabs for one account: the permanent ones and the preview. */
export class CordSpaceTabs {
	readonly #get_storage: () => CordSpaceTabsStorage | null;

	/** The account the tabs are for, `null` before the first `load`. */
	account_id: Uuid | null = $state.raw(null);
	/** The cord the tabs are for, `null` before the first `load`. */
	cord_id: Uuid | null = $state.raw(null);
	/** The permanent tabs' space ids, in strip order. */
	permanent_ids: ReadonlyArray<Uuid> = $state.raw([]);
	/** The preview tab's space id, `null` when there's no preview tab. */
	preview_id: Uuid | null = $state.raw(null);

	/** Every tab's space id in strip order: the permanent tabs, then the preview. */
	readonly tab_ids: ReadonlyArray<Uuid> = $derived(
		this.preview_id === null ? this.permanent_ids : [...this.permanent_ids, this.preview_id]
	);

	constructor(options: CordSpaceTabsOptions) {
		this.#get_storage = options.get_storage;
	}

	/**
	 * Key the tabs to `account_id` and `cord_id`, reading that pair's
	 * permanent tabs from the store; a no-op when they're keyed so already.
	 * Moving to another pair drops the preview tab.
	 */
	load(account_id: Uuid, cord_id: Uuid): void {
		untrack(() => {
			if (this.account_id === account_id && this.cord_id === cord_id) return;
			this.account_id = account_id;
			this.cord_id = cord_id;
			this.permanent_ids = this.#read(account_id, cord_id);
			this.preview_id = null;
		});
	}

	/** Forget the tabs in memory, as when the account changes; the store keeps its own. */
	reset(): void {
		this.account_id = null;
		this.cord_id = null;
		this.permanent_ids = [];
		this.preview_id = null;
	}

	/**
	 * Open the space `space_id` of the cord `cord_id`; a no-op for a cord the
	 * tabs aren't keyed to. As `'preview'`, it takes the preview tab, unless
	 * it's already permanent. As `'permanent'`, it's kept as a permanent tab
	 * at the end of the permanent ones, and the preview tab goes: a preview of
	 * this space became the permanent tab, and as in an editor, keeping a
	 * space reuses the preview tab rather than opening another beside it.
	 */
	open(cord_id: Uuid, space_id: Uuid, mode: CordSpaceTabOpen): void {
		untrack(() => {
			if (this.cord_id !== cord_id || this.permanent_ids.includes(space_id)) return;
			if (mode === 'preview') {
				this.preview_id = space_id;
				return;
			}
			this.preview_id = null;
			this.#set_permanent([...this.permanent_ids, space_id]);
		});
	}

	/**
	 * Keep the preview tab as a permanent tab.
	 *
	 * @returns whether there was a preview tab to keep
	 */
	promote(): boolean {
		return untrack(() => {
			const { cord_id, preview_id } = this;
			if (cord_id === null || preview_id === null) return false;
			this.open(cord_id, preview_id, 'permanent');
			return true;
		});
	}

	/**
	 * Close the tab of the space `space_id` in the cord `cord_id`, permanent
	 * or preview; a no-op for a cord the tabs aren't keyed to or a space
	 * with no tab.
	 *
	 * @returns the space whose tab is its neighbour in the strip (the next,
	 * else the previous), for the caller to select when the closed tab was the
	 * selected one; `null` when no other space tab is left, or nothing closed
	 */
	close(cord_id: Uuid, space_id: Uuid): Uuid | null {
		return untrack(() => {
			if (this.cord_id !== cord_id) return null;
			const ids = this.tab_ids;
			const index = ids.indexOf(space_id);
			if (index === -1) return null;
			const neighbour = ids[index + 1] ?? ids[index - 1] ?? null;
			if (this.preview_id === space_id) {
				this.preview_id = null;
			} else {
				this.#set_permanent(this.permanent_ids.filter((id) => id !== space_id));
			}
			return neighbour;
		});
	}

	/**
	 * Drop the tabs of spaces not in `space_ids`, the cord's loaded list, so
	 * a space deleted here or elsewhere leaves the strip and the store; a
	 * no-op for a cord the tabs aren't keyed to.
	 */
	retain(cord_id: Uuid, space_ids: Iterable<Uuid>): void {
		untrack(() => {
			if (this.cord_id !== cord_id) return;
			const live = new Set(space_ids);
			if (this.preview_id !== null && !live.has(this.preview_id)) this.preview_id = null;
			const kept = this.permanent_ids.filter((id) => live.has(id));
			if (kept.length !== this.permanent_ids.length) this.#set_permanent(kept);
		});
	}

	#set_permanent(ids: ReadonlyArray<Uuid>): void {
		this.permanent_ids = ids;
		const { account_id, cord_id } = this;
		if (account_id === null || cord_id === null) return;
		try {
			const storage = this.#get_storage();
			const key = to_cord_space_tabs_storage_key(account_id, cord_id);
			if (ids.length === 0) {
				storage?.removeItem(key);
			} else {
				storage?.setItem(key, JSON.stringify(ids));
			}
		} catch {
			// a blocked or full store: the tabs last this page's life
		}
	}

	#read(account_id: Uuid, cord_id: Uuid): ReadonlyArray<Uuid> {
		try {
			const raw = this.#get_storage()?.getItem(to_cord_space_tabs_storage_key(account_id, cord_id));
			if (!raw) return [];
			const parsed = parse(CordSpaceTabsStored, JSON.parse(raw));
			if (!parsed.ok) return [];
			// a cord holds at most so many spaces, so a longer record isn't one this wrote
			return [...new Set(parsed.value)].slice(0, CORD_SPACE_COUNT_MAX);
		} catch {
			return [];
		}
	}
}

/**
 * Svelte context for the one `CordSpaceTabs`, which the root layout provides
 * so the rail and the cord's pages share it.
 */
export const cord_space_tabs_context = create_context<CordSpaceTabs>();
