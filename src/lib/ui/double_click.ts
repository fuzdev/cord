/**
 * A `mousedown` handler for an element a double click acts on (keeping a
 * space's tab): the second press doesn't select words. Unlike
 * `user-select: none`, this leaves the text selectable.
 *
 * @module
 */

/** Cancels the word selection a double click's second press would start. */
export const prevent_double_click_selection = (e: MouseEvent): void => {
	if (e.detail > 1) e.preventDefault();
};
