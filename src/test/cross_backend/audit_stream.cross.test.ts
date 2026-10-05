import { inject } from 'vitest';
import {
	default_cross_process_setup,
	reconstruct_bootstrapped_handle
} from '@fuzdev/fuz_app/testing/cross_backend/setup.ts';
import { describe_cross_process_sse_tests } from '@fuzdev/fuz_app/testing/cross_backend/sse_round_trip.ts';

import './cross_test_types.ts';

// fuz_app's shared SSE suite over `GET /api/admin/audit/stream` with the
// keeper (an admin) signed in: the connect comment, an audit data frame, the
// stream closing when its session is revoked or its account deleted, and the
// per-session stream cap. The gate's refusals are in `auth.cross.test.ts`.
const handle = reconstruct_bootstrapped_handle(inject('backend_handle'));

describe_cross_process_sse_tests({
	setup_test: default_cross_process_setup(handle),
	capabilities: handle.config.capabilities,
	base_url: handle.config.base_url,
	sse_path: '/api/admin/audit/stream',
	rpc_path: handle.config.rpc_path
});
