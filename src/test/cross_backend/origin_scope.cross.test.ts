import { inject } from 'vitest';
import {
	default_cross_process_setup,
	reconstruct_bootstrapped_handle
} from '@fuzdev/fuz_app/testing/cross_backend/setup.ts';
import { describe_origin_scope_cross_tests } from '@fuzdev/fuz_app/testing/cross_backend/origin_scope.ts';

import './cross_test_types.ts';

// fuz_app's shared origin-scope suite: the origin check covers paths outside
// `/api` — a foreign Origin on `GET /ready` or an unrouted path is refused, an
// absent or allowed one passes, and an over-cap `POST` to an unrouted path
// answers 413 ahead of the check. Gated on `capabilities.ready`, which cord
// advertises
const handle = reconstruct_bootstrapped_handle(inject('backend_handle'));

describe_origin_scope_cross_tests({
	setup_test: default_cross_process_setup(handle),
	capabilities: handle.config.capabilities,
	base_url: handle.config.base_url
});
