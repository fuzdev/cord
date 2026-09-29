/**
 * Reactive state for one cord's pages: the cord the route names, its
 * members, and what the viewer's role there lets them see.
 *
 * The `/c/[cord_id]` layout creates one `CordPageState`, drives its fetches,
 * and provides it through `cord_page_state_context` to the tab pages under
 * it, so the cord and its member list load once for every tab, and the
 * members tab refreshes the list when it opens. Its derived views (`cord`,
 * `role`, `members_loaded`, and the rest) answer only for the cord the
 * route names, so a navigation to another cord drops the last one's in the
 * same update that changes the route; the held `cord_state` and
 * `members_state` may still hold the last cord until the next fetch. The
 * invite links belong to the invites page alone, which builds its own
 * `CordInvitesState`.
 *
 * @module
 */

import { untrack } from 'svelte';
import type { Uuid } from '@fuzdev/fuz_util/id.ts';
import { create_context } from '@fuzdev/fuz_ui/context_helpers.ts';

import { ERROR_CORD_NOT_FOUND, type CordJson } from '../cord_action_specs.ts';
import { cord_role_can_manage, type CordRole } from '../cord_roles.ts';
import { rpc_error_reason } from './cord_errors.ts';
import { CordMembersState } from './cord_members_state.svelte.ts';
import { CordState, type CordsStateOptions } from './cords_state.svelte.ts';

/** Options for `CordPageState`: the RPC client, plus readers of its reactive inputs. */
export interface CordPageStateOptions extends CordsStateOptions {
	/**
	 * Reads the id of the cord the route names, `undefined` when the route's
	 * segment isn't a uuid (and so can't name a cord).
	 */
	get_cord_id: () => Uuid | undefined;
	/** Reads the viewer's actor id, `null` while signed out. */
	get_self_actor_id: () => Uuid | null;
}

/** One cord as its pages show it: the cord, its members, and the viewer's place in it. */
export class CordPageState {
	readonly cord_state: CordState;
	readonly members_state: CordMembersState;

	readonly #get_cord_id: () => Uuid | undefined;
	readonly #get_self_actor_id: () => Uuid | null;

	/** The id of the cord the route names, `undefined` for a malformed one. */
	readonly cord_id: Uuid | undefined = $derived.by(() => this.#get_cord_id());
	/** The viewer's actor id. */
	readonly self_actor_id: Uuid | null = $derived.by(() => this.#get_self_actor_id());

	/**
	 * The route's cord once loaded; `null` while loading, when it isn't
	 * found, and while the state still holds another cord, so a navigation
	 * to another cord clears it in the same update that changes the route.
	 */
	readonly cord: CordJson | null = $derived.by(() => {
		const cord = this.cord_state.cord;
		return cord && cord.id === this.cord_id ? cord : null;
	});

	/**
	 * Whether loading the route's cord failed; a failed load of a cord the
	 * state has moved off says nothing about this one.
	 */
	readonly load_failed: boolean = $derived.by(
		() => this.cord_state.cord_id === this.cord_id && this.cord_state.load.failed
	);

	/**
	 * Whether the route names no cord the viewer is in. The server answers a
	 * cord you aren't in exactly as one that doesn't exist, and a malformed id
	 * names none.
	 */
	readonly not_found: boolean = $derived.by(
		() =>
			!this.cord_id ||
			(this.load_failed &&
				rpc_error_reason(this.cord_state.load.error_data) === ERROR_CORD_NOT_FOUND)
	);

	/**
	 * Whether the member list is this cord's, loaded; a list still loading,
	 * failed, or left from another cord says nothing about this one.
	 */
	readonly members_loaded: boolean = $derived.by(
		() => !!this.cord && this.members_state.loaded_cord_id === this.cord.id
	);

	/**
	 * The viewer's role, from this cord's member list once it's loaded (so
	 * their own role change shows at once), else from the cord.
	 */
	readonly role: CordRole | undefined = $derived.by(
		() =>
			(this.members_loaded ? this.members_state.get_member(this.self_actor_id)?.role : undefined) ??
			this.cord?.role
	);

	/** Whether the viewer's role manages the cord: renaming it, and its invite links. */
	readonly can_manage: boolean = $derived.by(
		() => this.role !== undefined && cord_role_can_manage(this.role)
	);

	constructor(options: CordPageStateOptions) {
		this.cord_state = new CordState(options);
		this.members_state = new CordMembersState(options);
		this.#get_cord_id = options.get_cord_id;
		this.#get_self_actor_id = options.get_self_actor_id;
	}

	/**
	 * Load the member list of the cord `cord_id`, unless a load of it is
	 * already running. The layout calls it when a cord loads and the members
	 * page when it opens, so whichever comes first sends the one request. A
	 * load for the cord the list already holds keeps the list up while it
	 * runs. Reads nothing trackably, so an `$effect` may call it.
	 */
	async fetch_members(cord_id: Uuid): Promise<void> {
		const running = untrack(
			() => this.members_state.cord_id === cord_id && this.members_state.list.loading
		);
		if (running) return;
		await this.members_state.fetch(cord_id);
	}
}

/**
 * Svelte context for the `CordPageState` the `/c/[cord_id]` layout provides
 * to its tab pages.
 */
export const cord_page_state_context = create_context<CordPageState>();
