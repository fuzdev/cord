import { readdirSync, readFileSync } from 'node:fs';
import { assert, describe, test } from 'vitest';
import { is, parse, parse_or_throw } from '@fuzdev/fuz_schema/schema.ts';
import { get_meta } from '@fuzdev/fuz_schema/meta.ts';
import { ROLE_ADMIN } from '@fuzdev/fuz_app/auth/role_schema.ts';
import {
	resolve_issues_invalid_reason,
	schema_has_sensitive_field
} from '@fuzdev/fuz_app/schema_meta.ts';
import { parse_any } from '@fuzdev/fuz_app/schema_issues.ts';
import { generate_valid_body } from '@fuzdev/fuz_app/testing/schema_generators.ts';

import {
	CORD_NAME_FORBIDDEN_CHAR_RANGES,
	CORD_NAME_JOIN_CHARS,
	CORD_NAME_LENGTH_MAX,
	CordName,
	is_forbidden_cord_name_char,
	all_cord_domain_action_specs,
	cord_admin_list_action_spec,
	cord_create_action_spec,
	CORD_INVITE_EXPIRES_IN_SECONDS_MAX,
	CORD_INVITE_EXPIRES_IN_SECONDS_MIN,
	CORD_INVITE_MAX_USES_MAX,
	CordInviteCreateInput,
	CORD_INVITE_TOKEN_LENGTH_MAX,
	CordInviteToken,
	cord_invite_create_action_spec,
	cord_invite_preview_action_spec,
	cord_invite_redeem_action_spec,
	CordInviteRedeemInput,
	CordInvitePreviewOutput,
	CordAdminJson,
	CordCreateInput,
	CordInviteCreateOutput,
	CordInviteJson,
	CordJson,
	CordMemberJson,
	CordUpdateInput,
	ERROR_CORD_INVITE_USERNAME_INVALID,
	ERROR_CORD_NAME_INVALID
} from '#lib/cord_action_specs.ts';
import * as cord_action_specs from '#lib/cord_action_specs.ts';
import { all_cord_action_specs } from '#lib/action_specs.ts';

describe('CordName', () => {
	test('trims', () => {
		assert.strictEqual(parse_or_throw(CordName, '  the porch  '), 'the porch');
	});

	test('refuses an empty or blank name', () => {
		assert.isFalse(is(CordName, ''));
		assert.isFalse(is(CordName, '   '));
	});

	test('counts characters as the server does, by code point', () => {
		const longest = '🌙'.repeat(CORD_NAME_LENGTH_MAX);
		assert.isTrue(is(CordName, longest), 'astral characters count once each');
		assert.isFalse(is(CordName, 'a'.repeat(CORD_NAME_LENGTH_MAX + 1)));
	});

	test('refuses invisible, control, and line-breaking characters', () => {
		for (const name of [
			'two\nlines',
			'tab\there',
			'\u0085next line',
			'\u200B',
			'zero\u200Bwidth',
			'ltr\u200Emark',
			'evil\u202Etxt.exe',
			'isolate\u2066',
			'\uFEFFbom',
			'soft\u00ADhyphen',
			'word\u2060joiner',
			'tag\u{E0041}',
			'line\u2028separator',
			'\u3164',
			'hangul\u115Ffiller',
			'half\uFFA0width',
			'selector\u{E0100}',
			'grapheme\u034Fjoiner',
			'khmer\u17B4vowel',
			'mongolian\u180Bselector',
			'unassigned\u2065',
			'reserved\uFFF0',
			'tag\u{E0000}'
		]) {
			assert.isFalse(is(CordName, name), JSON.stringify(name));
		}
		// visible non-ASCII, interior spaces, and emoji variation selectors are fine
		assert.strictEqual(parse_or_throw(CordName, '\u3000la cabaña 🌙\u00A0'), 'la cabaña 🌙');
		assert.strictEqual(parse_or_throw(CordName, '☀\uFE0F'), '☀\uFE0F');
	});

	test('refuses lone surrogates, which the server cannot receive', () => {
		for (const name of ['a\uD800b', 'a\uDFFFb', '\uDC00', 'x\uD83D']) {
			assert.isFalse(is(CordName, name), JSON.stringify(name));
		}
		// a paired surrogate is an ordinary astral character
		assert.strictEqual(parse_or_throw(CordName, 'a\uD83C\uDF19b'), 'a🌙b');
	});

	test('allows join characters between letters and emoji', () => {
		for (const name of [
			'🏳\uFE0F\u200D🌈',
			'❤\uFE0F\u200D🔥',
			'🧑\u200D💻',
			'🧑🏽\u200D💻',
			'👨\u200D👩\u200D👧\u200D👦',
			'می\u200Cخواهم',
			'ශ්\u200Dරී'
		]) {
			assert.strictEqual(parse_or_throw(CordName, name), name, JSON.stringify(name));
		}
	});

	test('refuses join characters that only pad', () => {
		for (const name of [
			'\u200D',
			'\u200C',
			'a\u200D',
			'\u200Ca',
			' \u200Da ',
			'a \u200Db',
			'a\u200D b',
			'a\u200D\u200Db',
			'a\u200C\u200Db'
		]) {
			assert.isFalse(is(CordName, name), JSON.stringify(name));
		}
	});

	test('trims as the server does', () => {
		// JS `trim` would strip a leading BOM and keep U+0085; Rust's does the
		// reverse. Both are refused before trimming, so the twins agree.
		assert.isFalse(is(CordName, '\uFEFFx'));
		assert.isFalse(is(CordName, '\u0085x'));
		assert.strictEqual(parse_or_throw(CordName, '\u2000\u202F\u205F x \u1680\u00A0'), 'x');
	});

	test('the forbidden ranges match the server and the Unicode-property formula', () => {
		// the server's list, copied: keep the two in step
		const rust_source = readFileSync('crates/cord_server/src/cord_queries.rs', 'utf8');
		const list = rust_source.slice(
			rust_source.indexOf('pub const CORD_NAME_FORBIDDEN_CHAR_RANGES'),
			rust_source.indexOf('];', rust_source.indexOf('pub const CORD_NAME_FORBIDDEN_CHAR_RANGES'))
		);
		const rust_ranges = Array.from(
			list.matchAll(/\('\\u\{([0-9A-F]+)\}', '\\u\{([0-9A-F]+)\}'\)/g),
			(m) => [parseInt(m[1]!, 16), parseInt(m[2]!, 16)]
		);
		assert.deepEqual(
			rust_ranges,
			CORD_NAME_FORBIDDEN_CHAR_RANGES.map((r) => [...r])
		);
		// both directions at once: a listed code point is exactly one the
		// formula names, (Cf ∪ Default_Ignorable ∪ {U+2028, U+2029}) − the join
		// characters − U+FE00–FE0F, so a missing or over-broad range fails here;
		// the properties come from Node's ICU, so a Node upgrade that adds Cf or
		// default-ignorable characters fails this on purpose until both twins'
		// lists are updated
		const expected = (c: string, code: number): boolean =>
			(/\p{Cf}/u.test(c) ||
				/\p{Default_Ignorable_Code_Point}/u.test(c) ||
				code === 0x2028 ||
				code === 0x2029) &&
			!CORD_NAME_JOIN_CHARS.includes(c) &&
			!(code >= 0xfe00 && code <= 0xfe0f);
		const listed = (code: number): boolean =>
			CORD_NAME_FORBIDDEN_CHAR_RANGES.some(([start, end]) => code >= start && code <= end);
		const mismatches: Array<string> = [];
		for (let code = 0; code <= 0x10ffff; code++) {
			if (code >= 0xd800 && code <= 0xdfff) continue;
			const c = String.fromCodePoint(code);
			const want = expected(c, code);
			if (listed(code) !== want) mismatches.push(code.toString(16));
			// and the character check is the list plus the control characters
			if (is_forbidden_cord_name_char(c) !== (want || /\p{Cc}/u.test(c))) {
				mismatches.push(`check:${code.toString(16)}`);
			}
		}
		assert.deepEqual(mismatches, []);
	});
});

describe('cord action specs', () => {
	test('are all in the frontend surface', () => {
		for (const spec of all_cord_domain_action_specs) {
			assert.include(all_cord_action_specs, spec, spec.method);
		}
	});

	test('gate creating and the ops list on the admin role', () => {
		for (const spec of [cord_create_action_spec, cord_admin_list_action_spec]) {
			assert.deepEqual(spec.auth.roles, [ROLE_ADMIN], spec.method);
		}
	});

	test('declare side effects exactly on the writes', () => {
		const writes = all_cord_domain_action_specs
			.filter((spec) => spec.side_effects)
			.map((spec) => spec.method);
		assert.deepEqual(writes, [
			'cord_create',
			'cord_update',
			'cord_invite_create',
			'cord_invite_revoke',
			'cord_invite_redeem',
			'cord_invite_accept',
			'cord_member_role_set',
			'cord_member_remove',
			'cord_leave'
		]);
	});

	test('take invite accept over a cookie session only', () => {
		for (const spec of all_cord_domain_action_specs) {
			const session_only = spec.method === 'cord_invite_accept';
			assert.deepEqual(
				spec.auth?.credential_types,
				session_only ? ['session'] : undefined,
				spec.method
			);
		}
	});

	test('rate-limit minting per account, and what anonymous callers reach per IP', () => {
		for (const spec of all_cord_domain_action_specs) {
			const expected =
				spec === cord_invite_create_action_spec
					? 'account'
					: spec === cord_invite_preview_action_spec || spec === cord_invite_redeem_action_spec
						? 'ip'
						: undefined;
			assert.strictEqual(spec.rate_limit, expected, spec.method);
		}
	});

	test('admit anonymous callers only to the preview and redemption', () => {
		for (const spec of all_cord_domain_action_specs) {
			const expected: Array<string | undefined> =
				spec === cord_invite_preview_action_spec
					? ['optional', 'optional']
					: spec === cord_invite_redeem_action_spec
						? ['none', 'none']
						: ['required', 'required'];
			assert.deepEqual([spec.auth?.account, spec.auth?.actor], expected, spec.method);
		}
	});
});

describe('CordInviteRedeemInput', () => {
	test('takes a token and a username the instance would create, lowercased', () => {
		assert.deepEqual(parse_or_throw(CordInviteRedeemInput, { token: 't', username: 'NewComer' }), {
			token: 't',
			username: 'newcomer'
		});
		for (const username of ['ab', '1abc', 'abc_', 'a b c', 'ab.cd', 'x'.repeat(40)]) {
			assert.ok(!is(CordInviteRedeemInput, { token: 't', username }), JSON.stringify(username));
		}
	});

	test('takes no acting actor, since redeem never reads the caller', () => {
		assert.ok(
			!is(CordInviteRedeemInput, {
				token: 't',
				username: 'newcomer',
				acting: '11111111-1111-4111-8111-111111111111'
			})
		);
	});
});

describe('CordInvitePreviewOutput', () => {
	test('says whether the link can be redeemed now', () => {
		const preview = {
			status: 'active',
			admits_newcomers: true,
			newcomers_open: false,
			cord: null,
			role: null
		};
		assert.ok(is(CordInvitePreviewOutput, preview));
		const { newcomers_open: _, ...without } = preview;
		assert.ok(!is(CordInvitePreviewOutput, without));
	});
});

describe('CordInviteToken', () => {
	test('is bounded as the server looks tokens up', () => {
		assert.ok(is(CordInviteToken, 'x'));
		assert.ok(is(CordInviteToken, 'x'.repeat(CORD_INVITE_TOKEN_LENGTH_MAX)));
		assert.ok(!is(CordInviteToken, ''));
		assert.ok(!is(CordInviteToken, 'x'.repeat(CORD_INVITE_TOKEN_LENGTH_MAX + 1)));
	});

	test('counts UTF-16 code units, as the server does', () => {
		const emoji = '\u{1F600}';
		assert.ok(is(CordInviteToken, emoji.repeat(CORD_INVITE_TOKEN_LENGTH_MAX / 2)), '128 units');
		assert.ok(
			!is(CordInviteToken, emoji.repeat(CORD_INVITE_TOKEN_LENGTH_MAX / 2 + 1)),
			'130 units, 65 code points'
		);
	});

	test('is secret, so cord_invite_preview carries it in a POST body, not a GET URL', () => {
		assert.strictEqual(get_meta(CordInviteToken)?.sensitivity, 'secret');
		assert.isTrue(schema_has_sensitive_field(cord_invite_preview_action_spec.input));
	});
});

describe('CordInviteCreateInput', () => {
	const cord_id = '11111111-1111-4111-8111-111111111111';

	test('takes optional, bounded limits', () => {
		assert.ok(is(CordInviteCreateInput, { cord_id }));
		assert.ok(
			is(CordInviteCreateInput, {
				cord_id,
				expires_in_seconds: 3600,
				max_uses: 5,
				admits_newcomers: true
			})
		);
		assert.ok(
			is(CordInviteCreateInput, {
				cord_id,
				expires_in_seconds: CORD_INVITE_EXPIRES_IN_SECONDS_MAX,
				max_uses: CORD_INVITE_MAX_USES_MAX
			})
		);
		for (const limits of [
			{ expires_in_seconds: CORD_INVITE_EXPIRES_IN_SECONDS_MIN - 1 },
			{ expires_in_seconds: CORD_INVITE_EXPIRES_IN_SECONDS_MAX + 1 },
			{ expires_in_seconds: 90.5 },
			{ expires_in_seconds: null },
			{ max_uses: 0 },
			{ max_uses: 1.5 },
			{ max_uses: CORD_INVITE_MAX_USES_MAX + 1 },
			{ max_uses: null },
			{ admits_newcomers: null },
			{ admits_newcomers: 'yes' }
		]) {
			assert.ok(!is(CordInviteCreateInput, { cord_id, ...limits }), JSON.stringify(limits));
		}
	});
});

describe('the server twins', () => {
	const RUST_SRC = 'crates/cord_server/src';

	/**
	 * Every `pub const` in the server crate whose value is a string literal
	 * or an integer (a product of integer literals, as the limits are
	 * written), by name.
	 */
	const rust_consts = (): Map<string, string | number> => {
		const consts: Map<string, string | number> = new Map();
		for (const file of readdirSync(RUST_SRC, { recursive: true, encoding: 'utf8' })) {
			if (!file.endsWith('.rs')) continue;
			const source = readFileSync(`${RUST_SRC}/${file}`, 'utf8');
			for (const m of source.matchAll(/pub const (\w+): &str = "([^"]*)";/g)) {
				consts.set(m[1]!, m[2]!);
			}
			for (const m of source.matchAll(/pub const (\w+): (?:u32|u64|usize) = ([\d_ *]+);/g)) {
				const factors = m[2]!.split('*').map((f) => Number(f.trim().replaceAll('_', '')));
				consts.set(
					m[1]!,
					factors.reduce((a, b) => a * b, 1)
				);
			}
		}
		return consts;
	};

	/** The entries of `consts` whose names pass `keep`, sorted by name. */
	const entries = (
		consts: Iterable<[string, unknown]>,
		keep: (name: string, value: unknown) => boolean
	): Array<[string, unknown]> =>
		Array.from(consts)
			.filter(([name, value]) => keep(name, value))
			.sort(([a], [b]) => a.localeCompare(b));

	const rust = rust_consts();
	const ts = Object.entries(cord_action_specs);

	test('every error reason is on both sides, with the same value', () => {
		const is_reason = (name: string, value: unknown): boolean =>
			name.startsWith('ERROR_CORD_') && typeof value === 'string';
		const rust_reasons = entries(rust, is_reason);
		assert.ok(
			rust_reasons.some(([name]) => name === 'ERROR_CORD_NOT_FOUND'),
			'the sweep reads the Rust sources'
		);
		assert.deepEqual(rust_reasons, entries(ts, is_reason));
	});

	test('every limit is on both sides, with the same value', () => {
		const is_limit = (name: string, value: unknown): boolean =>
			name.startsWith('CORD_') && typeof value === 'number';
		const rust_limits = entries(rust, is_limit);
		assert.ok(
			rust_limits.some(([name]) => name === 'CORD_NAME_LENGTH_MAX'),
			'the sweep reads the Rust sources'
		);
		assert.deepEqual(rust_limits, entries(ts, is_limit));
	});

	test("the server's cord verbs are exactly the TS twin's", () => {
		// every spec the crate builds, by the method it's constructed with
		const rust_methods: Array<string> = [];
		for (const file of readdirSync(RUST_SRC, { recursive: true, encoding: 'utf8' })) {
			if (!file.endsWith('.rs')) continue;
			const source = readFileSync(`${RUST_SRC}/${file}`, 'utf8');
			for (const m of source.matchAll(/ActionSpec::\w+\(\s*"(\w+)"/g)) {
				rust_methods.push(m[1]!);
			}
		}
		assert.ok(rust_methods.includes('cord_create'), 'the sweep reads the Rust sources');
		const ts_methods = all_cord_action_specs
			.map((spec) => spec.method)
			.filter((method) => method.startsWith('cord_'));
		assert.deepEqual(rust_methods.sort(), ts_methods.sort());
	});
});

describe('the output shapes', () => {
	const id = '11111111-1111-4111-8111-111111111111';
	const invite = {
		id,
		cord_id: id,
		role: 'cord_member',
		admits_newcomers: false,
		created_by: id,
		created_at: 't',
		expires_at: 't',
		max_uses: 5,
		use_count: 0,
		revoked_at: 't',
		status: 'active',
		redemption_count: 0,
		redemption_accepted_count: 0
	};
	const none: Array<string> = [];
	const cases = [
		{
			schema: CordJson,
			value: { id, name: 'the porch', role: 'cord_owner', created_at: 't' },
			nullable: none
		},
		{
			schema: CordAdminJson,
			value: { id, name: 'the porch', member_count: 3, created_at: 't' },
			nullable: none
		},
		{
			schema: CordInviteJson,
			value: invite,
			nullable: ['created_by', 'expires_at', 'max_uses', 'revoked_at']
		},
		{
			schema: CordMemberJson,
			value: { actor_id: id, username: 'alice', display_name: 'Al', role: 'cord_admin' },
			nullable: ['display_name']
		},
		{ schema: CordInviteCreateOutput, value: { invite, token: 't' }, nullable: none }
	];

	test('require every field, and take null exactly where the server sends it', () => {
		for (const { schema, value, nullable } of cases) {
			assert.ok(is(schema, value), JSON.stringify(value));
			for (const key of Object.keys(value)) {
				const { [key]: _, ...without } = value as Record<string, unknown>;
				assert.ok(!is(schema, without), `${key} is required`);
				assert.strictEqual(is(schema, { ...value, [key]: null }), nullable.includes(key), key);
			}
		}
	});

	test('take integer counts and known enum values, and lowercase ids', () => {
		for (const bad of [
			{ use_count: 1.5 },
			{ max_uses: 2 ** 53 },
			{ status: 'paused' },
			{ role: 'cord_guest' }
		]) {
			assert.ok(!is(CordInviteJson, { ...invite, ...bad }), JSON.stringify(bad));
		}
		assert.ok(!is(CordAdminJson, { id, name: 'n', member_count: 0.5, created_at: 't' }));
		assert.ok(!is(CordJson, { id: 'x', name: 'n', role: 'cord_owner', created_at: 't' }));
		assert.strictEqual(
			parse_or_throw(CordJson, {
				id: id.toUpperCase(),
				name: 'n',
				role: 'cord_owner',
				created_at: 't'
			}).id,
			id
		);
	});
});

describe('the input contracts', () => {
	test('owe the server reasons for a bad name or username', () => {
		for (const [schema, input, reason] of [
			[CordCreateInput, { name: '' }, ERROR_CORD_NAME_INVALID],
			[
				CordUpdateInput,
				{ cord_id: '11111111-1111-4111-8111-111111111111', name: '' },
				ERROR_CORD_NAME_INVALID
			],
			[CordInviteRedeemInput, { token: 't', username: 'ab' }, ERROR_CORD_INVITE_USERNAME_INVALID]
		] as const) {
			const parsed = parse(schema, input);
			assert.ok(!parsed.ok);
			assert.strictEqual(resolve_issues_invalid_reason(schema, parsed.issues, input), reason);
		}
	});

	test('take an acting actor everywhere but redemption', () => {
		const acting = '11111111-1111-4111-8111-111111111111';
		for (const spec of all_cord_domain_action_specs) {
			const body = { ...(generate_valid_body(spec.input) as object), acting };
			assert.strictEqual(
				parse_any(spec.input, body).ok,
				spec !== cord_invite_redeem_action_spec,
				spec.method
			);
		}
	});
});
