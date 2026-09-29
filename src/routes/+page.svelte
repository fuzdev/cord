<script lang="ts">
	import { resolve } from '$app/paths';
	import { ROLE_ADMIN } from '@fuzdev/fuz_app/auth/role_schema.ts';

	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';

	const auth_state = cord_auth_state_context.get();

	const is_admin = $derived(auth_state.roles.includes(ROLE_ADMIN));
</script>

<main class="width_atmost_sm mx_auto p_lg">
	<h1>Cord</h1>
	<p>
		A self-hosted place for a friend group or small community to hang out: voice, video, and
		screenshare rooms, live chat, forums, and your Bluesky feeds, in one instance you run yourself.
	</p>
	<p>
		Cord is early. The design is settled and work has just begun, so there's nothing to join here
		yet.
	</p>
	{#if !auth_state.session_checked}
		<p class="text_50">checking session...</p>
	{:else if auth_state.signed_in}
		<p>Signed in as <strong>{auth_state.account?.username}</strong>.</p>
		<nav>
			<ul>
				<li><a href={resolve('/account')}>account</a></li>
				{#if is_admin}
					<li><a href={resolve('/admin')}>admin</a></li>
				{/if}
			</ul>
		</nav>
	{:else}
		{#if auth_state.needs_bootstrap}
			<p>
				This instance has no accounts yet. Its operator can
				<a href={resolve('/admin')}>claim it</a>.
			</p>
		{/if}
		<nav>
			<ul>
				<li><a href={resolve('/login')}>log in</a></li>
				<li><a href={resolve('/signup')}>sign up</a></li>
			</ul>
		</nav>
	{/if}
	<p><a href={resolve('/about')}>about</a></p>
</main>
