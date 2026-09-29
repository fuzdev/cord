import { configs } from '@ryanatkn/eslint-config';

// `zap.ts` is the deploy config zap evaluates under Deno, outside the
// website's TS project (see `tsconfig.json`'s `exclude`); `deno task
// check:zap` typechecks it instead.
export default [...configs, { ignores: ['zap.ts'] }];
