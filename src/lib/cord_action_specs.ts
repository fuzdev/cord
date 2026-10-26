/**
 * Action specs for the cord actions `cordd` defines: creating, listing,
 * reading, and renaming cords, the instance admins' ops list, a cord's invite
 * links (including newcomer links and their redemption), and its members.
 *
 * The server's handlers are in `crates/cord_server/src/cord_actions.rs`,
 * `invite/actions.rs`, and `cord_member_actions.rs`; these specs are
 * their wire contract, which the frontend RPC client and the generated action
 * types read. Authorization beyond the spec-level gates (cord membership and
 * role, the global admin grant) is the server's.
 *
 * @module
 */

import { z } from 'zod';
import { Username, Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';
import { ActingActor } from '@fuzdev/fuz_app/http/auth_shape.ts';
import { ROLE_ADMIN } from '@fuzdev/fuz_app/auth/role_schema.ts';
import type {
	ActionSpecUnion,
	RequestResponseActionSpec
} from '@fuzdev/fuz_app/actions/action_spec.ts';

import { CordRole } from './cord_roles.ts';

/** Error reason: no such cord, or the caller isn't in it. */
export const ERROR_CORD_NOT_FOUND = 'cord_not_found';
/** Error reason: only instance admins create cords. */
export const ERROR_CORD_CREATE_FORBIDDEN = 'cord_create_forbidden';
/** Error reason: only the cord's owner or admins may change it. */
export const ERROR_CORD_UPDATE_FORBIDDEN = 'cord_update_forbidden';
/** Error reason: the cord name is empty, too long, or has a forbidden or misplaced character. */
export const ERROR_CORD_NAME_INVALID = 'cord_name_invalid';
/** Error reason: no invite link has that token (or id, on this cord). */
export const ERROR_CORD_INVITE_NOT_FOUND = 'cord_invite_not_found';
/** Error reason: the invite link was revoked. */
export const ERROR_CORD_INVITE_REVOKED = 'cord_invite_revoked';
/** Error reason: the invite link has expired. */
export const ERROR_CORD_INVITE_EXPIRED = 'cord_invite_expired';
/** Error reason: the invite link has no uses left. */
export const ERROR_CORD_INVITE_EXHAUSTED = 'cord_invite_exhausted';
/** Error reason: only the cord's owner or admins manage its invite links. */
export const ERROR_CORD_INVITE_FORBIDDEN = 'cord_invite_forbidden';
/**
 * Error reason: a link that admits newcomers has an expiry past
 * `CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX`. The bounds every link keeps
 * are the schema's, refused at params validation with no reason.
 */
export const ERROR_CORD_INVITE_INVALID = 'cord_invite_invalid';
/** Error reason: only instance admins mint links that admit newcomers. */
export const ERROR_CORD_INVITE_NEWCOMER_FORBIDDEN = 'cord_invite_newcomer_forbidden';
/** Error reason: a link that admits newcomers needs both an expiry and a use limit. */
export const ERROR_CORD_INVITE_NEWCOMER_UNBOUNDED = 'cord_invite_newcomer_unbounded';
/** Error reason: the link is for people who already have an account. */
export const ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS = 'cord_invite_not_for_newcomers';
/** Error reason: the link's minter is no longer an instance admin, so it brings no one new onto the instance. */
export const ERROR_CORD_INVITE_NEWCOMERS_CLOSED = 'cord_invite_newcomers_closed';
/** Error reason: the username breaks the instance's username rules. */
export const ERROR_CORD_INVITE_USERNAME_INVALID = 'cord_invite_username_invalid';
/** Error reason: an account has the username, or an invite reserves it. */
export const ERROR_CORD_INVITE_USERNAME_UNAVAILABLE = 'cord_invite_username_unavailable';
/** Error reason: the target isn't a member of the cord. */
export const ERROR_CORD_MEMBER_NOT_FOUND = 'cord_member_not_found';
/** Error reason: the caller's cord role can't make this change. */
export const ERROR_CORD_MEMBER_FORBIDDEN = 'cord_member_forbidden';
/** Error reason: the change would leave the cord with no owner. */
export const ERROR_CORD_LAST_OWNER = 'cord_last_owner';
/** Error reason: `cord_member_remove` named the caller; a member takes themself out with `cord_leave`. */
export const ERROR_CORD_MEMBER_REMOVE_SELF = 'cord_member_remove_self';

/** The shortest expiry an invite link may be given, in seconds; the twin of the server's constant. */
export const CORD_INVITE_EXPIRES_IN_SECONDS_MIN = 60;
/** The longest expiry an invite link may be given, in seconds (365 days). */
export const CORD_INVITE_EXPIRES_IN_SECONDS_MAX = 365 * 24 * 60 * 60;
/** The longest expiry a link that admits newcomers may be given, in seconds (30 days); the twin of the server's constant. */
export const CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX = 30 * 24 * 60 * 60;
/**
 * How long after a newcomer link expires a newcomer who redeemed it before
 * then may still accept it, in seconds (one hour); the twin of the server's
 * constant.
 */
export const CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS = 60 * 60;
/** The most uses an invite link may be given. */
export const CORD_INVITE_MAX_USES_MAX = 1000;
/** The longest invite token the server looks up; the twin of the server's constant. */
export const CORD_INVITE_TOKEN_LENGTH_MAX = 128;
/**
 * The most `cord_invite_redeem` calls one IP may make per
 * `CORD_INVITE_REDEEM_RATE_LIMIT_WINDOW_MS`, on top of the IP budget the
 * public actions share; the twin of the server's constant.
 */
export const CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS = 10;
/** The window of `CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS`, in milliseconds (15 minutes); the twin of the server's constant. */
export const CORD_INVITE_REDEEM_RATE_LIMIT_WINDOW_MS = 15 * 60_000;

/** The longest cord name, in characters; the twin of the server's constant. */
export const CORD_NAME_LENGTH_MAX = 100;

/**
 * Characters refused anywhere in a cord name beside the control characters,
 * as inclusive code point ranges.
 *
 * The formula: (Cf ∪ `Default_Ignorable_Code_Point` ∪ {U+2028, U+2029}) −
 * {U+200C, U+200D} − [U+FE00, U+FE0F], as of Unicode 17, with the control
 * characters (Cc) refused separately. That is every format character, every
 * character Unicode says to render invisibly when unsupported (zero-width
 * spaces, bidi marks, overrides and isolates, fillers, tags, the byte order
 * mark, unassigned default-ignorables), and the line and paragraph
 * separators, except:
 *
 * - the join characters U+200C (ZWNJ) and U+200D (ZWJ), which Persian
 *   spelling, Indic conjuncts, and joined emoji need, allowed under the
 *   placement rule (`has_stray_cord_name_joiner`)
 * - the variation selectors U+FE00–FE0F, which emoji need
 *
 * The tag characters stay refused, at the cost of the three subdivision
 * flags (England, Scotland, Wales).
 *
 * Explicit rather than property escapes so it can't drift with the JS
 * engine's Unicode version. It must stay identical to
 * `CORD_NAME_FORBIDDEN_CHAR_RANGES` in the server's `cord_queries.rs`; a unit
 * test checks that, and checks the list against the formula in both
 * directions.
 */
export const CORD_NAME_FORBIDDEN_CHAR_RANGES: ReadonlyArray<readonly [number, number]> = [
	[0x00ad, 0x00ad], // soft hyphen
	[0x034f, 0x034f], // combining grapheme joiner
	[0x0600, 0x0605], // Arabic number signs (Cf)
	[0x061c, 0x061c], // Arabic letter mark
	[0x06dd, 0x06dd], // Arabic end of ayah (Cf)
	[0x070f, 0x070f], // Syriac abbreviation mark (Cf)
	[0x0890, 0x0891], // Arabic pound and piastre marks (Cf)
	[0x08e2, 0x08e2], // Arabic disputed end of ayah (Cf)
	[0x115f, 0x1160], // Hangul choseong and jungseong fillers
	[0x17b4, 0x17b5], // Khmer inherent vowels
	[0x180b, 0x180f], // Mongolian variation selectors and vowel separator
	[0x200b, 0x200b], // zero-width space
	[0x200e, 0x200f], // left-to-right and right-to-left marks
	[0x2028, 0x2029], // line and paragraph separators (Zl, Zp)
	[0x202a, 0x202e], // bidirectional embeddings and overrides
	[0x2060, 0x206f], // word joiner, invisible operators, bidi isolates, deprecated formats
	[0x3164, 0x3164], // Hangul filler
	[0xfeff, 0xfeff], // byte order mark
	[0xffa0, 0xffa0], // halfwidth Hangul filler
	[0xfff0, 0xfff8], // unassigned, default-ignorable
	[0xfff9, 0xfffb], // interlinear annotation marks
	[0x110bd, 0x110bd], // Kaithi number sign (Cf)
	[0x110cd, 0x110cd], // Kaithi number sign above (Cf)
	[0x13430, 0x1343f], // Egyptian hieroglyph format controls (Cf)
	[0x1bca0, 0x1bca3], // shorthand format controls
	[0x1d173, 0x1d17a], // musical symbol format controls
	[0xe0000, 0xe0fff] // tags and supplementary variation selectors
];

/** The two join characters: U+200C (zero-width non-joiner) and U+200D (zero-width joiner). */
export const CORD_NAME_JOIN_CHARS: ReadonlyArray<string> = ['\u200C', '\u200D'];

/**
 * Whether a character may never appear in a cord name: a control character
 * (Cc, which covers `\n`, `\t`, and U+0085), one of
 * `CORD_NAME_FORBIDDEN_CHAR_RANGES`, or a lone surrogate. The twin of the server's
 * `is_forbidden_cord_name_char`.
 */
export const is_forbidden_cord_name_char = (c: string): boolean => {
	const code = c.codePointAt(0);
	if (code === undefined) return false;
	// Cc: C0 controls, DEL, and C1 controls, as Rust's `char::is_control`
	if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) return true;
	// a lone surrogate: JS strings can hold one, but it isn't a Unicode scalar
	// value, so the server's JSON parser refuses it and a Rust `String` can
	// never contain one (no server-side twin of this check is needed)
	if (code >= 0xd800 && code <= 0xdfff) return true;
	return CORD_NAME_FORBIDDEN_CHAR_RANGES.some(([start, end]) => code >= start && code <= end);
};

/**
 * Whether a join character in `chars` (code points) sits where it only pads:
 * at either end, or beside whitespace or another join character. The twin of
 * the server's `has_stray_cord_name_joiner`; `\s` differs from Rust's
 * `char::is_whitespace` only on U+FEFF and U+0085, both refused before this
 * runs.
 */
export const has_stray_cord_name_joiner = (chars: ReadonlyArray<string>): boolean => {
	const is_join = (c: string): boolean => CORD_NAME_JOIN_CHARS.includes(c);
	const pads = (c: string | undefined): boolean => c === undefined || /\s/u.test(c) || is_join(c);
	return chars.some((c, i) => is_join(c) && (pads(chars[i - 1]) || pads(chars[i + 1])));
};

/**
 * A cord's name, validated as the server does and in the same order: no
 * forbidden character anywhere (checked before trimming, so the twins agree
 * even where JS and Rust trim different characters: JS strips U+FEFF and
 * Rust strips U+0085, and both are forbidden), then trimmed and non-empty,
 * then no stray join character, then at most `CORD_NAME_LENGTH_MAX`
 * characters counted by code point.
 */
export const CordName = z
	.string()
	.refine((s) => !Array.from(s).some(is_forbidden_cord_name_char), {
		message: 'name must not contain invisible, control, or line-breaking characters'
	})
	.trim()
	.refine((s) => s.length > 0, { message: 'name is required' })
	.refine((s) => !has_stray_cord_name_joiner(Array.from(s)), {
		message: 'name must not have a joining character at an edge, by a space, or doubled'
	})
	.refine((s) => Array.from(s).length <= CORD_NAME_LENGTH_MAX, {
		message: `name must be at most ${CORD_NAME_LENGTH_MAX} characters`
	});
export type CordName = z.infer<typeof CordName>;

/** A cord as one of its members sees it, with the caller's role in it. */
export const CordJson = z.strictObject({
	id: Uuid,
	name: z.string(),
	role: CordRole,
	created_at: z.string()
});
export type CordJson = z.infer<typeof CordJson>;

/** A cord in the instance admins' ops list. */
export const CordAdminJson = z.strictObject({
	id: Uuid,
	name: z.string(),
	member_count: z.number().int(),
	created_at: z.string()
});
export type CordAdminJson = z.infer<typeof CordAdminJson>;

/**
 * An invite link's state: usable, or why not. A revoked link reports
 * `revoked` even once expired, and an expired one `expired` even once used
 * up.
 */
export const CordInviteStatus = z.enum(['active', 'revoked', 'expired', 'exhausted']);
export type CordInviteStatus = z.infer<typeof CordInviteStatus>;

/**
 * An invite link as its cord's owner and admins see it. Never carries the
 * token, which is shown once, when the link is created.
 */
export const CordInviteJson = z.strictObject({
	id: Uuid,
	cord_id: Uuid,
	role: CordRole,
	/** Whether the link can bring someone with no account onto the instance. */
	admits_newcomers: z.boolean(),
	created_by: Uuid.nullable(),
	created_at: z.string(),
	expires_at: z.string().nullable(),
	max_uses: z.number().int().nullable(),
	use_count: z.number().int(),
	revoked_at: z.string().nullable(),
	status: CordInviteStatus,
	/** How many times the link was redeemed for a newcomer; each spent a use. */
	redemption_count: z.number().int(),
	/** How many of those newcomers have since accepted the link. */
	redemption_accepted_count: z.number().int()
});
export type CordInviteJson = z.infer<typeof CordInviteJson>;

/** A member of a cord as its members see them. */
export const CordMemberJson = z.strictObject({
	actor_id: Uuid,
	username: z.string(),
	/** The actor's name, or `null` when it's blank. */
	display_name: z.string().nullable(),
	role: CordRole
});
export type CordMemberJson = z.infer<typeof CordMemberJson>;

/**
 * An invite link's token, as it appears in the link.
 *
 * The Rust server refuses a token out of these bounds at params validation
 * too, counting UTF-16 code units as this does, with the same reasonless
 * `invalid params`; a token in bounds that it never minted gets
 * `cord_invite_not_found`.
 */
export const CordInviteToken = z
	.string()
	.min(1)
	.max(CORD_INVITE_TOKEN_LENGTH_MAX)
	.meta({ description: "The invite link's token." });
export type CordInviteToken = z.infer<typeof CordInviteToken>;

export const CordCreateInput = z.strictObject({
	name: CordName.meta({ description: "The new cord's name." }),
	acting: ActingActor
});
export type CordCreateInput = z.infer<typeof CordCreateInput>;

export const CordCreateOutput = z.strictObject({ cord: CordJson });
export type CordCreateOutput = z.infer<typeof CordCreateOutput>;

export const CordListInput = z.strictObject({ acting: ActingActor });
export type CordListInput = z.infer<typeof CordListInput>;

export const CordListOutput = z.strictObject({ cords: z.array(CordJson) });
export type CordListOutput = z.infer<typeof CordListOutput>;

export const CordGetInput = z.strictObject({
	cord_id: Uuid.meta({ description: 'The cord to read.' }),
	acting: ActingActor
});
export type CordGetInput = z.infer<typeof CordGetInput>;

export const CordGetOutput = z.strictObject({ cord: CordJson });
export type CordGetOutput = z.infer<typeof CordGetOutput>;

export const CordUpdateInput = z.strictObject({
	cord_id: Uuid.meta({ description: 'The cord to change.' }),
	name: CordName.meta({ description: "The cord's new name." }),
	acting: ActingActor
});
export type CordUpdateInput = z.infer<typeof CordUpdateInput>;

export const CordUpdateOutput = z.strictObject({ cord: CordJson });
export type CordUpdateOutput = z.infer<typeof CordUpdateOutput>;

export const CordAdminListInput = z.strictObject({ acting: ActingActor });
export type CordAdminListInput = z.infer<typeof CordAdminListInput>;

export const CordAdminListOutput = z.strictObject({ cords: z.array(CordAdminJson) });
export type CordAdminListOutput = z.infer<typeof CordAdminListOutput>;

export const CordInviteCreateInput = z.strictObject({
	cord_id: Uuid.meta({ description: 'The cord the link invites to.' }),
	expires_in_seconds: z
		.number()
		.int()
		.min(CORD_INVITE_EXPIRES_IN_SECONDS_MIN)
		.max(CORD_INVITE_EXPIRES_IN_SECONDS_MAX)
		.optional()
		.meta({ description: 'Seconds until the link expires; absent, it never does.' }),
	max_uses: z
		.number()
		.int()
		.min(1)
		.max(CORD_INVITE_MAX_USES_MAX)
		.optional()
		.meta({ description: 'How many people the link may bring in; absent, no limit.' }),
	admits_newcomers: z.boolean().optional().meta({
		description:
			'Whether the link can bring someone with no account onto the instance. Instance admins only, and the link needs both a use limit and an expiry, of at most `CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX`.'
	}),
	acting: ActingActor
});
export type CordInviteCreateInput = z.infer<typeof CordInviteCreateInput>;

export const CordInviteCreateOutput = z.strictObject({
	invite: CordInviteJson,
	/** The raw token, returned only here. */
	token: z.string()
});
export type CordInviteCreateOutput = z.infer<typeof CordInviteCreateOutput>;

export const CordInviteListInput = z.strictObject({
	cord_id: Uuid.meta({ description: 'The cord whose links to list.' }),
	acting: ActingActor
});
export type CordInviteListInput = z.infer<typeof CordInviteListInput>;

export const CordInviteListOutput = z.strictObject({ invites: z.array(CordInviteJson) });
export type CordInviteListOutput = z.infer<typeof CordInviteListOutput>;

export const CordInviteRevokeInput = z.strictObject({
	cord_id: Uuid.meta({ description: "The link's cord." }),
	invite_id: Uuid.meta({ description: 'The link to revoke.' }),
	acting: ActingActor
});
export type CordInviteRevokeInput = z.infer<typeof CordInviteRevokeInput>;

export const CordInviteRevokeOutput = z.strictObject({ invite: CordInviteJson });
export type CordInviteRevokeOutput = z.infer<typeof CordInviteRevokeOutput>;

export const CordInvitePreviewInput = z.strictObject({
	token: CordInviteToken,
	acting: ActingActor
});
export type CordInvitePreviewInput = z.infer<typeof CordInvitePreviewInput>;

/** A cord as a link's preview and redemption show it. */
export const CordInviteCordJson = z.strictObject({ id: Uuid, name: z.string() });
export type CordInviteCordJson = z.infer<typeof CordInviteCordJson>;

export const CordInvitePreviewOutput = z.strictObject({
	status: CordInviteStatus,
	/** Whether the link can bring someone with no account onto the instance. */
	admits_newcomers: z.boolean(),
	/**
	 * Whether the link can be redeemed now: it's active, admits newcomers,
	 * and its minter is still an instance admin.
	 */
	newcomers_open: z.boolean(),
	/**
	 * The cord: for a signed-in caller, when the link is active or they're
	 * already in it; for an anonymous one, only when `newcomers_open`.
	 */
	cord: CordInviteCordJson.nullable(),
	/** The caller's role in the cord, or `null` when they aren't in it or aren't signed in. */
	role: CordRole.nullable()
});
export type CordInvitePreviewOutput = z.infer<typeof CordInvitePreviewOutput>;

/** No `acting`: redeem is public and never reads the caller. */
export const CordInviteRedeemInput = z.strictObject({
	token: CordInviteToken,
	username: Username.meta({
		description: 'The username to reserve for the newcomer, who then signs up with it.'
	})
});
export type CordInviteRedeemInput = z.infer<typeof CordInviteRedeemInput>;

export const CordInviteRedeemOutput = z.strictObject({
	cord: CordInviteCordJson,
	/** The reserved username, lowercased as the instance stores it. */
	username: z.string(),
	/** `false` when this username's redemption through the link was already waiting, so nothing changed and no use was spent. */
	created: z.boolean()
});
export type CordInviteRedeemOutput = z.infer<typeof CordInviteRedeemOutput>;

export const CordInviteAcceptInput = z.strictObject({
	token: CordInviteToken,
	acting: ActingActor
});
export type CordInviteAcceptInput = z.infer<typeof CordInviteAcceptInput>;

export const CordInviteAcceptOutput = z.strictObject({
	cord: CordJson,
	/** `false` when the caller was already in the cord, so nothing changed. */
	joined: z.boolean()
});
export type CordInviteAcceptOutput = z.infer<typeof CordInviteAcceptOutput>;

export const CordMemberListInput = z.strictObject({
	cord_id: Uuid.meta({ description: 'The cord whose members to list.' }),
	acting: ActingActor
});
export type CordMemberListInput = z.infer<typeof CordMemberListInput>;

export const CordMemberListOutput = z.strictObject({ members: z.array(CordMemberJson) });
export type CordMemberListOutput = z.infer<typeof CordMemberListOutput>;

export const CordMemberRoleSetInput = z.strictObject({
	cord_id: Uuid.meta({ description: 'The cord.' }),
	actor_id: Uuid.meta({ description: 'The member whose role to set.' }),
	role: CordRole.meta({ description: 'Their new role.' }),
	acting: ActingActor
});
export type CordMemberRoleSetInput = z.infer<typeof CordMemberRoleSetInput>;

export const CordMemberRoleSetOutput = z.strictObject({ member: CordMemberJson });
export type CordMemberRoleSetOutput = z.infer<typeof CordMemberRoleSetOutput>;

export const CordMemberRemoveInput = z.strictObject({
	cord_id: Uuid.meta({ description: 'The cord.' }),
	actor_id: Uuid.meta({ description: 'The member to remove.' }),
	acting: ActingActor
});
export type CordMemberRemoveInput = z.infer<typeof CordMemberRemoveInput>;

export const CordMemberRemoveOutput = z.strictObject({ ok: z.literal(true) });
export type CordMemberRemoveOutput = z.infer<typeof CordMemberRemoveOutput>;

export const CordLeaveInput = z.strictObject({
	cord_id: Uuid.meta({ description: 'The cord to leave.' }),
	acting: ActingActor
});
export type CordLeaveInput = z.infer<typeof CordLeaveInput>;

export const CordLeaveOutput = z.strictObject({ ok: z.literal(true) });
export type CordLeaveOutput = z.infer<typeof CordLeaveOutput>;

export const cord_create_action_spec = {
	method: 'cord_create',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required', roles: [ROLE_ADMIN] },
	side_effects: true,
	input: CordCreateInput,
	output: CordCreateOutput,
	async: true,
	description:
		'Create a cord. Instance admins only (a global `admin` grant). The caller becomes its `cord_owner`.'
} satisfies RequestResponseActionSpec;

export const cord_list_action_spec = {
	method: 'cord_list',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required' },
	side_effects: false,
	input: CordListInput,
	output: CordListOutput,
	async: true,
	description: "The caller's cords, with their role in each."
} satisfies RequestResponseActionSpec;

export const cord_get_action_spec = {
	method: 'cord_get',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required' },
	side_effects: false,
	input: CordGetInput,
	output: CordGetOutput,
	async: true,
	description:
		'Read one cord. Members only; anyone else gets `cord_not_found`, as for a cord that does not exist.'
} satisfies RequestResponseActionSpec;

export const cord_update_action_spec = {
	method: 'cord_update',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required' },
	side_effects: true,
	input: CordUpdateInput,
	output: CordUpdateOutput,
	async: true,
	description: "Rename a cord. The cord's owner or admins only."
} satisfies RequestResponseActionSpec;

export const cord_admin_list_action_spec = {
	method: 'cord_admin_list',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required', roles: [ROLE_ADMIN] },
	side_effects: false,
	input: CordAdminListInput,
	output: CordAdminListOutput,
	async: true,
	description:
		'Every cord on the instance with its member count, for instance admins (a global `admin` grant).'
} satisfies RequestResponseActionSpec;

export const cord_invite_create_action_spec = {
	method: 'cord_invite_create',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required' },
	side_effects: true,
	input: CordInviteCreateInput,
	output: CordInviteCreateOutput,
	async: true,
	rate_limit: 'account',
	description: `Create an invite link to a cord, with an optional expiry and use limit. The cord's owner or admins only; a link that admits newcomers also needs an instance admin (a global \`admin\` grant) and both limits, with an expiry of at most ${CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX / (24 * 60 * 60)} days. Returns the link's token, which is never shown again.`
} satisfies RequestResponseActionSpec;

export const cord_invite_list_action_spec = {
	method: 'cord_invite_list',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required' },
	side_effects: false,
	input: CordInviteListInput,
	output: CordInviteListOutput,
	async: true,
	description:
		"A cord's invite links with their status and redemption counts, newest first. The cord's owner or admins only."
} satisfies RequestResponseActionSpec;

export const cord_invite_revoke_action_spec = {
	method: 'cord_invite_revoke',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required' },
	side_effects: true,
	input: CordInviteRevokeInput,
	output: CordInviteRevokeOutput,
	async: true,
	description:
		"Revoke an invite link. The cord's owner or admins only. Revoking a revoked link succeeds and changes nothing."
} satisfies RequestResponseActionSpec;

export const cord_invite_preview_action_spec = {
	method: 'cord_invite_preview',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'optional', actor: 'optional' },
	side_effects: false,
	input: CordInvitePreviewInput,
	output: CordInvitePreviewOutput,
	async: true,
	rate_limit: 'ip',
	description:
		"An invite link's status, whether it admits newcomers, and whether it can be redeemed now, signed in or not. Names its cord to a signed-in caller when the link is active or they're already in the cord, and to an anonymous caller only when the link can be redeemed now."
} satisfies RequestResponseActionSpec;

export const cord_invite_redeem_action_spec = {
	method: 'cord_invite_redeem',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'none', actor: 'none' },
	side_effects: true,
	input: CordInviteRedeemInput,
	output: CordInviteRedeemOutput,
	async: true,
	rate_limit: 'ip',
	description: `Reserve a username on the instance through a newcomer link, spending one of its uses; the newcomer then signs up with that username and accepts the link. Needs no session, and ignores one. Redeeming the same username through the same link again, before its signup, changes nothing and answers \`created: false\` wherever accept would still take it: once the link is used up, expired within the accept grace, or closed because its minter is no longer an instance admin. A new username needs an active link whose minter is still an instance admin. A dead link (revoked, expired past the grace, or not for newcomers) refuses a waiting username as it would any other. Besides the IP budget the public actions share, each IP gets ${CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS} redeems per ${CORD_INVITE_REDEEM_RATE_LIMIT_WINDOW_MS / 60_000} minutes.`
} satisfies RequestResponseActionSpec;

export const cord_invite_accept_action_spec = {
	method: 'cord_invite_accept',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required', credential_types: ['session'] },
	side_effects: true,
	input: CordInviteAcceptInput,
	output: CordInviteAcceptOutput,
	async: true,
	description: `Join a cord through an invite link, as a member. A cookie session only. Someone already in the cord keeps their role and spends no use of the link, and so does a newcomer who redeemed the link before signing up, whose use was spent then; they may accept until ${CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS / 60} minutes after the link expires.`
} satisfies RequestResponseActionSpec;

export const cord_member_list_action_spec = {
	method: 'cord_member_list',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required' },
	side_effects: false,
	input: CordMemberListInput,
	output: CordMemberListOutput,
	async: true,
	description: "A cord's members with their roles, owners first. Members only."
} satisfies RequestResponseActionSpec;

export const cord_member_role_set_action_spec = {
	method: 'cord_member_role_set',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required' },
	side_effects: true,
	input: CordMemberRoleSetInput,
	output: CordMemberRoleSetOutput,
	async: true,
	description:
		"Set a member's role in a cord. The cord's owners only; a cord keeps at least one owner."
} satisfies RequestResponseActionSpec;

export const cord_member_remove_action_spec = {
	method: 'cord_member_remove',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required' },
	side_effects: true,
	input: CordMemberRemoveInput,
	output: CordMemberRemoveOutput,
	async: true,
	description:
		'Remove another member from a cord. An owner removes anyone but the last owner; an admin removes plain members. Nobody removes themself; they leave with `cord_leave`.'
} satisfies RequestResponseActionSpec;

export const cord_leave_action_spec = {
	method: 'cord_leave',
	kind: 'request_response',
	initiator: 'frontend',
	auth: { account: 'required', actor: 'required' },
	side_effects: true,
	input: CordLeaveInput,
	output: CordLeaveOutput,
	async: true,
	description: "Leave a cord. Anyone but the cord's last owner."
} satisfies RequestResponseActionSpec;

/** The cord actions, in the order the server registers them. */
export const all_cord_domain_action_specs: Array<ActionSpecUnion> = [
	cord_create_action_spec,
	cord_list_action_spec,
	cord_get_action_spec,
	cord_update_action_spec,
	cord_admin_list_action_spec,
	cord_invite_create_action_spec,
	cord_invite_list_action_spec,
	cord_invite_revoke_action_spec,
	cord_invite_preview_action_spec,
	cord_invite_redeem_action_spec,
	cord_invite_accept_action_spec,
	cord_member_list_action_spec,
	cord_member_role_set_action_spec,
	cord_member_remove_action_spec,
	cord_leave_action_spec
];
