<script lang="ts">
	/**
	 * The layout for a cord's pages. It owns the one `CordPageState`, loads
	 * the cord and its members once for every tab, and provides the state
	 * through `cord_page_state_context`. The session gates, the not-found
	 * page (a cord you aren't in reads the same as a missing one), and the
	 * loading and error states render here without the tab pages, each with
	 * its own title, so a page mounts only for the route's loaded cord and
	 * the viewer's role in it. The state's cord is the route's alone, so a
	 * navigation to another cord unmounts the tab page in the same update,
	 * before any fetch for the new cord, and the new cord's page mounts
	 * only once that cord loads: no tab page mounts against a cord the route
	 * has left. The invites and settings tabs show only to the cord's owners
	 * and admins, and their pages guard themselves too, since anyone can
	 * open their URLs.
	 *
	 * The overview tab sits at the section's root, so `Tab`'s descendant
	 * matching would keep it lit on every other tab's page; it takes an
	 * exact-only `selected` from `page.route.id`.
	 *
	 * The cord's spaces load with the cord, into the root layout's
	 * `CordSpacesState` (the rail nests them), and this layout keys the
	 * root's `CordSpaceTabs` to the viewer's account and the route's cord.
	 * After the management tabs, the strip shows the permanent space tabs,
	 * then the preview tab: a space opened by URL or a single click takes
	 * the preview tab unless it's permanent, a double click on the preview
	 * tab keeps it, and the space page offers the same from the keyboard.
	 * Closing the selected tab goes to its neighbour, else the overview, and
	 * focus follows to the selected tab. Tabs of spaces the loaded list
	 * lacks drop whenever it loads.
	 *
	 * @module
	 */

	import { tick, untrack, type Snippet } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';
	import { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';
	import { parse } from '@fuzdev/fuz_schema/schema.ts';

	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';
	import { cord_rpc_context } from '#lib/ui/rpc.ts';
	import { cords_state_context } from '#lib/ui/cords_state.svelte.ts';
	import { CordPageState, cord_page_state_context } from '#lib/ui/cord_page_state.svelte.ts';
	import { cord_spaces_state_context } from '#lib/ui/cord_spaces_state.svelte.ts';
	import { cord_space_tabs_context } from '#lib/ui/cord_space_tabs.svelte.ts';
	import Tabs from '#lib/ui/Tabs.svelte';
	import Tab from '#lib/ui/Tab.svelte';

	const { children }: { children: Snippet } = $props();

	const auth_state = cord_auth_state_context.get();
	const { api } = cord_rpc_context.get();

	// the account's cord list, shared with the rail, follows what these pages learn
	const cords_state = cords_state_context.get();

	// a malformed id can't name a cord, so it reads as not found without a
	// request; `Uuid` lowercases, the way the server answers ids
	const cord_page = cord_page_state_context.set(
		new CordPageState({
			api,
			get_cord_id: () => {
				const parsed = parse(Uuid, page.params.cord_id);
				return parsed.ok ? parsed.value : undefined;
			},
			get_self_actor_id: () => auth_state.actor?.id ?? null
		})
	);
	const { cord_state } = cord_page;

	const account_id = $derived(auth_state.signed_in ? auth_state.account?.id : undefined);
	$effect(() => {
		const { cord_id } = cord_page;
		if (account_id && cord_id) void cord_state.fetch(cord_id);
	});

	// a cord the server doesn't find for you leaves the rail, so a stale entry
	// heals on click (`remove_cord` is a no-op once it's gone)
	$effect(() => {
		const { cord_id } = cord_page;
		if (cord_page.not_found && cord_id) cords_state.remove_cord(cord_id);
	});

	// `fetch_members` reads nothing trackably, so this reruns only when the cord changes
	const loaded_cord_id = $derived(cord_page.cord?.id);
	$effect(() => {
		if (loaded_cord_id) void cord_page.fetch_members(loaded_cord_id);
	});

	// the rail's entry follows the page's view of the cord: its name, loaded
	// or renamed here, and the viewer's role, from the member list once it's
	// loaded; untracked, since `put_cord` reads and writes the list
	$effect(() => {
		const { cord, role } = cord_page;
		if (!cord || !role) return;
		const current = role === cord.role ? cord : { ...cord, role };
		untrack(() => cords_state.put_cord(current));
	});

	// the cord's spaces load with it; `refresh` reads nothing trackably and
	// skips a load already running, as the overview's own refresh may be
	const spaces_state = cord_spaces_state_context.get();
	$effect(() => {
		if (loaded_cord_id) void spaces_state.refresh(loaded_cord_id);
	});

	// the space tabs are the viewer's in this cord; `load` is a no-op for the
	// pair it already holds, and reads the store only on a change
	const space_tabs = cord_space_tabs_context.get();
	$effect(() => {
		if (account_id && loaded_cord_id) space_tabs.load(account_id, loaded_cord_id);
	});

	// tabs for spaces this cord no longer has drop whenever its list loads or changes
	const loaded_spaces = $derived(
		loaded_cord_id ? spaces_state.loaded_spaces(loaded_cord_id) : null
	);
	$effect(() => {
		if (loaded_cord_id && loaded_spaces) {
			space_tabs.retain(
				loaded_cord_id,
				loaded_spaces.map((s) => s.id)
			);
		}
	});

	// a space opened by URL (a link, a reload, the rail, the overview) takes
	// the preview tab unless it's permanent; only a space the loaded list has,
	// so a missing one gets no tab, and keyed on the id alone, so a rename
	// doesn't reopen a tab closed meanwhile
	const route_space_id = $derived.by(() => {
		const parsed = parse(Uuid, page.params.space_id);
		return parsed.ok ? parsed.value : undefined;
	});
	const opened_space_id = $derived(
		loaded_cord_id && route_space_id
			? spaces_state.get_space(loaded_cord_id, route_space_id)?.id
			: undefined
	);
	$effect(() => {
		if (loaded_cord_id && opened_space_id) {
			space_tabs.open(loaded_cord_id, opened_space_id, 'preview');
		}
	});

	// the space tabs in strip order, once the list they name has loaded
	const tab_spaces = $derived.by(() => {
		if (!loaded_spaces || space_tabs.cord_id !== loaded_cord_id) return [];
		const by_id = new Map(loaded_spaces.map((s) => [s.id, s]));
		return space_tabs.tab_ids.flatMap((id) => by_id.get(id) ?? []);
	});

	const strip_id = $props.id();

	// closing the selected tab goes to its neighbour, else the overview; focus,
	// which the close control took with it, lands on the selected tab
	const close_space_tab = async (space_id: Uuid): Promise<void> => {
		const cord_id = loaded_cord_id;
		if (!cord_id) return;
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

	// whether a tab page renders, rather than one of this layout's own states
	const signed_in = $derived(auth_state.session_checked && auth_state.signed_in);
	const signed_in_not_found = $derived(signed_in && cord_page.not_found);
	const tab_page_shown = $derived(
		signed_in &&
			!cord_page.not_found &&
			!cord_page.load_failed &&
			!!cord_page.cord &&
			!!cord_page.role
	);
</script>

<!-- the tab pages title themselves; the root layout's title isn't restored
     when one unmounts, so each of this layout's own states sets one -->
<svelte:head>
	{#if !tab_page_shown}
		<title>{signed_in_not_found ? 'cord not found - Cord' : 'Cord'}</title>
	{/if}
</svelte:head>

<main class="width_atmost_md mx_auto p_lg">
	{#if !auth_state.session_checked}
		<p class="text_50">checking session...</p>
	{:else if !auth_state.signed_in}
		<p>You're not signed in. <a href={resolve('/login')}>Log in</a></p>
	{:else if cord_page.not_found}
		<h1>cord not found</h1>
		<p>There's no cord here, or you're not in it.</p>
	{:else if cord_page.load_failed}
		<Alert status="error"><p>Couldn't load the cord: {cord_state.load.error}</p></Alert>
	{:else if !cord_page.cord}
		<p class="text_50">loading cord...</p>
	{:else if cord_page.role}
		{@const cord_id = cord_page.cord.id}
		<h1>{cord_page.cord.name}</h1>
		<Tabs label="cord pages" id={strip_id}>
			<Tab href={resolve('/c/[cord_id]', { cord_id })} selected={page.route.id === '/c/[cord_id]'}>
				overview
			</Tab>
			<Tab href={resolve('/c/[cord_id]/members', { cord_id })}>members</Tab>
			{#if cord_page.can_manage}
				<Tab href={resolve('/c/[cord_id]/invites', { cord_id })}>invites</Tab>
				<Tab href={resolve('/c/[cord_id]/settings', { cord_id })}>settings</Tab>
			{/if}
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
		</Tabs>
		{@render children()}
	{/if}
</main>
