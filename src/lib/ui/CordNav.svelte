<script lang="ts">
	/**
	 * The rail on every page: home, the signed-in account's cords, and the
	 * account, admin, a/v check, about, and log-out links, or for someone
	 * signed out, log in, sign up, a/v check, and about.
	 *
	 * Pages prerender signed out, so until the first session check settles,
	 * and between a login's `verified` and its account loading, the rail
	 * shows only the links both shapes share; the rest arrive with the
	 * session rather than one shape flashing before the other.
	 *
	 * @module
	 */

	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import { cord_auth_state_context } from './cord_auth_state.svelte.ts';
	import { cords_state_context } from './cords_state.svelte.ts';

	const auth_state = cord_auth_state_context.get();
	const cords_state = cords_state_context.get();

	const signed_out = $derived(auth_state.session_checked && !auth_state.verified);

	// selected on its own page and any page under it, as fuz_ui's `MenuLink`
	// selects, except that `'/'` matches only itself (no path starts with `'//'`),
	// which `MenuLink` does with `exact`; TODO consider `MenuLink`
	const is_selected = (href: string): boolean =>
		page.url.pathname === href || page.url.pathname.startsWith(href + '/');
	const to_aria_current = (href: string): 'page' | undefined =>
		page.url.pathname === href ? 'page' : undefined;

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
	{@render menu_link(resolve('/'), 'Cord', 'font_size_lg')}
	{#if auth_state.signed_in}
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
