import { inject } from 'vitest';
import {
	default_cross_process_setup,
	reconstruct_bootstrapped_handle
} from '@fuzdev/fuz_app/testing/cross_backend/setup.ts';
import { describe_transport_composition_cross_tests } from '@fuzdev/fuz_app/testing/cross_backend/transport_composition.ts';

import './cross_test_types.ts';

// fuz_app's front-door table: on REST `POST`, RPC `POST` and `GET`, the
// WebSocket upgrade, and the audit stream, which refusal answers when a request
// trips several — a foreign, unreadable, or duplicated Origin → 403, an over-cap
// declared length with a foreign Origin → 413, no credential → 401, a narrowed
// api token → the surface's denial. The shared origin suite leaves its refusal
// cases to this table
const handle = reconstruct_bootstrapped_handle(inject('backend_handle'));

describe_transport_composition_cross_tests({
	setup_test: default_cross_process_setup(handle),
	capabilities: handle.config.capabilities,
	rpc_path: handle.config.rpc_path,
	base_url: handle.config.base_url,
	ws_path: handle.config.ws_path,
	sse_path: '/api/admin/audit/stream'
});
