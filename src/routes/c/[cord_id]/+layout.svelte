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
	 * @module
	 */

	import { untrack, type Snippet } from 'svelte';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';
	import { ZodUuid } from '@fuzdev/fuz_app/zod_schemas.ts';

	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';
	import { cord_rpc_context } from '#lib/ui/rpc.ts';
	import { cords_state_context } from '#lib/ui/cords_state.svelte.ts';
	import { CordPageState, cord_page_state_context } from '#lib/ui/cord_page_state.svelte.ts';
	import Tabs from '#lib/ui/Tabs.svelte';
	import Tab from '#lib/ui/Tab.svelte';

	const { children }: { children: Snippet } = $props();

	const auth_state = cord_auth_state_context.get();
	const { api } = cord_rpc_context.get();

	// the account's cord list, shared with the rail, follows what these pages learn
	const cords_state = cords_state_context.get();

	// a malformed id can't name a cord, so it reads as not found without a
	// request; ids compare lowercase, the way the server answers them
	const cord_page = cord_page_state_context.set(
		new CordPageState({
			api,
			get_cord_id: () => ZodUuid.safeParse(page.params.cord_id?.toLowerCase()).data,
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

<main class="width_atmost_sm mx_auto p_lg">
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
		<Tabs label="cord pages">
			<Tab href={resolve('/c/[cord_id]', { cord_id })} selected={page.route.id === '/c/[cord_id]'}>
				overview
			</Tab>
			<Tab href={resolve('/c/[cord_id]/members', { cord_id })}>members</Tab>
			{#if cord_page.can_manage}
				<Tab href={resolve('/c/[cord_id]/invites', { cord_id })}>invites</Tab>
				<Tab href={resolve('/c/[cord_id]/settings', { cord_id })}>settings</Tab>
			{/if}
		</Tabs>
		{@render children()}
	{/if}
</main>
