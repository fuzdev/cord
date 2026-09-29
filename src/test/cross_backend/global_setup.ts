import { create_cross_backend_global_setup } from '@fuzdev/fuz_app/testing/cross_backend/create_cross_backend_global_setup.ts';

import './cross_test_types.ts';
import { rust_backend_config } from './cord_backend_config.ts';

// Spawns `testing_cordd` once per run, bootstraps the keeper through the
// bootstrap token, and provides the serialized handle as `backend_handle`.
export default create_cross_backend_global_setup({
	configs: { rust: rust_backend_config }
});
