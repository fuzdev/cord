<script lang="ts">
	import { resolve } from '$app/paths';
	import SignupForm from '@fuzdev/fuz_app/ui/SignupForm.svelte';
	import LogoutButton from '@fuzdev/fuz_app/ui/LogoutButton.svelte';

	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';

	const auth_state = cord_auth_state_context.get();
</script>

<svelte:head>
	<title>sign up - Cord</title>
</svelte:head>

<main class="width_atmost_sm mx_auto p_lg">
	{#if !auth_state.session_checked}
		<p class="text_50">checking session...</p>
	{:else if auth_state.signed_in}
		<p>You're signed in as <strong>{auth_state.account?.username}</strong>.</p>
		<LogoutButton />
	{:else if auth_state.needs_bootstrap}
		<h1>sign up</h1>
		<p>
			This instance has no accounts yet. Its operator can <a href={resolve('/admin')}>claim it</a>.
		</p>
	{:else}
		<h1>sign up</h1>
		<p>
			Signing up usually takes an invite from this instance's admin, matching your username or
			email.
		</p>
		<SignupForm />
		<p>Already have an account? <a href={resolve('/login')}>Log in</a></p>
	{/if}
</main>
