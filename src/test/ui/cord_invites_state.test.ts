import { assert, describe, test } from 'vitest';
import type { Uuid } from '@fuzdev/fuz_util/id.ts';

import {
	CORD_INVITE_MAX_USES_MAX,
	CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX,
	CordInviteCreateInput,
	ERROR_CORD_INVITE_NEWCOMER_FORBIDDEN,
	ERROR_CORD_INVITE_NOT_FOUND,
	type CordInviteJson,
	type CordMemberJson
} from '#lib/cord_action_specs.ts';
import {
	CORD_INVITE_EXPIRES_IN_SECONDS_DEFAULT,
	CORD_INVITE_EXPIRY_CHOICES,
	CordInvitesState,
	cord_invite_expiry_choices,
	to_cord_invite_create_request,
	to_cord_invite_minter,
	to_cord_join_url
} from '#lib/ui/cord_invites_state.svelte.ts';

import { rpc_error, stub_api } from './stub_api.ts';

const CORD_ID = '11111111-1111-4111-8111-111111111111' as Uuid;
const INVITE_ID = '33333333-3333-4333-8333-333333333333' as Uuid;
const ALICE = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' as Uuid;
const BOB = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb' as Uuid;
const DAY = 24 * 60 * 60;

const invite = (overrides: Partial<CordInviteJson> = {}): CordInviteJson =>
	({
		id: INVITE_ID,
		cord_id: CORD_ID,
		role: 'cord_member',
		admits_newcomers: false,
		created_by: ALICE,
		created_at: '2026-01-01T00:00:00Z',
		expires_at: null,
		max_uses: null,
		use_count: 0,
		revoked_at: null,
		status: 'active',
		redemption_count: 0,
		redemption_accepted_count: 0,
		...overrides
	}) as CordInviteJson;

const member = (actor_id: string, role: CordMemberJson['role']): CordMemberJson =>
	({ actor_id, username: 'alice', display_name: null, role }) as CordMemberJson;

describe('the expiry choices', () => {
	test('start at 7 days, and offer never for ordinary links', () => {
		assert.strictEqual(CORD_INVITE_EXPIRES_IN_SECONDS_DEFAULT, 7 * DAY);
		assert.ok(
			CORD_INVITE_EXPIRY_CHOICES.some((c) => c.seconds === CORD_INVITE_EXPIRES_IN_SECONDS_DEFAULT)
		);
		assert.ok(cord_invite_expiry_choices(false).some((c) => c.seconds === null));
	});

	test('for a newcomer link, are bounded and within the cap', () => {
		const choices = cord_invite_expiry_choices(true);
		assert.isNotEmpty(choices);
		for (const choice of choices) {
			assert.isNotNull(choice.seconds, choice.label);
			assert.isAtMost(choice.seconds, CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX, choice.label);
		}
		assert.ok(choices.some((c) => c.seconds === CORD_INVITE_EXPIRES_IN_SECONDS_DEFAULT));
	});
});

describe('to_cord_invite_create_request', () => {
	const draft = { expires_in_seconds: 7 * DAY, max_uses: null, admits_newcomers: false };

	test('builds a request the wire accepts, leaving out what is unset', () => {
		const cases = [
			[draft, { cord_id: CORD_ID, expires_in_seconds: 7 * DAY }],
			[
				{ ...draft, expires_in_seconds: null, max_uses: 5 },
				{ cord_id: CORD_ID, max_uses: 5 }
			],
			[{ ...draft, expires_in_seconds: null, max_uses: undefined }, { cord_id: CORD_ID }],
			[
				{ expires_in_seconds: 30 * DAY, max_uses: 3, admits_newcomers: true },
				{ cord_id: CORD_ID, expires_in_seconds: 30 * DAY, max_uses: 3, admits_newcomers: true }
			]
		] as const;
		for (const [input, expected] of cases) {
			const result = to_cord_invite_create_request(CORD_ID, input);
			assert.ok(result.ok, JSON.stringify(input));
			assert.deepEqual(result.value, expected);
			assert.ok(CordInviteCreateInput.safeParse(result.value).success, JSON.stringify(input));
		}
	});

	test('refuses what the server would', () => {
		for (const [input, pattern] of [
			[{ ...draft, max_uses: 0 }, /at least 1/],
			[{ ...draft, max_uses: 2.5 }, /whole number/],
			[{ ...draft, max_uses: CORD_INVITE_MAX_USES_MAX + 1 }, /at most/],
			[
				{ ...draft, admits_newcomers: true, max_uses: 3, expires_in_seconds: null },
				/needs an expiry/
			],
			[
				{
					...draft,
					admits_newcomers: true,
					max_uses: 3,
					expires_in_seconds: CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX + 1
				},
				/within 30 days/
			],
			[{ ...draft, admits_newcomers: true }, /needs a use limit/]
		] as const) {
			const result = to_cord_invite_create_request(CORD_ID, input);
			assert.ok(!result.ok, JSON.stringify(input));
			assert.match(result.message, pattern);
		}
	});
});

describe('to_cord_join_url', () => {
	test('puts the token in the fragment of the join page', () => {
		assert.strictEqual(
			to_cord_join_url('https://cord.example/join', 'tok_123'),
			'https://cord.example/join#tok_123'
		);
		assert.strictEqual(
			to_cord_join_url('https://cord.example/join#old', 'tok_123'),
			'https://cord.example/join#tok_123'
		);
	});
});

describe('to_cord_invite_minter', () => {
	test('flags a minter who no longer manages the cord', () => {
		assert.strictEqual(
			to_cord_invite_minter(invite(), [member(ALICE, 'cord_owner')]).kind,
			'manager'
		);
		assert.strictEqual(
			to_cord_invite_minter(invite(), [member(ALICE, 'cord_admin')]).kind,
			'manager'
		);
		assert.strictEqual(
			to_cord_invite_minter(invite(), [member(ALICE, 'cord_member')]).kind,
			'former_manager'
		);
		assert.strictEqual(
			to_cord_invite_minter(invite(), [member(BOB, 'cord_owner')]).kind,
			'former_member'
		);
		assert.strictEqual(
			to_cord_invite_minter(invite({ created_by: null }), [member(ALICE, 'cord_owner')]).kind,
			'gone'
		);
	});
});

describe('CordInvitesState', () => {
	test('create_invite keeps the token once and reloads the list', async () => {
		let invites: Array<CordInviteJson> = [];
		const state = new CordInvitesState({
			api: stub_api({
				cord_invite_list: () => Promise.resolve({ invites }),
				cord_invite_create: (input: { max_uses?: number }) => {
					const created = invite({ max_uses: input.max_uses ?? null });
					invites = [created];
					return Promise.resolve({ invite: created, token: 'tok_secret' });
				}
			})
		});
		await state.fetch(CORD_ID);
		assert.deepEqual(state.invites, []);
		assert.isTrue(await state.create_invite({ cord_id: CORD_ID, max_uses: 5 }));
		assert.strictEqual(state.minted?.token, 'tok_secret');
		assert.strictEqual(state.invites[0]?.max_uses, 5);
		assert.notProperty(state.invites[0]!, 'token', 'the list never carries tokens');
		state.dismiss_minted();
		assert.isNull(state.minted);
	});

	test('a refused mint keeps its reason in words', async () => {
		const state = new CordInvitesState({
			api: stub_api({
				cord_invite_create: () => Promise.reject(rpc_error(ERROR_CORD_INVITE_NEWCOMER_FORBIDDEN))
			})
		});
		assert.isFalse(
			await state.create_invite({ cord_id: CORD_ID, admits_newcomers: true, max_uses: 1 })
		);
		assert.isNull(state.minted);
		assert.match(state.create.error!, /Only instance admins/);
	});

	test('revoke_invite updates the link in place', async () => {
		const state = new CordInvitesState({
			api: stub_api({
				cord_invite_list: () => Promise.resolve({ invites: [invite()] }),
				cord_invite_revoke: () =>
					Promise.resolve({
						invite: invite({ status: 'revoked', revoked_at: '2026-01-02T00:00:00Z' })
					})
			})
		});
		await state.fetch(CORD_ID);
		assert.isTrue(await state.revoke_invite(INVITE_ID));
		assert.strictEqual(state.invites[0]?.status, 'revoked');
		assert.isTrue(state.revoke.succeeded(INVITE_ID));
	});

	test('fetching another cord forgets the minted link', async () => {
		const state = new CordInvitesState({
			api: stub_api({
				cord_invite_list: () => Promise.resolve({ invites: [] }),
				cord_invite_create: () => Promise.resolve({ invite: invite(), token: 'tok' })
			})
		});
		await state.fetch(CORD_ID);
		await state.create_invite({ cord_id: CORD_ID });
		assert.isNotNull(state.minted);
		await state.fetch('22222222-2222-4222-8222-222222222222' as Uuid);
		assert.isNull(state.minted);
	});

	test('a revoke of a link the server no longer has says so, to a manager, and reloads', async () => {
		let lists = 0;
		const state = new CordInvitesState({
			api: stub_api({
				cord_invite_list: () => {
					lists++;
					return Promise.resolve({ invites: lists === 1 ? [invite()] : [] });
				},
				cord_invite_revoke: () => Promise.reject(rpc_error(ERROR_CORD_INVITE_NOT_FOUND))
			})
		});
		await state.fetch(CORD_ID);
		assert.isFalse(await state.revoke_invite(INVITE_ID));
		assert.strictEqual(state.revoke.error(INVITE_ID), "That link's already gone.");
		assert.strictEqual(lists, 2);
		assert.deepEqual(state.invites, []);
	});

	test('drops a late response, or a mint, for a cord the state has moved off', async () => {
		const OTHER_CORD_ID = '22222222-2222-4222-8222-222222222222' as Uuid;
		let release_list!: () => void;
		let release_create!: () => void;
		const lists: Array<string> = [];
		const state = new CordInvitesState({
			api: stub_api({
				cord_invite_list: async (input: { cord_id: Uuid }) => {
					lists.push(input.cord_id);
					if (input.cord_id === CORD_ID && lists.length === 1) {
						await new Promise<void>((r) => (release_list = r));
						return { invites: [invite()] };
					}
					return { invites: [] };
				},
				cord_invite_create: async () => {
					await new Promise<void>((r) => (release_create = r));
					return { invite: invite(), token: 'tok' };
				}
			})
		});
		const first = state.fetch(CORD_ID);
		const creating = state.create_invite({ cord_id: CORD_ID });
		await state.fetch(OTHER_CORD_ID);
		release_list();
		await first;
		assert.deepEqual(state.invites, [], 'the first cord’s links stay out');
		release_create();
		assert.isFalse(await creating, 'moving to another cord drops the mint in flight');
		assert.isNull(state.minted, 'the first cord’s new link isn’t shown on the other');
		assert.deepEqual(lists, [CORD_ID, OTHER_CORD_ID]);
	});
});
