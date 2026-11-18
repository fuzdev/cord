import type { CordSocketTimers } from '#lib/ui/cord_socket.svelte.ts';

/** A clock and timers that move only when the test calls `advance`. */
export class FakeTimers implements CordSocketTimers {
	#now = 0;
	#next_handle = 1;
	readonly #pending: Map<number, { at: number; callback: () => void }> = new Map();

	now = (): number => this.#now;

	set_timeout = (callback: () => void, ms: number): unknown => {
		const handle = this.#next_handle++;
		this.#pending.set(handle, { at: this.#now + ms, callback });
		return handle;
	};

	clear_timeout = (handle: unknown): void => {
		this.#pending.delete(handle as number);
	};

	/** Timers not yet run or cleared. */
	get pending_count(): number {
		return this.#pending.size;
	}

	/** Move the clock forward `ms`, running each timer that comes due, in order. */
	advance(ms: number): void {
		const end = this.#now + ms;
		for (;;) {
			let due: [number, { at: number; callback: () => void }] | null = null;
			for (const entry of this.#pending) {
				if (entry[1].at <= end && (!due || entry[1].at < due[1].at)) due = entry;
			}
			if (!due) break;
			this.#pending.delete(due[0]);
			this.#now = due[1].at;
			due[1].callback();
		}
		this.#now = end;
	}
}
