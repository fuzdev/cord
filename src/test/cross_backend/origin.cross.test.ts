import { inject } from 'vitest';
import {
	default_cross_process_setup,
	reconstruct_bootstrapped_handle
} from '@fuzdev/fuz_app/testing/cross_backend/setup.ts';
import { describe_origin_cross_tests } from '@fuzdev/fuz_app/testing/cross_backend/origin.ts';

import './cross_test_types.ts';

// fuz_app's shared origin suite: a disallowed Origin is refused before
// dispatch, and a request with no Origin (a non-browser caller) passes
const handle = reconstruct_bootstrapped_handle(inject('backend_handle'));

describe_origin_cross_tests({
	setup_test: default_cross_process_setup(handle),
	rpc_path: handle.config.rpc_path
});
