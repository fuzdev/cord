<script lang="ts">
	import { resolve } from '$app/paths';
	import { account_sessions_rpc_context } from '@fuzdev/fuz_app/ui/account_sessions_state.svelte.ts';
	import AccountSessions from '@fuzdev/fuz_app/ui/AccountSessions.svelte';
	import LogoutButton from '@fuzdev/fuz_app/ui/LogoutButton.svelte';

	import { cord_rpc_context } from '#lib/ui/rpc.ts';
	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';

	const auth_state = cord_auth_state_context.get();
	const { api } = cord_rpc_context.get();

	// `AccountSessions` reads its RPC through this context.
	account_sessions_rpc_context.set(() => ({
		list: () => api.account_session_list(),
		revoke: (params) => api.account_session_revoke(params),
		revoke_all: () => api.account_session_revoke_all()
	}));
</script>

<svelte:head>
	<title>account - Cord</title>
</svelte:head>

<main class="width_atmost_md mx_auto p_lg">
	<h1>account</h1>
	{#if !auth_state.session_checked}
		<p class="text_50">checking session...</p>
	{:else if auth_state.signed_in}
		<p>Signed in as <strong>{auth_state.account?.username}</strong>.</p>
		<LogoutButton />
		<AccountSessions />
	{:else}
		<p>You're not signed in. <a href={resolve('/login')}>Log in</a></p>
	{/if}
</main>
