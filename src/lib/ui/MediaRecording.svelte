<script lang="ts">
	/**
	 * A short recording of the stream played back through an ordinary media
	 * element, for `MediaCheck`: playback buffers generously, so a clean
	 * recording rules out the mic and points at live playback. It reads one
	 * stream for its lifetime; key it on the stream.
	 *
	 * @module
	 */

	import { onDestroy } from 'svelte';
	import { to_error_message } from '@fuzdev/fuz_util/error.ts';

	import { set_media_sink } from './media_output.ts';

	const {
		stream,
		sink_id
	}: {
		stream: MediaStream;
		/** the output device to play back through, empty for the default */
		sink_id: string;
	} = $props();

	const has_video = $derived(stream.getVideoTracks().length > 0);
	const has_audio = $derived(stream.getAudioTracks().length > 0);

	let audio_only = $state(true);
	let recorder: MediaRecorder | null = $state.raw(null);
	let elapsed_ms = $state.raw(0);
	let clip: { url: string; video: boolean; type: string; size: number; seconds: number } | null =
		$state.raw(null);
	let error: string | null = $state.raw(null);

	let timer: ReturnType<typeof setInterval> | null = null;

	let playback_el: HTMLMediaElement | undefined = $state();
	$effect(() => {
		if (!playback_el) return;
		set_media_sink(playback_el, sink_id).catch((err: unknown) => {
			error = to_error_message(err);
		});
	});

	const pick_type = (video: boolean): string | undefined =>
		(video
			? ['video/webm;codecs=vp8,opus', 'video/webm', 'video/mp4']
			: ['audio/webm;codecs=opus', 'audio/ogg;codecs=opus', 'audio/mp4']
		).find((t) => MediaRecorder.isTypeSupported(t));

	const forget_clip = (): void => {
		if (clip) URL.revokeObjectURL(clip.url);
		clip = null;
	};

	const start = (): void => {
		error = null;
		forget_clip();
		const video = has_video && !(audio_only && has_audio);
		const source = video ? stream : new MediaStream(stream.getAudioTracks());
		const type = pick_type(video);
		let next: MediaRecorder;
		try {
			next = new MediaRecorder(source, type ? { mimeType: type } : undefined);
		} catch (err) {
			error = to_error_message(err);
			return;
		}
		const chunks: Array<Blob> = [];
		const started_at = performance.now();
		next.addEventListener('dataavailable', (e) => chunks.push(e.data));
		next.addEventListener('stop', () => {
			const blob = new Blob(chunks, { type: next.mimeType });
			clip = {
				url: URL.createObjectURL(blob),
				video,
				type: next.mimeType,
				size: blob.size,
				seconds: (performance.now() - started_at) / 1000
			};
		});
		next.start();
		recorder = next;
		elapsed_ms = 0;
		timer = setInterval(() => (elapsed_ms = performance.now() - started_at), 200);
	};

	const stop = (): void => {
		if (timer !== null) clearInterval(timer);
		timer = null;
		if (recorder?.state === 'recording') recorder.stop();
		recorder = null;
	};

	onDestroy(() => {
		stop();
		forget_clip();
	});
</script>

{#if has_video && has_audio}
	<label class="row gap_xs">
		<input type="checkbox" bind:checked={audio_only} disabled={!!recorder} /> audio only
	</label>
{/if}
<div class="row gap_sm">
	{#if recorder}
		<button type="button" onclick={stop}>stop recording</button>
		<span class="font_family_mono">● {(elapsed_ms / 1000).toFixed(1)} s</span>
	{:else}
		<button type="button" onclick={start}>{clip ? 'record again' : 'record'}</button>
	{/if}
</div>
{#if error}<p class="color_c_50">{error}</p>{/if}
{#if clip}
	{#if clip.video}
		<!-- the viewer's own recording, nothing to caption -->
		<!-- svelte-ignore a11y_media_has_caption -->
		<video bind:this={playback_el} src={clip.url} controls playsinline class="playback"></video>
	{:else}
		<audio bind:this={playback_el} src={clip.url} controls class="width:100%"></audio>
	{/if}
	<p class="font_family_mono font_size_sm text_70">
		{clip.seconds.toFixed(1)} s · {(clip.size / 1024).toFixed(0)} KiB · {clip.type}
	</p>
{:else if !recorder}
	<p class="text_50 font_size_sm">
		Record ten seconds or so of talking, then play it back with headphones. Clicks, gaps, or a
		robotic sound in the recording come from the capture side: the mic, its USB link, or the
		system’s audio.
	</p>
{/if}

<style>
	.playback {
		display: block;
		width: 100%;
		max-height: 60vh;
		border-radius: var(--border_radius_xs);
	}
</style>
