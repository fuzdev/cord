import { assert, describe, test } from 'vitest';

import {
	CORD_INVITE_TOKEN_LENGTH_MAX,
	ERROR_CORD_INVITE_EXHAUSTED,
	ERROR_CORD_INVITE_EXPIRED,
	ERROR_CORD_INVITE_NEWCOMERS_CLOSED,
	ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS,
	ERROR_CORD_INVITE_NOT_FOUND,
	ERROR_CORD_INVITE_REVOKED,
	ERROR_CORD_INVITE_USERNAME_INVALID,
	ERROR_CORD_INVITE_USERNAME_UNAVAILABLE,
	type CordInviteCordJson,
	type CordInvitePreviewOutput,
	type CordJson
} from '#lib/cord_action_specs.ts';
import {
	CORD_JOIN_STORAGE_KEY,
	CordJoinState,
	type CordJoinStorage,
	type CordJoinStored,
	type CordJoinView,
	type CordJoinViewInput,
	check_cord_newcomer_fields,
	cord_join_redemption_applies,
	forget_cord_join,
	is_dead_link_error,
	load_cord_join,
	save_cord_join,
	to_cord_join_recheck_error,
	to_cord_join_view
} from '#lib/ui/cord_join_state.svelte.ts';

import { CORD_RATE_LIMITED_MESSAGE } from '#lib/ui/cord_errors.ts';

import { rpc_error, stub_api } from './stub_api.ts';

const CORD = {
	id: '11111111-1111-4111-8111-111111111111',
	name: 'the porch'
} as CordInviteCordJson;
const ACCOUNT = '44444444-4444-4444-8444-444444444444';
const TOKEN = 'tok_abc';

const memory_storage = (initial: CordJoinStored | null = null) => {
	const map = new Map<string, string>();
	if (initial) map.set(CORD_JOIN_STORAGE_KEY, JSON.stringify(initial));
	const storage: CordJoinStorage = {
		getItem: (key) => map.get(key) ?? null,
		setItem: (key, value) => void map.set(key, value),
		removeItem: (key) => void map.delete(key)
	};
	const stored = (): CordJoinStored | null => {
		const raw = map.get(CORD_JOIN_STORAGE_KEY);
		return raw === undefined ? null : JSON.parse(raw);
	};
	return { storage, map, stored };
};

const blocked_storage: CordJoinStorage = {
	getItem: () => {
		throw new Error('blocked');
	},
	setItem: () => {
		throw new Error('blocked');
	},
	removeItem: () => {
		throw new Error('blocked');
	}
};

const REDEEMED = { cord: CORD, username: 'newcomer' };

describe('the stored join link', () => {
	test('comes from the fragment, which replaces another stored link', () => {
		const { storage, stored } = memory_storage({ token: 'old', redeemed: REDEEMED });
		assert.deepEqual(load_cord_join(`#${TOKEN}`, storage), { token: TOKEN, redeemed: null });
		assert.deepEqual(stored(), { token: TOKEN, redeemed: null });
	});

	test('keeps its redemption when the fragment names the same link', () => {
		const { storage } = memory_storage({ token: TOKEN, redeemed: REDEEMED });
		assert.deepEqual(load_cord_join(`#${TOKEN}`, storage), { token: TOKEN, redeemed: REDEEMED });
	});

	test('comes from storage, with its redemption, when the fragment is gone', () => {
		const { storage } = memory_storage({ token: TOKEN, redeemed: REDEEMED });
		assert.deepEqual(load_cord_join('', storage), { token: TOKEN, redeemed: REDEEMED });
		assert.deepEqual(load_cord_join('#', storage), { token: TOKEN, redeemed: REDEEMED });
	});

	test('is null with neither, or with a record in another shape', () => {
		assert.isNull(load_cord_join('', memory_storage().storage));
		assert.isNull(load_cord_join('', null));
		const { storage, map } = memory_storage();
		map.set(CORD_JOIN_STORAGE_KEY, 'not json');
		assert.isNull(load_cord_join('', storage));
		map.set(CORD_JOIN_STORAGE_KEY, JSON.stringify({ token: TOKEN }));
		assert.isNull(load_cord_join('', storage));
	});

	test('survives a blocked store, in memory', () => {
		assert.deepEqual(load_cord_join(`#${TOKEN}`, blocked_storage), {
			token: TOKEN,
			redeemed: null
		});
		assert.isNull(load_cord_join('', blocked_storage));
		save_cord_join(blocked_storage, { token: TOKEN, redeemed: null });
		forget_cord_join(blocked_storage);
	});

	test('is forgotten on request', () => {
		const { storage, map } = memory_storage({ token: TOKEN, redeemed: null });
		forget_cord_join(storage);
		assert.isFalse(map.has(CORD_JOIN_STORAGE_KEY));
	});
});

describe('is_dead_link_error', () => {
	test('names the refusals that mean ask for a new link', () => {
		for (const reason of [
			ERROR_CORD_INVITE_NOT_FOUND,
			ERROR_CORD_INVITE_REVOKED,
			ERROR_CORD_INVITE_EXPIRED,
			ERROR_CORD_INVITE_EXHAUSTED,
			ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS
		]) {
			assert.isTrue(is_dead_link_error(rpc_error(reason)), reason);
		}
		// the link still works for existing accounts
		assert.isFalse(is_dead_link_error(rpc_error(ERROR_CORD_INVITE_NEWCOMERS_CLOSED)));
		// a reasonless invalid-params error is wire drift, not a dead link
		assert.isFalse(
			is_dead_link_error(Object.assign(new Error('invalid params'), { code: -32602 }))
		);
		assert.isFalse(is_dead_link_error(rpc_error(ERROR_CORD_INVITE_USERNAME_UNAVAILABLE)));
		assert.isFalse(is_dead_link_error(new TypeError('fetch failed')));
		assert.isFalse(is_dead_link_error(Object.assign(new Error('slow down'), { code: -32029 })));
	});
});

describe('to_cord_join_view', () => {
	const preview = (overrides: Partial<CordInvitePreviewOutput> = {}): CordInvitePreviewOutput => ({
		status: 'active',
		admits_newcomers: false,
		newcomers_open: false,
		cord: null,
		role: null,
		...overrides
	});

	/** A signed-out visitor whose preview has loaded. */
	const signed_out = (overrides: Partial<CordJoinViewInput> = {}): CordJoinViewInput => ({
		token_loaded: true,
		token: TOKEN,
		session_checked: true,
		verified: false,
		signed_in: false,
		account_id: null,
		account_username: null,
		preview_for: null,
		preview_status: 'success',
		preview: preview(),
		preview_error_data: null,
		dead: false,
		newcomer_running: false,
		redeemed: null,
		newcomers_closed: false,
		...overrides
	});

	/** A signed-in visitor whose preview has loaded. */
	const signed_in = (overrides: Partial<CordJoinViewInput> = {}): CordJoinViewInput =>
		signed_out({
			verified: true,
			signed_in: true,
			account_id: ACCOUNT,
			account_username: 'newcomer',
			preview_for: ACCOUNT,
			...overrides
		});

	const cases: Array<[string, CordJoinViewInput, CordJoinView]> = [
		// waiting
		['before the token is read', signed_out({ token_loaded: false }), { kind: 'checking' }],
		['before the session check', signed_out({ session_checked: false }), { kind: 'checking' }],
		[
			'while the preview loads',
			signed_out({ preview_status: 'pending', preview: undefined }),
			{ kind: 'checking' }
		],
		[
			'while a first preview loads for this identity',
			signed_in({ preview_status: 'initial', preview: undefined }),
			{ kind: 'checking' }
		],
		[
			'with a preview for someone else (signed out, now signed in)',
			signed_in({ preview_for: null }),
			{ kind: 'checking' }
		],
		[
			'in the gap after a login, before the account loads',
			signed_out({ verified: true }),
			{ kind: 'checking' }
		],
		// dead links
		['with no token', signed_out({ token: null }), { kind: 'dead' }],
		['with an empty token', signed_out({ token: '' }), { kind: 'dead' }],
		[
			'with an over-long token, before any request',
			signed_out({ token: 'x'.repeat(CORD_INVITE_TOKEN_LENGTH_MAX + 1), preview_for: undefined }),
			{ kind: 'dead' }
		],
		[
			'with an unknown token',
			signed_out({
				preview_status: 'failure',
				preview: undefined,
				preview_error_data: rpc_error(ERROR_CORD_INVITE_NOT_FOUND)
			}),
			{ kind: 'dead' }
		],
		[
			'with a reasonless invalid-params failure, which says nothing about the link',
			signed_out({
				preview_status: 'failure',
				preview: undefined,
				preview_error_data: Object.assign(new Error('invalid params'), { code: -32602 })
			}),
			{ kind: 'error', message: 'invalid params' }
		],
		[
			'with a revoked link',
			signed_out({ preview: preview({ status: 'revoked' }) }),
			{ kind: 'dead' }
		],
		[
			'with an expired link, signed in',
			signed_in({ preview: preview({ status: 'expired' }) }),
			{ kind: 'dead' }
		],
		[
			'with a used-up link, signed in',
			signed_in({ preview: preview({ status: 'exhausted' }) }),
			{ kind: 'dead' }
		],
		[
			'once redeem or accept called the link dead',
			signed_out({ dead: true, preview: preview({ newcomers_open: true, cord: CORD }) }),
			{ kind: 'dead' }
		],
		// other failures
		[
			'when the preview fails for another reason',
			signed_out({
				preview_status: 'failure',
				preview: undefined,
				preview_error_data: new TypeError('fetch failed')
			}),
			{ kind: 'error', message: 'fetch failed' }
		],
		// a failed re-preview keeps the last preview for the same identity
		[
			'signed out, when a re-preview fails over an open newcomer link',
			signed_out({
				preview_status: 'failure',
				preview: preview({ admits_newcomers: true, newcomers_open: true, cord: CORD }),
				preview_error_data: new TypeError('fetch failed')
			}),
			{ kind: 'newcomer', cord: CORD }
		],
		[
			'signed out after redeeming here, when a re-preview fails over a used-up link',
			signed_out({
				preview_status: 'failure',
				preview: preview({ status: 'exhausted' }),
				preview_error_data: new TypeError('fetch failed'),
				redeemed: REDEEMED
			}),
			{ kind: 'newcomer', cord: CORD }
		],
		[
			'signed in, when a re-preview fails over an active link',
			signed_in({
				preview_status: 'failure',
				preview: preview({ cord: CORD }),
				preview_error_data: Object.assign(new Error('slow down'), { code: -32029 })
			}),
			{ kind: 'join', cord: CORD }
		],
		[
			'signed out, when a re-preview fails over an ordinary link',
			signed_out({
				preview_status: 'failure',
				preview: preview(),
				preview_error_data: new TypeError('fetch failed')
			}),
			{ kind: 'sign_in', newcomers_closed: false }
		],
		[
			'when a re-preview finds the link dead, whatever the last preview said',
			signed_out({
				preview_status: 'failure',
				preview: preview({ admits_newcomers: true, newcomers_open: true, cord: CORD }),
				preview_error_data: rpc_error(ERROR_CORD_INVITE_REVOKED),
				redeemed: REDEEMED
			}),
			{ kind: 'dead' }
		],
		[
			'when a re-preview fails over a revoked link',
			signed_out({
				preview_status: 'failure',
				preview: preview({ status: 'revoked' }),
				preview_error_data: new TypeError('fetch failed')
			}),
			{ kind: 'dead' }
		],
		[
			'with a failed preview for someone else',
			signed_in({
				preview_for: null,
				preview_status: 'failure',
				preview: preview({ cord: CORD }),
				preview_error_data: new TypeError('fetch failed')
			}),
			{ kind: 'checking' }
		],
		// signed in
		[
			'signed in, to someone already in the cord',
			signed_in({ preview: preview({ cord: CORD, role: 'cord_admin' }) }),
			{ kind: 'member', cord: CORD }
		],
		[
			'signed in, to someone already in the cord, even on a dead link',
			signed_in({ preview: preview({ status: 'revoked', cord: CORD, role: 'cord_member' }) }),
			{ kind: 'member', cord: CORD }
		],
		[
			'signed in, on an active link',
			signed_in({ preview: preview({ cord: CORD }) }),
			{ kind: 'join', cord: CORD }
		],
		[
			'signed in, on an active newcomer link',
			signed_in({ preview: preview({ admits_newcomers: true, newcomers_open: true, cord: CORD }) }),
			{ kind: 'join', cord: CORD }
		],
		[
			'signed in after redeeming here, on a link the reservations used up',
			signed_in({ preview: preview({ status: 'exhausted' }), redeemed: REDEEMED }),
			{ kind: 'join', cord: CORD }
		],
		[
			'signed in after redeeming here, on an expired link accept may still take',
			signed_in({ preview: preview({ status: 'expired' }), redeemed: REDEEMED }),
			{ kind: 'join', cord: CORD }
		],
		[
			'signed in after redeeming here, on a revoked link',
			signed_in({ preview: preview({ status: 'revoked' }), redeemed: REDEEMED }),
			{ kind: 'dead' }
		],
		[
			'signed in as the reserved username in another case, on a used-up link',
			signed_in({
				account_username: 'NewComer',
				preview: preview({ status: 'exhausted' }),
				redeemed: REDEEMED
			}),
			{ kind: 'join', cord: CORD }
		],
		// a redemption answers only for the username it reserved
		[
			'signed in as another account after a redemption here, on a used-up link',
			signed_in({
				account_username: 'someone_else',
				preview: preview({ status: 'exhausted' }),
				redeemed: REDEEMED
			}),
			{ kind: 'dead' }
		],
		[
			'signed in as another account after a redemption here, on an expired link',
			signed_in({
				account_username: 'someone_else',
				preview: preview({ status: 'expired' }),
				redeemed: REDEEMED
			}),
			{ kind: 'dead' }
		],
		[
			'signed in as another account after a redemption here, on an active link',
			signed_in({
				account_username: 'someone_else',
				preview: preview({ cord: CORD }),
				redeemed: REDEEMED
			}),
			{ kind: 'join', cord: CORD }
		],
		[
			'signed in with no username loaded, after a redemption here',
			signed_in({
				account_username: null,
				preview: preview({ status: 'exhausted' }),
				redeemed: REDEEMED
			}),
			{ kind: 'dead' }
		],
		// signed out
		[
			'signed out, on an open newcomer link',
			signed_out({
				preview: preview({ admits_newcomers: true, newcomers_open: true, cord: CORD })
			}),
			{ kind: 'newcomer', cord: CORD }
		],
		[
			'signed out, on an ordinary link',
			signed_out({ preview: preview() }),
			{ kind: 'sign_in', newcomers_closed: false }
		],
		[
			'signed out, on a newcomer link whose minter is no longer an admin',
			signed_out({ preview: preview({ admits_newcomers: true, newcomers_open: false }) }),
			{ kind: 'sign_in', newcomers_closed: false }
		],
		[
			'signed out, after a redeem found the link closed to someone new',
			signed_out({
				preview: preview({ admits_newcomers: true, newcomers_open: false }),
				newcomers_closed: true
			}),
			{ kind: 'sign_in', newcomers_closed: true }
		],
		// signed out after redeeming here (a failed signup, or a reload)
		[
			'signed out after redeeming here, on a link the redemption used up',
			signed_out({
				preview: preview({ status: 'exhausted', admits_newcomers: true, newcomers_open: false }),
				redeemed: REDEEMED
			}),
			{ kind: 'newcomer', cord: CORD }
		],
		[
			'signed out after redeeming here, on an active link no longer open to newcomers',
			signed_out({
				preview: preview({ admits_newcomers: true, newcomers_open: false }),
				redeemed: REDEEMED
			}),
			{ kind: 'newcomer', cord: CORD }
		],
		[
			'signed out after redeeming here, on an expired link (redeem decides)',
			signed_out({ preview: preview({ status: 'expired' }), redeemed: REDEEMED }),
			{ kind: 'newcomer', cord: CORD }
		],
		[
			'signed out after redeeming here, on a revoked link',
			signed_out({ preview: preview({ status: 'revoked' }), redeemed: REDEEMED }),
			{ kind: 'dead' }
		],
		[
			'signed out after redeeming here, once redeem or accept called the link dead',
			signed_out({ dead: true, redeemed: REDEEMED }),
			{ kind: 'dead' }
		],
		// a re-preview for the same identity keeps showing the last one
		[
			'signed out, while a re-preview loads over an open newcomer link',
			signed_out({
				preview_status: 'pending',
				preview: preview({ admits_newcomers: true, newcomers_open: true, cord: CORD })
			}),
			{ kind: 'newcomer', cord: CORD }
		],
		[
			'signed out after redeeming here, while a re-preview loads over a used-up link',
			signed_out({
				preview_status: 'pending',
				preview: preview({ status: 'exhausted' }),
				redeemed: REDEEMED
			}),
			{ kind: 'newcomer', cord: CORD }
		],
		[
			'signed out, while a re-preview loads over an ordinary link',
			signed_out({ preview_status: 'pending', preview: preview() }),
			{ kind: 'sign_in', newcomers_closed: false }
		],
		[
			'signed in, while a re-preview loads over an active link',
			signed_in({ preview_status: 'pending', preview: preview({ cord: CORD }) }),
			{ kind: 'join', cord: CORD }
		],
		[
			'while a re-preview loads over a revoked link',
			signed_out({ preview_status: 'pending', preview: preview({ status: 'revoked' }) }),
			{ kind: 'dead' }
		],
		[
			'with the last preview for someone else, while theirs loads',
			signed_in({ preview_for: null, preview_status: 'pending', preview: preview({ cord: CORD }) }),
			{ kind: 'checking' }
		],
		// the newcomer flow
		[
			'while the flow signs up, before the account loads',
			signed_out({ verified: true, newcomer_running: true, redeemed: REDEEMED }),
			{ kind: 'newcomer', cord: CORD }
		],
		[
			'while the flow accepts, signed in on a stale preview',
			signed_in({ preview_for: null, newcomer_running: true, redeemed: REDEEMED }),
			{ kind: 'newcomer', cord: CORD }
		]
	];

	for (const [name, input, expected] of cases) {
		test(name, () => {
			assert.deepEqual(to_cord_join_view(input), expected);
		});
	}

	describe('to_cord_join_recheck_error', () => {
		const stale = preview({ admits_newcomers: true, newcomers_open: true, cord: CORD });
		const recheck_cases: Array<[string, CordJoinViewInput, string | null]> = [
			['with a current preview', signed_out({ preview: stale }), null],
			['while a re-preview loads', signed_out({ preview_status: 'pending', preview: stale }), null],
			[
				'when a re-preview fails for a reason that says nothing about the link',
				signed_out({
					preview_status: 'failure',
					preview: stale,
					preview_error_data: new TypeError('fetch failed')
				}),
				'fetch failed'
			],
			[
				'when a first preview fails (the error view says so)',
				signed_out({
					preview_status: 'failure',
					preview: undefined,
					preview_error_data: new TypeError('fetch failed')
				}),
				null
			],
			[
				'when a re-preview finds the link dead',
				signed_out({
					preview_status: 'failure',
					preview: stale,
					preview_error_data: rpc_error(ERROR_CORD_INVITE_EXPIRED)
				}),
				null
			],
			[
				'when the failed preview was for someone else',
				signed_in({
					preview_for: null,
					preview_status: 'failure',
					preview: stale,
					preview_error_data: new TypeError('fetch failed')
				}),
				null
			]
		];
		for (const [name, input, expected] of recheck_cases) {
			test(name, () => {
				assert.strictEqual(to_cord_join_recheck_error(input), expected);
			});
		}
	});
});

describe('cord_join_redemption_applies', () => {
	test('answers for anyone signed out, or the account with the reserved username', () => {
		assert.isFalse(cord_join_redemption_applies(null, false, null));
		assert.isTrue(cord_join_redemption_applies(REDEEMED, false, null));
		assert.isTrue(cord_join_redemption_applies(REDEEMED, true, 'newcomer'));
		assert.isTrue(cord_join_redemption_applies(REDEEMED, true, 'NewComer'));
		assert.isFalse(cord_join_redemption_applies(REDEEMED, true, 'someone_else'));
		assert.isFalse(cord_join_redemption_applies(REDEEMED, true, null));
	});
});

describe('check_cord_newcomer_fields', () => {
	const password = 'correct horse battery';

	test('passes good fields', () => {
		assert.isNull(check_cord_newcomer_fields('newcomer', password, password));
		assert.isNull(check_cord_newcomer_fields('  NewComer ', password, password));
	});

	test('names the first field that is wrong', () => {
		for (const [fields, field] of [
			[['ab', password, password], 'username'],
			[['1abc', password, password], 'username'],
			[['', password, password], 'username'],
			[['newcomer', 'short', 'short'], 'password'],
			[['newcomer', 'x'.repeat(301), 'x'.repeat(301)], 'password'],
			[['newcomer', password, `${password}!`], 'password_confirm']
		] as const) {
			assert.strictEqual(
				check_cord_newcomer_fields(fields[0], fields[1], fields[2])?.field,
				field,
				fields.join(' / ')
			);
		}
	});
});

describe('CordJoinState', () => {
	const cord_json = {
		...CORD,
		role: 'cord_member',
		created_at: '2026-01-01T00:00:00Z'
	} as CordJson;
	const password = 'correct horse battery';

	test('fetch_preview previews for the identity asked, and skips a token no link can have', async () => {
		const tokens: Array<string> = [];
		const state = new CordJoinState({
			api: stub_api({
				cord_invite_preview: (input: { token: string }) => {
					tokens.push(input.token);
					return Promise.resolve({
						status: 'active',
						admits_newcomers: false,
						newcomers_open: false,
						cord: CORD,
						role: null
					});
				}
			})
		});
		state.set_token(TOKEN);
		await state.fetch_preview(ACCOUNT);
		assert.strictEqual(state.preview_for, ACCOUNT);
		assert.strictEqual(state.preview.data?.cord?.name, 'the porch');
		state.set_token('x'.repeat(CORD_INVITE_TOKEN_LENGTH_MAX + 1));
		assert.isUndefined(state.preview_for, 'a new token forgets the old preview');
		await state.fetch_preview(null);
		assert.deepEqual(tokens, [TOKEN]);
	});

	test('accept_invite joins, and marks a dead link', async () => {
		const joining = new CordJoinState({
			api: stub_api({
				cord_invite_accept: () => Promise.resolve({ cord: cord_json, joined: true })
			})
		});
		joining.set_token(TOKEN);
		assert.strictEqual((await joining.accept_invite())?.id, CORD.id);

		const dead = new CordJoinState({
			api: stub_api({
				cord_invite_accept: () => Promise.reject(rpc_error(ERROR_CORD_INVITE_REVOKED))
			})
		});
		dead.set_token(TOKEN);
		assert.isUndefined(await dead.accept_invite());
		assert.isTrue(dead.dead);
	});

	test('join_as_newcomer runs redeem, signup, then accept, back to back', async () => {
		const steps: Array<string> = [];
		const state = new CordJoinState({
			api: stub_api({
				cord_invite_redeem: (input: { token: string; username: string }) => {
					steps.push(`redeem:${input.token}:${input.username}`);
					return Promise.resolve({ cord: CORD, username: 'newcomer', created: true });
				},
				cord_invite_accept: (input: { token: string }) => {
					steps.push(`accept:${input.token}`);
					return Promise.resolve({ cord: cord_json, joined: true });
				}
			})
		});
		state.set_token(TOKEN);
		const outcome = await state.join_as_newcomer(' NewComer ', password, password, (u, p) => {
			steps.push(`signup:${u}:${p === password}`);
			assert.isTrue(state.newcomer_running, 'the flow pins the view while it runs');
			return Promise.resolve(true);
		});
		assert.deepEqual(outcome, { kind: 'joined', cord: cord_json });
		// the server lowercases the username; signup takes the one it reserved
		assert.deepEqual(steps, [
			`redeem:${TOKEN}:NewComer`,
			'signup:newcomer:true',
			`accept:${TOKEN}`
		]);
		assert.isTrue(state.newcomer_running, 'stays pinned while the page moves on');
		assert.deepEqual(state.redeemed?.cord, CORD);
	});

	test('join_as_newcomer checks the fields before sending anything', async () => {
		const state = new CordJoinState({ api: stub_api({}) });
		state.set_token(TOKEN);
		const outcome = await state.join_as_newcomer('newcomer', 'short', 'short', () => {
			throw new Error('no signup expected');
		});
		assert.strictEqual(outcome.kind, 'invalid');
		assert.isFalse(state.newcomer_running);
	});

	test('join_as_newcomer keeps the form for a username the server refuses', async () => {
		for (const reason of [
			ERROR_CORD_INVITE_USERNAME_UNAVAILABLE,
			ERROR_CORD_INVITE_USERNAME_INVALID
		]) {
			const state = new CordJoinState({
				api: stub_api({ cord_invite_redeem: () => Promise.reject(rpc_error(reason)) })
			});
			state.set_token(TOKEN);
			const outcome = await state.join_as_newcomer('newcomer', password, password, () => {
				throw new Error('no signup expected');
			});
			assert.strictEqual(outcome.kind, 'username', reason);
			assert.isFalse(state.dead);
			assert.isFalse(state.newcomer_running);
		}
	});

	test('join_as_newcomer turns a dead link into the ask-for-a-new-link view', async () => {
		for (const reason of [
			ERROR_CORD_INVITE_EXPIRED,
			ERROR_CORD_INVITE_REVOKED,
			ERROR_CORD_INVITE_EXHAUSTED,
			ERROR_CORD_INVITE_NOT_FOUND
		]) {
			const state = new CordJoinState({
				api: stub_api({ cord_invite_redeem: () => Promise.reject(rpc_error(reason)) })
			});
			state.set_token(TOKEN);
			const outcome = await state.join_as_newcomer('newcomer', password, password, () => {
				throw new Error('no signup expected');
			});
			assert.strictEqual(outcome.kind, 'dead', reason);
			assert.isTrue(state.dead, reason);
		}
	});

	test('join_as_newcomer stops at a failed signup, and a retry redeems again', async () => {
		let redeems = 0;
		const state = new CordJoinState({
			api: stub_api({
				cord_invite_redeem: () => {
					redeems++;
					return Promise.resolve({ cord: CORD, username: 'newcomer', created: redeems === 1 });
				},
				cord_invite_accept: () => Promise.resolve({ cord: cord_json, joined: true })
			})
		});
		state.set_token(TOKEN);
		const first = await state.join_as_newcomer('newcomer', password, password, () =>
			Promise.resolve(false)
		);
		assert.deepEqual(first, { kind: 'signup_failed' });
		assert.isFalse(state.newcomer_running);
		const second = await state.join_as_newcomer('newcomer', password, password, () =>
			Promise.resolve(true)
		);
		assert.strictEqual(second.kind, 'joined');
		assert.strictEqual(redeems, 2);
	});

	test('join_as_newcomer reports a failed accept after signing up, for a retry', async () => {
		const state = new CordJoinState({
			api: stub_api({
				cord_invite_redeem: () =>
					Promise.resolve({ cord: CORD, username: 'newcomer', created: true }),
				cord_invite_accept: () => Promise.reject(new TypeError('fetch failed'))
			})
		});
		state.set_token(TOKEN);
		const outcome = await state.join_as_newcomer('newcomer', password, password, () =>
			Promise.resolve(true)
		);
		assert.deepEqual(outcome, { kind: 'accept_failed', message: 'fetch failed' });
		assert.isFalse(state.dead);
		assert.isFalse(state.newcomer_running);
		assert.deepEqual(state.redeemed?.cord, CORD, 'the signed-in view can still offer to join');
	});

	/** What the page would decide its view from for `state`, signed out or signed in as `ACCOUNT`. */
	const view_input_of = (
		state: CordJoinState,
		signed_in = false,
		account_username = 'newcomer'
	): CordJoinViewInput => ({
		token_loaded: state.token_loaded,
		token: state.token,
		session_checked: true,
		verified: signed_in,
		signed_in,
		account_id: signed_in ? ACCOUNT : null,
		account_username: signed_in ? account_username : null,
		preview_for: state.preview_for,
		preview_status: state.preview.status,
		preview: state.preview.data,
		preview_error_data: state.preview.error_data,
		dead: state.dead,
		newcomer_running: state.newcomer_running,
		redeemed: state.redeemed,
		newcomers_closed: state.newcomers_closed
	});

	/** The view the page would show for `state`; see `view_input_of`. */
	const view_of = (
		state: CordJoinState,
		signed_in = false,
		account_username = 'newcomer'
	): CordJoinView => to_cord_join_view(view_input_of(state, signed_in, account_username));

	const preview_output = (
		overrides: Partial<CordInvitePreviewOutput> = {}
	): CordInvitePreviewOutput => ({
		status: 'active',
		admits_newcomers: true,
		newcomers_open: true,
		cord: CORD,
		role: null,
		...overrides
	});

	/** An API whose preview answers from `previews` in turn, with redeem and accept as given. */
	const flow_api = (
		previews: Array<CordInvitePreviewOutput>,
		methods: Record<string, (input: any) => Promise<unknown>> = {}
	) =>
		stub_api({
			cord_invite_preview: () => Promise.resolve(previews.shift()),
			cord_invite_redeem: () =>
				Promise.resolve({ cord: CORD, username: 'newcomer', created: true }),
			...methods
		});

	test('a failed signup on a link the redemption used up keeps the form and the link', async () => {
		const { storage, stored } = memory_storage();
		const state = new CordJoinState({
			api: flow_api([
				preview_output(),
				// the redemption spent the last use
				preview_output({ status: 'exhausted', newcomers_open: false, cord: null })
			]),
			storage
		});
		state.load(`#${TOKEN}`);
		await state.fetch_preview(null);
		assert.deepEqual(view_of(state), { kind: 'newcomer', cord: CORD });
		const outcome = await state.join_as_newcomer('newcomer', password, password, () =>
			Promise.resolve(false)
		);
		assert.deepEqual(outcome, { kind: 'signup_failed' });
		// the page's effect previews again once the flow ends
		await state.fetch_preview(null);
		assert.strictEqual(state.preview.data?.status, 'exhausted');
		assert.deepEqual(view_of(state), { kind: 'newcomer', cord: CORD }, 'the form stays');
		assert.strictEqual(state.token, TOKEN);
		assert.deepEqual(stored(), { token: TOKEN, redeemed: REDEEMED }, 'the link stays stored');
	});

	test('a reload restores the redemption, which carries a used-up link', async () => {
		const { storage } = memory_storage({ token: TOKEN, redeemed: REDEEMED });
		const state = new CordJoinState({
			api: flow_api([preview_output({ status: 'exhausted', newcomers_open: false, cord: null })]),
			storage
		});
		state.load('');
		assert.strictEqual(state.token, TOKEN);
		assert.deepEqual(state.redeemed, REDEEMED);
		await state.fetch_preview(null);
		assert.deepEqual(view_of(state), { kind: 'newcomer', cord: CORD });
	});

	test('a restored redemption dies with a revoked link', async () => {
		const { storage } = memory_storage({ token: TOKEN, redeemed: REDEEMED });
		const state = new CordJoinState({
			api: flow_api([preview_output({ status: 'revoked', newcomers_open: false, cord: null })]),
			storage
		});
		state.load('');
		await state.fetch_preview(null);
		// the page forgets the stored link on a dead view
		assert.deepEqual(view_of(state), { kind: 'dead' });
	});

	test('a restored redemption on an expired link lasts until redeem refuses it', async () => {
		const { storage } = memory_storage({ token: TOKEN, redeemed: REDEEMED });
		const state = new CordJoinState({
			api: flow_api([preview_output({ status: 'expired', newcomers_open: false, cord: null })], {
				cord_invite_redeem: () => Promise.reject(rpc_error(ERROR_CORD_INVITE_EXPIRED))
			}),
			storage
		});
		state.load('');
		await state.fetch_preview(null);
		assert.deepEqual(view_of(state), { kind: 'newcomer', cord: CORD });
		const outcome = await state.join_as_newcomer('newcomer', password, password, () => {
			throw new Error('no signup expected');
		});
		assert.deepEqual(outcome, { kind: 'dead' });
		assert.deepEqual(view_of(state), { kind: 'dead' });
	});

	test('a link closed to newcomers sends a newcomer to sign in, keeping the link', async () => {
		const { storage, stored } = memory_storage({ token: TOKEN, redeemed: REDEEMED });
		const state = new CordJoinState({
			api: flow_api([preview_output({ newcomers_open: false, cord: null })], {
				cord_invite_redeem: () => Promise.reject(rpc_error(ERROR_CORD_INVITE_NEWCOMERS_CLOSED))
			}),
			storage
		});
		state.load('');
		const outcome = await state.join_as_newcomer('newcomer', password, password, () => {
			throw new Error('no signup expected');
		});
		assert.deepEqual(outcome, { kind: 'newcomers_closed' });
		assert.isFalse(state.dead);
		assert.isTrue(state.newcomers_closed);
		assert.isNull(state.redeemed);
		assert.deepEqual(stored(), { token: TOKEN, redeemed: null });
		await state.fetch_preview(null);
		assert.deepEqual(view_of(state), { kind: 'sign_in', newcomers_closed: true });
	});

	test('while the newcomer flow runs, a second submit and a preview do nothing', async () => {
		let redeems = 0;
		let previews = 0;
		let finish_signup!: (ok: boolean) => void;
		const state = new CordJoinState({
			api: stub_api({
				cord_invite_preview: () => {
					previews++;
					return Promise.resolve(preview_output());
				},
				cord_invite_redeem: () => {
					redeems++;
					return Promise.resolve({ cord: CORD, username: 'newcomer', created: true });
				},
				cord_invite_accept: () => Promise.resolve({ cord: cord_json, joined: true })
			})
		});
		state.set_token(TOKEN);
		const first = state.join_as_newcomer(
			'newcomer',
			password,
			password,
			() => new Promise<boolean>((resolve) => (finish_signup = resolve))
		);
		// let redeem settle, leaving the flow waiting on signup
		await new Promise((resolve) => setTimeout(resolve, 0));
		assert.isTrue(state.newcomer_running);
		const second = await state.join_as_newcomer('newcomer', password, password, () => {
			throw new Error('no second signup expected');
		});
		assert.deepEqual(second, { kind: 'busy' });
		await state.fetch_preview(ACCOUNT);
		assert.strictEqual(previews, 0, 'the flow answers for itself');
		assert.isUndefined(state.preview_for);
		finish_signup(true);
		assert.strictEqual((await first).kind, 'joined');
		assert.strictEqual(redeems, 1);
	});

	describe('the view never falls back to checking while the page re-previews after', () => {
		for (const [name, redeem, expected] of [
			[
				'a failed signup',
				() => Promise.resolve({ cord: CORD, username: 'newcomer', created: true }),
				'signup_failed'
			],
			[
				'a refused username',
				() => Promise.reject(rpc_error(ERROR_CORD_INVITE_USERNAME_UNAVAILABLE)),
				'username'
			],
			['a failed redeem', () => Promise.reject(new TypeError('fetch failed')), 'redeem_failed']
		] as const) {
			test(name, async () => {
				let finish_preview!: () => void;
				let previews = 0;
				const state = new CordJoinState({
					api: stub_api({
						// the first preview answers at once, the re-preview when released
						cord_invite_preview: () =>
							++previews === 1
								? Promise.resolve(preview_output())
								: new Promise((resolve) => {
										finish_preview = () => resolve(preview_output());
									}),
						cord_invite_redeem: redeem
					})
				});
				state.set_token(TOKEN);
				await state.fetch_preview(null);
				assert.deepEqual(view_of(state), { kind: 'newcomer', cord: CORD });
				const outcome = await state.join_as_newcomer('newcomer', password, password, () =>
					Promise.resolve(false)
				);
				assert.strictEqual(outcome.kind, expected);
				const redeem_error = state.redeem.error;
				// what the page's effect does once `newcomer_running` flips false
				const previewing = state.fetch_preview(null);
				assert.strictEqual(state.preview.status, 'pending');
				assert.deepEqual(view_of(state), { kind: 'newcomer', cord: CORD }, 'the form stays');
				finish_preview();
				await previewing;
				assert.deepEqual(view_of(state), { kind: 'newcomer', cord: CORD });
				assert.strictEqual(state.redeem.error, redeem_error, 'the error survives');
				if (expected !== 'signup_failed') assert.isNotNull(state.redeem.error);
			});
		}
	});

	test('a failed redeem and a failed re-preview keep the form, and a dead refusal ends it', async () => {
		const previews: Array<() => Promise<CordInvitePreviewOutput>> = [
			() => Promise.resolve(preview_output()),
			() => Promise.reject(new TypeError('fetch failed')),
			() => Promise.reject(rpc_error(ERROR_CORD_INVITE_EXPIRED))
		];
		const state = new CordJoinState({
			api: stub_api({
				cord_invite_preview: () => previews.shift()!(),
				cord_invite_redeem: () => Promise.reject(new TypeError('fetch failed'))
			})
		});
		state.set_token(TOKEN);
		await state.fetch_preview(null);
		assert.deepEqual(view_of(state), { kind: 'newcomer', cord: CORD });
		const outcome = await state.join_as_newcomer('newcomer', password, password, () => {
			throw new Error('no signup expected');
		});
		assert.deepEqual(outcome, { kind: 'redeem_failed', message: 'fetch failed' });
		// what the page's effect does once `newcomer_running` flips false
		await state.fetch_preview(null);
		assert.strictEqual(state.preview.status, 'failure');
		assert.deepEqual(view_of(state), { kind: 'newcomer', cord: CORD }, 'the form stays');
		assert.strictEqual(to_cord_join_recheck_error(view_input_of(state)), 'fetch failed');
		assert.strictEqual(state.redeem.error, 'fetch failed', 'the form keeps its error');
		// a later re-preview that finds the link dead still ends it
		await state.fetch_preview(null);
		assert.deepEqual(view_of(state), { kind: 'dead' });
		assert.isNull(to_cord_join_recheck_error(view_input_of(state)));
	});

	test('a rate-limited redeem keeps the form and says to wait', async () => {
		// the server's rate limit: its code and a retry_after, but no reason
		const rate_limited = Object.assign(new Error('rate limited'), {
			code: -32006,
			data: { retry_after: 600 }
		});
		const { storage, stored } = memory_storage({ token: TOKEN, redeemed: REDEEMED });
		const state = new CordJoinState({
			api: stub_api({
				cord_invite_preview: () => Promise.resolve(preview_output()),
				cord_invite_redeem: () => Promise.reject(rate_limited)
			}),
			storage
		});
		state.load('');
		await state.fetch_preview(null);
		const outcome = await state.join_as_newcomer(REDEEMED.username, password, password, () => {
			throw new Error('no signup expected');
		});
		assert.deepEqual(outcome, { kind: 'redeem_failed', message: CORD_RATE_LIMITED_MESSAGE });
		assert.isFalse(is_dead_link_error(rate_limited));
		assert.isFalse(state.dead);
		assert.deepEqual(view_of(state), { kind: 'newcomer', cord: REDEEMED.cord });
		assert.deepEqual(stored(), { token: TOKEN, redeemed: REDEEMED }, 'the reservation stays');
	});

	test('another account signed in leaves the stored redemption be, unused', async () => {
		const { storage, stored } = memory_storage({ token: TOKEN, redeemed: REDEEMED });
		const state = new CordJoinState({
			api: flow_api([preview_output({ status: 'exhausted', newcomers_open: false, cord: null })]),
			storage
		});
		state.load('');
		await state.fetch_preview(ACCOUNT);
		assert.deepEqual(view_of(state, true, 'someone_else'), { kind: 'dead' });
		// what the page's effect does on a dead view
		state.forget(true, 'someone_else');
		assert.deepEqual(stored(), { token: TOKEN, redeemed: REDEEMED }, 'kept for the newcomer');
		assert.deepEqual(state.redeemed, REDEEMED);
		// the newcomer, signed in, is answered by it
		assert.deepEqual(view_of(state, true, 'newcomer'), { kind: 'join', cord: CORD });
		state.forget(true, 'newcomer');
		assert.isNull(stored());
	});

	test('a preview for another identity is dropped before the next one loads', async () => {
		let finish_preview!: () => void;
		let calls = 0;
		const state = new CordJoinState({
			api: stub_api({
				cord_invite_preview: () =>
					++calls === 1
						? Promise.resolve(preview_output())
						: new Promise((resolve) => {
								finish_preview = () => resolve(preview_output({ cord: CORD }));
							})
			})
		});
		state.set_token(TOKEN);
		await state.fetch_preview(null);
		assert.deepEqual(view_of(state), { kind: 'newcomer', cord: CORD });
		const previewing = state.fetch_preview(ACCOUNT);
		assert.isUndefined(state.preview.data, "the signed-out preview isn't the account's");
		assert.deepEqual(view_of(state, true), { kind: 'checking' });
		finish_preview();
		await previewing;
		assert.deepEqual(view_of(state, true), { kind: 'join', cord: CORD });
	});

	test('a refusal of the reserved username drops the redemption, which no longer holds', async () => {
		const { storage, stored } = memory_storage({ token: TOKEN, redeemed: REDEEMED });
		const state = new CordJoinState({
			api: stub_api({
				cord_invite_redeem: () => Promise.reject(rpc_error(ERROR_CORD_INVITE_USERNAME_UNAVAILABLE))
			}),
			storage
		});
		state.load('');
		// another name, as only the state API allows, leaves the reservation be
		const other = await state.join_as_newcomer('another', password, password, () => {
			throw new Error('no signup expected');
		});
		assert.strictEqual(other.kind, 'username');
		assert.deepEqual(state.redeemed, REDEEMED);
		const outcome = await state.join_as_newcomer(' NewComer ', password, password, () => {
			throw new Error('no signup expected');
		});
		assert.strictEqual(outcome.kind, 'username');
		assert.isNull(state.redeemed);
		assert.deepEqual(stored(), { token: TOKEN, redeemed: null });
		assert.isFalse(state.dead);
	});

	test('forget_on_logout drops the redemption and the stored link, keeping the token', async () => {
		const { storage, stored } = memory_storage({ token: TOKEN, redeemed: REDEEMED });
		const state = new CordJoinState({ api: stub_api({}), storage });
		state.load('');
		state.forget_on_logout();
		assert.isNull(state.redeemed);
		assert.isNull(stored());
		assert.strictEqual(state.token, TOKEN);
	});

	test('load keeps the token in memory when storage is blocked and the fragment is gone', () => {
		const state = new CordJoinState({ api: stub_api({}), storage: blocked_storage });
		state.load(`#${TOKEN}`);
		assert.strictEqual(state.token, TOKEN);
		state.load('');
		assert.strictEqual(state.token, TOKEN);
	});
});
