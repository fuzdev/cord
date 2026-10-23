<script lang="ts">
	/**
	 * A short tone in the left ear, the right, or both, through the chosen
	 * output, for `MediaCheck`: it confirms the device and that the channels
	 * aren't swapped.
	 *
	 * @module
	 */

	import { onDestroy } from 'svelte';
	import { to_error_message } from '@fuzdev/fuz_util/error.ts';

	import { create_media_output } from './media_output.ts';

	const {
		sink_id
	}: {
		/** the output device to play through, empty for the default */
		sink_id: string;
	} = $props();

	const TONE_HZ = 440;
	const TONE_S = 0.8;
	const SIDES = [
		{ label: 'left', pan: -1 },
		{ label: 'right', pan: 1 },
		{ label: 'both', pan: 0 }
	];

	let playing: string | null = $state.raw(null);
	let error: string | null = $state.raw(null);
	let ctx: AudioContext | null = null;

	const play = async (label: string, pan: number): Promise<void> => {
		void ctx?.close();
		error = null;
		playing = label;
		const next = new AudioContext();
		ctx = next;
		try {
			const output = await create_media_output(next, sink_id);
			const osc = next.createOscillator();
			osc.frequency.value = TONE_HZ;
			const panner = next.createStereoPanner();
			panner.pan.value = pan;
			// a short fade each end, so it doesn't click
			const gain = next.createGain();
			const t = next.currentTime;
			gain.gain.setValueAtTime(0, t);
			gain.gain.linearRampToValueAtTime(0.25, t + 0.03);
			gain.gain.setValueAtTime(0.25, t + TONE_S - 0.05);
			gain.gain.linearRampToValueAtTime(0, t + TONE_S);
			osc.connect(gain).connect(panner).connect(output.node);
			osc.start(t);
			osc.stop(t + TONE_S);
			osc.addEventListener('ended', () => {
				output.close();
				void next.close();
				if (ctx === next) {
					ctx = null;
					playing = null;
				}
			});
		} catch (err) {
			error = to_error_message(err);
			void next.close();
			if (ctx === next) {
				ctx = null;
				playing = null;
			}
		}
	};

	onDestroy(() => void ctx?.close());
</script>

<div class="row gap_sm flex-wrap:wrap">
	<span>test tone</span>
	{#each SIDES as side (side.label)}
		<button
			type="button"
			class:selected={playing === side.label}
			onclick={() => play(side.label, side.pan)}
		>
			{side.label}
		</button>
	{/each}
</div>
{#if error}<p class="color_c_50">{error}</p>{/if}
