/**
 * Reactive state for cords: the signed-in account's cord list with the
 * create form behind it, one cord's page, and the instance admins' ops list.
 *
 * Each class holds `AsyncSlot`s from fuz_app, one per operation, and calls
 * the throwing RPC proxy. Pages construct them with the `api` from
 * `cord_rpc_context`.
 *
 * @module
 */

import type { ThrowingApi } from '@fuzdev/fuz_app/actions/rpc_client.ts';
import { AsyncSlot } from '@fuzdev/fuz_app/ui/async_slot.svelte.ts';
import type { Uuid } from '@fuzdev/fuz_util/id.ts';

import type { FrontendActionsApi } from '../action_metatypes.ts';
import type {
	CordAdminJson,
	CordCreateOutput,
	CordJson,
	CordUpdateOutput
} from '../cord_action_specs.ts';

/** Options shared by the cord state classes. */
export interface CordsStateOptions {
	api: ThrowingApi<FrontendActionsApi>;
}

/** The signed-in account's cords, and creating one. */
export class CordsState {
	readonly #api: ThrowingApi<FrontendActionsApi>;

	readonly list: AsyncSlot = new AsyncSlot();
	readonly create: AsyncSlot<CordCreateOutput> = new AsyncSlot();

	/** The caller's cords, in the order they joined them. */
	cords: Array<CordJson> = $state.raw([]);

	constructor(options: CordsStateOptions) {
		this.#api = options.api;
	}

	/** Load the caller's cords. */
	async fetch(): Promise<void> {
		await this.list.run(async (signal) => {
			const { cords } = await this.#api.cord_list({}, { signal });
			this.cords = cords;
		});
	}

	/** Forget the loaded cords, as when the account signs out. */
	reset(): void {
		this.list.reset();
		this.create.reset();
		this.cords = [];
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
		return result.cord;
	}
}

/** One cord, as its page shows it, and renaming it. */
export class CordState {
	readonly #api: ThrowingApi<FrontendActionsApi>;

	readonly load: AsyncSlot = new AsyncSlot();
	readonly rename: AsyncSlot<CordUpdateOutput> = new AsyncSlot();

	/** The cord once loaded; `null` before that, and when it isn't found. */
	cord: CordJson | null = $state.raw(null);

	constructor(options: CordsStateOptions) {
		this.#api = options.api;
	}

	/** Load the cord `cord_id`, clearing any cord shown before. */
	async fetch(cord_id: Uuid): Promise<void> {
		this.cord = null;
		await this.load.run(async (signal) => {
			const { cord } = await this.#api.cord_get({ cord_id }, { signal });
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
			this.cords = cords;
		});
	}
}
