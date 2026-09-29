import type { SerializableBootstrappedBackendHandle } from '@fuzdev/fuz_app/testing/cross_backend/setup.ts';

// types `inject('backend_handle')`, the serialized bootstrapped handle
// `global_setup.ts` provides
declare module 'vitest' {
	export interface ProvidedContext {
		backend_handle: SerializableBootstrappedBackendHandle;
	}
}
