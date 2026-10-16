<script lang="ts">
	/**
	 * A cord's invite links, for its owners and admins: the mint form, the
	 * link just minted (shown in full once, with a copy button), and the list
	 * of links with their status, uses, expiry, newcomer redemptions, and who
	 * minted each, flagging links whose minter is no longer an owner or admin.
	 *
	 * @module
	 */

	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';
	import CopyToClipboard from '@fuzdev/fuz_ui/CopyToClipboard.svelte';
	import ConfirmButton from '@fuzdev/fuz_app/ui/ConfirmButton.svelte';
	import { format_datetime_local, format_relative_time } from '@fuzdev/fuz_app/ui/ui_format.ts';
	import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';

	import type { CordInviteJson, CordInviteStatus, CordMemberJson } from '../cord_action_specs.ts';
	import CordInviteForm from './CordInviteForm.svelte';
	import {
		type CordInvitesState,
		to_cord_invite_minter,
		to_cord_join_url
	} from './cord_invites_state.svelte.ts';

	const {
		invites_state,
		cord_id,
		members,
		can_admit_newcomers
	}: {
		invites_state: CordInvitesState;
		cord_id: Uuid;
		/**
		 * The cord's members, to name each link's minter and flag former
		 * managers; `null` until the list has loaded for this cord, which
		 * leaves minters unnamed and unflagged rather than wrongly flagged.
		 */
		members: ReadonlyArray<CordMemberJson> | null;
		/** Whether the viewer may mint links that bring someone new onto the instance. */
		can_admit_newcomers: boolean;
	} = $props();

	const STATUS_LABELS: Record<CordInviteStatus, string> = {
		active: 'active',
		revoked: 'revoked',
		expired: 'expired',
		exhausted: 'used up'
	};

	const minted_url = $derived(
		invites_state.minted
			? to_cord_join_url(
					new URL(resolve('/join'), window.location.origin).href,
					invites_state.minted.token
				)
			: null
	);

	const format_uses = (invite: CordInviteJson): string =>
		invite.max_uses === null
			? `${invite.use_count} used, no limit`
			: `${invite.use_count} of ${invite.max_uses} used`;
</script>

<section>
	<h2>invite links</h2>
	<p>
		A link lets someone who already has an account on this instance join this cord as a member,
		while the link is active. Only you and the cord's other owners and admins see these.
	</p>
	<p>
		A link that welcomes someone new also lets a person without an account pick a username and
		password, then join. Only instance admins make those, each with an expiry and a use limit.
	</p>

	{#if invites_state.minted && minted_url}
		<Alert status="inform">
			<p>
				Here's the new link. This is the only time Cord shows it in full, so copy it now; if it's
				lost, you can revoke it and make another.
			</p>
			<div class="row gap_sm">
				<input
					class="flex:1"
					type="text"
					readonly
					value={minted_url}
					aria-label="the new invite link"
					onfocus={(e) => e.currentTarget.select()}
				/>
				<CopyToClipboard text={minted_url} title="copy the link" />
			</div>
			<button type="button" onclick={() => invites_state.dismiss_minted()}>done</button>
		</Alert>
	{/if}

	<h3>make a link</h3>
	<CordInviteForm
		{cord_id}
		{can_admit_newcomers}
		onsubmit={(request) => invites_state.create_invite(request)}
		pending={invites_state.create.loading}
		error={invites_state.create.error}
	/>

	<h3>links</h3>
	{#if invites_state.list.failed && invites_state.invites.length === 0}
		<Alert status="error"><p>Couldn't load the links: {invites_state.list.error}</p></Alert>
	{:else if !invites_state.list.succeeded && invites_state.invites.length === 0}
		<p class="text_50">loading links...</p>
	{:else if invites_state.invites.length === 0}
		<p>No links yet.</p>
	{:else}
		<ul class="unstyled">
			{#each invites_state.invites as invite (invite.id)}
				{@const minter = members && to_cord_invite_minter(invite, members)}
				{@const revoke_error = invites_state.revoke.error(invite.id)}
				<li class="py_xs">
					<div class="row gap_sm flex-wrap:wrap">
						<span class="chip" class:palette_b={invite.status === 'active'}>
							{STATUS_LABELS[invite.status]}
						</span>
						{#if invite.admits_newcomers}
							<span class="chip">welcomes someone new</span>
						{/if}
						<span>{format_uses(invite)}</span>
						<span>
							{#if invite.expires_at === null}
								never expires
							{:else}
								<time datetime={invite.expires_at} title={format_datetime_local(invite.expires_at)}>
									{invite.status === 'expired' ? 'expired' : 'expires'}
									{format_relative_time(invite.expires_at)}
								</time>
							{/if}
						</span>
						{#if invite.status !== 'revoked'}
							<ConfirmButton
								onconfirm={() => invites_state.revoke_invite(invite.id)}
								title="revoke this link"
								class="sized_sm"
								label="revoke"
								pending={invites_state.revoke.loading(invite.id)}
							>
								{#snippet popover_content(_popover, do_confirm)}
									<button
										type="button"
										class="palette_c background-color:var(--paper_100)"
										onclick={() => do_confirm()}
									>
										<span class="py_sm">revoke this link</span>
									</button>
								{/snippet}
							</ConfirmButton>
						{/if}
					</div>
					{#if invite.admits_newcomers}
						<p class="font_size_sm mt_0 mb_0">
							{invite.redemption_count} redeemed by someone new, {invite.redemption_accepted_count}
							of them joined
						</p>
					{/if}
					<p class="font_size_sm text_50 mt_0 mb_0">
						made
						{#if !minter}
							{#if invite.created_by === null}by someone no longer on this instance{/if}
						{:else if minter.kind === 'manager' || minter.kind === 'former_manager'}
							by {minter.member.display_name ?? minter.member.username}
						{:else if minter.kind === 'former_member'}
							by someone no longer in this cord
						{:else}
							by someone no longer on this instance
						{/if}
						<time datetime={invite.created_at} title={format_datetime_local(invite.created_at)}>
							{format_relative_time(invite.created_at)}
						</time>
					</p>
					{#if minter && minter.kind !== 'manager'}
						<p class="font_size_sm mt_0 mb_0">
							Whoever made this link isn't an owner or admin here anymore.
							{#if invite.status === 'active'}
								It still works until it's
								revoked{invite.expires_at === null && invite.max_uses === null
									? ''
									: ' or runs out'}.
							{/if}
						</p>
					{/if}
					{#if revoke_error}
						<p class="color_c_50 font_size_sm mt_0 mb_0">{revoke_error}</p>
					{/if}
				</li>
			{/each}
		</ul>
		{#if invites_state.list.failed}
			<p class="color_c_50 font_size_sm">Couldn't refresh the links: {invites_state.list.error}</p>
		{/if}
	{/if}
</section>
