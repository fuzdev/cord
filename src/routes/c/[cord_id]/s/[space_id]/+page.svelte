<script lang="ts">
	/**
	 * A space's page: its name and kind, the keep control for its preview
	 * tab, and for the cord's owners and admins, renaming and deleting it.
	 * A space id the cord's loaded list doesn't have, or a malformed one,
	 * shows the not-found view; a space deleted from its own page shows a
	 * quiet line instead until the move to the overview lands.
	 *
	 * @module
	 */

	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';
	import ConfirmButton from '@fuzdev/fuz_app/ui/ConfirmButton.svelte';
	import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';

	import { CordSpaceName } from '#lib/cord_action_specs.ts';
	import { cord_page_state_context } from '#lib/ui/cord_page_state.svelte.ts';
	import { cord_spaces_state_context } from '#lib/ui/cord_spaces_state.svelte.ts';
	import { cord_space_tabs_context } from '#lib/ui/cord_space_tabs.svelte.ts';
	import { to_uuid_param } from '#lib/ui/route_params.ts';
	import CordNameForm from '#lib/ui/CordNameForm.svelte';

	// the layout mounts this page only for a loaded cord and the viewer's role
	// in it, and loads the cord's spaces
	const cord_page = cord_page_state_context.get();
	const spaces_state = cord_spaces_state_context.get();
	const space_tabs = cord_space_tabs_context.get();

	const cord = $derived(cord_page.cord);
	const space_id = $derived(to_uuid_param(page.params.space_id));
	const spaces = $derived(cord ? spaces_state.loaded_spaces(cord.id) : null);
	const space = $derived(cord && space_id ? spaces_state.get_space(cord.id, space_id) : undefined);

	// a space deleted here leaves the list before the navigation away lands,
	// so its page holds a quiet view meanwhile rather than flashing not-found
	let deleting_id: Uuid | null = $state.raw(null);
	const deleted = $derived(!!space_id && space_id === deleting_id && !space);

	// a malformed id names no space, without waiting on the list
	const not_found = $derived(!deleted && (!space_id || (!!spaces && !space)));
	const list_error = $derived(cord ? spaces_state.list_error_for(cord.id) : null);
	const preview = $derived(!!space && space_tabs.preview_id === space.id);

	// the keep control leaves once the tab is permanent, so focus moves to
	// the space's heading rather than falling to the page
	let heading: HTMLElement | undefined = $state.raw();
	const keep_tab = (): void => {
		if (space_tabs.promote()) heading?.focus();
	};

	// goes to the overview only while the route still shows the deleted space
	const delete_space = async (): Promise<void> => {
		const s = space;
		if (!s) return;
		deleting_id = s.id;
		const gone = await spaces_state.delete_space(s.id);
		if (gone && space_id === s.id) {
			await goto(resolve('/c/[cord_id]', { cord_id: s.cord_id }));
			return;
		}
		deleting_id = null;
	};
</script>

<!-- the cord layout's other gates title themselves the same way -->
<svelte:head>
	<title>
		{not_found
			? 'space not found - Cord'
			: space && cord
				? `${space.name} - ${cord.name} - Cord`
				: cord
					? `${cord.name} - Cord`
					: 'Cord'}
	</title>
</svelte:head>

{#if deleted}
	<p class="text_50">deleted</p>
{:else if not_found}
	<h2>space not found</h2>
	<p>There's no space here. It may have been deleted.</p>
{:else if !space}
	{#if list_error !== null}
		<Alert status="error"><p>Couldn't load the spaces: {list_error}</p></Alert>
	{:else}
		<p class="text_50">loading space...</p>
	{/if}
{:else}
	<section>
		<h2 bind:this={heading} tabindex="-1">{space.name}</h2>
		<p>
			A line: a live room for voice, video, and screenshare, with text that isn't kept. The room
			isn't here yet, so for now there's nothing to join.
		</p>
		{#if preview}
			<button type="button" class="sized_sm" onclick={keep_tab}>keep as a permanent tab</button>
		{/if}
	</section>
	{#if cord_page.can_manage}
		<section>
			<h3>rename</h3>
			{#key space.id}
				<CordNameForm
					onsubmit={(name) => spaces_state.rename_space(space.id, name)}
					submit_label="rename"
					schema={CordSpaceName}
					placeholder="line name"
					initial_name={space.name}
					pending={spaces_state.rename.loading(space.id)}
					error={spaces_state.rename.error(space.id)}
				/>
			{/key}
		</section>
		<section>
			<h3>delete</h3>
			<p>Deleting this line takes it away for everyone in the cord.</p>
			<ConfirmButton
				onconfirm={delete_space}
				title="delete this line"
				label="delete this line"
				pending={spaces_state.delete.loading(space.id)}
			>
				{#snippet popover_content(_popover, do_confirm)}
					<button
						type="button"
						class="palette_c background-color:var(--paper_100)"
						onclick={() => do_confirm()}
					>
						<span class="py_sm">yes, delete it</span>
					</button>
				{/snippet}
			</ConfirmButton>
			{#if spaces_state.delete.error(space.id)}
				<p class="color_c_50 font_size_sm">{spaces_state.delete.error(space.id)}</p>
			{/if}
		</section>
	{/if}
{/if}
