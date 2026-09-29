import { assert, describe, test } from 'vitest';

import { create_cord_rpc } from '#lib/ui/rpc.ts';

describe('create_cord_rpc', () => {
	test('exposes the methods the auth shell calls', () => {
		// the Result proxy resolves only spec'd methods; the throwing proxy wraps any name
		const { api_result } = create_cord_rpc();
		for (const method of [
			'account_verify',
			'account_session_list',
			'account_session_revoke',
			'account_session_revoke_all',
			'invite_create',
			'invite_list',
			'invite_delete',
			'admin_account_list',
			'admin_session_list',
			'audit_log_list',
			'app_settings_get',
			'cord_create',
			'cord_list',
			'cord_get',
			'cord_update',
			'cord_admin_list'
		] as const) {
			assert.typeOf(api_result[method], 'function', method);
		}
	});

	test('a method outside the specs resolves to undefined', () => {
		const { api_result } = create_cord_rpc();
		assert.isUndefined((api_result as unknown as Record<string, unknown>).cell_list);
	});
});
