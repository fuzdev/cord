<script lang="ts">
	import 'virtual:fuz.css';
	import '@fuzdev/fuz_code/theme.css';
	import './style.css';

	import ThemeRoot from '@fuzdev/fuz_ui/ThemeRoot.svelte';
	import { SiteState, site_context } from '@fuzdev/fuz_ui/site.svelte.ts';
	import { auth_state_context } from '@fuzdev/fuz_app/ui/auth_state.svelte.ts';
	import { onMount, type Snippet } from 'svelte';
	import pkg_json from 'virtual:pkg.json';

	import { create_cord_rpc, cord_rpc_context } from '#lib/ui/rpc.ts';
	import { CordAuthState, cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';
	import { cord_join_session_storage, forget_cord_join } from '#lib/ui/cord_join_state.svelte.ts';

	const {
		children
	}: {
		children: Snippet;
	} = $props();

	// `glyph` and `repo_url` derive from `pkg_json`.
	site_context.set(new SiteState({ pkg_json }));

	// One RPC client and one auth state for the whole site, shared by context.
	// Pages prerender before any session exists; the browser checks it once
	// on mount, and `session_checked` marks when that settles. The one auth
	// state goes in both contexts: fuz_app's components read `auth_state_context`.
	cord_rpc_context.set(create_cord_rpc());
	const auth_state = cord_auth_state_context.set(new CordAuthState());
	auth_state_context.set(auth_state);

	// a join link and its redemption stored in this tab belong to whoever was
	// signed in or signing up, so a logout through `auth_state.logout` on any
	// page forgets them; a session that ends another way (fuz_app's
	// `AccountSessions` revoking them all, an expiry) doesn't call it
	auth_state.on_logout(() => forget_cord_join(cord_join_session_storage()));

	onMount(() => {
		void auth_state.check_session();
	});
</script>

<svelte:head>
	<title>Cord</title>
</svelte:head>

<ThemeRoot>
	{@render children()}
</ThemeRoot>
