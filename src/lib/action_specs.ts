/**
 * Action specs for cord's frontend RPC client.
 *
 * Mirrors the action surface `cordd` mounts: the protocol actions, which run
 * over `/api/ws`, plus the spine's standard auth surface (account
 * self-service, admin, and role-grant offers) and cord's own actions
 * (`cord_action_specs.ts`), which run over `/api/rpc`.
 * Keep it in step with the server's action registry; the generated
 * `action_collections.ts` and `action_metatypes.ts` derive their types from
 * the same sources.
 *
 * @module
 */

import type { ActionSpecUnion } from '@fuzdev/fuz_app/actions/action_spec.ts';
import { protocol_action_specs } from '@fuzdev/fuz_app/actions/protocol.ts';
import { all_account_action_specs } from '@fuzdev/fuz_app/auth/account_action_specs.ts';
import { all_admin_action_specs } from '@fuzdev/fuz_app/auth/admin_action_specs.ts';
import { all_role_grant_offer_action_specs } from '@fuzdev/fuz_app/auth/role_grant_offer_action_specs.ts';

import { all_cord_domain_action_specs } from './cord_action_specs.ts';

/**
 * Every action spec the cord server mounts. The protocol actions (`heartbeat`,
 * `cancel`, `peer/ping`) work only over the `/api/ws` WebSocket; sent over
 * `/api/rpc` they fail with method-not-found or `peer_no_transport`.
 * They're listed so the typed surface matches the server's registry and is
 * ready for the WebSocket transport.
 */
export const all_cord_action_specs: Array<ActionSpecUnion> = [
	...protocol_action_specs,
	...all_account_action_specs,
	...all_admin_action_specs,
	...all_role_grant_offer_action_specs,
	...all_cord_domain_action_specs
];
