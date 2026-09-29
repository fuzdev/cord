import { readFileSync } from 'node:fs';
import { assert, describe, test } from 'vitest';
import { ROLE_ADMIN } from '@fuzdev/fuz_app/auth/role_schema.ts';

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
	ERROR_CORD_INVITE_EXHAUSTED,
	ERROR_CORD_INVITE_EXPIRED,
	ERROR_CORD_INVITE_FORBIDDEN,
	ERROR_CORD_INVITE_INVALID,
	ERROR_CORD_INVITE_NOT_FOUND,
	ERROR_CORD_INVITE_REVOKED,
	ERROR_CORD_LAST_OWNER,
	ERROR_CORD_MEMBER_FORBIDDEN,
	ERROR_CORD_MEMBER_NOT_FOUND,
	ERROR_CORD_MEMBER_REMOVE_SELF,
	CORD_INVITE_TOKEN_LENGTH_MAX,
	CordInviteToken,
	cord_invite_create_action_spec
} from '#lib/cord_action_specs.ts';
import { all_cord_action_specs } from '#lib/action_specs.ts';

describe('CordName', () => {
	test('trims', () => {
		assert.strictEqual(CordName.parse('  the porch  '), 'the porch');
	});

	test('refuses an empty or blank name', () => {
		assert.isFalse(CordName.safeParse('').success);
		assert.isFalse(CordName.safeParse('   ').success);
	});

	test('counts characters as the server does, by code point', () => {
		const longest = '🌙'.repeat(CORD_NAME_LENGTH_MAX);
		assert.isTrue(CordName.safeParse(longest).success, 'astral characters count once each');
		assert.isFalse(CordName.safeParse('a'.repeat(CORD_NAME_LENGTH_MAX + 1)).success);
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
			assert.isFalse(CordName.safeParse(name).success, JSON.stringify(name));
		}
		// visible non-ASCII, interior spaces, and emoji variation selectors are fine
		assert.strictEqual(CordName.parse('\u3000la cabaña 🌙\u00A0'), 'la cabaña 🌙');
		assert.strictEqual(CordName.parse('☀\uFE0F'), '☀\uFE0F');
	});

	test('refuses lone surrogates, which the server cannot receive', () => {
		for (const name of ['a\uD800b', 'a\uDFFFb', '\uDC00', 'x\uD83D']) {
			assert.isFalse(CordName.safeParse(name).success, JSON.stringify(name));
		}
		// a paired surrogate is an ordinary astral character
		assert.strictEqual(CordName.parse('a\uD83C\uDF19b'), 'a🌙b');
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
			assert.strictEqual(CordName.parse(name), name, JSON.stringify(name));
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
			assert.isFalse(CordName.safeParse(name).success, JSON.stringify(name));
		}
	});

	test('trims as the server does', () => {
		// JS `trim` would strip a leading BOM and keep U+0085; Rust's does the
		// reverse. Both are refused before trimming, so the twins agree.
		assert.isFalse(CordName.safeParse('\uFEFFx').success);
		assert.isFalse(CordName.safeParse('\u0085x').success);
		assert.strictEqual(CordName.parse('\u2000\u202F\u205F x \u1680\u00A0'), 'x');
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

	test('rate-limit only minting, per account', () => {
		for (const spec of all_cord_domain_action_specs) {
			assert.strictEqual(
				spec.rate_limit,
				spec === cord_invite_create_action_spec ? 'account' : undefined,
				spec.method
			);
		}
	});
});

describe('CordInviteToken', () => {
	test('is bounded as the server looks tokens up', () => {
		assert.ok(CordInviteToken.safeParse('x').success);
		assert.ok(CordInviteToken.safeParse('x'.repeat(CORD_INVITE_TOKEN_LENGTH_MAX)).success);
		assert.ok(!CordInviteToken.safeParse('').success);
		assert.ok(!CordInviteToken.safeParse('x'.repeat(CORD_INVITE_TOKEN_LENGTH_MAX + 1)).success);
	});
});

describe('CordInviteCreateInput', () => {
	const cord_id = '11111111-1111-4111-8111-111111111111';

	test('takes optional, bounded limits', () => {
		assert.ok(CordInviteCreateInput.safeParse({ cord_id }).success);
		assert.ok(
			CordInviteCreateInput.safeParse({
				cord_id,
				expires_in_seconds: CORD_INVITE_EXPIRES_IN_SECONDS_MAX,
				max_uses: CORD_INVITE_MAX_USES_MAX
			}).success
		);
		for (const limits of [
			{ expires_in_seconds: CORD_INVITE_EXPIRES_IN_SECONDS_MIN - 1 },
			{ expires_in_seconds: CORD_INVITE_EXPIRES_IN_SECONDS_MAX + 1 },
			{ expires_in_seconds: 90.5 },
			{ expires_in_seconds: null },
			{ max_uses: 0 },
			{ max_uses: CORD_INVITE_MAX_USES_MAX + 1 },
			{ max_uses: null }
		]) {
			assert.ok(
				!CordInviteCreateInput.safeParse({ cord_id, ...limits }).success,
				JSON.stringify(limits)
			);
		}
	});
});

describe('the invite and member twins', () => {
	/** The value of `pub const {name}` in a server source file. */
	const rust_const = (file: string, name: string): string => {
		const source = readFileSync(`crates/cord_server/src/${file}`, 'utf8');
		const match = new RegExp(`pub const ${name}: [^=]+= ([^;]+);`).exec(source);
		assert.ok(match, `${name} in ${file}`);
		return match[1]!.trim();
	};

	test('the limits match the server', () => {
		assert.strictEqual(
			rust_const('cord_invite_queries.rs', 'CORD_INVITE_EXPIRES_IN_SECONDS_MIN'),
			String(CORD_INVITE_EXPIRES_IN_SECONDS_MIN)
		);
		assert.strictEqual(
			rust_const('cord_invite_queries.rs', 'CORD_INVITE_EXPIRES_IN_SECONDS_MAX'),
			'365 * 24 * 60 * 60'
		);
		assert.strictEqual(CORD_INVITE_EXPIRES_IN_SECONDS_MAX, 365 * 24 * 60 * 60);
		assert.strictEqual(
			rust_const('cord_invite_queries.rs', 'CORD_INVITE_MAX_USES_MAX'),
			String(CORD_INVITE_MAX_USES_MAX)
		);
		assert.strictEqual(
			rust_const('cord_invite_queries.rs', 'CORD_INVITE_TOKEN_LENGTH_MAX'),
			String(CORD_INVITE_TOKEN_LENGTH_MAX)
		);
	});

	test('the error reasons match the server', () => {
		for (const [file, name, value] of [
			['cord_invite_actions.rs', 'ERROR_CORD_INVITE_NOT_FOUND', ERROR_CORD_INVITE_NOT_FOUND],
			['cord_invite_actions.rs', 'ERROR_CORD_INVITE_REVOKED', ERROR_CORD_INVITE_REVOKED],
			['cord_invite_actions.rs', 'ERROR_CORD_INVITE_EXPIRED', ERROR_CORD_INVITE_EXPIRED],
			['cord_invite_actions.rs', 'ERROR_CORD_INVITE_EXHAUSTED', ERROR_CORD_INVITE_EXHAUSTED],
			['cord_invite_actions.rs', 'ERROR_CORD_INVITE_FORBIDDEN', ERROR_CORD_INVITE_FORBIDDEN],
			['cord_invite_actions.rs', 'ERROR_CORD_INVITE_INVALID', ERROR_CORD_INVITE_INVALID],
			['cord_member_actions.rs', 'ERROR_CORD_MEMBER_NOT_FOUND', ERROR_CORD_MEMBER_NOT_FOUND],
			['cord_member_actions.rs', 'ERROR_CORD_MEMBER_FORBIDDEN', ERROR_CORD_MEMBER_FORBIDDEN],
			['cord_member_actions.rs', 'ERROR_CORD_LAST_OWNER', ERROR_CORD_LAST_OWNER],
			['cord_member_actions.rs', 'ERROR_CORD_MEMBER_REMOVE_SELF', ERROR_CORD_MEMBER_REMOVE_SELF]
		] as const) {
			assert.strictEqual(rust_const(file, name), JSON.stringify(value), name);
		}
	});
});
