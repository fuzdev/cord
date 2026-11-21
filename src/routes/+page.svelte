<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';
	import { onMount } from 'svelte';
	import { PUBLIC_CORD_SITE } from '$app/env/public';
	import { site_context } from '@fuzdev/fuz_ui/site.svelte.ts';

	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';
	import { cords_state_context } from '#lib/ui/cords_state.svelte.ts';
	import CordNameForm from '#lib/ui/CordNameForm.svelte';
	import { CORD_ROLE_LABELS } from '#lib/cord_roles.ts';

	const site = site_context.get();
	const auth_state = cord_auth_state_context.get();
	// the root layout loads it for whoever is signed in
	const cords_state = cords_state_context.get();

	onMount(() => {
		// the rail keeps the list for the visit; coming home refreshes it, once
		// the root layout's own load has settled
		if (cords_state.list.succeeded || cords_state.list.failed) void cords_state.fetch();
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
	<p class="font_size_lg">plug unplug</p>
	<p>
		Cord is a self-hosted place for your people: a family, a friend group, a club, a team. Voice,
		video, and screenshare rooms, live chat, and forums, with your Bluesky and ActivityPub feeds and
		the RSS, Atom, and JSON Feed sources you follow, in one instance you run yourself.
	</p>
	<p>
		Cord is pre-alpha and not ready to use: cords and their spaces exist, but nothing happens inside
		a space yet.
	</p>
	{#if PUBLIC_CORD_SITE}
		<p>
			Each instance runs its own Cord. The source, and how to run one, is at
			<a href={site.repo_url}>{site.repo_url}</a>.
		</p>
	{:else if !auth_state.session_checked}
		<p class="text_50">checking session...</p>
	{:else if auth_state.signed_in}
		<p>Signed in as <strong>{auth_state.account?.username}</strong>.</p>
		<section>
			<h2>your cords</h2>
			<!-- a refresh keeps the loaded list up, and a failure keeps its error
				through a retry -->
			{#if cords_state.list.error !== null}
				<Alert status="error"><p>Couldn't load your cords: {cords_state.list.error}</p></Alert>
			{/if}
			{#if !cords_state.loaded}
				{#if cords_state.list.error === null}
					<p class="text_50">loading cords...</p>
				{/if}
			{:else if cords_state.cords.length > 0}
				<ul>
					{#each cords_state.cords as cord (cord.id)}
						<li>
							<a href={resolve('/c/[cord_id]', { cord_id: cord.id })}>{cord.name}</a>
							<small class="text_50">{CORD_ROLE_LABELS[cord.role]}</small>
						</li>
					{/each}
				</ul>
			{:else if auth_state.is_admin}
				<p>You're not in any cords yet. Create the first one.</p>
			{:else}
				<p>You're not in any cords yet. Ask someone in one for an invite.</p>
			{/if}
		</section>
		{#if auth_state.is_admin && cords_state.loaded}
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
	{:else if auth_state.needs_bootstrap}
		<p>
			This instance has no accounts yet. Its operator can <a href={resolve('/admin')}>claim it</a>.
		</p>
	{/if}
</main>
