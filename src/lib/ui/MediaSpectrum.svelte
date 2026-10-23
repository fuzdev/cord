<script lang="ts">
	/**
	 * A live frequency view of the mic, for `MediaMic`: hum shows as a spike
	 * at 50 or 60 Hz and its multiples, fans and hiss as a raised floor, and
	 * noise suppression as that floor dropping.
	 *
	 * @module
	 */

	import { onMount } from 'svelte';

	const {
		analyser
	}: {
		analyser: AnalyserNode;
	} = $props();

	const FREQ_MIN = 30;
	const FREQ_MAX = 16_000;
	const DB_MIN = -110;
	const DB_MAX = -20;
	const TICKS: Array<[number, string]> = [
		[50, '50'],
		[100, '100'],
		[200, '200'],
		[500, '500'],
		[1000, '1k'],
		[2000, '2k'],
		[5000, '5k'],
		[10_000, '10k']
	];

	const to_x = (freq: number): number => Math.log(freq / FREQ_MIN) / Math.log(FREQ_MAX / FREQ_MIN);
	const to_freq = (x: number): number => FREQ_MIN * (FREQ_MAX / FREQ_MIN) ** x;
	const format_freq = (freq: number): string =>
		freq >= 1000 ? `${(freq / 1000).toFixed(1)} kHz` : `${Math.round(freq)} Hz`;

	let canvas: HTMLCanvasElement | undefined = $state();
	let hover: { x: number; text: string } | null = $state.raw(null);

	// the latest frame, which the hover reads too
	let data = new Float32Array(0);
	const bin_hz = (): number => analyser.context.sampleRate / analyser.fftSize;

	const on_pointer_move = (e: PointerEvent): void => {
		if (!canvas) return;
		const rect = canvas.getBoundingClientRect();
		const x = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
		const freq = to_freq(x);
		const db = data[Math.round(freq / bin_hz())] ?? -Infinity;
		hover = { x, text: `${format_freq(freq)} · ${Number.isFinite(db) ? db.toFixed(0) : '−∞'} dB` };
	};

	onMount(() => {
		const el = canvas;
		const g = el?.getContext('2d');
		if (!el || !g) return;
		data = new Float32Array(analyser.frequencyBinCount);
		let frame: number;
		const draw = (): void => {
			const ratio = window.devicePixelRatio || 1;
			const width = Math.round(el.clientWidth * ratio);
			const height = Math.round(el.clientHeight * ratio);
			if (el.width !== width || el.height !== height) {
				el.width = width;
				el.height = height;
			}
			analyser.getFloatFrequencyData(data);
			// the line takes the canvas's own text color, set in the styles
			const color = getComputedStyle(el).color;
			g.clearRect(0, 0, width, height);

			g.globalAlpha = 0.2;
			g.fillStyle = color;
			for (const [freq] of TICKS) g.fillRect(Math.round(to_x(freq) * width), 0, ratio, height);
			g.globalAlpha = 1;

			g.beginPath();
			const hz = bin_hz();
			for (let px = 0; px <= width; px += ratio) {
				const db = data[Math.round(to_freq(px / width) / hz)] ?? DB_MIN;
				const y =
					(1 - (Math.max(DB_MIN, Math.min(DB_MAX, db)) - DB_MIN) / (DB_MAX - DB_MIN)) * height;
				if (px === 0) g.moveTo(px, y);
				else g.lineTo(px, y);
			}
			g.strokeStyle = color;
			g.lineWidth = 2 * ratio;
			g.lineJoin = 'round';
			g.stroke();
			frame = requestAnimationFrame(draw);
		};
		frame = requestAnimationFrame(draw);
		return () => cancelAnimationFrame(frame);
	});
</script>

<div class="spectrum">
	<canvas
		bind:this={canvas}
		aria-label="the mic's frequencies, live"
		onpointermove={on_pointer_move}
		onpointerleave={() => (hover = null)}
	></canvas>
	{#if hover}
		<div class="spectrum-hover font_size_xs font_family_mono" style:left="{hover.x * 100}%">
			{hover.text}
		</div>
	{/if}
</div>
<div class="spectrum-scale font_size_xs text_50" aria-hidden="true">
	{#each TICKS as [freq, label] (freq)}
		<span style:left="{to_x(freq) * 100}%">{label}</span>
	{/each}
	<span class="spectrum-unit">Hz</span>
</div>

<style>
	.spectrum {
		position: relative;
	}
	canvas {
		display: block;
		width: 100%;
		height: 120px;
		color: var(--palette_b_60);
		background: var(--ink_05);
		border-radius: var(--border_radius_xs);
		cursor: crosshair;
	}
	.spectrum-hover {
		position: absolute;
		top: var(--space_xs);
		transform: translateX(-50%);
		padding: 0 var(--space_xs);
		background: var(--shade_00);
		border-radius: var(--border_radius_xs);
		pointer-events: none;
		white-space: nowrap;
	}
	.spectrum-scale {
		position: relative;
		height: 1.5em;
	}
	.spectrum-scale span {
		position: absolute;
		transform: translateX(-50%);
	}
	.spectrum-scale .spectrum-unit {
		right: 0;
		transform: none;
	}
</style>
