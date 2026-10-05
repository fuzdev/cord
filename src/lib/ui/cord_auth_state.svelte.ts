/**
 * cord's session state: fuz_app's `AuthState` plus whether the first session
 * check has settled.
 *
 * Pages prerender without a session, so before the first check finishes
 * `verified` is false for everyone. `session_checked` lets a page show a
 * neutral "checking session" state until then, instead of flashing the
 * signed-out view at someone who is signed in. Gates in front of fuz_app's
 * forms must not also wait on `verifying`: it turns true during `login`,
 * `signup`, and `bootstrap` too, and unmounting a form mid-submit loses its
 * typed input when the attempt fails.
 *
 * Admin UI gates on the inherited `is_admin`, a global `admin` grant: the
 * rule the server applies to instance-admin actions like `cord_create`.
 *
 * @module
 */

import { AuthState } from '@fuzdev/fuz_app/ui/auth_state.svelte.ts';
import { create_context } from '@fuzdev/fuz_ui/context_helpers.ts';

/**
 * `AuthState` that records when its first `check_session` settles. The root
 * layout provides one instance through both `auth_state_context` (for
 * fuz_app's components) and `cord_auth_state_context`.
 */
export class CordAuthState extends AuthState {
	/** True once a `check_session` call has settled, whatever its outcome. */
	session_checked: boolean = $state.raw(false);

	readonly #logout_listeners: Set<() => void> = new Set();

	/**
	 * Call `listener` after each `logout`, to drop what belonged to whoever
	 * was signed in (the join page's stored link and redemption). Only
	 * `logout` calls it: a session that ends another way (fuz_app's
	 * `AccountSessions` revoking every session, which signs this one out
	 * without `logout`, or an expiry) leaves what the listeners would drop
	 * in place.
	 *
	 * @returns a function that removes the listener, fit to return from an `$effect`
	 */
	on_logout(listener: () => void): () => void {
		this.#logout_listeners.add(listener);
		return () => {
			this.#logout_listeners.delete(listener);
		};
	}

	/**
	 * True when the session is verified and its account has loaded. After a
	 * successful `login`, `signup`, or `bootstrap`, `verified` turns true
	 * before the follow-up `check_session` fills in the account and role
	 * grants; gating signed-in views on this skips that gap.
	 */
	get signed_in(): boolean {
		return this.verified && this.account !== null;
	}

	/**
	 * Check the session, then mark it checked. A check that ends unverified
	 * also clears the account, actor, and role grants, which fuz_app's
	 * `check_session` leaves in place; otherwise a later `login` would set
	 * `verified` while the previous identity's account and role grants still
	 * read as `signed_in`.
	 *
	 * @mutates `this`
	 */
	override async check_session(): Promise<void> {
		try {
			await super.check_session();
		} finally {
			if (!this.verified) {
				this.account = null;
				this.actor = null;
				this.role_grants = [];
			}
			this.session_checked = true;
		}
	}

	/**
	 * Log out, then call the `on_logout` listeners.
	 *
	 * @mutates `this`
	 */
	override async logout(): Promise<void> {
		await super.logout();
		for (const listener of this.#logout_listeners) listener();
	}
}

/** Svelte context for the `CordAuthState` the root layout provides. */
export const cord_auth_state_context = create_context<CordAuthState>();
