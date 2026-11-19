<script lang="ts">
	/**
	 * A cord's space tabs, rendered inside the cord layout's `Tabs` after its
	 * management tabs, and the wiring that keeps them: it keys the root's
	 * `CordSpaceTabs` to the viewer's account and the cord, drops the tabs of
	 * spaces the cord's loaded list lacks whenever it loads or changes, and
	 * opens the route's space in the preview tab unless it's permanent.
	 *
	 * The strip shows the permanent tabs, then the preview tab: a double click
	 * on the preview tab keeps it, and the space page offers the same from the
	 * keyboard. Closing the selected tab goes to its neighbour, else the
	 * overview, and focus follows to the selected tab in the bar `strip_id`
	 * names.
	 *
	 * The layout mounts this only while it shows the route's loaded cord
	 * (signed in, found, not failed), so a sign-out or a failed load unmounts
	 * it; the tabs live in the root's `CordSpaceTabs`, and these effects are
	 * idempotent, so a remount picks up where it left off.
	 *
	 * @module
	 */

	import { tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';

	import { cord_spaces_state_context } from './cord_spaces_state.svelte.ts';
	import { cord_space_tabs_context } from './cord_space_tabs.svelte.ts';
	import { to_uuid_param } from './route_params.ts';
	import Tab from './Tab.svelte';

	const {
		cord_id,
		account_id,
		strip_id
	}: {
		/** The loaded cord the route names. */
		cord_id: Uuid;
		/** The signed-in viewer's account, once loaded. */
		account_id: Uuid | undefined;
		/** The id of the `Tabs` bar this renders in, where focus lands after a close. */
		strip_id: string;
	} = $props();

	const spaces_state = cord_spaces_state_context.get();
	const space_tabs = cord_space_tabs_context.get();

	// the space tabs are the viewer's in this cord; `load` is a no-op for the
	// pair it already holds, and reads the store only on a change
	$effect(() => {
		if (account_id) space_tabs.load(account_id, cord_id);
	});

	// tabs for spaces this cord no longer has drop whenever its list loads or changes
	const loaded_spaces = $derived(spaces_state.loaded_spaces(cord_id));
	$effect(() => {
		if (loaded_spaces) {
			space_tabs.retain(
				cord_id,
				loaded_spaces.map((s) => s.id)
			);
		}
	});

	// a space opened by URL (a link, a reload, the rail, the overview) takes
	// the preview tab unless it's permanent; only a space the loaded list has,
	// so a missing one gets no tab, and keyed on the id alone, so a rename
	// doesn't reopen a tab closed meanwhile
	const route_space_id = $derived(to_uuid_param(page.params.space_id));
	const opened_space_id = $derived(
		route_space_id ? spaces_state.get_space(cord_id, route_space_id)?.id : undefined
	);
	$effect(() => {
		if (opened_space_id) space_tabs.open(cord_id, opened_space_id, 'preview');
	});

	// the space tabs in strip order, once the list they name has loaded
	const tab_spaces = $derived.by(() => {
		if (!loaded_spaces || space_tabs.cord_id !== cord_id) return [];
		const by_id = new Map(loaded_spaces.map((s) => [s.id, s]));
		return space_tabs.tab_ids.flatMap((id) => by_id.get(id) ?? []);
	});

	// closing the selected tab goes to its neighbour, else the overview; focus,
	// which the close control took with it, lands on the selected tab
	const close_space_tab = async (space_id: Uuid): Promise<void> => {
		const was_selected = route_space_id === space_id;
		const neighbour = space_tabs.close(cord_id, space_id);
		if (was_selected) {
			await goto(
				neighbour
					? resolve('/c/[cord_id]/s/[space_id]', { cord_id, space_id: neighbour })
					: resolve('/c/[cord_id]', { cord_id })
			);
		}
		await tick();
		document.getElementById(strip_id)?.querySelector<HTMLElement>('.selected a')?.focus();
	};
</script>

{#each tab_spaces as space (space.id)}
	{@const preview = space.id === space_tabs.preview_id}
	<Tab
		href={resolve('/c/[cord_id]/s/[space_id]', { cord_id, space_id: space.id })}
		{preview}
		title={preview
			? `${space.name} (preview; double-click to keep it as a permanent tab)`
			: space.name}
		ondblclick={preview ? () => space_tabs.promote() : undefined}
		onclose={() => close_space_tab(space.id)}
		close_label="close {space.name}"
	>
		{space.name}
	</Tab>
{/each}
