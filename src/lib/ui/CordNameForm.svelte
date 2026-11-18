<script lang="ts">
	/**
	 * A one-field form for a name, shared by creating and renaming a cord
	 * or a space. Checks the name against its schema (`CordName` unless
	 * given another, as `CordSpaceName` for a space) before calling
	 * `onsubmit`, and clears the field after a successful create. While a
	 * submit runs the field is `readonly` and the button `aria-disabled`, so
	 * focus stays put and a failed attempt keeps its input.
	 *
	 * @module
	 */

	import PendingButton from '@fuzdev/fuz_ui/PendingButton.svelte';
	import { parse, type Schema } from '@fuzdev/fuz_schema/schema.ts';

	import { CordName } from '../cord_action_specs.ts';

	const {
		onsubmit,
		submit_label,
		schema = CordName,
		placeholder = 'cord name',
		initial_name = '',
		clear_on_success = false,
		pending = false,
		error = null
	}: {
		/** Called with the trimmed, valid name; resolves to whether it succeeded. */
		onsubmit: (name: string) => Promise<boolean>;
		/** The submit button's label. */
		submit_label: string;
		/** The name's rules, checked before submitting; `CordName` by default. */
		schema?: Schema<string>;
		/** The field's placeholder. */
		placeholder?: string;
		/** The field's starting value. */
		initial_name?: string;
		/** Empty the field after a successful submit. */
		clear_on_success?: boolean;
		/** True while the submitted request runs. */
		pending?: boolean;
		/** The server's error for the last submit, if any. */
		error?: string | null;
	} = $props();

	// svelte-ignore state_referenced_locally
	let name = $state.raw(initial_name);
	let invalid: string | null = $state.raw(null);

	const submit = async (): Promise<void> => {
		if (pending) return;
		const parsed = parse(schema, name);
		if (!parsed.ok) {
			invalid = parsed.issues[0]?.message ?? 'invalid name';
			return;
		}
		invalid = null;
		const ok = await onsubmit(parsed.value);
		if (ok && clear_on_success) name = '';
	};
</script>

<form
	class="width_atmost_sm"
	oninput={() => {
		invalid = null;
	}}
	onsubmit={(e) => {
		e.preventDefault();
		void submit();
	}}
>
	<label>
		<div class="title">name</div>
		<input name="name" type="text" bind:value={name} {placeholder} readonly={pending} />
	</label>
	<div class="row gap_sm">
		<PendingButton {pending} disabled={false} aria-disabled={pending} onclick={submit}>
			{submit_label}
		</PendingButton>
	</div>
	{#if invalid ?? error}
		<p class="color_c_50 font_size_sm mt_xs mb_0">{invalid ?? error}</p>
	{/if}
</form>
