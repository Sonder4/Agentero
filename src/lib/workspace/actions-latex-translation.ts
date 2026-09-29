import { notifyError } from "@/lib/core/notify";

/**
 * Open a LaTeX-source translation panel to the right of the referenced paper
 * panel. Stub: the full implementation will compile / translate the arXiv LaTeX
 * source and present it in a dual-pane reader.
 */
export function openLatexTranslationTab(
	_paperTabId: string,
	_paperAbsPath: string,
	_filename: string,
): void {
	notifyError("LaTeX source translation is not yet implemented.");
}
