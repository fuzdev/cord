/**
 * Reading the cord server's JSON-RPC errors on the frontend: the `reason` a
 * refusal carries, and the words to show for it.
 *
 * The throwing RPC proxy throws an `Error` carrying the JSON-RPC `code` and
 * `data`; the cord verbs put their refusal's reason in `data.reason`. A rate
 * limit carries no reason, only its code and a `data.retry_after`.
 *
 * @module
 */

import { to_error_message } from '@fuzdev/fuz_util/error.ts';
import { USERNAME_LENGTH_MAX, USERNAME_LENGTH_MIN } from '@fuzdev/fuz_app/primitive_schemas.ts';
import { JSONRPC_ERROR_CODES } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';

import {
	ERROR_CORD_INVITE_EXHAUSTED,
	ERROR_CORD_INVITE_EXPIRED,
	ERROR_CORD_INVITE_FORBIDDEN,
	ERROR_CORD_INVITE_INVALID,
	ERROR_CORD_INVITE_NEWCOMER_FORBIDDEN,
	ERROR_CORD_INVITE_NEWCOMER_UNBOUNDED,
	ERROR_CORD_INVITE_NEWCOMERS_CLOSED,
	ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS,
	ERROR_CORD_INVITE_NOT_FOUND,
	ERROR_CORD_INVITE_REVOKED,
	ERROR_CORD_INVITE_USERNAME_INVALID,
	ERROR_CORD_INVITE_USERNAME_UNAVAILABLE,
	ERROR_CORD_LAST_OWNER,
	ERROR_CORD_MEMBER_FORBIDDEN,
	ERROR_CORD_MEMBER_NOT_FOUND,
	ERROR_CORD_MEMBER_REMOVE_SELF,
	ERROR_CORD_NOT_FOUND,
	ERROR_CORD_UPDATE_FORBIDDEN
} from '../cord_action_specs.ts';

/** The `data.reason` of a thrown RPC error, if it has one. */
export const rpc_error_reason = (e: unknown): string | undefined => {
	const reason = (e as { data?: { reason?: unknown } } | null)?.data?.reason;
	return typeof reason === 'string' ? reason : undefined;
};

/** Whether a thrown RPC error is the server's rate limit, which carries no reason. */
export const is_rpc_rate_limited = (e: unknown): boolean =>
	(e as { code?: unknown } | null)?.code === JSONRPC_ERROR_CODES.rate_limited;

/** The words for a rate limit, whichever action hit it. */
export const CORD_RATE_LIMITED_MESSAGE =
	'Too many tries for now. Wait a few minutes and try again.';

/** The instance's username rules, in words, for both the form's own check and the server's refusal. */
export const CORD_USERNAME_RULE = `A username is ${USERNAME_LENGTH_MIN} to ${USERNAME_LENGTH_MAX} characters: letters, numbers, dashes, and underscores, starting with a letter and ending with a letter or number.`;

/** The words for the cord refusals a person can run into, by reason. */
export const CORD_ERROR_MESSAGES: Partial<Record<string, string>> = {
	[ERROR_CORD_NOT_FOUND]: "There's no cord here, or you're not in it.",
	[ERROR_CORD_UPDATE_FORBIDDEN]: "Only the cord's owners and admins can change it.",
	[ERROR_CORD_LAST_OWNER]: 'A cord always keeps an owner. Make someone else an owner first.',
	[ERROR_CORD_MEMBER_FORBIDDEN]: "Your role here can't make that change.",
	[ERROR_CORD_MEMBER_NOT_FOUND]: "They're not in this cord anymore.",
	[ERROR_CORD_MEMBER_REMOVE_SELF]: 'To take yourself out, leave the cord instead.',
	[ERROR_CORD_INVITE_FORBIDDEN]: "Only the cord's owners and admins manage its links.",
	[ERROR_CORD_INVITE_INVALID]: 'That expiry is too long for a link that brings someone new.',
	[ERROR_CORD_INVITE_NEWCOMER_FORBIDDEN]:
		'Only instance admins make links that bring someone new onto the instance.',
	[ERROR_CORD_INVITE_NEWCOMER_UNBOUNDED]:
		'A link for someone new needs both an expiry and a use limit.',
	[ERROR_CORD_INVITE_NOT_FOUND]: "This link doesn't work anymore.",
	[ERROR_CORD_INVITE_REVOKED]: "This link doesn't work anymore.",
	[ERROR_CORD_INVITE_EXPIRED]: "This link doesn't work anymore.",
	[ERROR_CORD_INVITE_EXHAUSTED]: "This link doesn't work anymore.",
	[ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS]: "This link doesn't work anymore.",
	[ERROR_CORD_INVITE_NEWCOMERS_CLOSED]: "This link doesn't work anymore.",
	[ERROR_CORD_INVITE_USERNAME_INVALID]: CORD_USERNAME_RULE,
	[ERROR_CORD_INVITE_USERNAME_UNAVAILABLE]: "That username's taken. Try another."
};

/**
 * The words to show for a thrown RPC error: the cord copy for its reason
 * when there is some, the rate-limit copy for a rate limit, otherwise the
 * error's own message.
 */
export const to_cord_error_message = (e: unknown): string => {
	const reason = rpc_error_reason(e);
	if (reason && CORD_ERROR_MESSAGES[reason]) return CORD_ERROR_MESSAGES[reason];
	if (is_rpc_rate_limited(e)) return CORD_RATE_LIMITED_MESSAGE;
	return to_error_message(e, 'Something went wrong.');
};

/**
 * The words to show a cord's owner or admin for a failed revoke. A link the
 * server can't find is one that's already gone, which the join page's
 * "doesn't work anymore" would misdescribe to the person managing it.
 */
export const to_cord_invite_revoke_error_message = (e: unknown): string =>
	rpc_error_reason(e) === ERROR_CORD_INVITE_NOT_FOUND
		? "That link's already gone."
		: to_cord_error_message(e);
