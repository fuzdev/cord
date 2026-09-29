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

	// a global admin grant, the same rule the server's admin actions apply
	const is_admin = $derived(auth_state.is_instance_admin);

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

<div class="admin-layout">
	{#if auth_state.signed_in && is_admin}
		<ColumnLayout class="flex:1 overflow:hidden">
			{#snippet aside()}
				<MenuLink path="/admin" class="menuitem font_size_lg">admin</MenuLink>
				<hr />
				<MenuLink path="/admin/invites" class="menuitem">invites</MenuLink>
				<MenuLink path="/admin/accounts" class="menuitem">accounts</MenuLink>
				<MenuLink path="/admin/sessions" class="menuitem">sessions</MenuLink>
				<MenuLink path="/admin/audit" class="menuitem">audit log</MenuLink>
				<MenuLink path="/admin/cords" class="menuitem">cords</MenuLink>
				<hr />
				<MenuLink path="/account" class="menuitem">your account</MenuLink>
				<MenuLink path="/" class="menuitem">home</MenuLink>
			{/snippet}
			{@render children()}
		</ColumnLayout>
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
					No accounts exist yet. Create the first one, which becomes the admin. The bootstrap token
					is in the file the server's <code>FUZ_BOOTSTRAP_TOKEN_PATH</code> names.
				</p>
				<BootstrapForm redirect_on_bootstrap={resolve('/admin')} />
			{:else if auth_state.signed_in && !auth_state.verifying}
				<h1>admin</h1>
				<p>
					You're signed in as <strong>{auth_state.account?.username}</strong>, which isn't an admin
					account.
				</p>
				<p>Back to the <a href={resolve('/')}>home page</a>.</p>
			{:else if auth_state.verified}
				<!-- a login or bootstrap succeeded and the account is still loading -->
				<p class="text_50">checking session...</p>
			{:else}
				<h1>log in</h1>
				<LoginForm username_label="username" redirect_on_login={resolve('/admin')} />
				<p>Back to the <a href={resolve('/')}>home page</a>.</p>
			{/if}
		</main>
	{/if}
</div>

<style>
	.admin-layout {
		display: flex;
		flex-direction: column;
		height: 100%;
		/* lets the nested `ColumnLayout` columns scroll independently */
		min-height: 0;
		overflow: hidden;
	}
</style>
