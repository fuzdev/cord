/**
 * Reactive state for one cord's members: the list, setting a member's role,
 * removing a member, and leaving.
 *
 * The server applies the member rules to membership it reads under the
 * cord's lock; the page uses the twins in `cord_roles.ts` only to decide
 * which controls to offer.
 *
 * `fetch` reads no reactive state trackably, so a page may call it from an
 * `$effect` without the effect depending on what it writes. A response
 * for a cord the state has since moved off is dropped, and a write's
 * follow-up reload is skipped once the state has moved to another cord.
 *
 * @module
 */

import { untrack } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import type { ThrowingApi } from '@fuzdev/fuz_app/actions/rpc_client.ts';
import { AsyncSlot } from '@fuzdev/fuz_app/ui/async_slot.svelte.ts';
import { KeyedAsyncSlot } from '@fuzdev/fuz_app/ui/keyed_async_slot.svelte.ts';
import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';

import type { FrontendActionsApi } from '../action_metatypes.ts';
import type {
	CordLeaveOutput,
	CordMemberJson,
	CordMemberRemoveOutput,
	CordMemberRoleSetOutput
} from '../cord_action_specs.ts';
import { CORD_OWNER, type CordRole } from '../cord_roles.ts';
import { to_cord_error_message } from './cord_errors.ts';
import type { CordsStateOptions } from './cords_state.svelte.ts';

/** One cord's members, and the changes its members can make to them. */
export class CordMembersState {
	readonly #api: ThrowingApi<FrontendActionsApi>;

	readonly list: AsyncSlot = new AsyncSlot({ map_error: to_cord_error_message });
	/** Role changes, by the member's actor id. */
	readonly role_set: KeyedAsyncSlot<Uuid, CordMemberRoleSetOutput> = new KeyedAsyncSlot({
		map_error: to_cord_error_message
	});
	/** Removals, by the member's actor id. */
	readonly remove: KeyedAsyncSlot<Uuid, CordMemberRemoveOutput> = new KeyedAsyncSlot({
		map_error: to_cord_error_message
	});
	readonly leave: AsyncSlot<CordLeaveOutput> = new AsyncSlot({ map_error: to_cord_error_message });

	/** The cord whose members are loaded, or `null` before the first fetch. */
	cord_id: Uuid | null = $state.raw(null);
	/**
	 * The cord `members` came from, once a fetch has succeeded for it; `null`
	 * before then, so `members` is empty for want of a list, not for want of
	 * members. A failed refresh keeps it, since the list it loaded stands.
	 */
	loaded_cord_id: Uuid | null = $state.raw(null);
	/** The members, owners first, then by username. */
	members: Array<CordMemberJson> = $state.raw([]);

	// the members whose role change or removal is still running, through the
	// list reload after it, each with its call's own token: a call that ends
	// after the state moved off its cord and back must not clear a newer
	// call's entry for the same member
	readonly #role_changing: SvelteMap<Uuid, symbol> = new SvelteMap();
	readonly #removing: SvelteMap<Uuid, symbol> = new SvelteMap();

	/** How many owners the cord has, for the last-owner rule. */
	readonly owner_count: number = $derived(this.members.filter((m) => m.role === CORD_OWNER).length);

	constructor(options: CordsStateOptions) {
		this.#api = options.api;
	}

	/** Load the members of `cord_id`, forgetting another cord's. */
	async fetch(cord_id: Uuid): Promise<void> {
		// untracked: an effect calling this must not depend on the `cord_id` it writes
		untrack(() => {
			if (this.cord_id !== cord_id) {
				this.members = [];
				this.loaded_cord_id = null;
				this.role_set.reset();
				this.remove.reset();
				this.#role_changing.clear();
				this.#removing.clear();
				this.leave.reset();
			}
			this.cord_id = cord_id;
		});
		await this.list.run(async (signal) => {
			const { members } = await this.#api.cord_member_list({ cord_id }, { signal });
			// a superseded fetch (another cord's, or an older one) keeps out
			if (signal.aborted || this.cord_id !== cord_id) return;
			this.members = members;
			this.loaded_cord_id = cord_id;
		});
	}

	/** The member with the actor id `actor_id`, if they're in the loaded list. */
	get_member(actor_id: Uuid | null | undefined): CordMemberJson | undefined {
		return actor_id ? this.members.find((m) => m.actor_id === actor_id) : undefined;
	}

	/**
	 * Whether a role change for the member `actor_id` is running, from the
	 * call through the list reload after it. Wider than `role_set.loading`,
	 * which ends before the reload: until the list shows the new role, the
	 * member's row still shows the old one, and applying from it again would
	 * send the change twice.
	 */
	role_changing(actor_id: Uuid): boolean {
		return this.#role_changing.has(actor_id);
	}

	/** Whether removing the member `actor_id` is running, through the reload after it. */
	removing(actor_id: Uuid): boolean {
		return this.#removing.has(actor_id);
	}

	/**
	 * Set a member's role, then reload the list (the order follows roles).
	 * Does nothing while a change for that member is already running.
	 *
	 * @returns whether the change succeeded (the error is on `role_set`)
	 */
	async set_role(actor_id: Uuid, role: CordRole): Promise<boolean> {
		const cord_id = this.cord_id;
		if (!cord_id || this.#role_changing.has(actor_id)) return false;
		const token = Symbol('role_set');
		this.#role_changing.set(actor_id, token);
		try {
			const result = await this.role_set.run(actor_id, () =>
				this.#api.cord_member_role_set({ cord_id, actor_id, role })
			);
			// a refusal may mean the list is stale (someone else changed it), so reload either way
			if (this.cord_id === cord_id) await this.fetch(cord_id);
			return result !== undefined;
		} finally {
			// a move off the cord cleared it, and a newer call may hold it now
			if (this.#role_changing.get(actor_id) === token) this.#role_changing.delete(actor_id);
		}
	}

	/**
	 * Remove a member, then reload the list. Does nothing while removing that
	 * member is already running.
	 *
	 * @returns whether the removal succeeded (the error is on `remove`)
	 */
	async remove_member(actor_id: Uuid): Promise<boolean> {
		const cord_id = this.cord_id;
		if (!cord_id || this.#removing.has(actor_id)) return false;
		const token = Symbol('remove');
		this.#removing.set(actor_id, token);
		try {
			const result = await this.remove.run(actor_id, () =>
				this.#api.cord_member_remove({ cord_id, actor_id })
			);
			if (this.cord_id === cord_id) await this.fetch(cord_id);
			return result !== undefined;
		} finally {
			if (this.#removing.get(actor_id) === token) this.#removing.delete(actor_id);
		}
	}

	/**
	 * Leave the loaded cord.
	 *
	 * @returns whether leaving succeeded (the error is on `leave`)
	 */
	async leave_cord(): Promise<boolean> {
		const cord_id = this.cord_id;
		if (!cord_id) return false;
		const result = await this.leave.run(() => this.#api.cord_leave({ cord_id }));
		return result !== undefined;
	}
}
