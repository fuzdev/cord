/**
 * Reactive state for the join page, `/join#<token>`, and the pure pieces it
 * decides with: where the token comes from, which view to show for a
 * session and a preview, and the newcomer form's checks.
 *
 * The token rides in the URL fragment, which browsers never send to a
 * server, so it stays out of access logs and `Referer` headers, and `/join`
 * prerenders like any other page. The page reads it once, keeps it in
 * `sessionStorage` with the newcomer's redemption once there is one (the
 * cord and the reserved username), and replaces the URL with a bare `/join`,
 * dropping the token from the address bar and that history entry. So a
 * reload mid-join still finds the link, and a newcomer whose signup failed
 * after redeeming can retry, even on a link their own redemption used up.
 * The browser's own history and autocomplete may still record
 * `/join#<token>` before the page drops it, which is why links carry an
 * expiry and can be revoked.
 *
 * The stored link's lifecycle:
 *
 * - A retry keeps the reserved username: the form shows it read-only, so a
 *   retry never spends another use on a new name. If redeem refuses the
 *   reserved name as taken or invalid, the reservation is gone, and the
 *   page drops it and opens the field again.
 * - While the page previews the link again after a failed attempt, the view
 *   keeps the last preview, so the form stays mounted with its input and
 *   error. A re-preview that fails for a reason that says nothing about the
 *   link (the network) keeps the view on the last preview with a quiet note
 *   (`to_cord_join_recheck_error`); a dead-link refusal ends it.
 * - A redemption answers only for whoever it reserved a username for:
 *   anyone signed out, or the account with that username (lowercased, as
 *   the spine compares them). Another account signed in on the tab gets its
 *   own view, and the stored redemption stays for the newcomer to come back
 *   to.
 * - The stored link is forgotten once it has done its job or can't do one
 *   (unless it holds a redemption for someone other than the signed-in
 *   account), and when `logout` runs, which also drops the redemption
 *   (`AccountSessions` runs it for its revokes of this session). A session
 *   ended another way, as by an expiry, doesn't forget it (see
 *   `CordAuthState.on_logout`).
 * - A restored redemption dies with a revoked link at once. On an expired
 *   link it lasts until redeem or accept refuses, since the preview can't
 *   show accept's grace.
 *
 * @module
 */

import { untrack } from 'svelte';
import type { ThrowingApi } from '@fuzdev/fuz_app/actions/rpc_client.ts';
import { AsyncSlot } from '@fuzdev/fuz_app/ui/async_slot.svelte.ts';
import { Password, PASSWORD_LENGTH_MIN } from '@fuzdev/fuz_app/auth/password.ts';
import { is, nullable, object, parse, string, type Infer } from '@fuzdev/fuz_schema/schema.ts';
import { Username } from '@fuzdev/fuz_app/primitive_schemas.ts';
import type { AsyncStatus } from '@fuzdev/fuz_util/async.ts';

import type { FrontendActionsApi } from '../action_metatypes.ts';
import {
	CordInviteCordJson,
	CordInviteToken,
	ERROR_CORD_INVITE_EXHAUSTED,
	ERROR_CORD_INVITE_EXPIRED,
	ERROR_CORD_INVITE_NEWCOMERS_CLOSED,
	ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS,
	ERROR_CORD_INVITE_NOT_FOUND,
	ERROR_CORD_INVITE_REVOKED,
	ERROR_CORD_INVITE_USERNAME_INVALID,
	ERROR_CORD_INVITE_USERNAME_UNAVAILABLE,
	type CordInviteAcceptOutput,
	type CordInvitePreviewOutput,
	type CordInviteRedeemOutput,
	type CordJson
} from '../cord_action_specs.ts';
import {
	CORD_ERROR_MESSAGES,
	CORD_USERNAME_RULE,
	rpc_error_reason,
	to_cord_error_message
} from './cord_errors.ts';
import type { CordsStateOptions } from './cords_state.svelte.ts';

/** The `sessionStorage` key the join page keeps its link under. */
export const CORD_JOIN_STORAGE_KEY = 'cord_join';

/** The slice of `Storage` the join page needs. */
export type CordJoinStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

/** A newcomer's redemption of the link: the cord it's for and the username it reserved. */
export const CordJoinRedeemed = object({ cord: CordInviteCordJson, username: string });
export type CordJoinRedeemed = Infer<typeof CordJoinRedeemed>;

/** What the join page keeps in `sessionStorage`: its link, and a redemption once there is one. */
export const CordJoinStored = object({
	token: string,
	redeemed: nullable(CordJoinRedeemed)
});
export type CordJoinStored = Infer<typeof CordJoinStored>;

/** This tab's `sessionStorage`, or `null` where there's none or it's blocked. */
export const cord_join_session_storage = (): CordJoinStorage | null => {
	try {
		return globalThis.sessionStorage ?? null;
	} catch {
		return null;
	}
};

/** Read the stored link, ignoring a blocked store or a record in another shape. */
const read_cord_join = (storage: CordJoinStorage | null): CordJoinStored | null => {
	try {
		const raw = storage?.getItem(CORD_JOIN_STORAGE_KEY);
		if (!raw) return null;
		const parsed = parse(CordJoinStored, JSON.parse(raw));
		return parsed.ok ? parsed.value : null;
	} catch {
		return null;
	}
};

/** Store the link; a blocked or full store costs only the reload case. */
export const save_cord_join = (storage: CordJoinStorage | null, stored: CordJoinStored): void => {
	try {
		storage?.setItem(CORD_JOIN_STORAGE_KEY, JSON.stringify(stored));
	} catch {
		// the link still lives in memory for this load
	}
};

/** Forget the stored link and its redemption. */
export const forget_cord_join = (storage: CordJoinStorage | null): void => {
	try {
		storage?.removeItem(CORD_JOIN_STORAGE_KEY);
	} catch {
		// nothing to forget in a blocked store
	}
};

/**
 * The join page's link for a page load: the URL fragment's token when it
 * has one, else the link stored by an earlier load in this tab, else
 * `null`. A fragment naming the stored link keeps its redemption; any other
 * fragment replaces the stored link.
 */
export const load_cord_join = (
	hash: string,
	storage: CordJoinStorage | null
): CordJoinStored | null => {
	const from_hash = hash.startsWith('#') ? hash.slice(1) : hash;
	const stored = read_cord_join(storage);
	if (!from_hash) return stored;
	if (stored?.token === from_hash) return stored;
	const fresh: CordJoinStored = { token: from_hash, redeemed: null };
	save_cord_join(storage, fresh);
	return fresh;
};

/**
 * The refusals that mean a link can't be used: an unknown token, or a real
 * link that's revoked, expired, used up, or not for newcomers. The join
 * page answers all of them alike: ask for a new link. A token no link can
 * have (empty, or over-long) never gets this far; the page refuses it
 * before any request. `cord_invite_newcomers_closed` isn't here: that link
 * still works for existing accounts (see `join_as_newcomer`).
 */
const DEAD_LINK_REASONS: ReadonlySet<string> = new Set([
	ERROR_CORD_INVITE_NOT_FOUND,
	ERROR_CORD_INVITE_REVOKED,
	ERROR_CORD_INVITE_EXPIRED,
	ERROR_CORD_INVITE_EXHAUSTED,
	ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS
]);

/**
 * Whether a thrown RPC error means the link can't be used. Only a named
 * refusal does; anything else (a reasonless invalid-params error included,
 * which would mean the client and server disagree on the wire) is a
 * failure to show, not a dead link.
 */
export const is_dead_link_error = (e: unknown): boolean => {
	const reason = rpc_error_reason(e);
	return reason !== undefined && DEAD_LINK_REASONS.has(reason);
};

/**
 * What the join page shows:
 *
 * - `checking` — the token, session, or first preview for this identity is
 *   still loading (a re-preview shows the last one meanwhile)
 * - `dead` — no token, an unknown one, or a link that isn't active
 * - `error` — the preview failed for a reason that says nothing about the
 *   link, with no earlier preview for this identity to show instead (a
 *   failed re-preview keeps showing the last one; see `to_cord_join_recheck_error`)
 * - `member` — signed in and already in the cord
 * - `join` — signed in, and the link admits them
 * - `newcomer` — signed out, and the link can bring someone new onto the
 *   instance, or already reserved them a username here
 * - `sign_in` — signed out, and the link is for people who already have an
 *   account; `newcomers_closed` when a newcomer's redeem just found it closed to them
 */
export type CordJoinView =
	| { kind: 'checking' }
	| { kind: 'dead' }
	| { kind: 'error'; message: string }
	| { kind: 'member'; cord: CordInviteCordJson }
	| { kind: 'join'; cord: CordInviteCordJson }
	| { kind: 'newcomer'; cord: CordInviteCordJson }
	| { kind: 'sign_in'; newcomers_closed: boolean };

/** What the join page decides its view from. */
export interface CordJoinViewInput {
	/** Whether the page has read its token (from the fragment or storage). */
	token_loaded: boolean;
	token: string | null;
	session_checked: boolean;
	/** `AuthState.verified`: true from a successful login, before the account loads. */
	verified: boolean;
	/** `CordAuthState.signed_in`: verified with the account loaded. */
	signed_in: boolean;
	/** The signed-in account's id, `null` when signed out. */
	account_id: string | null;
	/** The signed-in account's username, `null` when signed out. */
	account_username: string | null;
	/** The account the preview was last requested for, `null` for signed out, `undefined` before any. */
	preview_for: string | null | undefined;
	preview_status: AsyncStatus;
	/** The preview's data; while a re-preview is pending, the last one, for the same identity. */
	preview: CordInvitePreviewOutput | undefined;
	/** The preview's thrown error, when it failed. */
	preview_error_data: unknown;
	/** True once redeem or accept has refused the link as dead, whatever the preview said. */
	dead: boolean;
	/** True while the newcomer flow runs (redeem, signup, accept), which pins the newcomer view. */
	newcomer_running: boolean;
	/**
	 * A newcomer's redemption of the link in this tab, if any. Only whoever
	 * it reserved a username for is answered by it: signed out, or signed in
	 * as that username.
	 */
	redeemed: CordJoinRedeemed | null;
	/** True once a newcomer's redeem found the link closed to someone new. */
	newcomers_closed: boolean;
}

/**
 * Whether a redemption answers for whoever is here: anyone signed out (the
 * newcomer it's for, mid-join), or the account with its username, compared
 * lowercased as the spine compares usernames. Another account signed in on
 * this tab leaves it stored, for the newcomer to come back to, but unused.
 */
export const cord_join_redemption_applies = (
	redeemed: CordJoinRedeemed | null,
	signed_in: boolean,
	account_username: string | null
): redeemed is CordJoinRedeemed =>
	redeemed !== null &&
	(!signed_in ||
		(account_username !== null &&
			account_username.toLowerCase() === redeemed.username.toLowerCase()));

/** Decide the join page's view. Pure, so every branch is table-tested. */
export const to_cord_join_view = (input: CordJoinViewInput): CordJoinView => {
	// the newcomer view stays put from redeem through signup and accept, while
	// the session changes under it
	if (input.newcomer_running && input.redeemed) {
		return { kind: 'newcomer', cord: input.redeemed.cord };
	}
	if (!input.token_loaded || !input.session_checked) return { kind: 'checking' };
	if (input.token === null || !is(CordInviteToken, input.token) || input.dead) {
		return { kind: 'dead' };
	}
	// a login succeeded and the account is still loading
	if (input.verified && !input.signed_in) return { kind: 'checking' };
	const identity = input.signed_in ? input.account_id : null;
	if (input.preview_for !== identity) return { kind: 'checking' };
	// a re-preview for the same identity keeps showing the last one while it
	// loads, and when it fails for a reason that says nothing about the link,
	// so a form on screen stays mounted with its input; `fetch_preview` drops
	// a preview for anyone else, so what's left is this identity's
	const preview = input.preview;
	if (input.preview_status === 'failure') {
		if (is_dead_link_error(input.preview_error_data)) return { kind: 'dead' };
		if (!preview) {
			return { kind: 'error', message: to_cord_error_message(input.preview_error_data) };
		}
	} else if (!preview || input.preview_status === 'initial') {
		return { kind: 'checking' };
	}
	if (input.signed_in) {
		if (preview.cord && preview.role !== null) return { kind: 'member', cord: preview.cord };
		if (preview.cord && preview.status === 'active') return { kind: 'join', cord: preview.cord };
	}
	// a revoked link admits nobody, a redemption included
	if (preview.status === 'revoked') return { kind: 'dead' };
	// a newcomer who redeemed here holds a use, so the link still answers them
	// when used up, and accept takes an expired one for a while, which the
	// preview can't know; a refusal then marks it `dead`
	if (cord_join_redemption_applies(input.redeemed, input.signed_in, input.account_username)) {
		return { kind: input.signed_in ? 'join' : 'newcomer', cord: input.redeemed.cord };
	}
	if (input.signed_in) return { kind: 'dead' };
	if (preview.newcomers_open && preview.cord) return { kind: 'newcomer', cord: preview.cord };
	if (preview.status === 'active') {
		return { kind: 'sign_in', newcomers_closed: input.newcomers_closed };
	}
	return { kind: 'dead' };
};

/**
 * The note to show beside a view decided from an earlier preview because
 * the latest re-preview failed for a reason that says nothing about the
 * link (a flaky network), or `null`. The view keeps its form and input; the
 * note says the link couldn't be checked again just now.
 *
 * @returns the failure's message, or `null` when the view is current
 */
export const to_cord_join_recheck_error = (input: CordJoinViewInput): string | null => {
	if (input.preview_status !== 'failure' || !input.preview) return null;
	if (is_dead_link_error(input.preview_error_data)) return null;
	const identity = input.signed_in ? input.account_id : null;
	if (input.preview_for !== identity) return null;
	return to_cord_error_message(input.preview_error_data);
};

/** The newcomer form's fields. */
export type CordNewcomerField = 'username' | 'password' | 'password_confirm';

/**
 * Check the newcomer form before anything is sent, as signup would.
 *
 * @returns the first field that's wrong and why, or `null` when all are fine
 */
export const check_cord_newcomer_fields = (
	username: string,
	password: string,
	password_confirm: string
): { field: CordNewcomerField; message: string } | null => {
	if (!is(Username, username.trim())) {
		return { field: 'username', message: CORD_USERNAME_RULE };
	}
	if (!parse(Password, password).ok) {
		return {
			field: 'password',
			message:
				password.length < PASSWORD_LENGTH_MIN
					? `A password is at least ${PASSWORD_LENGTH_MIN} characters.`
					: 'That password is too long.'
		};
	}
	if (password !== password_confirm) {
		return { field: 'password_confirm', message: "The passwords don't match." };
	}
	return null;
};

/**
 * How the newcomer flow ended:
 *
 * - `joined` — redeemed, signed up, signed in, and in the cord
 * - `invalid` — a field failed the local check; nothing was sent
 * - `username` — the server refused the username (taken, reserved, or against its rules)
 * - `dead` — the link can't be used
 * - `newcomers_closed` — the link can't bring someone new anymore, though it
 *   still works for existing accounts; the preview, fetched again, says so
 * - `busy` — the flow is already running; this call did nothing
 * - `redeem_failed` — redeem failed for another reason (the network, a rate limit)
 * - `signup_failed` — signup refused; its reason is on the auth state's `verify_error`
 * - `accept_failed` — signed up and signed in, but joining failed; retry with `accept_invite`
 */
export type CordNewcomerOutcome =
	| { kind: 'joined'; cord: CordJson }
	| { kind: 'invalid'; field: CordNewcomerField; message: string }
	| { kind: 'username'; message: string }
	| { kind: 'dead' }
	| { kind: 'newcomers_closed' }
	| { kind: 'busy' }
	| { kind: 'redeem_failed'; message: string }
	| { kind: 'signup_failed' }
	| { kind: 'accept_failed'; message: string };

/** Signs up and in with a username and password; `AuthState.signup` fits. */
export type CordJoinSignup = (username: string, password: string) => Promise<boolean>;

/** What `CordJoinState` needs: the API, and where to keep the link across reloads. */
export interface CordJoinStateOptions extends CordsStateOptions {
	/** This tab's `sessionStorage` in the page; `null` (the default) keeps the link in memory only. */
	storage?: CordJoinStorage | null;
}

/** The join page's link: its token, its preview, and redeeming and accepting it. */
export class CordJoinState {
	readonly #api: ThrowingApi<FrontendActionsApi>;
	readonly #storage: CordJoinStorage | null;

	readonly preview: AsyncSlot<CordInvitePreviewOutput> = new AsyncSlot({
		map_error: to_cord_error_message
	});
	readonly redeem: AsyncSlot<CordInviteRedeemOutput> = new AsyncSlot({
		map_error: to_cord_error_message
	});
	readonly accept: AsyncSlot<CordInviteAcceptOutput> = new AsyncSlot({
		map_error: to_cord_error_message
	});

	/** Whether the token has been read, from the fragment or storage. */
	token_loaded: boolean = $state.raw(false);
	/** The link's token, or `null` when there's none. */
	token: string | null = $state.raw(null);
	/** The account the preview was last requested for; see `CordJoinViewInput.preview_for`. */
	preview_for: string | null | undefined = $state.raw<string | null | undefined>(undefined);
	/** See `CordJoinViewInput.dead`. */
	dead: boolean = $state.raw(false);
	/** See `CordJoinViewInput.newcomer_running`. */
	newcomer_running: boolean = $state.raw(false);
	/**
	 * The newcomer's redemption of this link in this tab, kept in storage
	 * with the token so a reload mid-join can still sign up and accept.
	 */
	redeemed: CordJoinRedeemed | null = $state.raw(null);
	/** See `CordJoinViewInput.newcomers_closed`. */
	newcomers_closed: boolean = $state.raw(false);

	constructor(options: CordJoinStateOptions) {
		this.#api = options.api;
		this.#storage = options.storage ?? null;
	}

	/**
	 * Load the page's link from the URL fragment (`hash`), else from this
	 * tab's storage, restoring a stored redemption. With neither, as after
	 * the fragment is dropped while storage is blocked, the token already in
	 * memory stays.
	 */
	load(hash: string): void {
		const loaded = load_cord_join(hash, this.#storage);
		if (!loaded) {
			this.set_token(this.token);
			return;
		}
		this.set_token(loaded.token);
		if (loaded.redeemed) this.redeemed = loaded.redeemed;
	}

	/** Take the page's token, forgetting everything learned about another. */
	set_token(token: string | null): void {
		if (token !== this.token) {
			this.preview.reset();
			this.redeem.reset();
			this.accept.reset();
			this.preview_for = undefined;
			this.dead = false;
			this.redeemed = null;
			this.newcomers_closed = false;
		}
		this.token = token;
		this.token_loaded = true;
	}

	/**
	 * Forget the stored link and its redemption, once the link has done its
	 * job or can't do one for whoever is here. A redemption held for another
	 * username (someone else signed in on this tab) keeps the link stored,
	 * for the newcomer to come back to. What's in memory stays for this
	 * page's view.
	 *
	 * @param signed_in - whether an account is signed in
	 * @param account_username - the signed-in account's username, `null` when signed out
	 */
	forget(signed_in: boolean, account_username: string | null): void {
		if (
			this.redeemed !== null &&
			!cord_join_redemption_applies(this.redeemed, signed_in, account_username)
		) {
			return;
		}
		forget_cord_join(this.#storage);
	}

	/**
	 * On logout: forget the redemption, which belonged to whoever was signed
	 * in or signing up, and the stored link. The token stays in memory, so
	 * someone else can still sign in and use it on this page.
	 */
	forget_on_logout(): void {
		this.redeemed = null;
		this.newcomers_closed = false;
		forget_cord_join(this.#storage);
	}

	/**
	 * Preview the link for `account_id` (`null` when signed out). Does nothing
	 * without a usable token, or while the newcomer flow runs, which answers
	 * for itself. The page calls this from an effect, which reads
	 * `newcomer_running` here and so previews again when the flow ends.
	 *
	 * A preview for someone else is dropped first, so whatever the slot still
	 * holds while this one loads is this identity's, and the view keeps
	 * showing it (the newcomer form stays mounted through a re-preview).
	 */
	async fetch_preview(account_id: string | null): Promise<void> {
		const token = this.token;
		if (this.newcomer_running || token === null) return;
		if (!is(CordInviteToken, token)) return;
		// untracked, so the calling effect doesn't rerun on the write below
		if (untrack(() => this.preview_for) !== account_id) this.preview.reset();
		this.preview_for = account_id;
		await this.preview.run((signal) => this.#api.cord_invite_preview({ token }, { signal }));
	}

	/**
	 * Join the cord through the link, signed in.
	 *
	 * @returns the cord, or `undefined` when joining failed (the error is on
	 *   `accept`, and a dead link sets `dead`)
	 */
	async accept_invite(): Promise<CordJson | undefined> {
		const token = this.token;
		if (token === null) return undefined;
		const result = await this.accept.run(() => this.#api.cord_invite_accept({ token }));
		if (result === undefined) {
			if (is_dead_link_error(this.accept.error_data)) this.dead = true;
			return undefined;
		}
		return result.cord;
	}

	/**
	 * Bring someone new onto the instance and into the cord, back to back:
	 * check the fields, redeem the link for the username, sign up (which signs
	 * in), then accept. Running them together keeps short the window in which
	 * the reserved username waits for its signup.
	 *
	 * Retrying after a failed signup redeems the same username again, which
	 * the server answers without spending another use wherever accept would
	 * still take it: once the link is used up, expired within the accept
	 * grace, or closed to new usernames. The page's form holds the username
	 * read-only once reserved, so a retry never spends a second use on a new
	 * name; a `username` refusal of the reserved name (unavailable or
	 * invalid) means the reservation is gone, so it drops the redemption and
	 * the field opens again. A redeem refused as `cord_invite_newcomers_closed`
	 * drops the redemption and marks `newcomers_closed`: the link still works
	 * for existing accounts, and the preview (fetched again as the flow ends)
	 * shows the sign-in view.
	 */
	async join_as_newcomer(
		username: string,
		password: string,
		password_confirm: string,
		signup: CordJoinSignup
	): Promise<CordNewcomerOutcome> {
		if (this.newcomer_running) return { kind: 'busy' };
		const token = this.token;
		if (token === null) return { kind: 'dead' };
		const invalid = check_cord_newcomer_fields(username, password, password_confirm);
		if (invalid) return { kind: 'invalid', ...invalid };
		this.newcomer_running = true;
		let joined = false;
		try {
			const redeemed = await this.redeem.run(() =>
				this.#api.cord_invite_redeem({ token, username: username.trim() })
			);
			if (redeemed === undefined) {
				const e = this.redeem.error_data;
				const reason = rpc_error_reason(e);
				if (
					reason === ERROR_CORD_INVITE_USERNAME_UNAVAILABLE ||
					reason === ERROR_CORD_INVITE_USERNAME_INVALID
				) {
					// the server answers a waiting reservation before it checks the
					// username, so this one no longer holds; the form frees the field
					if (this.redeemed?.username === username.trim().toLowerCase()) {
						this.redeemed = null;
						save_cord_join(this.#storage, { token, redeemed: null });
					}
					return { kind: 'username', message: CORD_ERROR_MESSAGES[reason]! };
				}
				if (reason === ERROR_CORD_INVITE_NEWCOMERS_CLOSED) {
					this.redeemed = null;
					this.newcomers_closed = true;
					save_cord_join(this.#storage, { token, redeemed: null });
					return { kind: 'newcomers_closed' };
				}
				if (is_dead_link_error(e)) {
					this.dead = true;
					return { kind: 'dead' };
				}
				return { kind: 'redeem_failed', message: this.redeem.error ?? 'Something went wrong.' };
			}
			this.redeemed = { cord: redeemed.cord, username: redeemed.username };
			save_cord_join(this.#storage, { token, redeemed: this.redeemed });
			if (!(await signup(redeemed.username, password))) return { kind: 'signup_failed' };
			const cord = await this.accept_invite();
			if (!cord) {
				if (this.dead) return { kind: 'dead' };
				return { kind: 'accept_failed', message: this.accept.error ?? 'Something went wrong.' };
			}
			joined = true;
			return { kind: 'joined', cord };
		} finally {
			// a joined newcomer stays pinned while the page moves on to the cord
			if (!joined) this.newcomer_running = false;
		}
	}
}
