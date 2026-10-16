<script lang="ts">
	import type { Snippet } from 'svelte';
	import { BROWSER } from 'esm-env';
	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';
	import LoginForm from '@fuzdev/fuz_app/ui/LoginForm.svelte';
	import BootstrapForm from '@fuzdev/fuz_app/ui/BootstrapForm.svelte';
	import ColumnLayout from '@fuzdev/fuz_app/ui/ColumnLayout.svelte';
	import MenuLink from '@fuzdev/fuz_app/ui/MenuLink.svelte';
	import {
		create_admin_rpc_adapters,
		provide_admin_rpc_contexts
	} from '@fuzdev/fuz_app/ui/admin_rpc_adapters.ts';

	import { cord_rpc_context } from '#lib/ui/rpc.ts';
	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';

	const { children }: { children: Snippet } = $props();

	// the root layout provides auth and RPC and checks the session on mount
	const auth_state = cord_auth_state_context.get();
	const { api } = cord_rpc_context.get();

	// `check_session` can't tell a down server from a logged-out 401, so probe
	// `/health` to show a hint instead of a login form that can't work
	let probing = $state.raw(true);
	let server_unreachable = $state.raw(false);
	if (BROWSER) {
		void (async () => {
			try {
				const response = await fetch('/health');
				server_unreachable = !response.ok;
			} catch {
				server_unreachable = true;
			} finally {
				probing = false;
			}
		})();
	}

	// the RPC contexts the `Admin*` components read: accounts, invites, audit log, settings
	provide_admin_rpc_contexts(create_admin_rpc_adapters(api));
</script>

{#if auth_state.signed_in && auth_state.is_admin}
	<div class="admin-layout">
		<ColumnLayout class="flex:1 overflow:hidden" toggle_label="admin">
			{#snippet aside()}
				<nav aria-label="admin">
					<MenuLink path="/admin" class="menuitem font_size_lg">admin</MenuLink>
					<hr class="nav-separator unstyled" />
					<MenuLink path="/admin/invites" class="menuitem">invites</MenuLink>
					<MenuLink path="/admin/accounts" class="menuitem">accounts</MenuLink>
					<MenuLink path="/admin/sessions" class="menuitem">sessions</MenuLink>
					<MenuLink path="/admin/audit" class="menuitem">audit log</MenuLink>
					<MenuLink path="/admin/cords" class="menuitem">cords</MenuLink>
				</nav>
			{/snippet}
			<main>
				{@render children()}
			</main>
		</ColumnLayout>
	</div>
{:else}
	<main class="width_atmost_sm mx_auto p_lg">
		{#if !auth_state.session_checked || probing}
			<p class="text_50">checking session...</p>
		{:else if server_unreachable}
			<Alert status="error">
				<p>The Cord server (<code>cordd</code>) isn't responding. Start it, then reload.</p>
			</Alert>
		{:else if auth_state.needs_bootstrap}
			<h1>claim this instance</h1>
			<p>
				No accounts exist yet. Create the first one, which becomes the admin. The bootstrap token is
				in the file the server's <code>FUZ_BOOTSTRAP_TOKEN_PATH</code> names.
			</p>
			<BootstrapForm redirect_on_bootstrap={resolve('/admin')} />
		{:else if auth_state.signed_in && !auth_state.verifying}
			<h1>admin</h1>
			<p>
				You're signed in as <strong>{auth_state.account?.username}</strong>, which isn't an admin
				account.
			</p>
		{:else if auth_state.verified}
			<!-- a login or bootstrap succeeded and the account is still loading -->
			<p class="text_50">checking session...</p>
		{:else}
			<h1>log in</h1>
			<LoginForm username_label="username" redirect_on_login={resolve('/admin')} />
		{/if}
	</main>
{/if}

<style>
	/* a viewport tall, not `100%` of the shell's content column, which has
	   only a min-height and so wouldn't bound it; with `min-height: 0` and
	   `overflow: hidden`, the nested `ColumnLayout` columns scroll
	   independently inside it, and when it's too narrow and stacks, it
	   scrolls as one column under its sticky toggle */
	.admin-layout {
		display: flex;
		flex-direction: column;
		height: 100dvh;
		min-height: 0;
		overflow: hidden;
	}

	.nav-separator {
		margin: var(--space_md) 0;
		border-top: 1px solid var(--ink_10);
	}
</style>
