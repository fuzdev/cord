import { create_cross_backend_global_setup } from '@fuzdev/fuz_app/testing/cross_backend/create_cross_backend_global_setup.ts';

import { CORD_CARGO_BINARY, rust_backend_config } from './cord_backend_config.ts';

// Builds `testing_cordd`, opens its database (a `pglet_server` built in cord's
// Cargo workspace, or a PostgreSQL database under `FUZ_TEST_POSTGRES_URL`),
// spawns it once per run, bootstraps the keeper through the bootstrap token,
// and provides the serialized handle as `backend_handle`.
export default create_cross_backend_global_setup({
	configs: { rust: rust_backend_config },
	cargo_binaries: { rust: CORD_CARGO_BINARY }
});
