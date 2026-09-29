/**
 * Reactive state for one cord's invite links: the list, minting a link, and
 * revoking one, plus the pure helpers the invite UI builds on (the expiry
 * choices, the mint form's checks, the join link's shape, and who minted a
 * link).
 *
 * The server returns a link's raw token only when it's minted, so the state
 * keeps that one link (`minted`) until the page dismisses it; the list never
 * carries tokens.
 *
 * Like `CordMembersState`, `fetch` is safe to call from an `$effect`, and
 * responses for a cord the state has since moved off are dropped.
 *
 * @module
 */

import { untrack } from 'svelte';
import type { ThrowingApi } from '@fuzdev/fuz_app/actions/rpc_client.ts';
import { AsyncSlot } from '@fuzdev/fuz_app/ui/async_slot.svelte.ts';
import { KeyedAsyncSlot } from '@fuzdev/fuz_app/ui/keyed_async_slot.svelte.ts';
import type { Uuid } from '@fuzdev/fuz_util/id.ts';
import type { Result } from '@fuzdev/fuz_util/result.ts';

import type { FrontendActionsApi } from '../action_metatypes.ts';
import {
	ERROR_CORD_INVITE_NOT_FOUND,
	CORD_INVITE_MAX_USES_MAX,
	CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX,
	type CordInviteCreateInput,
	type CordInviteCreateOutput,
	type CordInviteJson,
	type CordInviteRevokeOutput,
	type CordMemberJson
} from '../cord_action_specs.ts';
import { cord_role_can_manage } from '../cord_roles.ts';
import {
	rpc_error_reason,
	to_cord_error_message,
	to_cord_invite_revoke_error_message
} from './cord_errors.ts';
import type { CordsStateOptions } from './cords_state.svelte.ts';

const HOUR = 60 * 60;
const DAY = 24 * HOUR;

/** An expiry the mint form offers: a label and its seconds, `null` for never. */
export interface CordInviteExpiryChoice {
	label: string;
	seconds: number | null;
}

/** The expiries the mint form offers, shortest first, then never. */
export const CORD_INVITE_EXPIRY_CHOICES: ReadonlyArray<CordInviteExpiryChoice> = [
	{ label: '1 hour', seconds: HOUR },
	{ label: '1 day', seconds: DAY },
	{ label: '7 days', seconds: 7 * DAY },
	{ label: '30 days', seconds: 30 * DAY },
	{ label: 'never', seconds: null }
];

/** The mint form's starting expiry: 7 days. */
export const CORD_INVITE_EXPIRES_IN_SECONDS_DEFAULT = 7 * DAY;

/** The expiry choices open to a link, which for a newcomer link means bounded and at most 30 days. */
export const cord_invite_expiry_choices = (
	admits_newcomers: boolean
): ReadonlyArray<CordInviteExpiryChoice> =>
	admits_newcomers
		? CORD_INVITE_EXPIRY_CHOICES.filter(
				(c) => c.seconds !== null && c.seconds <= CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX
			)
		: CORD_INVITE_EXPIRY_CHOICES;

/** What the mint form holds. */
export interface CordInviteDraft {
	/** Seconds until the link expires, or `null` for never. */
	expires_in_seconds: number | null;
	/** How many people the link may bring in, or `null` (or an empty field) for no limit. */
	max_uses: number | null | undefined;
	/** Whether the link also lets someone new join the instance. */
	admits_newcomers: boolean;
}

/** A mint request, less the `acting` actor the client fills in. */
export type CordInviteCreateRequest = Omit<CordInviteCreateInput, 'acting'>;

/**
 * Check the mint form and build its request, as the server would judge it: a
 * use limit is a whole number from 1 to `CORD_INVITE_MAX_USES_MAX`, and a
 * newcomer link needs both an expiry of at most 30 days and a use limit.
 */
export const to_cord_invite_create_request = (
	cord_id: Uuid,
	draft: CordInviteDraft
): Result<{ value: CordInviteCreateRequest }, { message: string }> => {
	const { expires_in_seconds, max_uses, admits_newcomers } = draft;
	const has_max_uses = max_uses !== null && max_uses !== undefined;
	if (has_max_uses && !(Number.isInteger(max_uses) && max_uses >= 1)) {
		return { ok: false, message: 'The use limit is a whole number, at least 1.' };
	}
	if (has_max_uses && max_uses > CORD_INVITE_MAX_USES_MAX) {
		return { ok: false, message: `The use limit is at most ${CORD_INVITE_MAX_USES_MAX}.` };
	}
	if (admits_newcomers) {
		if (expires_in_seconds === null) {
			return { ok: false, message: 'A link for someone new needs an expiry.' };
		}
		if (expires_in_seconds > CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX) {
			return { ok: false, message: 'A link for someone new expires within 30 days.' };
		}
		if (!has_max_uses) {
			return { ok: false, message: 'A link for someone new needs a use limit.' };
		}
	}
	const value: CordInviteCreateRequest = { cord_id };
	if (expires_in_seconds !== null) value.expires_in_seconds = expires_in_seconds;
	if (has_max_uses) value.max_uses = max_uses;
	if (admits_newcomers) value.admits_newcomers = true;
	return { ok: true, value };
};

/**
 * The join link for `token`, given the join page's absolute URL (the
 * instance's origin plus its `/join` path). The token rides in the
 * fragment, which browsers never send to a server, so it stays out of
 * server logs and `Referer` headers.
 */
export const to_cord_join_url = (join_page_url: string, token: string): string =>
	`${join_page_url.split('#')[0]}#${token}`;

/**
 * Who minted a link, judged against the cord's current members:
 *
 * - `manager` — still one of its owners or admins
 * - `former_manager` — still in the cord, but no longer an owner or admin
 * - `former_member` — no longer in the cord
 * - `gone` — no longer on the instance (the server forgot the minter)
 */
export type CordInviteMinter =
	| { kind: 'manager'; member: CordMemberJson }
	| { kind: 'former_manager'; member: CordMemberJson }
	| { kind: 'former_member' }
	| { kind: 'gone' };

/** Who minted `invite`, from the cord's member list. */
export const to_cord_invite_minter = (
	invite: CordInviteJson,
	members: ReadonlyArray<CordMemberJson>
): CordInviteMinter => {
	if (invite.created_by === null) return { kind: 'gone' };
	const member = members.find((m) => m.actor_id === invite.created_by);
	if (!member) return { kind: 'former_member' };
	return cord_role_can_manage(member.role)
		? { kind: 'manager', member }
		: { kind: 'former_manager', member };
};

/** A link just minted, with the token the server returns only then. */
export interface CordInviteMinted {
	invite: CordInviteJson;
	token: string;
}

/** One cord's invite links, and minting and revoking them. */
export class CordInvitesState {
	readonly #api: ThrowingApi<FrontendActionsApi>;

	readonly list: AsyncSlot = new AsyncSlot({ map_error: to_cord_error_message });
	readonly create: AsyncSlot<CordInviteCreateOutput> = new AsyncSlot({
		map_error: to_cord_error_message
	});
	/** Revokes, by the link's id. */
	readonly revoke: KeyedAsyncSlot<Uuid, CordInviteRevokeOutput> = new KeyedAsyncSlot({
		map_error: to_cord_invite_revoke_error_message
	});

	/** The cord whose links are loaded, or `null` before the first fetch. */
	cord_id: Uuid | null = $state.raw(null);
	/** The cord's links, newest first. */
	invites: Array<CordInviteJson> = $state.raw([]);
	/** The link minted last, with its token, until `dismiss_minted`. */
	minted: CordInviteMinted | null = $state.raw(null);

	constructor(options: CordsStateOptions) {
		this.#api = options.api;
	}

	/** Load the links of `cord_id`, forgetting another cord's. */
	async fetch(cord_id: Uuid): Promise<void> {
		// untracked: an effect calling this must not depend on the `cord_id` it writes
		untrack(() => {
			if (this.cord_id !== cord_id) {
				this.invites = [];
				this.minted = null;
				this.create.reset();
				this.revoke.reset();
			}
			this.cord_id = cord_id;
		});
		await this.list.run(async (signal) => {
			const { invites } = await this.#api.cord_invite_list({ cord_id }, { signal });
			// a superseded fetch (another cord's, or an older one) keeps out
			if (signal.aborted || this.cord_id !== cord_id) return;
			this.invites = invites;
		});
	}

	/**
	 * Mint a link, keep it with its token in `minted`, and reload the list.
	 *
	 * @returns whether minting succeeded (the error is on `create`)
	 */
	async create_invite(request: CordInviteCreateRequest): Promise<boolean> {
		const result = await this.create.run(() => this.#api.cord_invite_create(request));
		if (result === undefined) return false;
		// the page moved to another cord meanwhile; that cord's section shows none of this
		if (this.cord_id !== request.cord_id) return true;
		this.minted = { invite: result.invite, token: result.token };
		await this.fetch(request.cord_id);
		return true;
	}

	/** Forget the minted link's token, once its link has been copied. */
	dismiss_minted(): void {
		this.minted = null;
	}

	/**
	 * Revoke a link, updating it in the list. A link the server no longer
	 * has reloads the list.
	 *
	 * @returns whether the revoke succeeded (the error is on `revoke`)
	 */
	async revoke_invite(invite_id: Uuid): Promise<boolean> {
		const cord_id = this.cord_id;
		if (!cord_id) return false;
		const result = await this.revoke.run(invite_id, () =>
			this.#api.cord_invite_revoke({ cord_id, invite_id })
		);
		if (this.cord_id !== cord_id) return result !== undefined;
		if (result === undefined) {
			if (
				rpc_error_reason(this.revoke.get(invite_id)?.error_data) === ERROR_CORD_INVITE_NOT_FOUND
			) {
				await this.fetch(cord_id);
			}
			return false;
		}
		this.invites = this.invites.map((i) => (i.id === invite_id ? result.invite : i));
		return true;
	}
}
