import { prepare_cargo_binary } from '@fuzdev/fuz_app/testing/cross_backend/cargo_binary.ts';
import { create_cross_backend_global_setup } from '@fuzdev/fuz_app/testing/cross_backend/create_cross_backend_global_setup.ts';

import { CORD_CARGO_BINARY, rust_backend_config } from './cord_backend_config.ts';

// Builds `testing_cordd`, creates its Postgres database when missing, spawns
// it once per run, bootstraps the keeper through the bootstrap token, and
// provides the serialized handle as `backend_handle`.
export default create_cross_backend_global_setup({
	configs: { rust: rust_backend_config },
	ensure_databases: true,
	before_spawn: { rust: () => prepare_cargo_binary(CORD_CARGO_BINARY) }
});
