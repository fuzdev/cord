/**
 * Shared steps for the cord cross-backend suites against a spawned
 * `testing_cordd`: the per-test setup, calling a verb as a given account,
 * asserting a refusal's reason, and the cord and link steps most tests
 * start with. Not a test file itself: the cross-backend project runs only
 * `*.cross.test.ts`.
 *
 * @module
 */

import { assert } from 'vitest';
import type { TestAccount } from '@fuzdev/fuz_app/testing/test_app.ts';
import { inject_backend_handle } from '@fuzdev/fuz_app/testing/cross_backend/provided_context.ts';
import { default_cross_process_setup } from '@fuzdev/fuz_app/testing/cross_backend/setup.ts';
import {
	cross_rpc_call,
	error_reason,
	expect_output
} from '@fuzdev/fuz_app/testing/cross_backend/cross_rpc.ts';
import { AuditLogListOutput } from '@fuzdev/fuz_app/auth/admin_action_specs.ts';
import { JSONRPC_ERROR_CODES } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';

import { CordCreateOutput, CordInviteCreateOutput, type CordJson } from '#lib/cord_action_specs.ts';

const handle = inject_backend_handle();

/** Fire `_testing_reset` for a fresh keeper (an instance admin) and no cords. */
export const setup_test = default_cross_process_setup(handle);

export const { rpc_path } = handle.config;

/** A test's fixture: the keeper, and how to make other accounts. */
export type Fixture = Awaited<ReturnType<typeof setup_test>>;

/** Whoever makes a call: the keeper's fixture or another account. */
export type Caller = Fixture | TestAccount;

/** Call `method` as `caller` on a clean cookie jar. */
export const call = (
	fixture: Fixture,
	caller: Caller,
	method: string,
	params: unknown
): ReturnType<typeof cross_rpc_call> =>
	cross_rpc_call(
		fixture.fresh_transport(),
		rpc_path,
		method,
		params,
		caller.create_session_headers()
	);

/** Assert a call failed with `reason` (and `code`, when given). */
export const assert_refused = (
	res: Awaited<ReturnType<typeof cross_rpc_call>>,
	reason: string,
	code?: number
): void => {
	assert.ok(!res.ok, `expected ${reason}, got ${JSON.stringify(res.result)}`);
	assert.strictEqual(error_reason(res), reason);
	if (code !== undefined) assert.strictEqual(res.error?.code, code);
};

/**
 * Assert `res` is the refusal a spec's input schema answers: `invalid params`
 * with no reason.
 */
export const assert_invalid_params = (
	res: Awaited<ReturnType<typeof cross_rpc_call>>,
	label: string
): void => {
	assert.ok(!res.ok, `${label}: expected invalid params, got ${JSON.stringify(res.result)}`);
	assert.strictEqual(res.error?.code, JSONRPC_ERROR_CODES.invalid_params, label);
	assert.strictEqual(error_reason(res), undefined, `${label}: no reason`);
};

/** A JSON `POST` request, with any extra `headers`. */
export const json_post = (body: unknown, headers: Record<string, string> = {}): RequestInit => ({
	method: 'POST',
	headers: { 'content-type': 'application/json', ...headers },
	body: JSON.stringify(body)
});

/** Create a cord as the keeper, who owns it. */
export const create_cord = async (fixture: Fixture, name = 'the porch'): Promise<CordJson> =>
	expect_output(await call(fixture, fixture, 'cord_create', { name }), CordCreateOutput).cord;

/** Mint a link on `cord_id` as `caller`. */
export const mint = async (
	fixture: Fixture,
	caller: Caller,
	cord_id: string,
	options: { expires_in_seconds?: number; max_uses?: number; admits_newcomers?: boolean } = {}
): Promise<CordInviteCreateOutput> =>
	expect_output(
		await call(fixture, caller, 'cord_invite_create', { cord_id, ...options }),
		CordInviteCreateOutput
	);

/** Audit events of `event_type`, oldest first, read by the keeper. */
export const audit_events = async (
	fixture: Fixture,
	event_type: string
): Promise<AuditLogListOutput['events']> =>
	expect_output(
		await call(fixture, fixture, 'audit_log_list', { event_type }),
		AuditLogListOutput
	).events.reverse();
