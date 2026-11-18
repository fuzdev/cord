<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';

	import { CORD_ROLE_LABELS } from '#lib/cord_roles.ts';
	import { CordSpaceName } from '#lib/cord_action_specs.ts';
	import { cord_page_state_context } from '#lib/ui/cord_page_state.svelte.ts';
	import { cord_spaces_state_context } from '#lib/ui/cord_spaces_state.svelte.ts';
	import CordNameForm from '#lib/ui/CordNameForm.svelte';

	// the layout mounts this page only for a loaded cord and the viewer's role in it
	const cord_page = cord_page_state_context.get();
	const spaces_state = cord_spaces_state_context.get();

	// opening the overview refreshes the list, so a space added or deleted
	// elsewhere catches up (and its tab drops); a refresh keeps the loaded list
	// up, and `refresh` reads nothing trackably, so this reruns only when the
	// cord changes
	const cord_id = $derived(cord_page.cord?.id);
	$effect(() => {
		if (cord_id) void spaces_state.refresh(cord_id);
	});

	const spaces = $derived(cord_id ? spaces_state.loaded_spaces(cord_id) : null);
	const list_error = $derived(spaces_state.cord_id === cord_id ? spaces_state.list.error : null);

	// a new line opens in the preview tab, by its URL, unless the route has
	// moved on while the create ran
	const add_line = async (name: string): Promise<boolean> => {
		const space = await spaces_state.create_space('line', name);
		if (!space) return false;
		if (page.route.id !== '/c/[cord_id]' || cord_page.cord_id !== space.cord_id) return true;
		await goto(
			resolve('/c/[cord_id]/s/[space_id]', { cord_id: space.cord_id, space_id: space.id })
		);
		return true;
	};
</script>

<svelte:head>
	<title>{cord_page.cord ? `${cord_page.cord.name} - Cord` : 'Cord'}</title>
</svelte:head>

{#if cord_page.role}
	<p>You're its <strong>{CORD_ROLE_LABELS[cord_page.role]}</strong>.</p>
{/if}
<section>
	<h2>spaces</h2>
	<!-- a refresh keeps the loaded list up, and its error shows beside it -->
	{#if list_error !== null}
		<Alert status="error"><p>Couldn't load the spaces: {list_error}</p></Alert>
	{/if}
	{#if !cord_id || !spaces}
		{#if list_error === null}
			<p class="text_50">loading spaces...</p>
		{/if}
	{:else if spaces.length > 0}
		<ul>
			{#each spaces as space (space.id)}
				<li>
					<a href={resolve('/c/[cord_id]/s/[space_id]', { cord_id, space_id: space.id })}>
						{space.name}
					</a>
					<small class="text_50">{space.kind}</small>
				</li>
			{/each}
		</ul>
	{:else if cord_page.can_manage}
		<p>
			No spaces yet. Add a line to begin: a live room for voice, video, and screenshare, though the
			room itself isn't here yet.
		</p>
	{:else}
		<p>No spaces yet. The cord's owners and admins add them.</p>
	{/if}
</section>
{#if cord_page.can_manage && spaces}
	<section>
		<h2>add a line</h2>
		<CordNameForm
			onsubmit={add_line}
			submit_label="add line"
			schema={CordSpaceName}
			placeholder="line name"
			clear_on_success
			pending={spaces_state.create.loading}
			error={spaces_state.create.error}
		/>
	</section>
{/if}
