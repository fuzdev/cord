<script lang="ts">
	import { resolve } from '$app/paths';
	import LoginForm from '@fuzdev/fuz_app/ui/LoginForm.svelte';
	import LogoutButton from '@fuzdev/fuz_app/ui/LogoutButton.svelte';

	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';

	const auth_state = cord_auth_state_context.get();
</script>

<svelte:head>
	<title>log in - Cord</title>
</svelte:head>

<main class="width_atmost_sm mx_auto p_lg">
	{#if !auth_state.session_checked}
		<p class="text_50">checking session...</p>
	{:else if auth_state.signed_in}
		<p>You're signed in as <strong>{auth_state.account?.username}</strong>.</p>
		<LogoutButton />
		<p>Back to the <a href={resolve('/')}>home page</a>.</p>
	{:else if auth_state.needs_bootstrap}
		<h1>log in</h1>
		<p>
			This instance has no accounts yet. Its operator can <a href={resolve('/admin')}>claim it</a>.
		</p>
	{:else}
		<h1>log in</h1>
		<LoginForm username_label="username" />
		<p>New here? <a href={resolve('/signup')}>Sign up</a></p>
	{/if}
</main>
