/**
 * Reactive state for one cord's spaces: the list, and adding, renaming, and
 * deleting a space.
 *
 * The root layout provides the one `CordSpacesState` through
 * `cord_spaces_state_context`, rather than the cord layout owning it beside
 * `CordPageState`, because the rail sits in the root layout and nests the
 * spaces of the cord you're on. The cord layout drives its fetches for the
 * route's cord, and its readers ask for a cord by id (`loaded_spaces`,
 * `get_space`, `list_error_for`), so a list left from another cord answers
 * for none.
 *
 * `fetch` reads no reactive state trackably, so a page may call it from an
 * `$effect`. A response for a cord the state has since moved off is
 * dropped, as is a write's result.
 *
 * @module
 */

import { untrack } from 'svelte';
import type { ThrowingApi } from '@fuzdev/fuz_app/actions/rpc_client.ts';
import { AsyncSlot } from '@fuzdev/fuz_app/ui/async_slot.svelte.ts';
import { KeyedAsyncSlot } from '@fuzdev/fuz_app/ui/keyed_async_slot.svelte.ts';
import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';
import { create_context } from '@fuzdev/fuz_ui/context_helpers.ts';

import type { FrontendActionsApi } from '../action_metatypes.ts';
import {
	ERROR_CORD_SPACE_NOT_FOUND,
	type CordSpaceCreateOutput,
	type CordSpaceDeleteOutput,
	type CordSpaceJson,
	type CordSpaceKind,
	type CordSpaceUpdateOutput
} from '../cord_action_specs.ts';
import { rpc_error_reason, to_cord_error_message } from './cord_errors.ts';
import type { CordsStateOptions } from './cords_state.svelte.ts';

/** One cord's spaces, and the changes its owners and admins make to them. */
export class CordSpacesState {
	readonly #api: ThrowingApi<FrontendActionsApi>;

	readonly list: AsyncSlot = new AsyncSlot({ map_error: to_cord_error_message });
	readonly create: AsyncSlot<CordSpaceCreateOutput> = new AsyncSlot({
		map_error: to_cord_error_message
	});
	/** Renames, by space id. */
	readonly rename: KeyedAsyncSlot<Uuid, CordSpaceUpdateOutput> = new KeyedAsyncSlot({
		map_error: to_cord_error_message
	});
	/** Deletions, by space id. */
	readonly delete: KeyedAsyncSlot<Uuid, CordSpaceDeleteOutput> = new KeyedAsyncSlot({
		map_error: to_cord_error_message
	});

	/** The cord whose spaces the state is for, or `null` before the first fetch. */
	cord_id: Uuid | null = $state.raw(null);
	/**
	 * The cord `spaces` came from, once a fetch has succeeded for it; `null`
	 * before then. A failed refresh keeps it, since the list it loaded stands.
	 */
	loaded_cord_id: Uuid | null = $state.raw(null);
	/** The spaces, oldest first. */
	spaces: ReadonlyArray<CordSpaceJson> = $state.raw([]);

	constructor(options: CordsStateOptions) {
		this.#api = options.api;
	}

	/**
	 * Load the spaces of `cord_id`, forgetting another cord's, along with
	 * the results and errors of its writes.
	 */
	async fetch(cord_id: Uuid): Promise<void> {
		// untracked: an effect calling this must not depend on the `cord_id` it writes
		untrack(() => {
			if (this.cord_id !== cord_id) {
				this.spaces = [];
				this.loaded_cord_id = null;
				this.create.reset();
				this.rename.reset();
				this.delete.reset();
			}
			this.cord_id = cord_id;
		});
		await this.list.run(async (signal) => {
			const { spaces } = await this.#api.cord_space_list({ cord_id }, { signal });
			// a superseded fetch (another cord's, or an older one) keeps out
			if (signal.aborted || this.cord_id !== cord_id) return;
			this.spaces = spaces;
			this.loaded_cord_id = cord_id;
		});
	}

	/**
	 * Load the spaces of `cord_id`, unless a load of them is already
	 * running, so the cord layout and a page opening together send one
	 * request. A load for the cord the list already holds keeps it up while
	 * it runs. Reads nothing trackably.
	 */
	async refresh(cord_id: Uuid): Promise<void> {
		const running = untrack(() => this.cord_id === cord_id && this.list.loading);
		if (running) return;
		await this.fetch(cord_id);
	}

	/**
	 * Forget every cord's spaces, as when the account changes. Untracked: a
	 * keyed slot's reset reads its entries, and an effect calling this must
	 * not rerun on every write.
	 */
	reset(): void {
		untrack(() => {
			this.list.reset();
			this.create.reset();
			this.rename.reset();
			this.delete.reset();
			this.cord_id = null;
			this.loaded_cord_id = null;
			this.spaces = [];
		});
	}

	/** The spaces of `cord_id` once loaded, else `null`. */
	loaded_spaces(cord_id: Uuid): ReadonlyArray<CordSpaceJson> | null {
		return this.loaded_cord_id === cord_id ? this.spaces : null;
	}

	/**
	 * The list's error for `cord_id`, `null` when there's none or the list is
	 * another cord's. A failed refresh keeps the loaded list, so the error
	 * may show beside it.
	 */
	list_error_for(cord_id: Uuid): string | null {
		return this.cord_id === cord_id ? this.list.error : null;
	}

	/** The space `space_id` of `cord_id`, if the loaded list has it. */
	get_space(cord_id: Uuid, space_id: Uuid): CordSpaceJson | undefined {
		return this.loaded_spaces(cord_id)?.find((s) => s.id === space_id);
	}

	/**
	 * Add a space to the cord the state is for, at the end of the list.
	 *
	 * @returns the new space, or `undefined` when adding failed (the error is on `create`)
	 */
	async create_space(kind: CordSpaceKind, name: string): Promise<CordSpaceJson | undefined> {
		const cord_id = this.cord_id;
		if (!cord_id) return undefined;
		const result = await this.create.run(() =>
			this.#api.cord_space_create({ cord_id, kind, name })
		);
		if (result === undefined || this.cord_id !== cord_id) return undefined;
		const { space } = result;
		if (this.loaded_cord_id === cord_id && !this.spaces.some((s) => s.id === space.id)) {
			this.spaces = [...this.spaces, space];
		}
		// a refresh in flight may have read the list before this space existed
		if (this.list.loading) void this.fetch(cord_id);
		return space;
	}

	/**
	 * Rename the space `space_id`.
	 *
	 * @returns whether the rename succeeded (the error is on `rename`)
	 */
	async rename_space(space_id: Uuid, name: string): Promise<boolean> {
		const cord_id = this.cord_id;
		if (!cord_id) return false;
		const result = await this.rename.run(space_id, () =>
			this.#api.cord_space_update({ cord_id, space_id, name })
		);
		if (this.cord_id !== cord_id) return false;
		if (result === undefined) {
			// a space the server no longer has means the list is stale
			const reason = rpc_error_reason(this.rename.get(space_id)?.error_data);
			if (reason === ERROR_CORD_SPACE_NOT_FOUND) void this.fetch(cord_id);
			return false;
		}
		const { space } = result;
		this.spaces = this.spaces.map((s) => (s.id === space.id ? space : s));
		// a refresh in flight may have read the list before the rename
		if (this.list.loading) void this.fetch(cord_id);
		return true;
	}

	/**
	 * Delete the space `space_id`. A space the server no longer has counts
	 * as deleted: it's gone either way, so it leaves the list.
	 *
	 * @returns whether the space is gone (the error is on `delete` otherwise)
	 */
	async delete_space(space_id: Uuid): Promise<boolean> {
		const cord_id = this.cord_id;
		if (!cord_id) return false;
		const result = await this.delete.run(space_id, () =>
			this.#api.cord_space_delete({ cord_id, space_id })
		);
		if (this.cord_id !== cord_id) return false;
		const gone =
			result !== undefined ||
			rpc_error_reason(this.delete.get(space_id)?.error_data) === ERROR_CORD_SPACE_NOT_FOUND;
		if (!gone) return false;
		this.delete.delete(space_id);
		this.spaces = this.spaces.filter((s) => s.id !== space_id);
		// a refresh in flight may have read the list before the delete
		if (this.list.loading) void this.fetch(cord_id);
		return true;
	}
}

/**
 * Svelte context for the one `CordSpacesState`, which the root layout
 * provides so the rail and the cord's pages share it.
 */
export const cord_spaces_state_context = create_context<CordSpacesState>();
