<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';

	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';
	import { cord_rpc_context } from '#lib/ui/rpc.ts';
	import { CordsState } from '#lib/ui/cords_state.svelte.ts';
	import CordNameForm from '#lib/ui/CordNameForm.svelte';
	import { CORD_ROLE_LABELS } from '#lib/cord_roles.ts';

	const auth_state = cord_auth_state_context.get();
	const { api } = cord_rpc_context.get();

	const cords_state = new CordsState({ api });

	// load the cords for whoever is signed in, and forget them on sign-out
	const account_id = $derived(auth_state.signed_in ? auth_state.account?.id : undefined);
	$effect(() => {
		if (account_id) {
			void cords_state.fetch();
		} else {
			cords_state.reset();
		}
	});

	const create_cord = async (name: string): Promise<boolean> => {
		const cord = await cords_state.create_cord(name);
		if (!cord) return false;
		await goto(resolve('/c/[cord_id]', { cord_id: cord.id }));
		return true;
	};
</script>

<main class="width_atmost_sm mx_auto p_lg">
	<h1>Cord</h1>
	<p>
		A self-hosted place for a friend group or small community to hang out: voice, video, and
		screenshare rooms, live chat, forums, and your Bluesky feeds, in one instance you run yourself.
	</p>
	<p>Cord is early: cords exist, but there's nothing to do inside one yet.</p>
	{#if !auth_state.session_checked}
		<p class="text_50">checking session...</p>
	{:else if auth_state.signed_in}
		<p>Signed in as <strong>{auth_state.account?.username}</strong>.</p>
		<section>
			<h2>your cords</h2>
			{#if cords_state.list.failed}
				<Alert status="error"><p>Couldn't load your cords: {cords_state.list.error}</p></Alert>
			{:else if !cords_state.list.succeeded}
				<p class="text_50">loading cords...</p>
			{:else if cords_state.cords.length > 0}
				<ul>
					{#each cords_state.cords as cord (cord.id)}
						<li>
							<a href={resolve('/c/[cord_id]', { cord_id: cord.id })}>{cord.name}</a>
							<small class="text_50">{CORD_ROLE_LABELS[cord.role]}</small>
						</li>
					{/each}
				</ul>
			{:else if auth_state.is_instance_admin}
				<p>You're not in any cords yet. Create the first one.</p>
			{:else}
				<p>You're not in any cords yet. Ask someone in one for an invite.</p>
			{/if}
		</section>
		{#if auth_state.is_instance_admin && cords_state.list.succeeded}
			<section>
				<h2>{cords_state.cords.length > 0 ? 'new cord' : 'create your first cord'}</h2>
				<CordNameForm
					onsubmit={create_cord}
					submit_label="create cord"
					clear_on_success
					pending={cords_state.create.loading}
					error={cords_state.create.error}
				/>
			</section>
		{/if}
		<nav>
			<ul>
				<li><a href={resolve('/account')}>account</a></li>
				{#if auth_state.is_instance_admin}
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
