<script lang="ts">
	/**
	 * The rail on every page: home, the signed-in account's cords, and the
	 * account, admin, a/v check, about, and log-out links, or for someone
	 * signed out, log in, sign up, a/v check, and about.
	 *
	 * Pages prerender signed out, so until the first session check settles,
	 * and between a login's `verified` and its account loading, the rail
	 * shows only the links both shapes share; the rest arrive with the
	 * session rather than one shape flashing before the other. The static
	 * site (`cord_site.ts`) never checks a session, so it keeps that shape,
	 * which links only to its own pages.
	 *
	 * Signed in, a quiet line under the home link says the socket is
	 * reconnecting, once it has been down for `CORD_SOCKET_RECONNECTING_DELAY`;
	 * requests wait for it meanwhile. A socket closed as superseded isn't
	 * trying until the next request reopens it, so it gets no line.
	 *
	 * @module
	 */

	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Svg from '@fuzdev/fuz_ui/Svg.svelte';
	import { site_context } from '@fuzdev/fuz_ui/site.svelte.ts';

	import { cord_auth_state_context } from './cord_auth_state.svelte.ts';
	import { cords_state_context } from './cords_state.svelte.ts';
	import { cord_spaces_state_context } from './cord_spaces_state.svelte.ts';
	import { cord_space_tabs_context } from './cord_space_tabs.svelte.ts';
	import { cord_rpc_context } from './rpc.ts';
	import { CORD_SOCKET_RECONNECTING_DELAY } from './cord_socket.svelte.ts';
	import { prevent_double_click_selection } from './double_click.ts';

	const site = site_context.get();
	const auth_state = cord_auth_state_context.get();
	const cords_state = cords_state_context.get();
	const spaces_state = cord_spaces_state_context.get();
	const space_tabs = cord_space_tabs_context.get();
	const { socket } = cord_rpc_context.get();

	const signed_out = $derived(auth_state.session_checked && !auth_state.verified);

	// selected on its own page and any page under it, as fuz_ui's `MenuLink`
	// selects, except that `'/'` matches only itself (no path starts with `'//'`),
	// which `MenuLink` does with `exact`; TODO consider `MenuLink`
	const is_selected = (href: string): boolean =>
		page.url.pathname === href || page.url.pathname.startsWith(href + '/');
	const to_aria_current = (href: string): 'page' | undefined =>
		page.url.pathname === href ? 'page' : undefined;

	// a blip passes unremarked; a socket down for longer gets a quiet line
	const socket_down = $derived(auth_state.signed_in && !socket.connected && !socket.superseded);
	let reconnecting_shown = $state.raw(false);
	$effect(() => {
		reconnecting_shown = false;
		if (!socket_down) return;
		const timer = setTimeout(() => {
			reconnecting_shown = true;
		}, CORD_SOCKET_RECONNECTING_DELAY);
		return () => clearTimeout(timer);
	});

	let logging_out = $state.raw(false);

	// `logout` runs the root layout's `on_logout` listeners; the signed-in
	// page left behind has nothing to show, so land on home
	const log_out = async (): Promise<void> => {
		if (logging_out) return;
		logging_out = true;
		try {
			await auth_state.logout();
		} finally {
			logging_out = false;
		}
		await goto(resolve('/'));
	};
</script>

<nav class="unstyled" aria-label="site">
	<a
		href={resolve('/')}
		class="menuitem font_size_lg justify-content:flex-start gap_sm"
		class:selected={is_selected(resolve('/'))}
		aria-current={to_aria_current(resolve('/'))}
	>
		{#if site.icon}<Svg data={site.icon} size="var(--icon_size_sm)" label="" inline />{/if}
		Cord
	</a>
	{#if auth_state.signed_in}
		<div class="px_xs text_50 font_size_sm" role="status">
			{#if reconnecting_shown}reconnecting…{/if}
		</div>
		<div class="px_xs pt_sm text_50 font_size_sm">cords</div>
		<ul class="unstyled pl_md">
			{#each cords_state.cords as cord (cord.id)}
				{@const href = resolve('/c/[cord_id]', { cord_id: cord.id })}
				<li>
					<a
						{href}
						class="menuitem"
						class:selected={is_selected(href)}
						aria-current={to_aria_current(href)}
						title={cord.name}
					>
						<span class="ellipsis min-width:0">{cord.name}</span>
					</a>
					{#if is_selected(href)}
						{@const spaces = spaces_state.loaded_spaces(cord.id)}
						{#if spaces?.length}
							<ul class="unstyled pl_md">
								{#each spaces as space (space.id)}
									{@const space_href = resolve('/c/[cord_id]/s/[space_id]', {
										cord_id: cord.id,
										space_id: space.id
									})}
									<li>
										<a
											href={space_href}
											class="menuitem"
											class:selected={is_selected(space_href)}
											aria-current={to_aria_current(space_href)}
											title={space.name}
											onmousedown={prevent_double_click_selection}
											ondblclick={(e) => {
												e.preventDefault();
												space_tabs.open(cord.id, space.id, 'permanent');
											}}
										>
											<span class="ellipsis min-width:0">{space.name}</span>
										</a>
									</li>
								{/each}
							</ul>
						{/if}
					{/if}
				</li>
			{:else}
				<!-- the slot keeps its error through a retry, so the button stays
					mounted, and `aria-disabled` rather than `disabled` keeps it focusable -->
				{#if cords_state.list.error !== null}
					<li>
						<button
							type="button"
							class="plain menuitem text_50"
							title={cords_state.list.error}
							aria-disabled={cords_state.list.loading}
							onclick={() => {
								if (!cords_state.list.loading) void cords_state.fetch();
							}}
						>
							{cords_state.list.loading ? 'retrying…' : "couldn't load, retry"}
						</button>
					</li>
				{:else if cords_state.loaded}
					<li class="px_xs text_50">none yet</li>
				{/if}
			{/each}
		</ul>
		<hr class="nav-separator unstyled" />
		{@render menu_link(resolve('/account'), 'account')}
		{#if auth_state.is_admin}
			{@render menu_link(resolve('/admin'), 'admin')}
		{/if}
		{@render menu_link(resolve('/av'), 'a/v check')}
		{@render menu_link(resolve('/about'), 'about')}
		<button
			type="button"
			class="plain menuitem text_50"
			aria-disabled={logging_out}
			onclick={log_out}
		>
			log out
		</button>
	{:else if signed_out}
		{#if auth_state.needs_bootstrap}
			{@render menu_link(resolve('/admin'), 'claim this instance')}
		{/if}
		{@render menu_link(resolve('/login'), 'log in')}
		{@render menu_link(resolve('/signup'), 'sign up')}
		<hr class="nav-separator unstyled" />
		{@render menu_link(resolve('/av'), 'a/v check')}
		{@render menu_link(resolve('/about'), 'about')}
	{:else}
		{@render menu_link(resolve('/av'), 'a/v check')}
		{@render menu_link(resolve('/about'), 'about')}
	{/if}
</nav>

<!-- every caller passes an `href` from `resolve` -->
<!-- eslint-disable svelte/no-navigation-without-resolve -->
{#snippet menu_link(href: string, label: string, class_name = '')}
	<a
		{href}
		class={['menuitem', class_name]}
		class:selected={is_selected(href)}
		aria-current={to_aria_current(href)}
	>
		{label}
	</a>
{/snippet}

<!-- eslint-enable svelte/no-navigation-without-resolve -->

<style>
	.nav-separator {
		margin: var(--space_md) 0;
		border-top: 1px solid var(--ink_10);
	}
</style>
