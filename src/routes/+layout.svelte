<script lang="ts">
	import 'virtual:fuz.css';
	import '@fuzdev/fuz_code/theme.css';

	import ThemeRoot from '@fuzdev/fuz_ui/ThemeRoot.svelte';
	import { SiteState, site_context } from '@fuzdev/fuz_ui/site.svelte.ts';
	import { auth_state_context } from '@fuzdev/fuz_app/ui/auth_state.svelte.ts';
	import AppShell from '@fuzdev/fuz_ui/AppShell.svelte';
	import { onMount, type Snippet } from 'svelte';
	import pkg_json from 'virtual:pkg.json';
	import { PUBLIC_CORD_SITE } from '$app/env/public';

	import { create_cord_rpc, cord_rpc_context } from '#lib/ui/rpc.ts';
	import { CordAuthState, cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';
	import { CordSocket } from '#lib/ui/cord_socket.svelte.ts';
	import { cord_join_session_storage, forget_cord_join } from '#lib/ui/cord_join_state.svelte.ts';
	import { CordsState, cords_state_context } from '#lib/ui/cords_state.svelte.ts';
	import { CordSpacesState, cord_spaces_state_context } from '#lib/ui/cord_spaces_state.svelte.ts';
	import {
		CordSpaceTabs,
		cord_space_tabs_context,
		cord_space_tabs_local_storage
	} from '#lib/ui/cord_space_tabs.svelte.ts';
	import CordNav from '#lib/ui/CordNav.svelte';

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
	const auth_state = cord_auth_state_context.set(new CordAuthState());
	auth_state_context.set(auth_state);
	// the signed-in account's WebSocket, which carries all of its requests;
	// when the server may have ended the session, it asks, and a session
	// that's gone, or now someone else's, gets the check a reload would run
	const signed_in_account_id = (): string | null =>
		auth_state.signed_in ? (auth_state.account?.id ?? null) : null;
	const socket = new CordSocket({
		probe_session: (account_id, signal) => auth_state.probe_session(account_id, signal),
		end_session: () => auth_state.check_session(),
		get_account_id: signed_in_account_id
	});
	const { api } = cord_rpc_context.set(create_cord_rpc({ socket }));

	// a join link and its redemption stored in this tab belong to whoever was
	// signed in or signing up, so a logout through `auth_state.logout` on any
	// page forgets them, as does fuz_app's `AccountSessions` when it revokes
	// this session or all of them; a session that ends another way (an
	// expiry, a revocation from elsewhere) doesn't call it
	auth_state.on_logout(() => forget_cord_join(cord_join_session_storage()));

	// the signed-in account's cords, shared by the rail and the home page:
	// forgotten whenever the account changes, then loaded for whoever signed in
	const cords_state = cords_state_context.set(new CordsState({ api }));
	// the spaces of the cord you're on, and its space tabs, live here rather
	// than in the cord layout because the rail nests them; the cord layout
	// keys both to the route's cord
	const spaces_state = cord_spaces_state_context.set(new CordSpacesState({ api }));
	const space_tabs = cord_space_tabs_context.set(
		new CordSpaceTabs({ get_storage: cord_space_tabs_local_storage })
	);
	const account_id = $derived(signed_in_account_id());
	// the socket is open while an account is signed in, replaced when another
	// signs in; effects don't run while prerendering, so no socket opens there.
	// It runs before the fetch below, though a request made first would open
	// the socket itself (`get_account_id`).
	$effect(() => {
		socket.set_account(account_id);
	});
	$effect(() => {
		cords_state.reset();
		spaces_state.reset();
		space_tabs.reset();
		if (account_id) void cords_state.fetch();
	});
	// a 4001 close, or reconnects that keep failing: ask whether the session
	// is still live
	$effect(() => {
		socket.recheck_session_if_in_doubt();
	});

	// the static site has no server, so no session is ever checked: it stays
	// unsettled, no account signs in, and none of the effects above sends a
	// request
	onMount(() => {
		if (!PUBLIC_CORD_SITE) void auth_state.check_session();
	});
</script>

<svelte:head>
	<title>Cord</title>
</svelte:head>

<ThemeRoot>
	<AppShell keyboard_shortcut="`">
		{#snippet sidebar()}
			<CordNav />
		{/snippet}
		{@render children()}
	</AppShell>
</ThemeRoot>
