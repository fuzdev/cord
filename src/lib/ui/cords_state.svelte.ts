/**
 * Reactive state for cords: the signed-in account's cord list with the
 * create form behind it, one cord, and the instance admins' ops list.
 *
 * Each class holds `AsyncSlot`s from fuz_app, one per operation, and calls
 * the throwing RPC proxy. The root layout provides the one `CordsState`
 * through `cords_state_context`, which the rail and the home page share;
 * pages construct the others with the `api` from `cord_rpc_context`
 * (`CordState` through a cord's `CordPageState`).
 * The create and rename slots map a refusal through `cord_errors.ts`, so a
 * name the server refuses shows the name rules; the read slots keep
 * `AsyncSlot`'s default error message, and `CordPageState` reads a load's
 * refusal reason with `rpc_error_reason`.
 *
 * @module
 */

import { untrack } from 'svelte';
import type { ThrowingApi } from '@fuzdev/fuz_app/actions/rpc_client.ts';
import { AsyncSlot } from '@fuzdev/fuz_app/ui/async_slot.svelte.ts';
import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';
import { create_context } from '@fuzdev/fuz_ui/context_helpers.ts';

import type { FrontendActionsApi } from '../action_metatypes.ts';
import type {
	CordAdminJson,
	CordCreateOutput,
	CordJson,
	CordUpdateOutput
} from '../cord_action_specs.ts';
import { to_cord_error_message } from './cord_errors.ts';

/** Options shared by the cord state classes. */
export interface CordsStateOptions {
	api: ThrowingApi<FrontendActionsApi>;
}

/** The signed-in account's cords, and creating one. */
export class CordsState {
	readonly #api: ThrowingApi<FrontendActionsApi>;

	/** Keeps its error through a retry, so a retry control stays mounted. */
	readonly list: AsyncSlot = new AsyncSlot({ preserve_error_on_retry: true });
	readonly create: AsyncSlot<CordCreateOutput> = new AsyncSlot({
		map_error: to_cord_error_message
	});

	/** The caller's cords, in the order they joined them. */
	cords: Array<CordJson> = $state.raw([]);
	/**
	 * Whether `cords` is a loaded list: set by a fetch that succeeds and
	 * cleared by `reset`, so a refresh in flight or failed keeps the list up.
	 */
	loaded: boolean = $state.raw(false);

	constructor(options: CordsStateOptions) {
		this.#api = options.api;
	}

	/** Load the caller's cords. */
	async fetch(): Promise<void> {
		await this.list.run(async (signal) => {
			const { cords } = await this.#api.cord_list({}, { signal });
			// a superseded fetch, or one a reset cut off, keeps out
			if (signal.aborted) return;
			this.cords = cords;
			this.loaded = true;
		});
	}

	/** Forget the loaded cords, as when the account signs out. */
	reset(): void {
		this.list.reset();
		this.create.reset();
		this.cords = [];
		this.loaded = false;
	}

	/**
	 * Create a cord and add it to the end of the list, where the newest
	 * membership goes.
	 *
	 * @returns the new cord, or `undefined` when creation failed (the error is on `create`)
	 */
	async create_cord(name: string): Promise<CordJson | undefined> {
		const result = await this.create.run(() => this.#api.cord_create({ name }));
		if (result === undefined) return undefined;
		this.cords = [...this.cords, result.cord];
		// a refresh in flight may have read the list before this cord existed
		if (this.list.loading) void this.fetch();
		return result.cord;
	}

	/**
	 * Replace the loaded cord with its id; a cord the list doesn't hold is
	 * left for the next fetch.
	 */
	put_cord(cord: CordJson): void {
		const index = this.cords.findIndex((c) => c.id === cord.id);
		if (index === -1) return;
		this.cords = this.cords.map((c, i) => (i === index ? cord : c));
	}

	/**
	 * Drop the cord `cord_id` from the list, as after leaving it; a no-op,
	 * without reassigning the list, when the list doesn't hold it.
	 */
	remove_cord(cord_id: Uuid): void {
		if (!this.cords.some((c) => c.id === cord_id)) return;
		this.cords = this.cords.filter((c) => c.id !== cord_id);
	}
}

/**
 * Svelte context for the signed-in account's `CordsState`, which the root
 * layout provides, fetches when an account signs in, and resets when it
 * signs out.
 */
export const cords_state_context = create_context<CordsState>();

/** One cord, as its pages show it, and renaming it. */
export class CordState {
	readonly #api: ThrowingApi<FrontendActionsApi>;

	readonly load: AsyncSlot = new AsyncSlot();
	readonly rename: AsyncSlot<CordUpdateOutput> = new AsyncSlot({
		map_error: to_cord_error_message
	});

	/** The cord the last fetch asked for, or `null` before the first. */
	cord_id: Uuid | null = $state.raw(null);
	/** The cord once loaded; `null` before that, and when it isn't found. */
	cord: CordJson | null = $state.raw(null);

	constructor(options: CordsStateOptions) {
		this.#api = options.api;
	}

	/**
	 * Load the cord `cord_id`, clearing any cord shown before. Moving to
	 * another cord also drops a rename of the last one, running or done, so
	 * its result and error stay with the cord it named.
	 */
	async fetch(cord_id: Uuid): Promise<void> {
		// untracked: an effect calling this must not depend on the `cord_id` it writes
		untrack(() => {
			if (this.cord_id !== cord_id) this.rename.reset();
			this.cord_id = cord_id;
			this.cord = null;
		});
		await this.load.run(async (signal) => {
			const { cord } = await this.#api.cord_get({ cord_id }, { signal });
			// a superseded fetch (another cord's, or an older one) keeps out
			if (signal.aborted) return;
			this.cord = cord;
		});
	}

	/**
	 * Rename the loaded cord.
	 *
	 * @returns whether the rename succeeded
	 */
	async rename_cord(name: string): Promise<boolean> {
		const cord = this.cord;
		if (!cord) return false;
		const result = await this.rename.run(() => this.#api.cord_update({ cord_id: cord.id, name }));
		if (result === undefined) return false;
		this.cord = result.cord;
		return true;
	}
}

/** Every cord on the instance, for instance admins' operations. */
export class AdminCordsState {
	readonly #api: ThrowingApi<FrontendActionsApi>;

	readonly list: AsyncSlot = new AsyncSlot();

	/** Every cord, newest first. */
	cords: Array<CordAdminJson> = $state.raw([]);

	constructor(options: CordsStateOptions) {
		this.#api = options.api;
	}

	/** Load every cord. */
	async fetch(): Promise<void> {
		await this.list.run(async (signal) => {
			const { cords } = await this.#api.cord_admin_list({}, { signal });
			// a superseded fetch keeps out
			if (signal.aborted) return;
			this.cords = cords;
		});
	}
}
