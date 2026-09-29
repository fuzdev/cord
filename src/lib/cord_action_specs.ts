/**
 * Action specs for the cord actions `cordd` defines: creating, listing,
 * reading, and renaming cords, plus the instance admins' ops list.
 *
 * The server's handlers are in `crates/cord_server/src/cord_actions.rs`;
 * these specs are their wire contract, which the frontend RPC client and the
 * generated action types read. Authorization beyond the spec-level gates
 * (cord membership, the global admin grant) is the server's.
 *
 * @module
 */

import { z } from 'zod';
import { Uuid } from '@fuzdev/fuz_util/id.ts';
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

/** The cord actions, in the order the server registers them. */
export const all_cord_domain_action_specs: Array<ActionSpecUnion> = [
	cord_create_action_spec,
	cord_list_action_spec,
	cord_get_action_spec,
	cord_update_action_spec,
	cord_admin_list_action_spec
];
