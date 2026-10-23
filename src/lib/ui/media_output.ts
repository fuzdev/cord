/**
 * Plays an `AudioContext` through a chosen output device, for the a/v check.
 *
 * Chromium sets an audio context's output directly; Firefox can't, so there
 * the context feeds a media element that can. The default device always
 * plays straight from the context, at its lowest delay.
 *
 * @module
 */

export interface MediaOutput {
	/** The node to connect sound to. */
	node: AudioNode;
	close: () => void;
}

interface SinkCapable {
	setSinkId: (sink_id: string) => Promise<void>;
}

const has_sink = (value: object): value is SinkCapable => 'setSinkId' in value;

/**
 * Routes `ctx` to the output device `sink_id`, the empty string being the
 * system default.
 *
 * @param ctx - the audio context whose sound should play
 * @param sink_id - an `audiooutput` device id, or the empty string for the default
 * @returns the node to connect sound to, and a close for the routing
 * @throws when the browser refuses the device
 */
export const create_media_output = async (
	ctx: AudioContext,
	sink_id: string
): Promise<MediaOutput> => {
	if (!sink_id) return { node: ctx.destination, close: () => {} };
	// cast, so a lib that already types `setSinkId` doesn't narrow `ctx` away below
	const sinkable: object = ctx;
	if (has_sink(sinkable)) {
		await sinkable.setSinkId(sink_id);
		return { node: ctx.destination, close: () => {} };
	}
	const destination = ctx.createMediaStreamDestination();
	const audio = new Audio();
	audio.srcObject = destination.stream;
	await set_media_sink(audio, sink_id);
	void audio.play();
	return {
		node: destination,
		close: () => {
			audio.pause();
			audio.srcObject = null;
		}
	};
};

/**
 * Sets a media element's output device, where the browser can.
 *
 * @param el - the element to route
 * @param sink_id - an `audiooutput` device id, or the empty string for the default
 * @throws when the browser refuses the device
 */
export const set_media_sink = async (el: HTMLMediaElement, sink_id: string): Promise<void> => {
	const sinkable: object = el;
	if (has_sink(sinkable)) await sinkable.setSinkId(sink_id);
};

/**
 * Whether this browser can send sound to a chosen output device at all.
 */
export const can_choose_output = (): boolean =>
	typeof HTMLMediaElement !== 'undefined' && 'setSinkId' in HTMLMediaElement.prototype;
